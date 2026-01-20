use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
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
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use walkdir::WalkDir;

mod agent;

#[derive(Parser)]
#[command(name = "pm-app-server")]
#[command(about = "CodePM v0.2.0 app-server (JSON-RPC over stdio)", long_about = None)]
struct Args {
    /// Override `.code_pm` root directory.
    #[arg(long)]
    pm_root: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH")]
    execpolicy_rules: Vec<PathBuf>,
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

#[derive(Clone)]
struct Server {
    cwd: PathBuf,
    thread_store: ThreadStore,
    threads: Arc<tokio::sync::Mutex<HashMap<ThreadId, Arc<ThreadRuntime>>>>,
    processes: Arc<tokio::sync::Mutex<HashMap<ProcessId, ProcessEntry>>>,
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

        let rt = Arc::new(ThreadRuntime::new(handle));
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
    Kill { reason: Option<String> },
}

struct ThreadRuntime {
    handle: tokio::sync::Mutex<pm_core::ThreadHandle>,
    active_turn: tokio::sync::Mutex<Option<ActiveTurn>>,
}

impl ThreadRuntime {
    fn new(handle: pm_core::ThreadHandle) -> Self {
        Self {
            handle: tokio::sync::Mutex::new(handle),
            active_turn: tokio::sync::Mutex::new(None),
        }
    }

    async fn start_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        input: String,
    ) -> anyhow::Result<TurnId> {
        let mut handle = self.handle.lock().await;
        if handle.state().active_turn_id.is_some() {
            anyhow::bail!("turn already active");
        }

        let turn_id = TurnId::new();
        let input_for_event = input.clone();
        handle
            .append(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: input_for_event,
            })
            .await?;
        drop(handle);

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
        handle.append(kind).await
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
        handle
            .append(pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason })
            .await?;
        drop(handle);

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
        let agent_fut = agent::run_agent_turn(server, self.clone(), turn_id, input, cancel.clone());

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
                    Err(err) => (TurnStatus::Failed, Some(err.to_string())),
                }
            },
        };

        let mut handle = self.handle.lock().await;
        let _ = handle
            .append(pm_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status,
                reason,
            })
            .await;
        drop(handle);

        let mut active = self.active_turn.lock().await;
        if active.as_ref().is_some_and(|a| a.turn_id == turn_id) {
            *active = None;
        }
    }
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
struct ThreadConfigureParams {
    thread_id: ThreadId,
    #[serde(default)]
    approval_policy: Option<pm_protocol::ApprovalPolicy>,
    #[serde(default)]
    sandbox_policy: Option<pm_protocol::SandboxPolicy>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadConfigExplainParams {
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
    stream: ProcessStream,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessFollowParams {
    process_id: ProcessId,
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
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileReadParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileGlobParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    pattern: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileGrepParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
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
    artifact_id: Option<ArtifactId>,
    artifact_type: String,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactListParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ArtifactReadParams {
    thread_id: ThreadId,
    artifact_id: ArtifactId,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ArtifactDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let cwd = std::env::current_dir()?;
    let pm_root = args
        .pm_root
        .or_else(|| std::env::var_os("CODE_PM_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| cwd.join(".code_pm"));

    let exec_policy = if args.execpolicy_rules.is_empty() {
        pm_execpolicy::Policy::empty()
    } else {
        pm_execpolicy::execpolicycheck::load_policies(&args.execpolicy_rules)?
    };

    let server = Arc::new(Server {
        cwd,
        thread_store: ThreadStore::new(PmPaths::new(pm_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        exec_policy,
    });

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let stdin = tokio::io::stdin();
            let mut lines = tokio::io::BufReader::new(stdin).lines();

            let mut initialized = false;

            while let Some(line) = lines.next_line().await? {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let request: JsonRpcRequest = match serde_json::from_str(line) {
                    Ok(req) => req,
                    Err(err) => {
                        eprintln!("app-server: invalid json: {err}");
                        continue;
                    }
                };

                let id = request.id.clone();
                let response = match request.method.as_str() {
            "initialize" => {
                if initialized {
                    JsonRpcResponse::err(
                        id,
                        CODE_PM_ALREADY_INITIALIZED,
                        "already initialized",
                        None,
                    )
                } else {
                    initialized = true;
                    JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "server": {
                                "name": "pm-app-server",
                                "version": env!("CARGO_PKG_VERSION"),
                            }
                        }),
                    )
                }
            }
            "initialized" => {
                if initialized {
                    JsonRpcResponse::ok(id, serde_json::json!({ "ok": true }))
                } else {
                    JsonRpcResponse::err(id, CODE_PM_NOT_INITIALIZED, "not initialized", None)
                }
            }
            _ if !initialized => {
                JsonRpcResponse::err(id, CODE_PM_NOT_INITIALIZED, "not initialized", None)
            }
            "thread/start" => match serde_json::from_value::<ThreadStartParams>(request.params) {
                Ok(params) => {
                    let cwd = params
                        .cwd
                        .map(PathBuf::from)
                        .unwrap_or_else(|| server.cwd.clone());
                    match server.thread_store.create_thread(cwd).await {
                        Ok(handle) => {
                            let thread_id = handle.thread_id();
                            let log_path = handle.log_path().display().to_string();
                            let last_seq = handle.last_seq().0;
                            let rt = Arc::new(ThreadRuntime::new(handle));
                            server.threads.lock().await.insert(thread_id, rt);

                            JsonRpcResponse::ok(
                                id,
                                serde_json::json!({
                                    "thread_id": thread_id,
                                    "log_path": log_path,
                                    "last_seq": last_seq,
                                }),
                            )
                        }
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    }
                }
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/resume" => match serde_json::from_value::<ThreadResumeParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => {
                        let handle = rt.handle.lock().await;
                        JsonRpcResponse::ok(
                            id,
                            serde_json::json!({
                                "thread_id": handle.thread_id(),
                                "log_path": handle.log_path().display().to_string(),
                                "last_seq": handle.last_seq().0,
                            }),
                        )
                    }
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/fork" => match serde_json::from_value::<ThreadForkParams>(request.params) {
                Ok(params) => match handle_thread_fork(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/delete" => match serde_json::from_value::<ThreadDeleteParams>(request.params) {
                Ok(params) => match handle_thread_delete(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/clear_artifacts" => {
                match serde_json::from_value::<ThreadClearArtifactsParams>(request.params) {
                    Ok(params) => match handle_thread_clear_artifacts(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/list" => match server.thread_store.list_threads().await {
                Ok(threads) => JsonRpcResponse::ok(
                    id,
                    serde_json::json!({
                        "threads": threads,
                    }),
                ),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            "thread/loaded" => {
                let mut threads = server
                    .threads
                    .lock()
                    .await
                    .keys()
                    .copied()
                    .collect::<Vec<_>>();
                threads.sort_unstable();
                JsonRpcResponse::ok(
                    id,
                    serde_json::json!({
                        "threads": threads,
                    }),
                )
            }
            "thread/events" => match serde_json::from_value::<ThreadEventsParams>(request.params) {
                Ok(params) => {
                    let since = EventSeq(params.since_seq);
                    match server
                        .thread_store
                        .read_events_since(params.thread_id, since)
                        .await
                    {
                        Ok(Some(mut events)) => {
                            let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);
                            let mut has_more = false;
                            if let Some(max_events) = params.max_events {
                                let max_events = max_events.clamp(1, 50_000);
                                if events.len() > max_events {
                                    events.truncate(max_events);
                                    has_more = true;
                                }
                            }

                            let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);
                            JsonRpcResponse::ok(
                                id,
                                serde_json::json!({
                                    "events": events,
                                    "last_seq": last_seq,
                                    "thread_last_seq": thread_last_seq,
                                    "has_more": has_more,
                                }),
                            )
                        }
                        Ok(None) => JsonRpcResponse::err(
                            id,
                            JSONRPC_INTERNAL_ERROR,
                            format!("thread not found: {}", params.thread_id),
                            None,
                        ),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    }
                }
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/subscribe" => {
                match serde_json::from_value::<ThreadSubscribeParams>(request.params) {
                    Ok(params) => match handle_thread_subscribe(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/state" => match serde_json::from_value::<ThreadStateParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => {
                        let handle = rt.handle.lock().await;
                        let state = handle.state();
                        JsonRpcResponse::ok(
                            id,
                            serde_json::json!({
                                "thread_id": handle.thread_id(),
                                "cwd": state.cwd,
                                "approval_policy": state.approval_policy,
                                "sandbox_policy": state.sandbox_policy,
                                "model": state.model,
                                "openai_base_url": state.openai_base_url,
                                "last_seq": handle.last_seq().0,
                                "active_turn_id": state.active_turn_id,
                                "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
                                "last_turn_id": state.last_turn_id,
                                "last_turn_status": state.last_turn_status,
                                "last_turn_reason": state.last_turn_reason,
                            }),
                        )
                    }
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/attention" => {
                match serde_json::from_value::<ThreadAttentionParams>(request.params) {
                    Ok(params) => match handle_thread_attention(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/disk_usage" => {
                match serde_json::from_value::<ThreadDiskUsageParams>(request.params) {
                    Ok(params) => match handle_thread_disk_usage(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/disk_report" => {
                match serde_json::from_value::<ThreadDiskReportParams>(request.params) {
                    Ok(params) => match handle_thread_disk_report(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/configure" => {
                match serde_json::from_value::<ThreadConfigureParams>(request.params) {
                    Ok(params) => match handle_thread_configure(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/config/explain" => {
                match serde_json::from_value::<ThreadConfigExplainParams>(request.params) {
                    Ok(params) => match handle_thread_config_explain(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "turn/start" => match serde_json::from_value::<TurnStartParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => match rt.start_turn(server.clone(), params.input).await {
                        Ok(turn_id) => JsonRpcResponse::ok(
                            id,
                            serde_json::json!({
                                "turn_id": turn_id,
                            }),
                        ),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "turn/interrupt" => match serde_json::from_value::<TurnInterruptParams>(request.params)
            {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => {
                        let kill_reason = params
                            .reason
                            .clone()
                            .or_else(|| Some("turn interrupted".to_string()));
                        match rt.interrupt_turn(params.turn_id, kill_reason.clone()).await {
                            Ok(()) => {
                                kill_processes_for_turn(
                                    &server,
                                    params.thread_id,
                                    params.turn_id,
                                    kill_reason,
                                )
                                .await;
                                JsonRpcResponse::ok(id, serde_json::json!({ "ok": true }))
                            }
                            Err(err) => JsonRpcResponse::err(
                                id,
                                JSONRPC_INTERNAL_ERROR,
                                err.to_string(),
                                None,
                            ),
                        }
                    }
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/start" => match serde_json::from_value::<ProcessStartParams>(request.params) {
                Ok(params) => match handle_process_start(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/list" => match serde_json::from_value::<ProcessListParams>(request.params) {
                Ok(params) => match handle_process_list(&server, params).await {
                    Ok(processes) => JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "processes": processes,
                        }),
                    ),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/inspect" => {
                match serde_json::from_value::<ProcessInspectParams>(request.params) {
                    Ok(params) => match handle_process_inspect(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "process/kill" => match serde_json::from_value::<ProcessKillParams>(request.params) {
                Ok(params) => {
                    let entry = {
                        let entries = server.processes.lock().await;
                        entries.get(&params.process_id).cloned()
                    };
                    if let Some(entry) = entry {
                        let _ = entry
                            .cmd_tx
                            .send(ProcessCommand::Kill {
                                reason: params.reason,
                            })
                            .await;
                        JsonRpcResponse::ok(id, serde_json::json!({ "ok": true }))
                    } else {
                        JsonRpcResponse::err(
                            id,
                            JSONRPC_INTERNAL_ERROR,
                            format!("process not found: {}", params.process_id),
                            None,
                        )
                    }
                }
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/tail" => match serde_json::from_value::<ProcessTailParams>(request.params) {
                Ok(params) => match handle_process_tail(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/follow" => match serde_json::from_value::<ProcessFollowParams>(request.params)
            {
                Ok(params) => match handle_process_follow(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/read" => match serde_json::from_value::<FileReadParams>(request.params) {
                Ok(params) => match handle_file_read(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/glob" => match serde_json::from_value::<FileGlobParams>(request.params) {
                Ok(params) => match handle_file_glob(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/grep" => match serde_json::from_value::<FileGrepParams>(request.params) {
                Ok(params) => match handle_file_grep(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/write" => match serde_json::from_value::<FileWriteParams>(request.params) {
                Ok(params) => match handle_file_write(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/patch" => match serde_json::from_value::<FilePatchParams>(request.params) {
                Ok(params) => match handle_file_patch(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/edit" => match serde_json::from_value::<FileEditParams>(request.params) {
                Ok(params) => match handle_file_edit(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/delete" => match serde_json::from_value::<FileDeleteParams>(request.params) {
                Ok(params) => match handle_file_delete(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "fs/mkdir" => match serde_json::from_value::<FsMkdirParams>(request.params) {
                Ok(params) => match handle_fs_mkdir(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/write" => match serde_json::from_value::<ArtifactWriteParams>(request.params)
            {
                Ok(params) => match handle_artifact_write(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/list" => match serde_json::from_value::<ArtifactListParams>(request.params) {
                Ok(params) => match handle_artifact_list(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/read" => match serde_json::from_value::<ArtifactReadParams>(request.params) {
                Ok(params) => match handle_artifact_read(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/delete" => {
                match serde_json::from_value::<ArtifactDeleteParams>(request.params) {
                    Ok(params) => match handle_artifact_delete(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "approval/decide" => {
                match serde_json::from_value::<ApprovalDecideParams>(request.params) {
                    Ok(params) => match handle_approval_decide(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "approval/list" => match serde_json::from_value::<ApprovalListParams>(request.params) {
                Ok(params) => match handle_approval_list(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            _ => JsonRpcResponse::err(
                id,
                JSONRPC_METHOD_NOT_FOUND,
                "method not found",
                Some(serde_json::json!({ "method": request.method })),
            ),
        };

                println!("{}", serde_json::to_string(&response)?);
            }

            Ok(())
        })
        .await
}

async fn handle_thread_attention(
    server: &Server,
    params: ThreadAttentionParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;

    let (
        last_seq,
        active_turn_id,
        active_turn_interrupt_requested,
        last_turn_id,
        last_turn_status,
        last_turn_reason,
        approval_policy,
        sandbox_policy,
        model,
        openai_base_url,
        cwd,
    ) = {
        let handle = rt.handle.lock().await;
        let state = handle.state();
        (
            handle.last_seq().0,
            state.active_turn_id,
            state.active_turn_interrupt_requested,
            state.last_turn_id,
            state.last_turn_status,
            state.last_turn_reason.clone(),
            state.approval_policy,
            state.sandbox_policy,
            state.model.clone(),
            state.openai_base_url.clone(),
            state.cwd.clone(),
        )
    };

    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut requested = BTreeMap::<pm_protocol::ApprovalId, serde_json::Value>::new();
    let mut decided = HashSet::<pm_protocol::ApprovalId>::new();

    for event in &events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match &event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action,
                params,
            } => {
                requested.insert(
                    *approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "turn_id": turn_id,
                        "action": action,
                        "params": params,
                        "requested_at": ts,
                    }),
                );
            }
            pm_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                decided.insert(*approval_id);
            }
            _ => {}
        }
    }

    let pending_approvals = requested
        .into_iter()
        .filter(|(id, _)| !decided.contains(id))
        .map(|(_, v)| v)
        .collect::<Vec<_>>();

    let processes = handle_process_list(
        server,
        ProcessListParams {
            thread_id: Some(params.thread_id),
        },
    )
    .await?;

    let running_processes = processes
        .into_iter()
        .filter(|p| matches!(p.status, ProcessStatus::Running))
        .map(|p| serde_json::to_value(p))
        .collect::<Result<Vec<_>, _>>()?;

    let attention_state = if !pending_approvals.is_empty() {
        "need_approval"
    } else if active_turn_id.is_some() || !running_processes.is_empty() {
        "running"
    } else {
        match last_turn_status {
            Some(pm_protocol::TurnStatus::Completed) => "done",
            Some(pm_protocol::TurnStatus::Interrupted) => "interrupted",
            Some(pm_protocol::TurnStatus::Failed) => "failed",
            Some(pm_protocol::TurnStatus::Cancelled) => "cancelled",
            None => "idle",
        }
    };

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "cwd": cwd,
        "approval_policy": approval_policy,
        "sandbox_policy": sandbox_policy,
        "model": model,
        "openai_base_url": openai_base_url,
        "last_seq": last_seq,
        "active_turn_id": active_turn_id,
        "active_turn_interrupt_requested": active_turn_interrupt_requested,
        "last_turn_id": last_turn_id,
        "last_turn_status": last_turn_status,
        "last_turn_reason": last_turn_reason,
        "attention_state": attention_state,
        "pending_approvals": pending_approvals,
        "running_processes": running_processes,
    }))
}

async fn handle_thread_subscribe(
    server: &Server,
    params: ThreadSubscribeParams,
) -> anyhow::Result<Value> {
    let wait_ms = params.wait_ms.unwrap_or(30_000).min(300_000);
    let poll_interval = Duration::from_millis(200);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);

    let since = EventSeq(params.since_seq);
    let mut timed_out = false;

    loop {
        let mut events = server
            .thread_store
            .read_events_since(params.thread_id, since)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

        let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

        let mut has_more = false;
        if let Some(max_events) = params.max_events {
            let max_events = max_events.clamp(1, 50_000);
            if events.len() > max_events {
                events.truncate(max_events);
                has_more = true;
            }
        }

        let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

        if !events.is_empty() || wait_ms == 0 {
            return Ok(serde_json::json!({
                "events": events,
                "last_seq": last_seq,
                "thread_last_seq": thread_last_seq,
                "has_more": has_more,
                "timed_out": false,
            }));
        }

        if tokio::time::Instant::now() >= deadline {
            timed_out = true;
        }

        if timed_out {
            return Ok(serde_json::json!({
                "events": events,
                "last_seq": last_seq,
                "thread_last_seq": thread_last_seq,
                "has_more": has_more,
                "timed_out": true,
            }));
        }

        tokio::time::sleep(poll_interval).await;
    }
}

#[derive(Debug)]
struct ThreadDiskUsage {
    total_bytes: u64,
    events_log_bytes: u64,
    artifacts_bytes: u64,
    file_count: usize,
    top_files: Vec<(u64, String)>,
}

fn scan_thread_disk_usage(
    thread_dir: &Path,
    events_log_path: &Path,
    top_n: usize,
) -> anyhow::Result<ThreadDiskUsage> {
    let artifacts_dir = thread_dir.join("artifacts");

    let events_log_bytes = std::fs::metadata(events_log_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let mut total_bytes = 0u64;
    let mut artifacts_bytes = 0u64;
    let mut file_count = 0usize;
    let mut top_files: Vec<(u64, String)> = Vec::new();

    for entry in WalkDir::new(thread_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_type().is_symlink())
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let meta = entry.metadata()?;
        let size = meta.len();
        file_count += 1;
        total_bytes = total_bytes.saturating_add(size);
        if entry.path().starts_with(&artifacts_dir) {
            artifacts_bytes = artifacts_bytes.saturating_add(size);
        }

        if top_n == 0 {
            continue;
        }

        let rel = entry
            .path()
            .strip_prefix(thread_dir)
            .unwrap_or(entry.path());
        let rel = rel.to_string_lossy().to_string();

        if top_files.len() < top_n {
            top_files.push((size, rel));
            top_files.sort_by_key(|(b, _)| *b);
            continue;
        }
        if let Some((smallest, _)) = top_files.first() {
            if size > *smallest {
                top_files[0] = (size, rel);
                top_files.sort_by_key(|(b, _)| *b);
            }
        }
    }

    top_files.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(ThreadDiskUsage {
        total_bytes,
        events_log_bytes,
        artifacts_bytes,
        file_count,
        top_files,
    })
}

async fn handle_thread_disk_usage(
    server: &Server,
    params: ThreadDiskUsageParams,
) -> anyhow::Result<Value> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let events_log_path = server.thread_store.events_log_path(params.thread_id);

    match tokio::fs::metadata(&thread_dir).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => anyhow::bail!("thread dir is not a directory: {}", thread_dir.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("thread not found: {}", params.thread_id)
        }
        Err(err) => return Err(err).with_context(|| format!("stat {}", thread_dir.display())),
    }

    let thread_dir_for_task = thread_dir.clone();
    let events_log_path_for_task = events_log_path.clone();
    let usage = tokio::task::spawn_blocking(move || {
        scan_thread_disk_usage(&thread_dir_for_task, &events_log_path_for_task, 0)
    })
    .await
    .context("join disk usage task")??;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "thread_dir": thread_dir.display().to_string(),
        "events_log_path": events_log_path.display().to_string(),
        "events_log_bytes": usage.events_log_bytes,
        "artifacts_bytes": usage.artifacts_bytes,
        "total_bytes": usage.total_bytes,
        "file_count": usage.file_count,
    }))
}

async fn handle_thread_disk_report(
    server: &Server,
    params: ThreadDiskReportParams,
) -> anyhow::Result<Value> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let events_log_path = server.thread_store.events_log_path(params.thread_id);

    match tokio::fs::metadata(&thread_dir).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => anyhow::bail!("thread dir is not a directory: {}", thread_dir.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("thread not found: {}", params.thread_id)
        }
        Err(err) => return Err(err).with_context(|| format!("stat {}", thread_dir.display())),
    }

    let top_n = params.top_files.unwrap_or(40).min(200);
    let thread_dir_for_task = thread_dir.clone();
    let events_log_path_for_task = events_log_path.clone();
    let usage = tokio::task::spawn_blocking(move || {
        scan_thread_disk_usage(&thread_dir_for_task, &events_log_path_for_task, top_n)
    })
    .await
    .context("join disk report task")??;

    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;

    let mut report = String::new();
    report.push_str("# Thread disk usage report\n\n");
    report.push_str(&format!("- thread_id: {}\n", params.thread_id));
    report.push_str(&format!("- generated_at: {}\n", now));
    report.push_str(&format!("- thread_dir: {}\n", thread_dir.display()));
    report.push_str(&format!(
        "- events_log_path: {}\n",
        events_log_path.display()
    ));
    report.push_str(&format!("- total_bytes: {}\n", usage.total_bytes));
    report.push_str(&format!("- artifacts_bytes: {}\n", usage.artifacts_bytes));
    report.push_str(&format!("- events_log_bytes: {}\n", usage.events_log_bytes));
    report.push_str(&format!("- file_count: {}\n", usage.file_count));

    if !usage.top_files.is_empty() {
        report.push_str("\n## Top files\n");
        for (size, rel) in &usage.top_files {
            report.push_str(&format!("- {}  {}\n", size, rel));
        }
    }

    report.push_str("\n## Cleanup\n");
    report.push_str("- Use `thread/clear_artifacts` to remove `artifacts/` (requires force=true if processes are running).\n");
    report.push_str("- Use `thread/delete` to remove the entire thread directory (requires force=true if processes are running).\n");

    let artifact = handle_artifact_write(
        server,
        ArtifactWriteParams {
            thread_id: params.thread_id,
            turn_id: None,
            artifact_id: None,
            artifact_type: "disk_report".to_string(),
            summary: "Thread disk usage report".to_string(),
            text: report,
        },
    )
    .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "disk_usage": {
            "events_log_bytes": usage.events_log_bytes,
            "artifacts_bytes": usage.artifacts_bytes,
            "total_bytes": usage.total_bytes,
            "file_count": usage.file_count,
        },
        "artifact": artifact,
    }))
}

async fn handle_thread_configure(
    server: &Server,
    params: ThreadConfigureParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    let (current_approval_policy, current_sandbox_policy, current_model, current_openai_base_url) = {
        let handle = rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.model.clone(),
            state.openai_base_url.clone(),
        )
    };

    let approval_policy = params.approval_policy.unwrap_or(current_approval_policy);
    let model = params.model.filter(|s| !s.trim().is_empty());
    let openai_base_url = params.openai_base_url.filter(|s| !s.trim().is_empty());

    let changed = approval_policy != current_approval_policy
        || params
            .sandbox_policy
            .is_some_and(|p| p != current_sandbox_policy)
        || model.as_ref() != current_model.as_ref()
        || openai_base_url.as_ref() != current_openai_base_url.as_ref();

    if changed {
        rt.append_event(pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy: params.sandbox_policy,
            model,
            openai_base_url,
        })
        .await?;
    }
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_thread_config_explain(
    server: &Server,
    params: ThreadConfigExplainParams,
) -> anyhow::Result<Value> {
    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let default_model = "gpt-4.1".to_string();
    let default_openai_base_url = "https://api.openai.com".to_string();

    let mut effective_approval_policy = pm_protocol::ApprovalPolicy::AutoApprove;
    let mut effective_sandbox_policy = pm_protocol::SandboxPolicy::WorkspaceWrite;
    let mut effective_model = default_model.clone();
    let mut effective_openai_base_url = default_openai_base_url.clone();
    let mut layers = vec![serde_json::json!({
        "source": "default",
        "approval_policy": effective_approval_policy,
        "sandbox_policy": effective_sandbox_policy,
        "model": effective_model,
        "openai_base_url": effective_openai_base_url,
    })];

    let env_model = std::env::var("CODE_PM_OPENAI_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_openai_base_url = std::env::var("CODE_PM_OPENAI_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    if env_model.is_some() || env_openai_base_url.is_some() {
        if let Some(model) = env_model {
            effective_model = model;
        }
        if let Some(openai_base_url) = env_openai_base_url {
            effective_openai_base_url = openai_base_url;
        }
        layers.push(serde_json::json!({
            "source": "env",
            "model": effective_model,
            "openai_base_url": effective_openai_base_url,
        }));
    }

    for event in events {
        if let pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            model,
            openai_base_url,
        } = event.kind
        {
            let ts = event.timestamp.format(&Rfc3339)?;
            effective_approval_policy = approval_policy;
            if let Some(policy) = sandbox_policy {
                effective_sandbox_policy = policy;
            }
            if let Some(model) = model {
                effective_model = model;
            }
            if let Some(openai_base_url) = openai_base_url {
                effective_openai_base_url = openai_base_url;
            }
            layers.push(serde_json::json!({
                "source": "thread",
                "seq": event.seq.0,
                "timestamp": ts,
                "approval_policy": approval_policy,
                "sandbox_policy": effective_sandbox_policy,
                "model": effective_model,
                "openai_base_url": effective_openai_base_url,
            }));
        }
    }

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "effective": {
            "approval_policy": effective_approval_policy,
            "sandbox_policy": effective_sandbox_policy,
            "model": effective_model,
            "openai_base_url": effective_openai_base_url,
        },
        "layers": layers,
    }))
}

async fn handle_thread_fork(server: &Server, params: ThreadForkParams) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let (cwd, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state
                .cwd
                .clone()
                .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", params.thread_id))?,
            state.active_turn_id,
        )
    };

    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut forked = server
        .thread_store
        .create_thread(PathBuf::from(&cwd))
        .await?;
    let forked_id = forked.thread_id();

    for event in events {
        let kind = event.kind;
        match kind {
            pm_protocol::ThreadEventKind::ThreadCreated { .. } => {}
            kind @ pm_protocol::ThreadEventKind::ThreadConfigUpdated { .. } => {
                forked.append(kind).await?;
            }
            pm_protocol::ThreadEventKind::TurnStarted { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::TurnCompleted { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::ApprovalRequested {
                turn_id: Some(turn_id),
                ..
            } if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                ..
            } if active_turn_id == Some(turn_id) => {}
            kind @ pm_protocol::ThreadEventKind::TurnStarted { .. }
            | kind @ pm_protocol::ThreadEventKind::TurnInterruptRequested { .. }
            | kind @ pm_protocol::ThreadEventKind::TurnCompleted { .. }
            | kind @ pm_protocol::ThreadEventKind::ApprovalRequested { .. }
            | kind @ pm_protocol::ThreadEventKind::ApprovalDecided { .. }
            | kind @ pm_protocol::ThreadEventKind::AssistantMessage { .. } => {
                forked.append(kind).await?;
            }
            pm_protocol::ThreadEventKind::ToolStarted { .. }
            | pm_protocol::ThreadEventKind::ToolCompleted { .. }
            | pm_protocol::ThreadEventKind::ProcessStarted { .. }
            | pm_protocol::ThreadEventKind::ProcessKillRequested { .. }
            | pm_protocol::ThreadEventKind::ProcessExited { .. } => {}
        }
    }

    let log_path = forked.log_path().display().to_string();
    let last_seq = forked.last_seq().0;

    let rt = Arc::new(ThreadRuntime::new(forked));
    server.threads.lock().await.insert(forked_id, rt);

    Ok(serde_json::json!({
        "thread_id": forked_id,
        "log_path": log_path,
        "last_seq": last_seq,
    }))
}

async fn handle_thread_delete(
    server: &Server,
    params: ThreadDeleteParams,
) -> anyhow::Result<Value> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);

    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    let mut to_remove = Vec::<ProcessId>::new();
    {
        let entries = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .map(|(process_id, entry)| (*process_id, entry.clone()))
                .collect::<Vec<_>>()
        };
        for (process_id, entry) in entries {
            let info = entry.info.lock().await;
            if info.thread_id != params.thread_id {
                continue;
            }
            to_remove.push(process_id);
            if matches!(info.status, ProcessStatus::Running) {
                running.push(process_id);
                to_kill.push(entry.clone());
            }
        }
    }

    if !running.is_empty() && !params.force {
        anyhow::bail!(
            "refusing to delete thread with running processes (use force=true): {:?}",
            running
        );
    }

    if params.force {
        for entry in to_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("thread deleted".to_string()),
                })
                .await;
        }
    }

    server.threads.lock().await.remove(&params.thread_id);
    {
        let mut entries = server.processes.lock().await;
        for process_id in to_remove {
            entries.remove(&process_id);
        }
    }

    let deleted = match tokio::fs::remove_dir_all(&thread_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", thread_dir.display())),
    };

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "deleted": deleted,
        "thread_dir": thread_dir.display().to_string(),
    }))
}

async fn handle_thread_clear_artifacts(
    server: &Server,
    params: ThreadClearArtifactsParams,
) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    {
        let entries = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .map(|(process_id, entry)| (*process_id, entry.clone()))
                .collect::<Vec<_>>()
        };
        for (process_id, entry) in entries {
            let info = entry.info.lock().await;
            if info.thread_id != params.thread_id {
                continue;
            }
            if matches!(info.status, ProcessStatus::Running) {
                running.push(process_id);
                to_kill.push(entry.clone());
            }
        }
    }

    if !running.is_empty() && !params.force {
        anyhow::bail!(
            "refusing to clear artifacts with running processes (use force=true): {:?}",
            running
        );
    }

    if params.force {
        for entry in to_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("artifacts cleared".to_string()),
                })
                .await;
        }
    }

    let tool_id = pm_protocol::ToolId::new();
    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: None,
            tool: "thread/clear_artifacts".to_string(),
            params: Some(serde_json::json!({
                "force": params.force,
            })),
        })
        .await?;

    let artifacts_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts");
    let removed = match tokio::fs::remove_dir_all(&artifacts_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", artifacts_dir.display())),
    };

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "removed": removed,
                "artifacts_dir": artifacts_dir.display().to_string(),
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "removed": removed,
        "artifacts_dir": artifacts_dir.display().to_string(),
    }))
}

async fn handle_approval_decide(
    server: &Server,
    params: ApprovalDecideParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    rt.append_event(pm_protocol::ThreadEventKind::ApprovalDecided {
        approval_id: params.approval_id,
        decision: params.decision,
        remember: params.remember,
        reason: params.reason,
    })
    .await?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_approval_list(
    server: &Server,
    params: ApprovalListParams,
) -> anyhow::Result<Value> {
    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut requested = BTreeMap::<pm_protocol::ApprovalId, serde_json::Value>::new();
    let mut decided = BTreeMap::<pm_protocol::ApprovalId, serde_json::Value>::new();

    for event in events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action,
                params,
            } => {
                requested.insert(
                    approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "turn_id": turn_id,
                        "action": action,
                        "params": params,
                        "requested_at": ts,
                    }),
                );
            }
            pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember,
                reason,
            } => {
                decided.insert(
                    approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "decision": decision,
                        "remember": remember,
                        "reason": reason,
                        "decided_at": ts,
                    }),
                );
            }
            _ => {}
        }
    }

    let mut approvals = Vec::new();
    for (id, req) in requested {
        if let Some(decision) = decided.get(&id) {
            if params.include_decided {
                approvals.push(serde_json::json!({
                    "request": req,
                    "decision": decision,
                }));
            }
        } else {
            approvals.push(serde_json::json!({
                "request": req,
                "decision": null,
            }));
        }
    }

    Ok(serde_json::json!({ "approvals": approvals }))
}

async fn ensure_approval(
    server: &Server,
    thread_id: ThreadId,
    approval_id: pm_protocol::ApprovalId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<()> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut found_request: Option<(String, serde_json::Value)> = None;
    let mut found_decision: Option<pm_protocol::ApprovalDecision> = None;

    for event in events {
        match event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: got,
                action,
                params,
                ..
            } if got == approval_id => {
                found_request = Some((action, params));
            }
            pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: got,
                decision,
                ..
            } if got == approval_id => {
                found_decision = Some(decision);
            }
            _ => {}
        }
    }

    let Some((action, params)) = found_request else {
        anyhow::bail!("approval not requested: {}", approval_id);
    };
    if action != expected_action {
        anyhow::bail!(
            "approval action mismatch: expected {}, got {}",
            expected_action,
            action
        );
    }
    if &params != expected_params {
        anyhow::bail!("approval params mismatch for {}", approval_id);
    }

    match found_decision {
        Some(pm_protocol::ApprovalDecision::Approved) => Ok(()),
        Some(pm_protocol::ApprovalDecision::Denied) => {
            anyhow::bail!("approval denied: {}", approval_id)
        }
        None => anyhow::bail!("approval not decided: {}", approval_id),
    }
}

fn approval_rule_key(action: &str, params: &serde_json::Value) -> anyhow::Result<String> {
    let obj = params.as_object();
    match action {
        "file/write" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let create_parent_dirs = obj
                .and_then(|o| o.get("create_parent_dirs"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(format!(
                "file/write|path={path}|create_parent_dirs={create_parent_dirs}"
            ))
        }
        "file/delete" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let recursive = obj
                .and_then(|o| o.get("recursive"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(format!("file/delete|path={path}|recursive={recursive}"))
        }
        "fs/mkdir" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let recursive = obj
                .and_then(|o| o.get("recursive"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(format!("fs/mkdir|path={path}|recursive={recursive}"))
        }
        "file/edit" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(format!("file/edit|path={path}"))
        }
        "file/patch" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(format!("file/patch|path={path}"))
        }
        "process/start" => {
            let serialized = serde_json::to_string(params).context("serialize approval params")?;
            Ok(format!("process/start|{serialized}"))
        }
        other => {
            let serialized = serde_json::to_string(params).context("serialize approval params")?;
            Ok(format!("{other}|{serialized}"))
        }
    }
}

async fn remembered_approval_decision(
    server: &Server,
    thread_id: ThreadId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<Option<pm_protocol::ApprovalDecision>> {
    let expected_key = approval_rule_key(expected_action, expected_params)?;
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut requested = HashMap::<pm_protocol::ApprovalId, (String, serde_json::Value)>::new();
    let mut remembered = HashMap::<String, pm_protocol::ApprovalDecision>::new();

    for event in events {
        match event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                action,
                params,
                ..
            } => {
                requested.insert(approval_id, (action, params));
            }
            pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember,
                ..
            } => {
                if !remember {
                    continue;
                }
                let Some((action, params)) = requested.get(&approval_id) else {
                    continue;
                };
                let key = approval_rule_key(action, params)?;
                remembered.insert(key, decision);
            }
            _ => {}
        }
    }

    Ok(remembered.get(&expected_key).copied())
}

async fn load_thread_root(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<(Arc<ThreadRuntime>, PathBuf)> {
    let thread_rt = server.get_or_load_thread(thread_id).await?;
    let thread_cwd = {
        let handle = thread_rt.handle.lock().await;
        handle
            .state()
            .cwd
            .clone()
            .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", thread_id))?
    };
    let thread_root = pm_core::resolve_dir(Path::new(&thread_cwd), Path::new(".")).await?;
    Ok((thread_rt, thread_root))
}

async fn resolve_dir_for_sandbox(
    thread_root: &Path,
    sandbox_policy: pm_protocol::SandboxPolicy,
    input: &Path,
) -> anyhow::Result<PathBuf> {
    match sandbox_policy {
        pm_protocol::SandboxPolicy::DangerFullAccess => {
            pm_core::resolve_dir_unrestricted(thread_root, input).await
        }
        _ => pm_core::resolve_dir(thread_root, input).await,
    }
}

async fn resolve_file_for_sandbox(
    thread_root: &Path,
    sandbox_policy: pm_protocol::SandboxPolicy,
    input: &Path,
    access: pm_core::PathAccess,
    create_parent_dirs: bool,
) -> anyhow::Result<PathBuf> {
    match sandbox_policy {
        pm_protocol::SandboxPolicy::DangerFullAccess => {
            pm_core::resolve_file_unrestricted(thread_root, input, access, create_parent_dirs).await
        }
        _ => pm_core::resolve_file(thread_root, input, access, create_parent_dirs).await,
    }
}

async fn handle_file_read(server: &Server, params: FileReadParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let sandbox_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().sandbox_policy
    };

    let max_bytes = params.max_bytes.unwrap_or(256 * 1024).min(4 * 1024 * 1024);
    let tool_id = pm_protocol::ToolId::new();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/read".to_string(),
            params: Some(serde_json::json!({
                "path": params.path.clone(),
                "max_bytes": max_bytes,
            })),
        })
        .await?;

    let outcome: anyhow::Result<(PathBuf, String, bool, usize)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Read,
            false,
        )
        .await?;

        let limit = max_bytes + 1;
        let file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?;
        let mut buf = Vec::new();
        file.take(limit).read_to_end(&mut buf).await?;

        let truncated = buf.len() > max_bytes as usize;
        if truncated {
            buf.truncate(max_bytes as usize);
        }
        let bytes = buf.len();
        let text = String::from_utf8(buf).context("file is not valid utf-8")?;
        Ok((path, text, truncated, bytes))
    }
    .await;

    match outcome {
        Ok((path, text, truncated, bytes)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "bytes": bytes,
                        "truncated": truncated,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "text": text,
                "truncated": truncated,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

const DEFAULT_IGNORED_DIRS: &[&str] = &[".git", ".code_pm", "target", "node_modules", "example"];

fn should_walk_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !DEFAULT_IGNORED_DIRS.iter().any(|dir| *dir == name)
}

async fn handle_file_glob(server: &Server, params: FileGlobParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_results = params.max_results.unwrap_or(2000).min(20_000);
    let tool_id = pm_protocol::ToolId::new();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/glob".to_string(),
            params: Some(serde_json::json!({
                "pattern": params.pattern.clone(),
                "max_results": max_results,
            })),
        })
        .await?;

    let pattern = params.pattern.clone();
    let root = thread_root.clone();
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<String>, bool)> {
        let matcher = Glob::new(&pattern)
            .with_context(|| format!("invalid glob pattern: {pattern}"))?
            .compile_matcher();

        let mut paths = Vec::new();
        let mut truncated = false;

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_walk_entry)
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
            if matcher.is_match(rel) {
                paths.push(rel.to_string_lossy().to_string());
                if paths.len() >= max_results {
                    truncated = true;
                    break;
                }
            }
        }

        Ok((paths, truncated))
    })
    .await
    .context("join glob task")?;

    match outcome {
        Ok((paths, truncated)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "matches": paths.len(),
                        "truncated": truncated,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "paths": paths,
                "truncated": truncated,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

#[derive(Debug, Serialize)]
struct GrepMatch {
    path: String,
    line_number: u64,
    line: String,
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in line.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

async fn handle_file_grep(server: &Server, params: FileGrepParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_matches = params.max_matches.unwrap_or(200).min(2000);
    let max_bytes_per_file = params
        .max_bytes_per_file
        .unwrap_or(1024 * 1024)
        .min(16 * 1024 * 1024);
    let max_files = params.max_files.unwrap_or(20_000).min(200_000);
    let tool_id = pm_protocol::ToolId::new();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/grep".to_string(),
            params: Some(serde_json::json!({
                "query": params.query.clone(),
                "is_regex": params.is_regex,
                "include_glob": params.include_glob,
                "max_matches": max_matches,
                "max_bytes_per_file": max_bytes_per_file,
                "max_files": max_files,
            })),
        })
        .await?;

    let pattern = if params.is_regex {
        params.query.clone()
    } else {
        regex::escape(&params.query)
    };
    let re = Regex::new(&pattern).with_context(|| format!("invalid regex: {}", params.query))?;
    let include_matcher = match params.include_glob.as_deref() {
        Some(glob) => Some(
            Glob::new(glob)
                .with_context(|| format!("invalid glob pattern: {glob}"))?
                .compile_matcher(),
        ),
        None => None,
    };

    let root = thread_root.clone();
    let outcome = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(Vec<GrepMatch>, bool, usize, usize, usize)> {
            let mut matches = Vec::new();
            let mut truncated = false;
            let mut files_scanned = 0usize;
            let mut files_skipped_too_large = 0usize;
            let mut files_skipped_binary = 0usize;

            for entry in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(should_walk_entry)
            {
                let entry = entry?;
                if !entry.file_type().is_file() {
                    continue;
                }
                if files_scanned >= max_files {
                    break;
                }
                let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                if let Some(ref matcher) = include_matcher {
                    if !matcher.is_match(rel) {
                        continue;
                    }
                }

                files_scanned += 1;

                let meta = entry.metadata()?;
                if meta.len() > max_bytes_per_file {
                    files_skipped_too_large += 1;
                    continue;
                }

                let bytes = match std::fs::read(entry.path()) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                if bytes.iter().any(|b| *b == 0) {
                    files_skipped_binary += 1;
                    continue;
                }

                let text = String::from_utf8_lossy(&bytes);
                for (idx, line) in text.lines().enumerate() {
                    if !re.is_match(line) {
                        continue;
                    }

                    matches.push(GrepMatch {
                        path: rel.to_string_lossy().to_string(),
                        line_number: (idx + 1) as u64,
                        line: truncate_line(line, 4000),
                    });
                    if matches.len() >= max_matches {
                        truncated = true;
                        break;
                    }
                }

                if truncated {
                    break;
                }
            }

            Ok((
                matches,
                truncated,
                files_scanned,
                files_skipped_too_large,
                files_skipped_binary,
            ))
        },
    )
    .await
    .context("join grep task")?;

    match outcome {
        Ok((matches, truncated, files_scanned, files_skipped_too_large, files_skipped_binary)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "matches": matches.len(),
                        "truncated": truncated,
                        "files_scanned": files_scanned,
                        "files_skipped_too_large": files_skipped_too_large,
                        "files_skipped_binary": files_skipped_binary,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "matches": matches,
                "truncated": truncated,
                "files_scanned": files_scanned,
                "files_skipped_too_large": files_skipped_too_large,
                "files_skipped_binary": files_skipped_binary,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_file_write(server: &Server, params: FileWriteParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let create_parent_dirs = params.create_parent_dirs.unwrap_or(true);
    let (approval_policy, sandbox_policy) = {
        let handle = thread_rt.handle.lock().await;
        (
            handle.state().approval_policy,
            handle.state().sandbox_policy,
        )
    };
    let tool_id = pm_protocol::ToolId::new();
    let bytes = params.text.as_bytes().len();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "bytes": bytes,
        "create_parent_dirs": create_parent_dirs,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/write".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/write".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }
    if approval_policy == pm_protocol::ApprovalPolicy::Manual {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "file/write",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "file/write",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "file/write".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "file/write".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/write".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let outcome: anyhow::Result<PathBuf> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            create_parent_dirs,
        )
        .await?;

        tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?
            .write_all(params.text.as_bytes())
            .await
            .with_context(|| format!("write {}", path.display()))?;

        Ok(path)
    }
    .await;

    match outcome {
        Ok(path) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({ "bytes": bytes })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "bytes_written": bytes,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_file_patch(server: &Server, params: FilePatchParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_bytes = params
        .max_bytes
        .unwrap_or(4 * 1024 * 1024)
        .min(16 * 1024 * 1024);
    let patch_bytes = params.patch.as_bytes().len();

    let (approval_policy, sandbox_policy) = {
        let handle = thread_rt.handle.lock().await;
        (
            handle.state().approval_policy,
            handle.state().sandbox_policy,
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "patch_bytes": patch_bytes,
        "max_bytes": max_bytes,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/patch".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/patch".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }
    if approval_policy == pm_protocol::ApprovalPolicy::Manual {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "file/patch",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "file/patch",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "file/patch".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "file/patch".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/patch".to_string(),
            params: Some(serde_json::json!({
                "path": params.path.clone(),
                "patch_bytes": patch_bytes,
                "max_bytes": max_bytes,
            })),
        })
        .await?;

    let outcome: anyhow::Result<(PathBuf, bool, usize, usize)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            false,
        )
        .await?;

        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        if bytes.len() > max_bytes as usize {
            anyhow::bail!(
                "file too large for patch: {} ({} bytes)",
                path.display(),
                bytes.len()
            );
        }

        let original = String::from_utf8(bytes).context("file is not valid utf-8")?;
        let patch = Patch::from_str(&params.patch).context("parse unified diff patch")?;
        let updated = apply(&original, &patch).context("apply patch")?;
        let changed = updated != original;
        let bytes_written = updated.as_bytes().len();
        if bytes_written > max_bytes as usize {
            anyhow::bail!(
                "patched file too large: {} ({} bytes)",
                path.display(),
                bytes_written
            );
        }

        tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?
            .write_all(updated.as_bytes())
            .await
            .with_context(|| format!("write {}", path.display()))?;

        Ok((path, changed, patch_bytes, bytes_written))
    }
    .await;

    match outcome {
        Ok((path, changed, patch_bytes, bytes_written)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "changed": changed,
                        "patch_bytes": patch_bytes,
                        "bytes": bytes_written,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "changed": changed,
                "patch_bytes": patch_bytes,
                "bytes_written": bytes_written,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

fn count_non_overlapping(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }

    let mut count = 0usize;
    let mut rest = haystack;
    while let Some(pos) = rest.find(needle) {
        count += 1;
        rest = &rest[(pos + needle.len())..];
    }
    count
}

async fn handle_file_edit(server: &Server, params: FileEditParams) -> anyhow::Result<Value> {
    if params.edits.is_empty() {
        anyhow::bail!("edits must not be empty");
    }
    if params.edits.iter().any(|e| e.old.is_empty()) {
        anyhow::bail!("edit.old must not be empty");
    }

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_bytes = params
        .max_bytes
        .unwrap_or(4 * 1024 * 1024)
        .min(16 * 1024 * 1024);

    let (approval_policy, sandbox_policy) = {
        let handle = thread_rt.handle.lock().await;
        (
            handle.state().approval_policy,
            handle.state().sandbox_policy,
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "edits": params.edits.len(),
        "max_bytes": max_bytes,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/edit".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/edit".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }
    if approval_policy == pm_protocol::ApprovalPolicy::Manual {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "file/edit",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "file/edit",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "file/edit".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "file/edit".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/edit".to_string(),
            params: Some(serde_json::json!({
                "path": params.path.clone(),
                "edits": params.edits.len(),
                "max_bytes": max_bytes,
            })),
        })
        .await?;

    let outcome: anyhow::Result<(PathBuf, bool, usize, usize)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            false,
        )
        .await?;

        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        if bytes.len() > max_bytes as usize {
            anyhow::bail!(
                "file too large for edit: {} ({} bytes)",
                path.display(),
                bytes.len()
            );
        }
        let mut text = String::from_utf8(bytes).context("file is not valid utf-8")?;

        let mut total_replacements = 0usize;
        let mut changed = false;
        for edit in &params.edits {
            let expected = edit.expected_replacements.unwrap_or(1);
            let found = count_non_overlapping(&text, &edit.old);
            if found != expected {
                anyhow::bail!(
                    "edit mismatch for {}: expected {} replacements, found {}",
                    path.display(),
                    expected,
                    found
                );
            }
            if edit.old != edit.new {
                changed = true;
            }
            total_replacements += expected;
            text = text.replacen(&edit.old, &edit.new, expected);
        }

        let bytes_written = text.as_bytes().len();
        tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?
            .write_all(text.as_bytes())
            .await
            .with_context(|| format!("write {}", path.display()))?;

        Ok((path, changed, total_replacements, bytes_written))
    }
    .await;

    match outcome {
        Ok((path, changed, replacements, bytes_written)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "changed": changed,
                        "replacements": replacements,
                        "bytes": bytes_written,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "changed": changed,
                "replacements": replacements,
                "bytes_written": bytes_written,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_file_delete(server: &Server, params: FileDeleteParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let (approval_policy, sandbox_policy) = {
        let handle = thread_rt.handle.lock().await;
        (
            handle.state().approval_policy,
            handle.state().sandbox_policy,
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/delete".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/delete".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }
    if approval_policy == pm_protocol::ApprovalPolicy::Manual {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "file/delete",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "file/delete",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "file/delete".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "file/delete".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/delete".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let thread_root = thread_root.clone();
    let outcome: anyhow::Result<(bool, PathBuf)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            false,
        )
        .await?;

        if path == thread_root {
            anyhow::bail!("refusing to delete thread root: {}", path.display());
        }

        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((false, path)),
            Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
        };

        if meta.is_dir() {
            if params.recursive {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .with_context(|| format!("remove dir {}", path.display()))?;
            } else {
                tokio::fs::remove_dir(&path)
                    .await
                    .with_context(|| format!("remove dir {}", path.display()))?;
            }
        } else {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("remove file {}", path.display()))?;
        }

        Ok((true, path))
    }
    .await;

    match outcome {
        Ok((deleted, path)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "deleted": deleted,
                        "path": path.display().to_string(),
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "deleted": deleted,
                "resolved_path": path.display().to_string(),
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_fs_mkdir(server: &Server, params: FsMkdirParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let (approval_policy, sandbox_policy) = {
        let handle = thread_rt.handle.lock().await;
        (
            handle.state().approval_policy,
            handle.state().sandbox_policy,
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "fs/mkdir".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids fs/mkdir".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }
    if approval_policy == pm_protocol::ApprovalPolicy::Manual {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "fs/mkdir",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "fs/mkdir",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "fs/mkdir".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "fs/mkdir".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "fs/mkdir".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let thread_root = thread_root.clone();
    let outcome: anyhow::Result<(bool, PathBuf)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            params.recursive,
        )
        .await?;

        if path == thread_root {
            anyhow::bail!("refusing to create thread root: {}", path.display());
        }

        match tokio::fs::create_dir(&path).await {
            Ok(()) => Ok((true, path)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let meta = tokio::fs::metadata(&path)
                    .await
                    .with_context(|| format!("stat {}", path.display()))?;
                if meta.is_dir() {
                    Ok((false, path))
                } else {
                    anyhow::bail!("path exists and is not a directory: {}", path.display());
                }
            }
            Err(err) => Err(err).with_context(|| format!("create dir {}", path.display())),
        }
    }
    .await;

    match outcome {
        Ok((created, path)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "created": created,
                        "path": path.display().to_string(),
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "created": created,
                "resolved_path": path.display().to_string(),
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

fn user_artifacts_dir_for_thread(server: &Server, thread_id: ThreadId) -> PathBuf {
    server
        .thread_store
        .thread_dir(thread_id)
        .join("artifacts")
        .join("user")
}

fn user_artifact_paths(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
) -> (PathBuf, PathBuf) {
    let dir = user_artifacts_dir_for_thread(server, thread_id);
    (
        dir.join(format!("{artifact_id}.md")),
        dir.join(format!("{artifact_id}.metadata.json")),
    )
}

async fn read_artifact_metadata(path: &Path) -> anyhow::Result<ArtifactMetadata> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let meta = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse artifact metadata {}", path.display()))?;
    Ok(meta)
}

async fn write_file_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create dir {}", parent.display()))?;

    let pid = std::process::id();
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let tmp_path = path.with_extension(format!("tmp.{pid}.{nanos}"));

    tokio::fs::write(&tmp_path, bytes)
        .await
        .with_context(|| format!("write {}", tmp_path.display()))?;

    if let Err(err) = tokio::fs::rename(&tmp_path, path).await {
        if matches!(
            err.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
        ) {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(remove_err) if remove_err.kind() == std::io::ErrorKind::NotFound => {}
                Err(remove_err) => {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                    return Err(remove_err)
                        .with_context(|| format!("remove old {}", path.display()));
                }
            }
            if let Err(rename_err) = tokio::fs::rename(&tmp_path, path).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(rename_err).with_context(|| {
                    format!("rename {} -> {}", tmp_path.display(), path.display())
                });
            }
        } else {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(err)
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
        }
    }

    Ok(())
}

async fn handle_artifact_write(
    server: &Server,
    params: ArtifactWriteParams,
) -> anyhow::Result<Value> {
    if params.artifact_type.trim().is_empty() {
        anyhow::bail!("artifact_type must not be empty");
    }
    if params.summary.trim().is_empty() {
        anyhow::bail!("summary must not be empty");
    }

    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let tool_id = pm_protocol::ToolId::new();
    let bytes_len = params.text.as_bytes().len();
    let artifact_type = params.artifact_type.clone();
    let summary = params.summary.clone();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/write".to_string(),
            params: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
                "artifact_type": artifact_type,
                "summary": summary,
                "bytes": bytes_len,
            })),
        })
        .await?;

    let artifact_id = params.artifact_id.unwrap_or_else(ArtifactId::new);
    let (content_path, metadata_path) = user_artifact_paths(server, params.thread_id, artifact_id);

    let now = OffsetDateTime::now_utc();
    let (created_at, version, created) = match tokio::fs::metadata(&metadata_path).await {
        Ok(_) => {
            let meta = read_artifact_metadata(&metadata_path).await?;
            (meta.created_at, meta.version.saturating_add(1), false)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (now, 1, true),
        Err(err) => return Err(err).with_context(|| format!("stat {}", metadata_path.display())),
    };

    let text = pm_core::redact_text(&params.text);
    let bytes = text.as_bytes().to_vec();
    write_file_atomic(&content_path, &bytes).await?;

    let meta = ArtifactMetadata {
        artifact_id,
        artifact_type: params.artifact_type,
        summary: params.summary,
        created_at,
        updated_at: now,
        version,
        content_path: content_path.display().to_string(),
        size_bytes: bytes.len() as u64,
        provenance: Some(ArtifactProvenance {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            tool_id: Some(tool_id),
            process_id: None,
        }),
    };

    let meta_bytes = serde_json::to_vec_pretty(&meta).context("serialize artifact metadata")?;
    write_file_atomic(&metadata_path, &meta_bytes).await?;

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "artifact_id": artifact_id,
                "created": created,
                "content_path": content_path.display().to_string(),
                "metadata_path": metadata_path.display().to_string(),
                "version": version,
                "size_bytes": bytes.len(),
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "artifact_id": artifact_id,
        "created": created,
        "content_path": content_path.display().to_string(),
        "metadata_path": metadata_path.display().to_string(),
        "metadata": meta,
    }))
}

async fn handle_artifact_list(
    server: &Server,
    params: ArtifactListParams,
) -> anyhow::Result<Value> {
    let dir = user_artifacts_dir_for_thread(server, params.thread_id);
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::json!({
                "artifacts": [],
                "errors": [],
            }));
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };

    let mut artifacts = Vec::<ArtifactMetadata>::new();
    let mut errors = Vec::<Value>::new();

    while let Some(entry) = read_dir.next_entry().await? {
        let ty = entry.file_type().await?;
        if !ty.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".metadata.json") {
            continue;
        }
        match read_artifact_metadata(&path).await {
            Ok(meta) => artifacts.push(meta),
            Err(err) => errors.push(serde_json::json!({
                "path": path.display().to_string(),
                "error": err.to_string(),
            })),
        }
    }

    artifacts.sort_by(|a, b| {
        b.updated_at
            .unix_timestamp_nanos()
            .cmp(&a.updated_at.unix_timestamp_nanos())
            .then_with(|| b.artifact_id.cmp(&a.artifact_id))
    });

    Ok(serde_json::json!({
        "artifacts": artifacts,
        "errors": errors,
    }))
}

async fn handle_artifact_read(
    server: &Server,
    params: ArtifactReadParams,
) -> anyhow::Result<Value> {
    let max_bytes = params.max_bytes.unwrap_or(256 * 1024).min(4 * 1024 * 1024);
    let (content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let meta = read_artifact_metadata(&metadata_path).await?;

    let bytes = tokio::fs::read(&content_path)
        .await
        .with_context(|| format!("read {}", content_path.display()))?;
    let truncated = bytes.len() > max_bytes as usize;
    let bytes = if truncated {
        bytes[..(max_bytes as usize)].to_vec()
    } else {
        bytes
    };
    let text = String::from_utf8_lossy(&bytes).to_string();
    let text = pm_core::redact_text(&text);

    Ok(serde_json::json!({
        "metadata": meta,
        "text": text,
        "truncated": truncated,
        "bytes": bytes.len(),
    }))
}

async fn handle_artifact_delete(
    server: &Server,
    params: ArtifactDeleteParams,
) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let tool_id = pm_protocol::ToolId::new();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/delete".to_string(),
            params: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
            })),
        })
        .await?;

    let (content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let mut removed = false;
    for path in [&content_path, &metadata_path] {
        match tokio::fs::remove_file(path).await {
            Ok(()) => removed = true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
                "removed": removed,
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "removed": removed,
    }))
}

async fn handle_process_list(
    server: &Server,
    params: ProcessListParams,
) -> anyhow::Result<Vec<ProcessInfo>> {
    let thread_ids = if let Some(thread_id) = params.thread_id {
        vec![thread_id]
    } else {
        server.thread_store.list_threads().await?
    };

    for thread_id in &thread_ids {
        server.get_or_load_thread(*thread_id).await?;
    }

    let mut derived = HashMap::<ProcessId, ProcessInfo>::new();
    for thread_id in &thread_ids {
        let events = server
            .thread_store
            .read_events_since(*thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

        for event in events {
            let ts = event.timestamp.format(&Rfc3339)?;
            match event.kind {
                pm_protocol::ThreadEventKind::ProcessStarted {
                    process_id,
                    turn_id,
                    argv,
                    cwd,
                    stdout_path,
                    stderr_path,
                } => {
                    derived.insert(
                        process_id,
                        ProcessInfo {
                            process_id,
                            thread_id: event.thread_id,
                            turn_id,
                            argv,
                            cwd,
                            started_at: ts.clone(),
                            status: ProcessStatus::Running,
                            exit_code: None,
                            stdout_path,
                            stderr_path,
                            last_update_at: ts,
                        },
                    );
                }
                pm_protocol::ThreadEventKind::ProcessKillRequested { process_id, .. } => {
                    if let Some(info) = derived.get_mut(&process_id) {
                        info.last_update_at = ts;
                    }
                }
                pm_protocol::ThreadEventKind::ProcessExited {
                    process_id,
                    exit_code,
                    ..
                } => {
                    if let Some(info) = derived.get_mut(&process_id) {
                        info.status = ProcessStatus::Exited;
                        info.exit_code = exit_code;
                        info.last_update_at = ts;
                    }
                }
                _ => {}
            }
        }
    }

    let mut in_mem_running = HashSet::<ProcessId>::new();
    {
        let entries = server.processes.lock().await;
        for entry in entries.values() {
            let info = entry.info.lock().await;
            if params.thread_id.is_some_and(|id| id != info.thread_id) {
                continue;
            }
            if matches!(info.status, ProcessStatus::Running) {
                in_mem_running.insert(info.process_id);
            }
            derived.insert(info.process_id, info.clone());
        }
    }

    for info in derived.values_mut() {
        if matches!(info.status, ProcessStatus::Running)
            && !in_mem_running.contains(&info.process_id)
        {
            info.status = ProcessStatus::Abandoned;
        }
    }

    let mut out = derived.into_values().collect::<Vec<_>>();
    out.sort_by(|a, b| {
        a.thread_id
            .cmp(&b.thread_id)
            .then_with(|| a.process_id.cmp(&b.process_id))
    });
    Ok(out)
}

async fn kill_processes_for_turn(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    reason: Option<String>,
) {
    let entries = {
        let entries = server.processes.lock().await;
        entries.values().cloned().collect::<Vec<_>>()
    };

    for entry in entries {
        let should_kill = {
            let info = entry.info.lock().await;
            info.thread_id == thread_id
                && info.turn_id == Some(turn_id)
                && matches!(info.status, ProcessStatus::Running)
        };
        if should_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: reason.clone(),
                })
                .await;
        }
    }
}

async fn handle_process_start(
    server: &Server,
    params: ProcessStartParams,
) -> anyhow::Result<Value> {
    if params.argv.is_empty() {
        anyhow::bail!("argv must not be empty");
    }

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, sandbox_policy) = {
        let handle = thread_rt.handle.lock().await;
        (
            handle.state().approval_policy,
            handle.state().sandbox_policy,
        )
    };

    let cwd_path = if let Some(cwd) = params.cwd.as_deref() {
        resolve_dir_for_sandbox(&thread_root, sandbox_policy, Path::new(cwd)).await?
    } else {
        thread_root.clone()
    };
    let cwd_str = cwd_path.display().to_string();

    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        let tool_id = pm_protocol::ToolId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/start".to_string(),
                params: Some(serde_json::json!({
                    "argv": params.argv.clone(),
                    "cwd": cwd_str,
                })),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids process/start".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let exec_matches = server.exec_policy.matches_for_command(&params.argv, None);
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();

    let effective_decision = match (approval_policy, exec_decision) {
        (_, Some(ExecDecision::Forbidden)) => ExecDecision::Forbidden,
        (pm_protocol::ApprovalPolicy::Manual, Some(ExecDecision::Prompt)) => ExecDecision::Prompt,
        (pm_protocol::ApprovalPolicy::Manual, Some(ExecDecision::Allow)) => ExecDecision::Allow,
        (pm_protocol::ApprovalPolicy::Manual, None) => ExecDecision::Prompt,
        (pm_protocol::ApprovalPolicy::AutoApprove, _) => ExecDecision::Allow,
    };

    if effective_decision == ExecDecision::Forbidden {
        let tool_id = pm_protocol::ToolId::new();
        let exec_matches_json = serde_json::to_value(&exec_matches)?;

        let justification = exec_matches.iter().find_map(|m| match m {
            ExecRuleMatch::PrefixRuleMatch {
                decision: ExecDecision::Forbidden,
                justification,
                ..
            } => justification.clone(),
            _ => None,
        });

        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/start".to_string(),
                params: Some(serde_json::json!({
                    "argv": params.argv,
                    "cwd": cwd_str,
                })),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("execpolicy forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "decision": ExecDecision::Forbidden,
                    "matched_rules": exec_matches_json,
                    "justification": justification,
                })),
            })
            .await?;

        return Ok(serde_json::json!({
            "denied": true,
            "decision": ExecDecision::Forbidden,
            "matched_rules": exec_matches_json,
            "justification": justification,
        }));
    }

    let approval_params = serde_json::json!({
        "argv": params.argv.clone(),
        "cwd": cwd_str.clone(),
    });
    if approval_policy == pm_protocol::ApprovalPolicy::Manual
        && effective_decision == ExecDecision::Prompt
    {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "process/start",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "process/start",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        let tool_id = pm_protocol::ToolId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "process/start".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "process/start".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    let process_id = ProcessId::new();
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let process_dir = thread_dir
        .join("artifacts")
        .join("processes")
        .join(process_id.to_string());
    tokio::fs::create_dir_all(&process_dir)
        .await
        .with_context(|| format!("create dir {}", process_dir.display()))?;

    let stdout_path = process_dir.join("stdout.log");
    let stderr_path = process_dir.join("stderr.log");

    let mut cmd = Command::new(&params.argv[0]);
    cmd.args(params.argv.iter().skip(1));
    cmd.current_dir(&cwd_path);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {:?}", params.argv))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let stdout_task = if let Some(mut stdout) = stdout {
        let stdout_path = stdout_path.clone();
        Some(tokio::spawn(async move {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&stdout_path)
                .await?;
            tokio::io::copy(&mut stdout, &mut file).await?;
            anyhow::Ok(())
        }))
    } else {
        None
    };

    let stderr_task = if let Some(mut stderr) = stderr {
        let stderr_path = stderr_path.clone();
        Some(tokio::spawn(async move {
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&stderr_path)
                .await?;
            tokio::io::copy(&mut stderr, &mut file).await?;
            anyhow::Ok(())
        }))
    } else {
        None
    };

    let started = thread_rt
        .append_event(pm_protocol::ThreadEventKind::ProcessStarted {
            process_id,
            turn_id: params.turn_id,
            argv: params.argv.clone(),
            cwd: cwd_str.clone(),
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
        })
        .await?;
    let started_at = started.timestamp.format(&Rfc3339)?;

    let info = ProcessInfo {
        process_id,
        thread_id: params.thread_id,
        turn_id: params.turn_id,
        argv: params.argv.clone(),
        cwd: cwd_str,
        started_at: started_at.clone(),
        status: ProcessStatus::Running,
        exit_code: None,
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        last_update_at: started_at,
    };

    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let entry = ProcessEntry {
        info: Arc::new(tokio::sync::Mutex::new(info)),
        cmd_tx,
    };
    server
        .processes
        .lock()
        .await
        .insert(process_id, entry.clone());

    tokio::spawn(run_process_actor(
        thread_rt,
        process_id,
        child,
        cmd_rx,
        stdout_task,
        stderr_task,
        entry.info.clone(),
    ));

    Ok(serde_json::json!({
        "process_id": process_id,
        "stdout_path": stdout_path.display().to_string(),
        "stderr_path": stderr_path.display().to_string(),
    }))
}

async fn run_process_actor(
    thread_rt: Arc<ThreadRuntime>,
    process_id: ProcessId,
    mut child: tokio::process::Child,
    mut cmd_rx: mpsc::Receiver<ProcessCommand>,
    stdout_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    stderr_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
) {
    let mut kill_reason: Option<String> = None;
    let mut kill_logged = false;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { /* sender dropped */ return; };
                match cmd {
                    ProcessCommand::Kill { reason } => {
                        if kill_reason.is_none() {
                            kill_reason = reason;
                        }
                        if !kill_logged {
                            let _ = thread_rt.append_event(pm_protocol::ThreadEventKind::ProcessKillRequested {
                                process_id,
                                reason: kill_reason.clone(),
                            }).await;
                            kill_logged = true;
                        }
                        let _ = child.start_kill();
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(task) = stdout_task {
                    let _ = task.await;
                }
                if let Some(task) = stderr_task {
                    let _ = task.await;
                }

                let exit_code = status.code();
                let exited = thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ProcessExited {
                        process_id,
                        exit_code,
                        reason: kill_reason.clone(),
                    })
                    .await;

                if let Ok(event) = exited {
                    if let Ok(ts) = event.timestamp.format(&Rfc3339) {
                        let mut info = info.lock().await;
                        info.status = ProcessStatus::Exited;
                        info.exit_code = exit_code;
                        info.last_update_at = ts;
                    }
                }
                return;
            }
            Ok(None) => {}
            Err(_) => return,
        }
    }
}

async fn handle_process_tail(server: &Server, params: ProcessTailParams) -> anyhow::Result<Value> {
    let (stdout_path, stderr_path) = resolve_process_log_paths(server, params.process_id).await?;

    let path = match params.stream {
        ProcessStream::Stdout => stdout_path,
        ProcessStream::Stderr => stderr_path,
    };

    let max_lines = params.max_lines.unwrap_or(200).min(2000);
    let text = tail_file_lines(PathBuf::from(path), max_lines).await?;
    let text = pm_core::redact_text(&text);
    Ok(serde_json::json!({ "text": text }))
}

async fn tail_file_lines(path: PathBuf, max_lines: usize) -> anyhow::Result<String> {
    let max_bytes: u64 = 64 * 1024;
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };
    let len = file
        .metadata()
        .await
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))
        .await
        .with_context(|| format!("seek {}", path.display()))?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .with_context(|| format!("read {}", path.display()))?;

    let mut text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            text = text[(pos + 1)..].to_string();
        }
    }

    let lines = text.lines().collect::<Vec<_>>();
    let start_line = lines.len().saturating_sub(max_lines);
    Ok(lines[start_line..].join("\n"))
}

async fn handle_process_follow(
    server: &Server,
    params: ProcessFollowParams,
) -> anyhow::Result<Value> {
    let (stdout_path, stderr_path) = resolve_process_log_paths(server, params.process_id).await?;

    let path = match params.stream {
        ProcessStream::Stdout => stdout_path,
        ProcessStream::Stderr => stderr_path,
    };

    let max_bytes = params.max_bytes.unwrap_or(64 * 1024).min(1024 * 1024);
    let (text, next_offset, eof) =
        read_file_chunk(PathBuf::from(path), params.since_offset, max_bytes).await?;
    let text = pm_core::redact_text(&text);

    Ok(serde_json::json!({
        "text": text,
        "next_offset": next_offset,
        "eof": eof,
    }))
}

async fn resolve_process_log_paths(
    server: &Server,
    process_id: ProcessId,
) -> anyhow::Result<(String, String)> {
    let entry = server.processes.lock().await.get(&process_id).cloned();

    if let Some(entry) = entry {
        let info = entry.info.lock().await;
        return Ok((info.stdout_path.clone(), info.stderr_path.clone()));
    }

    let mut processes = handle_process_list(server, ProcessListParams { thread_id: None }).await?;
    let info = processes
        .iter_mut()
        .find(|p| p.process_id == process_id)
        .ok_or_else(|| anyhow::anyhow!("process not found: {}", process_id))?;
    Ok((info.stdout_path.clone(), info.stderr_path.clone()))
}

async fn read_file_chunk(
    path: PathBuf,
    since_offset: u64,
    max_bytes: u64,
) -> anyhow::Result<(String, u64, bool)> {
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok((String::new(), since_offset, true));
        }
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };
    let len = file
        .metadata()
        .await
        .with_context(|| format!("stat {}", path.display()))?
        .len();

    let start = since_offset.min(len);
    file.seek(SeekFrom::Start(start))
        .await
        .with_context(|| format!("seek {}", path.display()))?;

    let max_bytes = max_bytes.min(1024 * 1024);
    let buf_len = usize::try_from(max_bytes).unwrap_or(1024 * 1024);
    let mut buf = vec![0u8; buf_len];
    let n = file
        .read(&mut buf)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    buf.truncate(n);

    let text = String::from_utf8_lossy(&buf).to_string();
    let next_offset = start + n as u64;
    let eof = next_offset >= len;
    Ok((text, next_offset, eof))
}

async fn handle_process_inspect(
    server: &Server,
    params: ProcessInspectParams,
) -> anyhow::Result<Value> {
    let mut info: Option<ProcessInfo> = None;
    if let Some(entry) = server
        .processes
        .lock()
        .await
        .get(&params.process_id)
        .cloned()
    {
        info = Some(entry.info.lock().await.clone());
    }

    let info = match info {
        Some(info) => info,
        None => {
            let processes =
                handle_process_list(server, ProcessListParams { thread_id: None }).await?;
            processes
                .into_iter()
                .find(|p| p.process_id == params.process_id)
                .ok_or_else(|| anyhow::anyhow!("process not found: {}", params.process_id))?
        }
    };

    let max_lines = params.max_lines.unwrap_or(200).min(2000);
    let stdout_tail =
        pm_core::redact_text(&tail_file_lines(PathBuf::from(&info.stdout_path), max_lines).await?);
    let stderr_tail =
        pm_core::redact_text(&tail_file_lines(PathBuf::from(&info.stderr_path), max_lines).await?);

    Ok(serde_json::json!({
        "process": info,
        "stdout_tail": stdout_tail,
        "stderr_tail": stderr_tail,
    }))
}
