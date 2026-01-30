use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;
use pm_protocol::{
    ApprovalId, ApprovalPolicy, EventSeq, ProcessId, SandboxNetworkAccess, SandboxPolicy,
    ThreadEvent, ThreadEventKind, ThreadId, TurnId, TurnStatus,
};
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
            .truncate(false)
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
        let event: ThreadEvent = match serde_json::from_slice(line) {
            Ok(event) => event,
            Err(err) => {
                if bytes.last() != Some(&b'\n') {
                    break;
                }
                return Err(err).context("parse event line from jsonl");
            }
        };
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
    pub cwd: Option<String>,
    pub archived: bool,
    pub archived_at: Option<OffsetDateTime>,
    pub archived_reason: Option<String>,
    pub paused: bool,
    pub paused_at: Option<OffsetDateTime>,
    pub paused_reason: Option<String>,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_policy: SandboxPolicy,
    pub sandbox_writable_roots: Vec<String>,
    pub sandbox_network_access: SandboxNetworkAccess,
    pub mode: String,
    pub openai_provider: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub openai_base_url: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub last_seq: EventSeq,
    pub active_turn_id: Option<TurnId>,
    pub active_turn_interrupt_requested: bool,
    pub last_turn_id: Option<TurnId>,
    pub last_turn_status: Option<TurnStatus>,
    pub last_turn_reason: Option<String>,
    pub pending_approvals: HashSet<ApprovalId>,
    pub running_processes: HashSet<ProcessId>,
    pub failed_processes: HashSet<ProcessId>,
    pub input_tokens_used: u64,
    pub cache_input_tokens_used: u64,
    pub output_tokens_used: u64,
    pub total_tokens_used: u64,
    token_usage_by_response: HashMap<String, SeenTokenUsage>,
}

#[derive(Debug, Clone, Default)]
struct SeenTokenUsage {
    input_tokens: Option<u64>,
    cache_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

impl ThreadState {
    pub fn new(thread_id: ThreadId) -> Self {
        Self {
            thread_id,
            cwd: None,
            archived: false,
            archived_at: None,
            archived_reason: None,
            paused: false,
            paused_at: None,
            paused_reason: None,
            approval_policy: ApprovalPolicy::AutoApprove,
            sandbox_policy: SandboxPolicy::WorkspaceWrite,
            sandbox_writable_roots: Vec::new(),
            sandbox_network_access: SandboxNetworkAccess::Deny,
            mode: "coder".to_string(),
            openai_provider: None,
            model: None,
            thinking: None,
            openai_base_url: None,
            allowed_tools: None,
            last_seq: EventSeq::ZERO,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
            last_turn_id: None,
            last_turn_status: None,
            last_turn_reason: None,
            pending_approvals: HashSet::new(),
            running_processes: HashSet::new(),
            failed_processes: HashSet::new(),
            input_tokens_used: 0,
            cache_input_tokens_used: 0,
            output_tokens_used: 0,
            total_tokens_used: 0,
            token_usage_by_response: HashMap::new(),
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
            ThreadEventKind::ThreadCreated { cwd } => {
                self.cwd = Some(cwd.clone());
            }
            ThreadEventKind::ThreadArchived { reason } => {
                self.archived = true;
                self.archived_at = Some(event.timestamp);
                self.archived_reason = reason.clone();
            }
            ThreadEventKind::ThreadUnarchived { reason: _ } => {
                self.archived = false;
                self.archived_at = None;
                self.archived_reason = None;
            }
            ThreadEventKind::ThreadPaused { reason } => {
                self.paused = true;
                self.paused_at = Some(event.timestamp);
                self.paused_reason = reason.clone();
            }
            ThreadEventKind::ThreadUnpaused { reason: _ } => {
                self.paused = false;
                self.paused_at = None;
                self.paused_reason = None;
            }
            ThreadEventKind::ThreadConfigUpdated {
                approval_policy,
                sandbox_policy,
                sandbox_writable_roots,
                sandbox_network_access,
                mode,
                openai_provider,
                model,
                thinking,
                openai_base_url,
                allowed_tools,
            } => {
                self.approval_policy = *approval_policy;
                if let Some(policy) = sandbox_policy {
                    self.sandbox_policy = *policy;
                }
                if let Some(roots) = sandbox_writable_roots {
                    self.sandbox_writable_roots = roots.clone();
                }
                if let Some(access) = sandbox_network_access {
                    self.sandbox_network_access = *access;
                }
                if let Some(mode) = mode {
                    self.mode = mode.clone();
                }
                if let Some(provider) = openai_provider {
                    self.openai_provider = Some(provider.clone());
                }
                if let Some(model) = model {
                    self.model = Some(model.clone());
                }
                if let Some(thinking) = thinking {
                    self.thinking = Some(thinking.clone());
                }
                if let Some(openai_base_url) = openai_base_url {
                    self.openai_base_url = Some(openai_base_url.clone());
                }
                if let Some(allowed_tools) = allowed_tools {
                    self.allowed_tools = allowed_tools.clone();
                }
            }
            ThreadEventKind::TurnStarted { turn_id, .. } => {
                if self.active_turn_id.is_some() {
                    anyhow::bail!("turn started while another turn is active");
                }
                self.active_turn_id = Some(*turn_id);
                self.active_turn_interrupt_requested = false;
                self.failed_processes.clear();
            }
            ThreadEventKind::TurnInterruptRequested { turn_id, .. } => {
                if self.active_turn_id != Some(*turn_id) {
                    anyhow::bail!("interrupt requested for non-active turn");
                }
                self.active_turn_interrupt_requested = true;
            }
            ThreadEventKind::TurnCompleted {
                turn_id,
                status,
                reason,
            } => {
                if self.active_turn_id != Some(*turn_id) {
                    anyhow::bail!("turn completed for non-active turn");
                }
                self.active_turn_id = None;
                self.active_turn_interrupt_requested = false;
                self.last_turn_id = Some(*turn_id);
                self.last_turn_status = Some(*status);
                self.last_turn_reason = reason.clone();
            }
            ThreadEventKind::ApprovalRequested { approval_id, .. } => {
                self.pending_approvals.insert(*approval_id);
            }
            ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                self.pending_approvals.remove(approval_id);
            }
            ThreadEventKind::ProcessStarted { process_id, .. } => {
                self.running_processes.insert(*process_id);
                self.failed_processes.remove(process_id);
            }
            ThreadEventKind::ProcessExited {
                process_id,
                exit_code,
                ..
            } => {
                self.running_processes.remove(process_id);
                match exit_code {
                    Some(code) if *code != 0 => {
                        self.failed_processes.insert(*process_id);
                    }
                    _ => {
                        self.failed_processes.remove(process_id);
                    }
                }
            }
            ThreadEventKind::AgentStep {
                response_id,
                token_usage,
                ..
            } => {
                self.record_token_usage(Some(response_id.as_str()), token_usage.as_ref());
            }
            ThreadEventKind::AssistantMessage {
                response_id,
                token_usage,
                ..
            } => {
                self.record_token_usage(response_id.as_deref(), token_usage.as_ref());
            }
            _ => {}
        }

        self.last_seq = event.seq;
        Ok(())
    }

    fn record_token_usage(&mut self, response_id: Option<&str>, usage: Option<&serde_json::Value>) {
        let Some(usage) = usage else {
            return;
        };

        let input_tokens = usage_input_tokens(usage);
        let cache_input_tokens = usage_cache_input_tokens(usage);
        let output_tokens = usage_output_tokens(usage);
        let total_tokens = usage_total_tokens(usage);
        let has_numeric_usage = input_tokens.is_some()
            || cache_input_tokens.is_some()
            || output_tokens.is_some()
            || total_tokens.is_some();
        if !has_numeric_usage {
            return;
        }

        fn apply_delta(slot: &mut Option<u64>, next: Option<u64>) -> u64 {
            let Some(next) = next else {
                return 0;
            };
            match *slot {
                None => {
                    *slot = Some(next);
                    next
                }
                Some(prev) if next > prev => {
                    *slot = Some(next);
                    next - prev
                }
                _ => 0,
            }
        }

        if let Some(response_id) = response_id.filter(|value| !value.trim().is_empty()) {
            let entry = self
                .token_usage_by_response
                .entry(response_id.to_string())
                .or_default();

            self.input_tokens_used = self
                .input_tokens_used
                .saturating_add(apply_delta(&mut entry.input_tokens, input_tokens));
            self.cache_input_tokens_used = self.cache_input_tokens_used.saturating_add(
                apply_delta(&mut entry.cache_input_tokens, cache_input_tokens),
            );
            self.output_tokens_used = self
                .output_tokens_used
                .saturating_add(apply_delta(&mut entry.output_tokens, output_tokens));
            self.total_tokens_used = self
                .total_tokens_used
                .saturating_add(apply_delta(&mut entry.total_tokens, total_tokens));
            return;
        }

        if let Some(tokens) = input_tokens {
            self.input_tokens_used = self.input_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = cache_input_tokens {
            self.cache_input_tokens_used = self.cache_input_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = output_tokens {
            self.output_tokens_used = self.output_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = total_tokens {
            self.total_tokens_used = self.total_tokens_used.saturating_add(tokens);
        }
    }
}

fn usage_total_tokens(usage: &serde_json::Value) -> Option<u64> {
    let total_tokens = usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64);
    let input_tokens = usage
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64);

    total_tokens.or_else(|| match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => input.checked_add(output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    })
}

fn usage_input_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(serde_json::Value::as_u64)
}

fn usage_output_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(serde_json::Value::as_u64)
}

fn usage_cache_input_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("cache_input_tokens")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(serde_json::Value::as_u64)
        })
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
                .and_then(serde_json::Value::as_u64)
        })
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
                reason: None,
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

    #[test]
    fn turn_started_clears_failed_processes() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);

        fn apply(
            state: &mut ThreadState,
            thread_id: ThreadId,
            seq: &mut u64,
            kind: ThreadEventKind,
        ) -> anyhow::Result<()> {
            let event = ThreadEvent {
                seq: EventSeq(*seq),
                timestamp: OffsetDateTime::now_utc(),
                thread_id,
                kind,
            };
            *seq += 1;
            state.apply(&event)?;
            Ok(())
        }

        let process_id = ProcessId::new();
        let mut seq = 1u64;

        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadCreated {
                cwd: "/tmp".to_string(),
            },
        )?;
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                argv: vec!["false".to_string()],
                cwd: "/tmp".to_string(),
                stdout_path: "stdout.log".to_string(),
                stderr_path: "stderr.log".to_string(),
            },
        )?;
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ProcessExited {
                process_id,
                exit_code: Some(1),
                reason: None,
            },
        )?;
        assert!(state.failed_processes.contains(&process_id));

        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::TurnStarted {
                turn_id: TurnId::new(),
                input: "hello".to_string(),
                context_refs: None,
                attachments: None,
                priority: pm_protocol::TurnPriority::Foreground,
            },
        )?;
        assert!(state.failed_processes.is_empty());

        Ok(())
    }
}
