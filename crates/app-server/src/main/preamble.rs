use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use diffy::{Patch, apply};
use globset::Glob;
use pm_core::{PmPaths, ThreadStore};
use pm_execpolicy::{Decision as ExecDecision, RuleMatch as ExecRuleMatch};
use pm_protocol::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, EventSeq, ProcessId, ThreadEvent, ThreadId,
    TurnId, TurnStatus,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use walkdir::WalkDir;

const CHILD_PROCESS_ENV_SCRUB_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "CODE_PM_OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENROUTER_API_KEY",
    "GEMINI_API_KEY",
];

const DEFAULT_PROCESS_LOG_MAX_BYTES_PER_PART: u64 = 8 * 1024 * 1024;
const MAX_PROCESS_LOG_MAX_BYTES_PER_PART: u64 = 512 * 1024 * 1024;

const DEFAULT_PROCESS_IDLE_WINDOW_SECONDS: u64 = 300;

const DEFAULT_THREAD_DISK_WARNING_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const DEFAULT_THREAD_DISK_CHECK_DEBOUNCE_MS: u64 = 30_000;
const DEFAULT_THREAD_DISK_REPORT_DEBOUNCE_MS: u64 = 30 * 60_000;

fn process_log_max_bytes_per_part() -> u64 {
    std::env::var("CODE_PM_PROCESS_LOG_MAX_BYTES_PER_PART")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_PROCESS_LOG_MAX_BYTES_PER_PART))
        .unwrap_or(DEFAULT_PROCESS_LOG_MAX_BYTES_PER_PART)
}

fn process_idle_window() -> Option<Duration> {
    let value = std::env::var("CODE_PM_PROCESS_IDLE_WINDOW_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_PROCESS_IDLE_WINDOW_SECONDS);
    if value == 0 {
        None
    } else {
        Some(Duration::from_secs(value))
    }
}

fn thread_disk_warning_threshold_bytes() -> Option<u64> {
    let value = std::env::var("CODE_PM_THREAD_DISK_WARNING_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_THREAD_DISK_WARNING_BYTES);
    if value == 0 { None } else { Some(value) }
}

fn thread_disk_check_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("CODE_PM_THREAD_DISK_CHECK_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_THREAD_DISK_CHECK_DEBOUNCE_MS),
    )
}

fn thread_disk_report_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("CODE_PM_THREAD_DISK_REPORT_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_THREAD_DISK_REPORT_DEBOUNCE_MS),
    )
}

fn scrub_child_process_env(cmd: &mut Command) {
    for key in CHILD_PROCESS_ENV_SCRUB_KEYS {
        cmd.env_remove(key);
    }
}

#[derive(Parser)]
#[command(name = "pm-app-server")]
#[command(about = "CodePM v0.2.0 app-server (JSON-RPC over stdio)", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Override project data root directory (default: `./.codepm_data/`).
    #[arg(long)]
    pm_root: Option<PathBuf>,

    /// Listen on a Unix socket instead of stdio (daemon mode).
    #[arg(long, value_name = "PATH")]
    listen: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH")]
    execpolicy_rules: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Generate TypeScript protocol types to an output directory.
    GenerateTs(GenerateOutArgs),
    /// Generate JSON Schema files to an output directory.
    GenerateJsonSchema(GenerateOutArgs),
}

#[derive(clap::Args)]
struct GenerateOutArgs {
    /// Output directory.
    #[arg(long = "out", value_name = "DIR")]
    out_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;
const JSONRPC_INTERNAL_ERROR: i64 = -32603;
const CODE_PM_NOT_INITIALIZED: i64 = -32_000;
const CODE_PM_ALREADY_INITIALIZED: i64 = -32_001;

struct DiskWarningState {
    last_checked_at: Option<tokio::time::Instant>,
    last_reported_at: Option<tokio::time::Instant>,
}

#[derive(Clone)]
struct Server {
    cwd: PathBuf,
    notify_tx: broadcast::Sender<String>,
    thread_store: ThreadStore,
    threads: Arc<tokio::sync::Mutex<HashMap<ThreadId, Arc<ThreadRuntime>>>>,
    processes: Arc<tokio::sync::Mutex<HashMap<ProcessId, ProcessEntry>>>,
    disk_warning: Arc<tokio::sync::Mutex<HashMap<ThreadId, DiskWarningState>>>,
    exec_policy: pm_execpolicy::Policy,
}

impl Server {
    async fn get_or_load_thread(&self, thread_id: ThreadId) -> anyhow::Result<Arc<ThreadRuntime>> {
        let mut threads = self.threads.lock().await;
        if let Some(rt) = threads.get(&thread_id) {
            return Ok(rt.clone());
        }

        let handle = self
            .thread_store
            .resume_thread(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

        let rt = Arc::new(ThreadRuntime::new(handle, self.notify_tx.clone()));
        threads.insert(thread_id, rt.clone());
        Ok(rt)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProcessStatus {
    Running,
    Exited,
    Abandoned,
}

#[derive(Clone, Debug, Serialize)]
struct ProcessInfo {
    process_id: ProcessId,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    argv: Vec<String>,
    cwd: String,
    started_at: String,
    status: ProcessStatus,
    exit_code: Option<i32>,
    stdout_path: String,
    stderr_path: String,
    last_update_at: String,
}

#[derive(Clone)]
struct ProcessEntry {
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
    cmd_tx: mpsc::Sender<ProcessCommand>,
}

enum ProcessCommand {
    Interrupt { reason: Option<String> },
    Kill { reason: Option<String> },
}

struct ThreadRuntime {
    handle: tokio::sync::Mutex<pm_core::ThreadHandle>,
    active_turn: tokio::sync::Mutex<Option<ActiveTurn>>,
    notify_tx: broadcast::Sender<String>,
}

fn validate_context_refs(refs: &[pm_protocol::ContextRef]) -> anyhow::Result<()> {
    for ctx in refs {
        match ctx {
            pm_protocol::ContextRef::File(file) => {
                if file.path.trim().is_empty() {
                    anyhow::bail!("context_refs.file.path must be non-empty");
                }
                if let Some(start_line) = file.start_line {
                    if start_line == 0 {
                        anyhow::bail!("context_refs.file.start_line must be >= 1");
                    }
                }
                if let Some(end_line) = file.end_line {
                    if end_line == 0 {
                        anyhow::bail!("context_refs.file.end_line must be >= 1");
                    }
                    let Some(start_line) = file.start_line else {
                        anyhow::bail!("context_refs.file.end_line requires start_line");
                    };
                    if end_line < start_line {
                        anyhow::bail!("context_refs.file.end_line must be >= start_line");
                    }
                }
                if let Some(max_bytes) = file.max_bytes {
                    if max_bytes == 0 {
                        anyhow::bail!("context_refs.file.max_bytes must be >= 1");
                    }
                }
            }
            pm_protocol::ContextRef::Diff(diff) => {
                if let Some(max_bytes) = diff.max_bytes {
                    if max_bytes == 0 {
                        anyhow::bail!("context_refs.diff.max_bytes must be >= 1");
                    }
                }
            }
        }
    }
    Ok(())
}

impl ThreadRuntime {
    fn new(handle: pm_core::ThreadHandle, notify_tx: broadcast::Sender<String>) -> Self {
        Self {
            handle: tokio::sync::Mutex::new(handle),
            active_turn: tokio::sync::Mutex::new(None),
            notify_tx,
        }
    }

    fn emit_notification<T>(&self, method: &'static str, params: &T)
    where
        T: Serialize,
    {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        if let Ok(line) = serde_json::to_string(&payload) {
            let _ = self.notify_tx.send(line);
        }
    }

    fn emit_event_notifications(&self, event: &ThreadEvent) {
        self.emit_notification("thread/event", event);

        match &event.kind {
            pm_protocol::ThreadEventKind::TurnStarted { .. } => {
                self.emit_notification("turn/started", event);
            }
            pm_protocol::ThreadEventKind::TurnCompleted { .. } => {
                self.emit_notification("turn/completed", event);
            }
            pm_protocol::ThreadEventKind::ToolStarted { .. }
            | pm_protocol::ThreadEventKind::ProcessStarted { .. }
            | pm_protocol::ThreadEventKind::ApprovalRequested { .. } => {
                self.emit_notification("item/started", event);
            }
            pm_protocol::ThreadEventKind::ToolCompleted { .. }
            | pm_protocol::ThreadEventKind::ProcessExited { .. }
            | pm_protocol::ThreadEventKind::ApprovalDecided { .. }
            | pm_protocol::ThreadEventKind::AssistantMessage { .. } => {
                self.emit_notification("item/completed", event);
            }
            _ => {}
        }
    }

    async fn start_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        input: String,
        context_refs: Option<Vec<pm_protocol::ContextRef>>,
    ) -> anyhow::Result<TurnId> {
        let mut handle = self.handle.lock().await;
        let state = handle.state();
        if state.archived {
            anyhow::bail!("thread is archived");
        }
        if state.paused {
            anyhow::bail!("thread is paused");
        }
        if state.active_turn_id.is_some() {
            anyhow::bail!("turn already active");
        }

        let context_refs = match context_refs {
            Some(refs) if refs.is_empty() => None,
            other => other,
        };
        if let Some(refs) = context_refs.as_deref() {
            validate_context_refs(refs)?;
        }

        let turn_id = TurnId::new();
        let input_for_event = input.clone();
        let event = handle
            .append(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: input_for_event,
                context_refs,
            })
            .await?;
        drop(handle);
        self.emit_event_notifications(&event);

        let cancel = CancellationToken::new();
        {
            let mut active = self.active_turn.lock().await;
            *active = Some(ActiveTurn {
                turn_id,
                cancel: cancel.clone(),
                interrupt_reason: None,
            });
        }

        tokio::task::spawn_local(async move {
            self.run_turn(server, turn_id, cancel, input).await;
        });

        Ok(turn_id)
    }

    async fn append_event(
        &self,
        kind: pm_protocol::ThreadEventKind,
    ) -> anyhow::Result<ThreadEvent> {
        let mut handle = self.handle.lock().await;
        let event = handle.append(kind).await?;
        drop(handle);
        self.emit_event_notifications(&event);
        Ok(event)
    }

    async fn interrupt_turn(&self, turn_id: TurnId, reason: Option<String>) -> anyhow::Result<()> {
        let cancel = {
            let mut active = self.active_turn.lock().await;
            let Some(active) = active.as_mut() else {
                anyhow::bail!("no active turn");
            };
            if active.turn_id != turn_id {
                anyhow::bail!("turn is not active");
            }
            if active.interrupt_reason.is_none() {
                active.interrupt_reason = reason.clone();
            }
            active.cancel.clone()
        };

        let mut handle = self.handle.lock().await;
        if handle.state().active_turn_interrupt_requested {
            cancel.cancel();
            return Ok(());
        }
        let event = handle
            .append(pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason })
            .await?;
        drop(handle);
        self.emit_event_notifications(&event);

        cancel.cancel();
        Ok(())
    }

    async fn run_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        turn_id: TurnId,
        cancel: CancellationToken,
        input: String,
    ) {
        let agent_fut =
            agent::run_agent_turn(server.clone(), self.clone(), turn_id, input, cancel.clone());

        let (status, reason) = tokio::select! {
            _ = cancel.cancelled() => {
                let reason = {
                    let active = self.active_turn.lock().await;
                    active.as_ref().and_then(|a| a.interrupt_reason.clone())
                };
                (TurnStatus::Interrupted, reason.or_else(|| Some("turn interrupted".to_string())))
            },
            result = agent_fut => {
                match result {
                    Ok(_completion) => (TurnStatus::Completed, None),
                    Err(err) => {
                        let status = classify_agent_turn_error(&err);
                        (status, Some(err.to_string()))
                    }
                }
            },
        };
        let reason_for_report = reason.clone();

        let mut handle = self.handle.lock().await;
        let thread_id = handle.thread_id();
        if let Ok(event) = handle
            .append(pm_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status,
                reason,
            })
            .await
        {
            self.emit_event_notifications(&event);
        }
        drop(handle);

        if matches!(status, TurnStatus::Stuck) {
            if let Err(err) = maybe_write_stuck_report(
                server.as_ref(),
                thread_id,
                turn_id,
                reason_for_report.as_deref(),
            )
            .await
            {
                tracing::debug!(
                    thread_id = %thread_id,
                    turn_id = %turn_id,
                    error = %err,
                    "stuck report write failed"
                );
            }
        }

        let _ = run_stop_hooks(
            server.as_ref(),
            thread_id,
            turn_id,
            status,
            reason_for_report.as_deref(),
        )
        .await;

        let mut active = self.active_turn.lock().await;
        if active.as_ref().is_some_and(|a| a.turn_id == turn_id) {
            *active = None;
        }
    }
}

fn classify_agent_turn_error(err: &anyhow::Error) -> TurnStatus {
    for cause in err.chain() {
        if let Some(agent_err) = cause.downcast_ref::<agent::AgentTurnError>() {
            return match agent_err {
                agent::AgentTurnError::Cancelled => TurnStatus::Interrupted,
                agent::AgentTurnError::BudgetExceeded { .. }
                | agent::AgentTurnError::TokenBudgetExceeded { .. }
                | agent::AgentTurnError::OpenAiRequestTimedOut
                | agent::AgentTurnError::LoopDetected { .. } => TurnStatus::Stuck,
            };
        }
    }
    TurnStatus::Failed
}

struct ActiveTurn {
    turn_id: TurnId,
    cancel: CancellationToken,
    interrupt_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadStartParams {
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadResumeParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadForkParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadArchiveParams {
    thread_id: ThreadId,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadUnarchiveParams {
    thread_id: ThreadId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadPauseParams {
    thread_id: ThreadId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadUnpauseParams {
    thread_id: ThreadId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadClearArtifactsParams {
    thread_id: ThreadId,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadStateParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadAttentionParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadListMetaParams {
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadDiskUsageParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadDiskReportParams {
    thread_id: ThreadId,
    #[serde(default)]
    top_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ThreadDiffParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ThreadPatchParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ThreadCheckpointCreateParams {
    thread_id: ThreadId,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadCheckpointListParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadCheckpointRestoreParams {
    thread_id: ThreadId,
    checkpoint_id: pm_protocol::CheckpointId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum WorkspaceHookName {
    Setup,
    Run,
    Archive,
}

#[derive(Debug, Deserialize)]
struct ThreadHookRunParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    hook: WorkspaceHookName,
}

#[derive(Debug, Deserialize)]
struct ThreadConfigureParams {
    thread_id: ThreadId,
    #[serde(default)]
    approval_policy: Option<pm_protocol::ApprovalPolicy>,
    #[serde(default)]
    sandbox_policy: Option<pm_protocol::SandboxPolicy>,
    #[serde(default)]
    sandbox_writable_roots: Option<Vec<String>>,
    #[serde(default)]
    sandbox_network_access: Option<pm_protocol::SandboxNetworkAccess>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
    #[serde(default)]
    allowed_tools: Option<Option<Vec<String>>>,
}

#[derive(Debug, Deserialize)]
struct ThreadConfigExplainParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadModelsParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadEventsParams {
    thread_id: ThreadId,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ThreadSubscribeParams {
    thread_id: ThreadId,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
    /// Long-poll timeout in milliseconds. When set to 0, returns immediately.
    #[serde(default)]
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TurnStartParams {
    thread_id: ThreadId,
    input: String,
    #[serde(default)]
    context_refs: Option<Vec<pm_protocol::ContextRef>>,
}

#[derive(Debug, Deserialize)]
struct TurnInterruptParams {
    thread_id: ThreadId,
    turn_id: TurnId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessStartParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessListParams {
    #[serde(default)]
    thread_id: Option<ThreadId>,
}

#[derive(Debug, Deserialize)]
struct ProcessKillParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessInterruptParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ProcessStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Deserialize)]
struct ProcessTailParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    stream: ProcessStream,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessFollowParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    stream: ProcessStream,
    #[serde(default)]
    since_offset: u64,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ProcessInspectParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum FileRoot {
    Workspace,
    Reference,
}

impl FileRoot {
    fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Reference => "reference",
        }
    }
}

#[derive(Debug, Deserialize)]
struct FileReadParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileGlobParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    pattern: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileGrepParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    query: String,
    #[serde(default)]
    is_regex: bool,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_matches: Option<usize>,
    #[serde(default)]
    max_bytes_per_file: Option<u64>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RepoSearchParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    query: String,
    #[serde(default)]
    is_regex: bool,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_matches: Option<usize>,
    #[serde(default)]
    max_bytes_per_file: Option<u64>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RepoIndexParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileWriteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    path: String,
    text: String,
    #[serde(default)]
    create_parent_dirs: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FilePatchParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    path: String,
    patch: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    path: String,
    edits: Vec<FileEditOp>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditOp {
    old: String,
    new: String,
    #[serde(default)]
    expected_replacements: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct FsMkdirParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct ArtifactWriteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    artifact_id: Option<ArtifactId>,
    artifact_type: String,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactListParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
}

#[derive(Debug, Deserialize)]
struct ArtifactReadParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    artifact_id: ArtifactId,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ArtifactDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    artifact_id: ArtifactId,
}

#[derive(Debug, Deserialize)]
struct ApprovalDecideParams {
    thread_id: ThreadId,
    approval_id: pm_protocol::ApprovalId,
    decision: pm_protocol::ApprovalDecision,
    #[serde(default)]
    remember: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApprovalListParams {
    thread_id: ThreadId,
    #[serde(default)]
    include_decided: bool,
}
