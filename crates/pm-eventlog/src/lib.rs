use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;
use pm_protocol::{EventSeq, ThreadEvent, ThreadEventKind, ThreadId, TurnId};
use time::OffsetDateTime;
use tokio::io::AsyncWriteExt;

pub struct EventLogWriter {
    thread_id: ThreadId,
    log_path: PathBuf,
    _lock_file: std::fs::File,
    file: tokio::fs::File,
    next_seq: u64,
}

impl EventLogWriter {
    pub async fn open(thread_id: ThreadId, log_path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create dir {}", parent.display()))?;
        }

        let lock_path = lock_path_for(&log_path);
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("open lock file {}", lock_path.display()))?;
        lock_file
            .lock_exclusive()
            .with_context(|| format!("lock {}", lock_path.display()))?;

        let last_seq = sanitize_and_get_last_seq(thread_id, &log_path).await?;
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .with_context(|| format!("open {}", log_path.display()))?;

        Ok(Self {
            thread_id,
            log_path,
            _lock_file: lock_file,
            file,
            next_seq: last_seq.0 + 1,
        })
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    pub fn next_seq(&self) -> EventSeq {
        EventSeq(self.next_seq)
    }

    pub async fn append(&mut self, kind: ThreadEventKind) -> anyhow::Result<ThreadEvent> {
        let seq = EventSeq(self.next_seq);
        let event = ThreadEvent {
            seq,
            timestamp: OffsetDateTime::now_utc(),
            thread_id: self.thread_id,
            kind,
        };

        let line = serde_json::to_vec(&event).context("serialize event")?;
        self.file
            .write_all(&line)
            .await
            .context("write event line")?;
        self.file.write_all(b"\n").await.context("write newline")?;
        self.file.flush().await.context("flush event log")?;

        self.next_seq += 1;
        Ok(event)
    }
}

pub async fn read_events_since(
    expected_thread_id: ThreadId,
    log_path: &Path,
    since_seq: EventSeq,
) -> anyhow::Result<Vec<ThreadEvent>> {
    let bytes = match tokio::fs::read(log_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read {}", log_path.display())),
    };

    let mut out = Vec::new();
    let mut expected_next = EventSeq(1);
    for line in bytes.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let event: ThreadEvent =
            serde_json::from_slice(line).context("parse event line from jsonl")?;
        if event.thread_id != expected_thread_id {
            anyhow::bail!(
                "event log thread_id mismatch: expected {}, got {}",
                expected_thread_id,
                event.thread_id
            );
        }
        if event.seq != expected_next {
            anyhow::bail!(
                "event log seq is not contiguous: expected {}, got {}",
                expected_next,
                event.seq
            );
        }
        expected_next = EventSeq(event.seq.0 + 1);

        if event.seq.0 > since_seq.0 {
            out.push(event);
        }
    }
    Ok(out)
}

fn lock_path_for(log_path: &Path) -> PathBuf {
    let mut lock_path = log_path.to_path_buf();
    lock_path.set_extension(format!(
        "{}.lock",
        log_path.extension().and_then(|s| s.to_str()).unwrap_or("")
    ));
    lock_path
}

async fn sanitize_and_get_last_seq(
    expected_thread_id: ThreadId,
    log_path: &Path,
) -> anyhow::Result<EventSeq> {
    let bytes = match tokio::fs::read(log_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(EventSeq::ZERO),
        Err(err) => return Err(err).with_context(|| format!("read {}", log_path.display())),
    };

    let mut last_seq = EventSeq::ZERO;
    let mut expected_next = EventSeq(1);
    let mut last_good_len = bytes.len();

    for (idx, line) in bytes.split(|b| *b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<ThreadEvent>(line) {
            Ok(event) => {
                if event.thread_id != expected_thread_id {
                    anyhow::bail!(
                        "event log thread_id mismatch at line {}: expected {}, got {}",
                        idx + 1,
                        expected_thread_id,
                        event.thread_id
                    );
                }
                if event.seq != expected_next {
                    anyhow::bail!(
                        "event log seq is not contiguous at line {}: expected {}, got {}",
                        idx + 1,
                        expected_next,
                        event.seq
                    );
                }
                expected_next = EventSeq(event.seq.0 + 1);
                last_seq = event.seq;
            }
            Err(err) => {
                if bytes.last() != Some(&b'\n') {
                    last_good_len = bytes.len() - line.len();
                    break;
                }
                return Err(err).context("parse event line from jsonl");
            }
        }
    }

    if last_good_len != bytes.len() {
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(log_path)
            .await
            .with_context(|| format!("open {}", log_path.display()))?;
        file.set_len(last_good_len as u64)
            .await
            .with_context(|| format!("truncate {}", log_path.display()))?;
    }

    Ok(last_seq)
}

#[derive(Debug, Clone)]
pub struct ThreadState {
    pub thread_id: ThreadId,
    pub last_seq: EventSeq,
    pub active_turn_id: Option<TurnId>,
    pub active_turn_interrupt_requested: bool,
}

impl ThreadState {
    pub fn new(thread_id: ThreadId) -> Self {
        Self {
            thread_id,
            last_seq: EventSeq::ZERO,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
        }
    }

    pub fn apply(&mut self, event: &ThreadEvent) -> anyhow::Result<()> {
        if event.thread_id != self.thread_id {
            anyhow::bail!(
                "thread_id mismatch: expected {}, got {}",
                self.thread_id,
                event.thread_id
            );
        }
        if event.seq.0 != self.last_seq.0 + 1 {
            anyhow::bail!(
                "non-contiguous seq: expected {}, got {}",
                self.last_seq.0 + 1,
                event.seq.0
            );
        }

        match &event.kind {
            ThreadEventKind::ThreadCreated { .. } => {}
            ThreadEventKind::TurnStarted { turn_id, .. } => {
                if self.active_turn_id.is_some() {
                    anyhow::bail!("turn started while another turn is active");
                }
                self.active_turn_id = Some(*turn_id);
                self.active_turn_interrupt_requested = false;
            }
            ThreadEventKind::TurnInterruptRequested { turn_id, .. } => {
                if self.active_turn_id != Some(*turn_id) {
                    anyhow::bail!("interrupt requested for non-active turn");
                }
                self.active_turn_interrupt_requested = true;
            }
            ThreadEventKind::TurnCompleted { turn_id, .. } => {
                if self.active_turn_id != Some(*turn_id) {
                    anyhow::bail!("turn completed for non-active turn");
                }
                self.active_turn_id = None;
                self.active_turn_interrupt_requested = false;
            }
        }

        self.last_seq = event.seq;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::{ThreadEventKind, ThreadId};

    #[tokio::test]
    async fn writer_appends_with_contiguous_seq() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let log_path = dir.path().join("events.jsonl");
        let thread_id = ThreadId::new();

        let mut w = EventLogWriter::open(thread_id, log_path.clone()).await?;
        let e1 = w
            .append(ThreadEventKind::ThreadCreated {
                cwd: "/tmp".to_string(),
            })
            .await?;
        let e2 = w
            .append(ThreadEventKind::TurnCompleted {
                turn_id: pm_protocol::TurnId::new(),
                status: pm_protocol::TurnStatus::Completed,
            })
            .await?;
        drop(w);

        assert_eq!(e1.seq.0, 1);
        assert_eq!(e2.seq.0, 2);

        let events = read_events_since(thread_id, &log_path, EventSeq::ZERO).await?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq.0, 1);
        assert_eq!(events[1].seq.0, 2);
        Ok(())
    }

    #[tokio::test]
    async fn writer_resumes_after_restart() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let log_path = dir.path().join("events.jsonl");
        let thread_id = ThreadId::new();

        let mut w = EventLogWriter::open(thread_id, log_path.clone()).await?;
        w.append(ThreadEventKind::ThreadCreated {
            cwd: "/tmp".to_string(),
        })
        .await?;
        drop(w);

        let mut w = EventLogWriter::open(thread_id, log_path.clone()).await?;
        let e2 = w
            .append(ThreadEventKind::ThreadCreated {
                cwd: "/tmp2".to_string(),
            })
            .await?;
        assert_eq!(e2.seq.0, 2);
        Ok(())
    }

    #[tokio::test]
    async fn writer_truncates_incomplete_trailing_line() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let log_path = dir.path().join("events.jsonl");
        let thread_id = ThreadId::new();

        let contents = format!(
            "{{\"seq\":1,\"timestamp\":\"2026-01-20T00:00:00Z\",\"thread_id\":\"{}\",\"type\":\"thread_created\",\"cwd\":\"/tmp\"}}\n{{\"seq\":2",
            thread_id
        );
        tokio::fs::write(&log_path, contents).await?;

        let mut w = EventLogWriter::open(thread_id, log_path.clone()).await?;
        let e = w
            .append(ThreadEventKind::ThreadCreated {
                cwd: "/ok".to_string(),
            })
            .await?;
        assert_eq!(e.seq.0, 2);
        Ok(())
    }
}
