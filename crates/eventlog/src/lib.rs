use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;
use omne_protocol::{
    ApprovalId, ApprovalPolicy, EventSeq, ProcessId, SandboxNetworkAccess, ThreadEvent,
    ThreadEventKind, ThreadId, TurnId, TurnStatus,
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
        let (writer, _events) = Self::open_internal(thread_id, log_path, false).await?;
        Ok(writer)
    }

    pub async fn open_with_events(
        thread_id: ThreadId,
        log_path: PathBuf,
    ) -> anyhow::Result<(Self, Vec<ThreadEvent>)> {
        let (writer, events) = Self::open_internal(thread_id, log_path, true).await?;
        Ok((writer, events.unwrap_or_default()))
    }

    async fn open_internal(
        thread_id: ThreadId,
        log_path: PathBuf,
        return_events: bool,
    ) -> anyhow::Result<(Self, Option<Vec<ThreadEvent>>)> {
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create dir {}", parent.display()))?;
            tighten_dir_permissions_best_effort(parent).await;
        }

        let lock_path = lock_path_for(&log_path);
        let lock_path_for_blocking = lock_path.clone();
        let lock_file = tokio::task::spawn_blocking(move || -> anyhow::Result<std::fs::File> {
            let lock_file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&lock_path_for_blocking)
                .with_context(|| format!("open lock file {}", lock_path_for_blocking.display()))?;
            lock_file
                .lock_exclusive()
                .with_context(|| format!("lock {}", lock_path_for_blocking.display()))?;
            Ok(lock_file)
        })
        .await
        .context("join lock file task")??;

        let events = sanitize_and_read_events(thread_id, &log_path).await?;
        let last_seq = events
            .last()
            .map(|event| event.seq)
            .unwrap_or(EventSeq::ZERO);
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .with_context(|| format!("open {}", log_path.display()))?;
        tighten_file_permissions_best_effort(&lock_path).await;
        tighten_file_permissions_best_effort(&log_path).await;

        let writer = Self {
            thread_id,
            log_path,
            _lock_file: lock_file,
            file,
            next_seq: last_seq.0 + 1,
        };

        Ok((writer, return_events.then_some(events)))
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

#[cfg(unix)]
async fn tighten_dir_permissions_best_effort(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let perm = std::fs::Permissions::from_mode(0o700);
    let _ = tokio::fs::set_permissions(path, perm).await;
}

#[cfg(not(unix))]
async fn tighten_dir_permissions_best_effort(_path: &Path) {}

#[cfg(unix)]
async fn tighten_file_permissions_best_effort(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let perm = std::fs::Permissions::from_mode(0o600);
    let _ = tokio::fs::set_permissions(path, perm).await;
}

#[cfg(not(unix))]
async fn tighten_file_permissions_best_effort(_path: &Path) {}

async fn sanitize_and_read_events(
    expected_thread_id: ThreadId,
    log_path: &Path,
) -> anyhow::Result<Vec<ThreadEvent>> {
    let bytes = match tokio::fs::read(log_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read {}", log_path.display())),
    };

    let mut events = Vec::new();
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
                events.push(event);
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

    Ok(events)
}

#[derive(Debug, Clone)]
pub struct ThreadState {
    pub thread_id: ThreadId,
    pub cwd: Option<String>,
    pub system_prompt_sha256: Option<String>,
    pub system_prompt_text: Option<String>,
    pub archived: bool,
    pub archived_at: Option<OffsetDateTime>,
    pub archived_reason: Option<String>,
    pub paused: bool,
    pub paused_at: Option<OffsetDateTime>,
    pub paused_reason: Option<String>,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_policy: policy_meta::WriteScope,
    pub sandbox_writable_roots: Vec<String>,
    pub sandbox_network_access: SandboxNetworkAccess,
    pub mode: String,
    pub role: String,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub show_thinking: Option<bool>,
    pub openai_base_url: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub execpolicy_rules: Vec<String>,
    pub last_seq: EventSeq,
    pub active_turn_id: Option<TurnId>,
    pub active_turn_interrupt_requested: bool,
    pub active_turn_interrupt_reason: Option<String>,
    pub last_turn_id: Option<TurnId>,
    pub last_turn_status: Option<TurnStatus>,
    pub last_turn_reason: Option<String>,
    pub pending_approvals: HashSet<ApprovalId>,
    pub running_processes: HashSet<ProcessId>,
    pub failed_processes: HashSet<ProcessId>,
    pub total_tokens_used: u64,
    pub input_tokens_used: u64,
    pub output_tokens_used: u64,
    pub cache_input_tokens_used: u64,
    pub cache_creation_input_tokens_used: u64,
    counted_usage_response_ids: HashSet<String>,
}

impl ThreadState {
    pub fn new(thread_id: ThreadId) -> Self {
        Self {
            thread_id,
            cwd: None,
            system_prompt_sha256: None,
            system_prompt_text: None,
            archived: false,
            archived_at: None,
            archived_reason: None,
            paused: false,
            paused_at: None,
            paused_reason: None,
            approval_policy: ApprovalPolicy::AutoApprove,
            sandbox_policy: policy_meta::WriteScope::WorkspaceWrite,
            sandbox_writable_roots: Vec::new(),
            sandbox_network_access: SandboxNetworkAccess::Deny,
            mode: "code".to_string(),
            role: "coder".to_string(),
            model: None,
            thinking: None,
            show_thinking: None,
            openai_base_url: None,
            allowed_tools: None,
            execpolicy_rules: Vec::new(),
            last_seq: EventSeq::ZERO,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
            active_turn_interrupt_reason: None,
            last_turn_id: None,
            last_turn_status: None,
            last_turn_reason: None,
            pending_approvals: HashSet::new(),
            running_processes: HashSet::new(),
            failed_processes: HashSet::new(),
            total_tokens_used: 0,
            input_tokens_used: 0,
            output_tokens_used: 0,
            cache_input_tokens_used: 0,
            cache_creation_input_tokens_used: 0,
            counted_usage_response_ids: HashSet::new(),
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
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256,
                prompt_text,
                ..
            } => {
                if let Some(existing_hash) = self.system_prompt_sha256.as_deref()
                    && existing_hash != prompt_sha256.as_str()
                {
                    anyhow::bail!(
                        "thread system prompt hash changed: existing={} new={}",
                        existing_hash,
                        prompt_sha256
                    );
                }
                if let Some(existing_text) = self.system_prompt_text.as_deref()
                    && existing_text != prompt_text.as_str()
                {
                    anyhow::bail!("thread system prompt text changed");
                }
                self.system_prompt_sha256 = Some(prompt_sha256.clone());
                self.system_prompt_text = Some(prompt_text.clone());
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
                role,
                model,
                clear_model,
                thinking,
                clear_thinking,
                show_thinking,
                clear_show_thinking,
                openai_base_url,
                clear_openai_base_url,
                allowed_tools,
                execpolicy_rules,
                clear_execpolicy_rules,
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
                if let Some(role) = role {
                    self.role = role.clone();
                }
                if *clear_model {
                    self.model = None;
                } else if let Some(model) = model {
                    self.model = Some(model.clone());
                }
                if *clear_thinking {
                    self.thinking = None;
                } else if let Some(thinking) = thinking {
                    self.thinking = Some(thinking.clone());
                }
                if *clear_show_thinking {
                    self.show_thinking = None;
                } else if let Some(show_thinking) = show_thinking {
                    self.show_thinking = Some(*show_thinking);
                }
                if *clear_openai_base_url {
                    self.openai_base_url = None;
                } else if let Some(openai_base_url) = openai_base_url {
                    self.openai_base_url = Some(openai_base_url.clone());
                }
                if let Some(allowed_tools) = allowed_tools {
                    self.allowed_tools = allowed_tools.clone();
                }
                if *clear_execpolicy_rules {
                    self.execpolicy_rules.clear();
                } else if let Some(execpolicy_rules) = execpolicy_rules {
                    self.execpolicy_rules = execpolicy_rules.clone();
                }
            }
            ThreadEventKind::TurnStarted { turn_id, .. } => {
                if self.active_turn_id.is_some() {
                    anyhow::bail!("turn started while another turn is active");
                }
                self.active_turn_id = Some(*turn_id);
                self.active_turn_interrupt_requested = false;
                self.active_turn_interrupt_reason = None;
                self.failed_processes.clear();
            }
            ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
                if self.active_turn_id != Some(*turn_id) {
                    anyhow::bail!("interrupt requested for non-active turn");
                }
                self.active_turn_interrupt_requested = true;
                self.active_turn_interrupt_reason = reason.clone();
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
                self.active_turn_interrupt_reason = None;
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

        let total_tokens = usage_total_tokens(usage);
        let input_tokens = usage_input_tokens(usage);
        let output_tokens = usage_output_tokens(usage);
        let cache_input_tokens = usage_cache_input_tokens(usage);
        let cache_creation_input_tokens = usage_cache_creation_input_tokens(usage);
        if total_tokens.is_none()
            && input_tokens.is_none()
            && output_tokens.is_none()
            && cache_input_tokens.is_none()
            && cache_creation_input_tokens.is_none()
        {
            return;
        }

        if let Some(response_id) = response_id.map(str::trim).filter(|s| !s.is_empty())
            && !self
                .counted_usage_response_ids
                .insert(response_id.to_string())
        {
            return;
        }

        if let Some(tokens) = total_tokens {
            self.total_tokens_used = self.total_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = input_tokens {
            self.input_tokens_used = self.input_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = output_tokens {
            self.output_tokens_used = self.output_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = cache_input_tokens {
            self.cache_input_tokens_used = self.cache_input_tokens_used.saturating_add(tokens);
        }
        if let Some(tokens) = cache_creation_input_tokens {
            self.cache_creation_input_tokens_used =
                self.cache_creation_input_tokens_used.saturating_add(tokens);
        }
    }
}

fn usage_total_tokens(usage: &serde_json::Value) -> Option<u64> {
    let total_tokens = usage_total_tokens_field(usage);
    let input_tokens = usage_input_tokens(usage);
    let output_tokens = usage_output_tokens(usage);

    total_tokens.or_else(|| match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => input.checked_add(output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    })
}

fn usage_total_tokens_field(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
}

fn usage_input_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
}

fn usage_output_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
}

fn usage_cache_input_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("cache_input_tokens")
        .and_then(serde_json::Value::as_u64)
}

fn usage_cache_creation_input_tokens(usage: &serde_json::Value) -> Option<u64> {
    usage
        .get("cache_creation_input_tokens")
        .and_then(serde_json::Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omne_protocol::{ThreadEventKind, ThreadId};

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
                turn_id: omne_protocol::TurnId::new(),
                status: omne_protocol::TurnStatus::Completed,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn writer_tightens_permissions() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir()?;
        let thread_id = ThreadId::new();
        let thread_dir = dir.path().join("threads").join(thread_id.to_string());
        let log_path = thread_dir.join("events.jsonl");

        let mut writer = EventLogWriter::open(thread_id, log_path.clone()).await?;
        writer
            .append(ThreadEventKind::ThreadCreated {
                cwd: "/tmp".to_string(),
            })
            .await?;
        drop(writer);

        let dir_mode = tokio::fs::metadata(&thread_dir).await?.permissions().mode() & 0o777u32;
        assert_eq!(dir_mode, 0o700);

        let log_mode = tokio::fs::metadata(&log_path).await?.permissions().mode() & 0o777u32;
        assert_eq!(log_mode, 0o600);

        let lock_path = lock_path_for(&log_path);
        let lock_mode = tokio::fs::metadata(&lock_path).await?.permissions().mode() & 0o777u32;
        assert_eq!(lock_mode, 0o600);
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
                os_pid: None,
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
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            },
        )?;
        assert!(state.failed_processes.is_empty());

        Ok(())
    }

    #[test]
    fn token_usage_counts_cache_fields_and_dedupes_by_response_id() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::AgentStep {
                turn_id: TurnId::new(),
                step: 1,
                model: "gpt-5".to_string(),
                response_id: "resp_1".to_string(),
                text: Some("step".to_string()),
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
                token_usage: Some(serde_json::json!({
                    "total_tokens": 120,
                    "input_tokens": 90,
                    "output_tokens": 30,
                    "cache_input_tokens": 70,
                    "cache_creation_input_tokens": 11
                })),
                warnings_count: None,
            },
        )?;
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::AssistantMessage {
                turn_id: None,
                text: "final".to_string(),
                model: Some("gpt-5".to_string()),
                response_id: Some("resp_1".to_string()),
                token_usage: Some(serde_json::json!({
                    "total_tokens": 120,
                    "input_tokens": 90,
                    "output_tokens": 30,
                    "cache_input_tokens": 70,
                    "cache_creation_input_tokens": 11
                })),
            },
        )?;

        assert_eq!(state.total_tokens_used, 120);
        assert_eq!(state.input_tokens_used, 90);
        assert_eq!(state.output_tokens_used, 30);
        assert_eq!(state.cache_input_tokens_used, 70);
        assert_eq!(state.cache_creation_input_tokens_used, 11);
        Ok(())
    }

    #[test]
    fn token_usage_counts_assistant_message_without_agent_step() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::AssistantMessage {
                turn_id: None,
                text: "final".to_string(),
                model: Some("gpt-5".to_string()),
                response_id: Some("resp_2".to_string()),
                token_usage: Some(serde_json::json!({
                    "input_tokens": 40,
                    "output_tokens": 10,
                    "cache_input_tokens": 16
                })),
            },
        )?;

        assert_eq!(state.total_tokens_used, 50);
        assert_eq!(state.input_tokens_used, 40);
        assert_eq!(state.output_tokens_used, 10);
        assert_eq!(state.cache_input_tokens_used, 16);
        assert_eq!(state.cache_creation_input_tokens_used, 0);
        Ok(())
    }

    #[test]
    fn token_usage_ignores_uncountable_response_before_dedup() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::AgentStep {
                turn_id: TurnId::new(),
                step: 1,
                model: "gpt-5".to_string(),
                response_id: "resp_3".to_string(),
                text: Some("step".to_string()),
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
                token_usage: Some(serde_json::json!({
                    "total_tokens": "<REDACTED>",
                    "cache_input_tokens": "<REDACTED>"
                })),
                warnings_count: None,
            },
        )?;
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::AssistantMessage {
                turn_id: None,
                text: "final".to_string(),
                model: Some("gpt-5".to_string()),
                response_id: Some("resp_3".to_string()),
                token_usage: Some(serde_json::json!({
                    "total_tokens": 22,
                    "cache_input_tokens": 5
                })),
            },
        )?;

        assert_eq!(state.total_tokens_used, 22);
        assert_eq!(state.input_tokens_used, 0);
        assert_eq!(state.output_tokens_used, 0);
        assert_eq!(state.cache_input_tokens_used, 5);
        assert_eq!(state.cache_creation_input_tokens_used, 0);
        Ok(())
    }

    #[test]
    fn thread_config_updated_clear_flags_remove_overrides() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::ThreadConfigUpdated {
                approval_policy: ApprovalPolicy::Manual,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: Some("gpt-5".to_string()),
                clear_model: false,
                thinking: Some("high".to_string()),
                clear_thinking: false,
                show_thinking: Some(true),
                clear_show_thinking: false,
                openai_base_url: Some("https://example.test/v1".to_string()),
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: Some(vec!["rules/a.rules".to_string()]),
                clear_execpolicy_rules: false,
            },
        )?;

        assert_eq!(state.model.as_deref(), Some("gpt-5"));
        assert_eq!(state.thinking.as_deref(), Some("high"));
        assert_eq!(state.show_thinking, Some(true));
        assert_eq!(
            state.openai_base_url.as_deref(),
            Some("https://example.test/v1")
        );
        assert_eq!(state.execpolicy_rules, vec!["rules/a.rules".to_string()]);

        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadConfigUpdated {
                approval_policy: ApprovalPolicy::Manual,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                clear_model: true,
                thinking: None,
                clear_thinking: true,
                show_thinking: None,
                clear_show_thinking: true,
                openai_base_url: None,
                clear_openai_base_url: true,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: true,
            },
        )?;

        assert_eq!(state.model, None);
        assert_eq!(state.thinking, None);
        assert_eq!(state.show_thinking, None);
        assert_eq!(state.openai_base_url, None);
        assert!(state.execpolicy_rules.is_empty());
        Ok(())
    }

    #[test]
    fn thread_config_updated_without_optional_values_keeps_existing_overrides() -> anyhow::Result<()>
    {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::ThreadConfigUpdated {
                approval_policy: ApprovalPolicy::Manual,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: Some("gpt-5".to_string()),
                clear_model: false,
                thinking: Some("high".to_string()),
                clear_thinking: false,
                show_thinking: Some(true),
                clear_show_thinking: false,
                openai_base_url: Some("https://example.test/v1".to_string()),
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: Some(vec!["rules/a.rules".to_string()]),
                clear_execpolicy_rules: false,
            },
        )?;
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadConfigUpdated {
                approval_policy: ApprovalPolicy::Manual,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
            },
        )?;

        assert_eq!(state.model.as_deref(), Some("gpt-5"));
        assert_eq!(state.thinking.as_deref(), Some("high"));
        assert_eq!(state.show_thinking, Some(true));
        assert_eq!(
            state.openai_base_url.as_deref(),
            Some("https://example.test/v1")
        );
        assert_eq!(state.execpolicy_rules, vec!["rules/a.rules".to_string()]);
        Ok(())
    }

    #[test]
    fn thread_system_prompt_snapshot_accepts_initial_and_identical_repeat() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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

        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadCreated {
                cwd: "/tmp".to_string(),
            },
        )?;

        let snapshot_hash = "hash-a".to_string();
        let snapshot_text = "# system prompt A".to_string();
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256: snapshot_hash.clone(),
                prompt_text: snapshot_text.clone(),
                source: Some("default".to_string()),
            },
        )?;
        apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256: snapshot_hash.clone(),
                prompt_text: snapshot_text.clone(),
                source: Some("fork".to_string()),
            },
        )?;

        assert_eq!(state.system_prompt_sha256.as_deref(), Some("hash-a"));
        assert_eq!(
            state.system_prompt_text.as_deref(),
            Some("# system prompt A")
        );
        Ok(())
    }

    #[test]
    fn thread_system_prompt_snapshot_rejects_hash_change() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256: "hash-a".to_string(),
                prompt_text: "# system prompt A".to_string(),
                source: None,
            },
        )?;

        let err = apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256: "hash-b".to_string(),
                prompt_text: "# system prompt B".to_string(),
                source: None,
            },
        )
        .expect_err("changing snapshot hash should be rejected");
        assert!(
            err.to_string()
                .contains("thread system prompt hash changed")
        );
        Ok(())
    }

    #[test]
    fn thread_system_prompt_snapshot_rejects_text_change_with_same_hash() -> anyhow::Result<()> {
        let thread_id = ThreadId::new();
        let mut state = ThreadState::new(thread_id);
        let mut seq = 1u64;

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
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256: "stable-hash".to_string(),
                prompt_text: "# system prompt A".to_string(),
                source: None,
            },
        )?;

        let err = apply(
            &mut state,
            thread_id,
            &mut seq,
            ThreadEventKind::ThreadSystemPromptSnapshot {
                prompt_sha256: "stable-hash".to_string(),
                prompt_text: "# system prompt B".to_string(),
                source: None,
            },
        )
        .expect_err("changing snapshot text should be rejected");
        assert!(
            err.to_string()
                .contains("thread system prompt text changed")
        );
        Ok(())
    }
}
