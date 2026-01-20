use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use pm_core::{PmPaths, ThreadStore};
use pm_protocol::{EventSeq, ThreadId, TurnId, TurnStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pm-app-server")]
#[command(about = "CodePM v0.2.0 app-server (JSON-RPC over stdio)", long_about = None)]
struct Args {
    /// Override `.code_pm` root directory.
    #[arg(long)]
    pm_root: Option<PathBuf>,
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

    let server = Server {
        cwd,
        thread_store: ThreadStore::new(PmPaths::new(pm_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
                    Ok(rt) => match rt.interrupt_turn(params.turn_id, params.reason).await {
                        Ok(()) => JsonRpcResponse::ok(id, serde_json::json!({ "ok": true })),
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
