use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use omne_eventlog::{EventLogWriter, ThreadState, read_events_since as read_events_since_jsonl};
use omne_protocol::{
    EventSeq, ProcessId, ThreadEvent, ThreadEventKind, ThreadId, ToolId, TurnStatus,
};

use crate::PmPaths;

const EVENTS_LOG_FILE_NAME: &str = "events.jsonl";
const READABLE_HISTORY_FILE_NAME: &str = "readable_history.jsonl";

async fn ensure_readable_history_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        if let Err(err) = tokio::fs::set_permissions(path, perm).await {
            tracing::debug!(
                path = %path.display(),
                error = %err,
                "failed to tighten readable history permissions"
            );
        }
    }
}

fn readable_history_path_from_log_path(log_path: &Path) -> PathBuf {
    let thread_dir = log_path.parent().unwrap_or(log_path);
    thread_dir.join(READABLE_HISTORY_FILE_NAME)
}

async fn append_readable_history_event(log_path: &Path, event: &ThreadEvent) -> anyhow::Result<()> {
    let timestamp = event
        .timestamp
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| event.timestamp.to_string());
    let record = match &event.kind {
        ThreadEventKind::TurnStarted { turn_id, input, .. } => serde_json::json!({
            "seq": event.seq,
            "timestamp": timestamp,
            "turn_id": turn_id,
            "role": "user",
            "text": input,
        }),
        ThreadEventKind::AssistantMessage {
            turn_id,
            text,
            model,
            response_id,
            token_usage,
        } => serde_json::json!({
            "seq": event.seq,
            "timestamp": timestamp,
            "turn_id": turn_id,
            "role": "assistant",
            "text": text,
            "model": model,
            "response_id": response_id,
            "token_usage": token_usage,
        }),
        _ => return Ok(()),
    };

    let record = serde_json::to_string(&record).context("serialize readable history record")?;
    let path = readable_history_path_from_log_path(log_path);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    ensure_readable_history_file_permissions(&path).await;

    use tokio::io::AsyncWriteExt;
    file.write_all(record.as_bytes()).await?;
    file.write_all(b"\n").await?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ThreadStore {
    paths: PmPaths,
}

impl ThreadStore {
    pub fn new(paths: PmPaths) -> Self {
        Self { paths }
    }

    pub fn root(&self) -> &Path {
        self.paths.root()
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
        let cwd = normalize_thread_cwd(&cwd).await?;

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

        for tool_id in handle.active_tools.clone() {
            handle
                .append(ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Cancelled,
                    structured_error: None,
                    error: Some("recovered incomplete tool on resume".to_string()),
                    result: None,
                })
                .await?;
        }
        let recovered_processes = handle.recover_incomplete_processes();
        if !recovered_processes.is_empty() {
            tracing::warn!(
                thread_id = %thread_id,
                process_count = recovered_processes.len(),
                process_ids = ?recovered_processes,
                "recovered incomplete processes on resume without synthesizing ProcessExited"
            );
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

async fn normalize_thread_cwd(cwd: &Path) -> anyhow::Result<PathBuf> {
    tokio::fs::canonicalize(cwd)
        .await
        .with_context(|| format!("canonicalize thread cwd {}", cwd.display()))
}

pub struct ThreadHandle {
    thread_id: ThreadId,
    writer: EventLogWriter,
    state: ThreadState,
    active_processes: HashSet<ProcessId>,
    active_tools: HashSet<ToolId>,
}

impl ThreadHandle {
    async fn open_new(thread_id: ThreadId, log_path: PathBuf) -> anyhow::Result<Self> {
        let writer = EventLogWriter::open(thread_id, log_path).await?;
        Ok(Self {
            thread_id,
            writer,
            state: ThreadState::new(thread_id),
            active_processes: HashSet::new(),
            active_tools: HashSet::new(),
        })
    }

    async fn open_existing(thread_id: ThreadId, log_path: PathBuf) -> anyhow::Result<Self> {
        let (writer, events) = EventLogWriter::open_with_events(thread_id, log_path).await?;

        let mut state = ThreadState::new(thread_id);
        let mut active_processes = HashSet::<ProcessId>::new();
        let mut active_tools = HashSet::<ToolId>::new();
        for event in &events {
            state.apply(event)?;
            apply_runtime_event_tracking(&mut active_processes, &mut active_tools, &event.kind);
        }

        Ok(Self {
            thread_id,
            writer,
            state,
            active_processes,
            active_tools,
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
        apply_runtime_event_tracking(
            &mut self.active_processes,
            &mut self.active_tools,
            &event.kind,
        );

        if let Err(err) = append_readable_history_event(self.writer.log_path(), &event).await {
            tracing::debug!(
                thread_id = %self.thread_id,
                error = %err,
                "failed to append readable history record"
            );
        }

        Ok(event)
    }

    pub async fn events_since(&self, since_seq: EventSeq) -> anyhow::Result<Vec<ThreadEvent>> {
        read_events_since_jsonl(self.thread_id, self.log_path(), since_seq).await
    }

    fn recover_incomplete_processes(&mut self) -> Vec<ProcessId> {
        let recovered = self.active_processes.drain().collect::<Vec<_>>();
        for process_id in &recovered {
            self.state.running_processes.remove(process_id);
        }
        recovered
    }
}

fn apply_runtime_event_tracking(
    active_processes: &mut HashSet<ProcessId>,
    active_tools: &mut HashSet<ToolId>,
    kind: &ThreadEventKind,
) {
    match kind {
        ThreadEventKind::ProcessStarted { process_id, .. } => {
            active_processes.insert(*process_id);
        }
        ThreadEventKind::ProcessExited { process_id, .. } => {
            active_processes.remove(process_id);
        }
        ThreadEventKind::ToolStarted { tool_id, .. } => {
            active_tools.insert(*tool_id);
        }
        ThreadEventKind::ToolCompleted { tool_id, .. } => {
            active_tools.remove(tool_id);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn cwd_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[tokio::test]
    async fn append_redacts_sensitive_tokens() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = ThreadStore::new(PmPaths::new(dir.path().join(".omne_data")));

        let mut thread = store.create_thread(PathBuf::from("/tmp")).await?;
        let thread_id = thread.thread_id();
        let turn_id = omne_protocol::TurnId::new();
        thread
            .append(ThreadEventKind::TurnStarted {
                turn_id,
                input: "token=sk-1234567890abcdefghijklmnop".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
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
        let store = ThreadStore::new(PmPaths::new(dir.path().join(".omne_data")));

        let mut thread = store.create_thread(PathBuf::from("/tmp")).await?;
        let thread_id = thread.thread_id();
        let turn_id = omne_protocol::TurnId::new();
        thread
            .append(ThreadEventKind::TurnStarted {
                turn_id,
                input: "x".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
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

    #[tokio::test]
    async fn resume_drops_stale_runtime_process_tracking_without_fabricating_exit()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = ThreadStore::new(PmPaths::new(dir.path().join(".omne_data")));

        let mut thread = store.create_thread(PathBuf::from("/tmp")).await?;
        let thread_id = thread.thread_id();
        let process_id = ProcessId::new();
        thread
            .append(ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                argv: vec!["sleep".to_string(), "999".to_string()],
                cwd: "/tmp".to_string(),
                stdout_path: "/tmp/stdout.log".to_string(),
                stderr_path: "/tmp/stderr.log".to_string(),
            })
            .await?;
        drop(thread);

        let resumed = store
            .resume_thread(thread_id)
            .await?
            .expect("thread exists");
        assert!(resumed.active_processes.is_empty());
        assert!(resumed.state().running_processes.is_empty());

        let events = resumed.events_since(EventSeq::ZERO).await?;
        assert!(events.iter().any(|event| matches!(
            event.kind,
            ThreadEventKind::ProcessStarted {
                process_id: got, ..
            } if got == process_id
        )));
        assert!(!events.iter().any(|event| matches!(
            event.kind,
            ThreadEventKind::ProcessExited {
                process_id: got, ..
            } if got == process_id
        )));
        Ok(())
    }

    #[tokio::test]
    async fn appends_readable_history_for_user_and_assistant() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = ThreadStore::new(PmPaths::new(dir.path().join(".omne_data")));

        let mut thread = store.create_thread(PathBuf::from("/tmp")).await?;
        let thread_id = thread.thread_id();
        let turn_id = omne_protocol::TurnId::new();

        thread
            .append(ThreadEventKind::TurnStarted {
                turn_id,
                input: "hello".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        thread
            .append(ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                text: "world".to_string(),
                model: Some("m".to_string()),
                response_id: Some("r".to_string()),
                token_usage: Some(serde_json::json!({"input_tokens": 1})),
            })
            .await?;
        drop(thread);

        let path = store.thread_dir(thread_id).join(READABLE_HISTORY_FILE_NAME);
        let raw = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        let lines = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);

        let user = serde_json::from_str::<serde_json::Value>(lines[0])?;
        assert_eq!(user.get("role").and_then(|v| v.as_str()), Some("user"));
        assert_eq!(user.get("text").and_then(|v| v.as_str()), Some("hello"));
        assert!(user.get("timestamp").and_then(|v| v.as_str()).is_some());

        let assistant = serde_json::from_str::<serde_json::Value>(lines[1])?;
        assert_eq!(
            assistant.get("role").and_then(|v| v.as_str()),
            Some("assistant")
        );
        assert_eq!(
            assistant.get("text").and_then(|v| v.as_str()),
            Some("world")
        );
        assert_eq!(assistant.get("model").and_then(|v| v.as_str()), Some("m"));
        assert_eq!(
            assistant.get("response_id").and_then(|v| v.as_str()),
            Some("r")
        );
        assert!(assistant.get("token_usage").is_some());
        assert!(
            assistant
                .get("timestamp")
                .and_then(|v| v.as_str())
                .is_some()
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_thread_persists_canonical_absolute_cwd_for_relative_input() -> anyhow::Result<()>
    {
        let _guard = cwd_test_lock().lock().expect("cwd test lock poisoned");
        let original_cwd = std::env::current_dir()?;

        let dir = tempfile::tempdir()?;
        let repo_dir = dir.path().join("repo");
        let nested_dir = repo_dir.join("nested");
        tokio::fs::create_dir_all(&nested_dir).await?;
        std::env::set_current_dir(&repo_dir)?;

        let result = async {
            let store = ThreadStore::new(PmPaths::new(dir.path().join(".omne_data")));
            let thread = store.create_thread(PathBuf::from("./nested/..")).await?;
            Ok::<_, anyhow::Error>((thread.thread_id(), store))
        }
        .await;

        std::env::set_current_dir(&original_cwd)?;

        let (thread_id, store) = result?;
        let state = store
            .read_state(thread_id)
            .await?
            .expect("thread state should exist");
        assert_eq!(
            state.cwd.as_deref(),
            Some(repo_dir.to_str().expect("utf-8 repo path"))
        );
        Ok(())
    }
}
