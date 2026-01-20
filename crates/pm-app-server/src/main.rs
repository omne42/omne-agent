use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use globset::Glob;
use pm_core::{PmPaths, ThreadStore};
use pm_execpolicy::{Decision as ExecDecision, RuleMatch as ExecRuleMatch};
use pm_protocol::{EventSeq, ProcessId, ThreadEvent, ThreadId, TurnId, TurnStatus};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use walkdir::WalkDir;

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

    async fn start_turn(self: Arc<Self>, input: String) -> anyhow::Result<TurnId> {
        let mut handle = self.handle.lock().await;
        if handle.state().active_turn_id.is_some() {
            anyhow::bail!("turn already active");
        }

        let turn_id = TurnId::new();
        handle
            .append(pm_protocol::ThreadEventKind::TurnStarted { turn_id, input })
            .await?;
        drop(handle);

        let cancel = CancellationToken::new();
        {
            let mut active = self.active_turn.lock().await;
            *active = Some(ActiveTurn {
                turn_id,
                cancel: cancel.clone(),
            });
        }

        tokio::spawn(async move {
            self.run_turn(turn_id, cancel).await;
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
            let active = self.active_turn.lock().await;
            let Some(active) = active.as_ref() else {
                anyhow::bail!("no active turn");
            };
            if active.turn_id != turn_id {
                anyhow::bail!("turn is not active");
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

    async fn run_turn(self: Arc<Self>, turn_id: TurnId, cancel: CancellationToken) {
        let status = tokio::select! {
            _ = cancel.cancelled() => TurnStatus::Interrupted,
            _ = tokio::time::sleep(Duration::from_secs(1)) => TurnStatus::Completed,
        };

        let mut handle = self.handle.lock().await;
        let _ = handle
            .append(pm_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status,
                reason: None,
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
struct ThreadStateParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadConfigureParams {
    thread_id: ThreadId,
    approval_policy: pm_protocol::ApprovalPolicy,
}

#[derive(Debug, Deserialize)]
struct ThreadEventsParams {
    thread_id: ThreadId,
    #[serde(default)]
    since_seq: u64,
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
struct ApprovalDecideParams {
    thread_id: ThreadId,
    approval_id: pm_protocol::ApprovalId,
    decision: pm_protocol::ApprovalDecision,
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

    let server = Server {
        cwd,
        thread_store: ThreadStore::new(PmPaths::new(pm_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        exec_policy,
    };

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
            "thread/list" => match server.thread_store.list_threads().await {
                Ok(threads) => JsonRpcResponse::ok(
                    id,
                    serde_json::json!({
                        "threads": threads,
                    }),
                ),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            "thread/events" => match serde_json::from_value::<ThreadEventsParams>(request.params) {
                Ok(params) => {
                    let since = EventSeq(params.since_seq);
                    match server
                        .thread_store
                        .read_events_since(params.thread_id, since)
                        .await
                    {
                        Ok(Some(events)) => {
                            let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);
                            JsonRpcResponse::ok(
                                id,
                                serde_json::json!({
                                    "events": events,
                                    "last_seq": last_seq,
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
                                "last_seq": handle.last_seq().0,
                                "active_turn_id": state.active_turn_id,
                                "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
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
            "turn/start" => match serde_json::from_value::<TurnStartParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => match rt.start_turn(params.input).await {
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
                        match rt.interrupt_turn(params.turn_id, params.reason).await {
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
}

async fn handle_thread_configure(
    server: &Server,
    params: ThreadConfigureParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    rt.append_event(pm_protocol::ThreadEventKind::ThreadConfigUpdated {
        approval_policy: params.approval_policy,
    })
    .await?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_approval_decide(
    server: &Server,
    params: ApprovalDecideParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    rt.append_event(pm_protocol::ThreadEventKind::ApprovalDecided {
        approval_id: params.approval_id,
        decision: params.decision,
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
                reason,
            } => {
                decided.insert(
                    approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "decision": decision,
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

async fn handle_file_read(server: &Server, params: FileReadParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

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
        let path = pm_core::resolve_file(
            &thread_root,
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
    let approval_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().approval_policy
    };
    let tool_id = pm_protocol::ToolId::new();
    let bytes = params.text.as_bytes().len();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "bytes": bytes,
        "create_parent_dirs": create_parent_dirs,
    });
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

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/write".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let outcome: anyhow::Result<PathBuf> = async {
        let path = pm_core::resolve_file(
            &thread_root,
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

    let approval_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().approval_policy
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "edits": params.edits.len(),
        "max_bytes": max_bytes,
    });
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
        let path = pm_core::resolve_file(
            &thread_root,
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

    let approval_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().approval_policy
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
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
        let path = pm_core::resolve_file(
            &thread_root,
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

    let approval_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().approval_policy
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
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
        let path = pm_core::resolve_file(
            &thread_root,
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
    let approval_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().approval_policy
    };

    let cwd_path = if let Some(cwd) = params.cwd.as_deref() {
        pm_core::resolve_dir(&thread_root, Path::new(cwd)).await?
    } else {
        thread_root.clone()
    };
    let cwd_str = cwd_path.display().to_string();

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
    Ok(serde_json::json!({ "text": text }))
}

async fn tail_file_lines(path: PathBuf, max_lines: usize) -> anyhow::Result<String> {
    let max_bytes: u64 = 64 * 1024;
    let mut file = tokio::fs::File::open(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
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
    let mut file = tokio::fs::File::open(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
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
