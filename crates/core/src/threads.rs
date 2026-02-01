use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use omne_agent_eventlog::{
    EventLogWriter, ThreadState, read_events_since as read_events_since_jsonl,
};
use omne_agent_protocol::{
    EventSeq, ProcessId, ThreadEvent, ThreadEventKind, ThreadId, TurnStatus,
};

use crate::AgentPaths;

const EVENTS_LOG_FILE_NAME: &str = "events.jsonl";

#[derive(Clone, Debug)]
pub struct ThreadStore {
    paths: AgentPaths,
}

impl ThreadStore {
    pub fn new(paths: AgentPaths) -> Self {
        Self { paths }
    }

    pub fn thread_dir(&self, thread_id: ThreadId) -> PathBuf {
        self.paths.thread_dir(thread_id)
    }

    pub fn events_log_path(&self, thread_id: ThreadId) -> PathBuf {
        self.thread_dir(thread_id).join(EVENTS_LOG_FILE_NAME)
    }

    pub async fn list_threads(&self) -> anyhow::Result<Vec<ThreadId>> {
        let dir = self.paths.threads_dir();
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(read_dir) => read_dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
        };

        let mut ids = Vec::new();
        while let Some(entry) = read_dir.next_entry().await? {
            let ty = entry.file_type().await?;
            if !ty.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Ok(id) = name.parse::<ThreadId>() else {
                continue;
            };
            ids.push(id);
        }
        ids.sort_unstable();
        Ok(ids)
    }

    pub async fn create_thread(&self, cwd: PathBuf) -> anyhow::Result<ThreadHandle> {
        let thread_id = ThreadId::new();
        let log_path = self.events_log_path(thread_id);

        let mut handle = ThreadHandle::open_new(thread_id, log_path).await?;
        handle
            .append(ThreadEventKind::ThreadCreated {
                cwd: cwd.display().to_string(),
            })
            .await?;
        Ok(handle)
    }

    pub async fn resume_thread(&self, thread_id: ThreadId) -> anyhow::Result<Option<ThreadHandle>> {
        let dir = self.thread_dir(thread_id);
        match tokio::fs::metadata(&dir).await {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => anyhow::bail!("thread dir is not a directory: {}", dir.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("stat {}", dir.display())),
        }

        let log_path = self.events_log_path(thread_id);
        let mut handle = ThreadHandle::open_existing(thread_id, log_path).await?;

        if let Some(turn_id) = handle.state.active_turn_id {
            let status = if handle.state.active_turn_interrupt_requested {
                TurnStatus::Interrupted
            } else {
                TurnStatus::Failed
            };
            handle
                .append(ThreadEventKind::TurnCompleted {
                    turn_id,
                    status,
                    reason: Some("recovered incomplete turn on resume".to_string()),
                })
                .await?;
        }

        let events = handle.events_since(EventSeq::ZERO).await?;
        let mut active_processes = HashSet::<ProcessId>::new();
        let mut active_tools = HashSet::<omne_agent_protocol::ToolId>::new();
        for event in events {
            match event.kind {
                ThreadEventKind::ProcessStarted { process_id, .. } => {
                    active_processes.insert(process_id);
                }
                ThreadEventKind::ProcessExited { process_id, .. } => {
                    active_processes.remove(&process_id);
                }
                ThreadEventKind::ToolStarted { tool_id, .. } => {
                    active_tools.insert(tool_id);
                }
                ThreadEventKind::ToolCompleted { tool_id, .. } => {
                    active_tools.remove(&tool_id);
                }
                _ => {}
            }
        }
        for process_id in active_processes {
            handle
                .append(ThreadEventKind::ProcessExited {
                    process_id,
                    exit_code: None,
                    reason: Some("recovered incomplete process on resume".to_string()),
                })
                .await?;
        }
        for tool_id in active_tools {
            handle
                .append(ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Cancelled,
                    error: Some("recovered incomplete tool on resume".to_string()),
                    result: None,
                })
                .await?;
        }

        Ok(Some(handle))
    }

    pub async fn read_events_since(
        &self,
        thread_id: ThreadId,
        since_seq: EventSeq,
    ) -> anyhow::Result<Option<Vec<ThreadEvent>>> {
        let dir = self.thread_dir(thread_id);
        match tokio::fs::metadata(&dir).await {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => anyhow::bail!("thread dir is not a directory: {}", dir.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).with_context(|| format!("stat {}", dir.display())),
        }

        let log_path = self.events_log_path(thread_id);
        Ok(Some(
            read_events_since_jsonl(thread_id, &log_path, since_seq).await?,
        ))
    }

    pub async fn read_state(&self, thread_id: ThreadId) -> anyhow::Result<Option<ThreadState>> {
        let Some(events) = self.read_events_since(thread_id, EventSeq::ZERO).await? else {
            return Ok(None);
        };

        let mut state = ThreadState::new(thread_id);
        for event in &events {
            state.apply(event)?;
        }
        Ok(Some(state))
    }
}

pub struct ThreadHandle {
    thread_id: ThreadId,
    writer: EventLogWriter,
    state: ThreadState,
}

impl ThreadHandle {
    async fn open_new(thread_id: ThreadId, log_path: PathBuf) -> anyhow::Result<Self> {
        let writer = EventLogWriter::open(thread_id, log_path).await?;
        Ok(Self {
            thread_id,
            writer,
            state: ThreadState::new(thread_id),
        })
    }

    async fn open_existing(thread_id: ThreadId, log_path: PathBuf) -> anyhow::Result<Self> {
        let writer = EventLogWriter::open(thread_id, log_path).await?;
        let events = read_events_since_jsonl(thread_id, writer.log_path(), EventSeq::ZERO).await?;

        let mut state = ThreadState::new(thread_id);
        for event in &events {
            state.apply(event)?;
        }

        Ok(Self {
            thread_id,
            writer,
            state,
        })
    }

    pub fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    pub fn log_path(&self) -> &Path {
        self.writer.log_path()
    }

    pub fn state(&self) -> &ThreadState {
        &self.state
    }

    pub fn last_seq(&self) -> EventSeq {
        self.state.last_seq
    }

    pub async fn append(&mut self, kind: ThreadEventKind) -> anyhow::Result<ThreadEvent> {
        let mut kind = kind;
        crate::redaction::redact_thread_event_kind(&mut kind);

        let event = self.writer.append(kind).await?;
        self.state.apply(&event)?;
        Ok(event)
    }

    pub async fn events_since(&self, since_seq: EventSeq) -> anyhow::Result<Vec<ThreadEvent>> {
        read_events_since_jsonl(self.thread_id, self.log_path(), since_seq).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn append_redacts_sensitive_tokens() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = ThreadStore::new(AgentPaths::new(dir.path().join(".omne_agent_data")));

        let mut thread = store.create_thread(PathBuf::from("/tmp")).await?;
        let thread_id = thread.thread_id();
        let turn_id = omne_agent_protocol::TurnId::new();
        thread
            .append(ThreadEventKind::TurnStarted {
                turn_id,
                input: "token=sk-1234567890abcdefghijklmnop".to_string(),
                context_refs: None,
                attachments: None,
                priority: omne_agent_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(thread);

        let resumed = store
            .resume_thread(thread_id)
            .await?
            .expect("thread exists");
        let events = resumed.events_since(EventSeq::ZERO).await?;
        let started = events
            .iter()
            .find(|e| matches!(e.kind, ThreadEventKind::TurnStarted { .. }))
            .expect("turn started");
        let ThreadEventKind::TurnStarted { input, .. } = &started.kind else {
            unreachable!();
        };
        assert!(!input.contains("sk-1234567890abcdefghijklmnop"));
        assert!(input.contains("sk-<REDACTED>"));
        Ok(())
    }

    #[tokio::test]
    async fn resume_repairs_incomplete_turn() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = ThreadStore::new(AgentPaths::new(dir.path().join(".omne_agent_data")));

        let mut thread = store.create_thread(PathBuf::from("/tmp")).await?;
        let thread_id = thread.thread_id();
        let turn_id = omne_agent_protocol::TurnId::new();
        thread
            .append(ThreadEventKind::TurnStarted {
                turn_id,
                input: "x".to_string(),
                context_refs: None,
                attachments: None,
                priority: omne_agent_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(thread);

        let resumed = store
            .resume_thread(thread_id)
            .await?
            .expect("thread exists");
        assert!(resumed.state().active_turn_id.is_none());

        let events = resumed.events_since(EventSeq::ZERO).await?;
        let last = events.last().expect("events");
        assert!(matches!(
            &last.kind,
            ThreadEventKind::TurnCompleted {
                turn_id: got,
                status: TurnStatus::Failed,
                reason: Some(_),
            } if *got == turn_id
        ));
        Ok(())
    }
}
