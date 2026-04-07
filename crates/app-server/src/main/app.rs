macro_rules! dispatch_typed_routes {
    ($id:expr, $method:expr, $params:expr, {
        $($route:literal => $params_ty:ty => $handler:expr),+ $(,)?
    }) => {{
        match $method {
            $(
                $route => {
                    dispatch_jsonrpc_request(&$id, $params, |params: $params_ty| $handler(params))
                        .await
                }
            )+
            _ => method_not_found($id, $method),
        }
    }};
}

#[path = "app/approval.rs"]
mod approval;
#[path = "app/artifact.rs"]
mod artifact;
#[path = "app/dispatch.rs"]
mod dispatch;
#[path = "app/file.rs"]
mod file;
#[path = "app/fs.rs"]
mod fs;
#[path = "app/mcp.rs"]
mod mcp;
#[path = "app/process.rs"]
mod process;
#[path = "app/repo.rs"]
mod repo;
#[path = "app/thread.rs"]
mod thread;
#[path = "app/turn.rs"]
mod turn;

use approval::handle_approval_request;
use artifact::handle_artifact_request;
use dispatch::{
    dispatch_jsonrpc_request, handle_initialized_request, invalid_params, jsonrpc_internal_error,
    jsonrpc_ok_or_internal, method_not_found, parse_jsonrpc_params,
};
use file::handle_file_request;
use fs::handle_fs_request;
use mcp::handle_mcp_request;
use process::handle_process_request;
use repo::handle_repo_request;
use thread::build_thread_subscribe_response;
use thread::{
    configured_total_token_budget_limit, filter_and_paginate_thread_events, handle_thread_request,
    read_thread_events_since_or_not_found, thread_token_budget_snapshot,
    thread_token_budget_snapshot_with_limit,
};
use turn::handle_turn_request;

const NOTIFY_CHANNEL_CAPACITY: usize = 1024;
const OUTBOUND_LINE_CHANNEL_CAPACITY: usize = 1024;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    apply_pre_main_hardening()?;
    init_notify_hub_from_env().context("configure notify-kit")?;

    let args = Args::parse();
    if let Some(command) = args.command {
        match command {
            CliCommand::GenerateTs(output) => {
                omne_app_server_protocol::generate_ts(&output.out_dir)?
            }
            CliCommand::GenerateJsonSchema(output) => {
                omne_app_server_protocol::generate_json_schema(&output.out_dir)?
            }
        }
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let omne_root = args
        .omne_root
        .or_else(|| std::env::var_os("OMNE_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| cwd.join(".omne_data"));

    let exec_policy = if args.execpolicy_rules.is_empty() {
        omne_execpolicy::Policy::empty()
    } else {
        omne_execpolicy::load_policies(&args.execpolicy_rules)?
    };

    let (notify_tx, _notify_rx) = broadcast::channel::<String>(NOTIFY_CHANNEL_CAPACITY);

    let server = Arc::new(Server {
        cwd,
        notify_tx,
        thread_store: ThreadStore::new(PmPaths::new(omne_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        thread_loads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        thread_observation_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
        disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        provider_runtimes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        exec_policy,
    });

    let listen = args.listen;

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            match listen {
                Some(path) => {
                    #[cfg(unix)]
                    serve_unix_socket(server.clone(), path).await?;
                    #[cfg(not(unix))]
                    anyhow::bail!("--listen is only supported on unix");
                }
                None => serve_stdio(server.clone()).await?,
            }

            Ok(())
        })
        .await
}

async fn serve_stdio(server: Arc<Server>) -> anyhow::Result<()> {
    let (out_tx, out_rx) = mpsc::channel::<String>(OUTBOUND_LINE_CHANNEL_CAPACITY);
    tokio::task::spawn_local(spawn_stdio_writer(out_rx));
    tokio::task::spawn_local(spawn_notification_forwarder(
        server.notify_tx.subscribe(),
        out_tx.clone(),
        #[cfg(unix)]
        None,
    ));
    run_request_loop(
        server.clone(),
        tokio::io::stdin(),
        out_tx,
        #[cfg(unix)]
        None,
    )
    .await?;
    shutdown_running_processes(&server).await;
    Ok(())
}

async fn run_request_loop<R>(
    server: Arc<Server>,
    reader: R,
    out_tx: mpsc::Sender<String>,
    #[cfg(unix)] notify_filter: Option<ConnectionNotificationFilter>,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = tokio::io::BufReader::new(reader).lines();
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
                let response = JsonRpcResponse::err(
                    Value::Null,
                    JSONRPC_PARSE_ERROR,
                    "parse error",
                    Some(serde_json::json!({ "error": err.to_string() })),
                );
                let line = serde_json::to_string(&response)?;
                if out_tx.send(line).await.is_err() {
                    break;
                }
                continue;
            }
        };

        #[cfg(unix)]
        if let Some(filter) = &notify_filter {
            filter.register_request(&request).await;
        }

        let id = request.id.clone();
        let response = match request.method.as_str() {
            "initialize" => {
                if initialized {
                    JsonRpcResponse::err(id, OMNE_ALREADY_INITIALIZED, "already initialized", None)
                } else {
                    initialized = true;
                    JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "server": {
                                "name": "omne-app-server",
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
                    JsonRpcResponse::err(id, OMNE_NOT_INITIALIZED, "not initialized", None)
                }
            }
            _ if !initialized => {
                JsonRpcResponse::err(id, OMNE_NOT_INITIALIZED, "not initialized", None)
            }
            _ => handle_initialized_request(&server, request).await,
        };

        let line = serde_json::to_string(&response)?;
        if out_tx.send(line).await.is_err() {
            break;
        }
    }

    Ok(())
}

async fn spawn_stdio_writer(mut out_rx: mpsc::Receiver<String>) {
    let mut stdout = tokio::io::stdout();
    while let Some(line) = out_rx.recv().await {
        if stdout.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        if stdout.write_all(b"\n").await.is_err() {
            break;
        }
        if stdout.flush().await.is_err() {
            break;
        }
    }
}

async fn spawn_notification_forwarder(
    mut notify_rx: broadcast::Receiver<String>,
    out_tx: mpsc::Sender<String>,
    #[cfg(unix)] filter: Option<ConnectionNotificationFilter>,
) {
    loop {
        match notify_rx.recv().await {
            Ok(line) => {
                #[cfg(unix)]
                if let Some(filter) = &filter
                    && !filter.matches_notification(&line).await
                {
                    continue;
                }
                if out_tx.send(line).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Default)]
struct ConnectionNotificationFilter {
    thread_ids: Arc<tokio::sync::RwLock<std::collections::HashSet<String>>>,
}

#[cfg(unix)]
impl ConnectionNotificationFilter {
    async fn register_request(&self, request: &JsonRpcRequest) {
        if let Some(thread_id) = extract_thread_id(&request.params) {
            self.thread_ids.write().await.insert(thread_id.to_string());
        }
    }

    async fn matches_notification(&self, line: &str) -> bool {
        let Some(thread_id) = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| value.get("params").cloned())
            .and_then(|params| extract_thread_id(&params).map(str::to_string))
        else {
            return false;
        };
        self.thread_ids.read().await.contains(&thread_id)
    }
}

#[cfg(unix)]
fn extract_thread_id(params: &serde_json::Value) -> Option<&str> {
    params
        .as_object()
        .and_then(|object| object.get("thread_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|thread_id| !thread_id.is_empty())
}

#[cfg(unix)]
async fn remove_stale_unix_socket_if_safe(listen_path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::FileTypeExt;
    use tokio::net::UnixStream;

    let metadata = tokio::fs::symlink_metadata(listen_path)
        .await
        .with_context(|| format!("stat unix socket path {}", listen_path.display()))?;
    if !metadata.file_type().is_socket() {
        anyhow::bail!(
            "listen path exists and is not a unix socket: {}",
            listen_path.display()
        );
    }

    if UnixStream::connect(listen_path).await.is_ok() {
        anyhow::bail!("daemon already running: {}", listen_path.display());
    }

    tokio::fs::remove_file(listen_path)
        .await
        .with_context(|| format!("remove stale unix socket {}", listen_path.display()))
}

#[cfg(unix)]
async fn serve_unix_socket(server: Arc<Server>, listen_path: PathBuf) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    if let Some(parent) = listen_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    if tokio::fs::try_exists(&listen_path).await? {
        remove_stale_unix_socket_if_safe(&listen_path).await?;
    }

    let listener = UnixListener::bind(&listen_path)?;
    tokio::fs::set_permissions(&listen_path, std::fs::Permissions::from_mode(0o600))
        .await
        .with_context(|| format!("set unix socket permissions {}", listen_path.display()))?;

    loop {
        tokio::select! {
            next = listener.accept() => {
                let (stream, _addr) = next?;
                let server = server.clone();
                tokio::task::spawn_local(async move {
                    if let Err(err) = serve_unix_connection(server, stream).await {
                        eprintln!("app-server: connection error: {err}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    if let Err(err) = tokio::fs::remove_file(&listen_path).await
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            path = %listen_path.display(),
            error = %err,
            "failed to remove unix socket on shutdown"
        );
    }
    shutdown_running_processes(&server).await;
    Ok(())
}

#[cfg(unix)]
async fn serve_unix_connection(
    server: Arc<Server>,
    stream: tokio::net::UnixStream,
) -> anyhow::Result<()> {
    let (read_half, write_half) = stream.into_split();
    let (out_tx, out_rx) = mpsc::channel::<String>(OUTBOUND_LINE_CHANNEL_CAPACITY);
    let notify_filter = ConnectionNotificationFilter::default();

    let notify_task = tokio::task::spawn_local(spawn_notification_forwarder(
        server.notify_tx.subscribe(),
        out_tx.clone(),
        Some(notify_filter.clone()),
    ));

    let writer_task = tokio::task::spawn_local(async move {
        write_lines_to_socket(write_half, out_rx).await;
    });

    let result = run_request_loop(server, read_half, out_tx, Some(notify_filter)).await;

    notify_task.abort();
    writer_task.abort();
    result
}

#[cfg(unix)]
async fn write_lines_to_socket(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut out_rx: mpsc::Receiver<String>,
) {
    while let Some(line) = out_rx.recv().await {
        if writer.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        if writer.write_all(b"\n").await.is_err() {
            break;
        }
        if writer.flush().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;

    async fn run_request_loop_for_test(
        server: Arc<Server>,
        input: &str,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let (mut writer, reader) = tokio::io::duplex(1024);
        writer.write_all(input.as_bytes()).await?;
        drop(writer);

        let (out_tx, mut out_rx) = mpsc::channel::<String>(8);
        run_request_loop(
            server,
            reader,
            out_tx,
            #[cfg(unix)]
            None,
        )
        .await?;

        let mut responses = Vec::new();
        while let Some(line) = out_rx.recv().await {
            responses.push(serde_json::from_str(&line)?);
        }
        Ok(responses)
    }

    #[tokio::test]
    async fn run_request_loop_invalid_json_returns_parse_error_response() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let responses = run_request_loop_for_test(server, "{invalid json}\n").await?;

        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["id"], serde_json::Value::Null);
        assert_eq!(
            responses[0]["error"]["code"].as_i64(),
            Some(JSONRPC_PARSE_ERROR)
        );
        assert_eq!(responses[0]["error"]["message"].as_str(), Some("parse error"));
        assert!(responses[0]["error"]["data"]["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_stale_unix_socket_rejects_non_socket_path() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let listen_path = tmp.path().join("daemon.sock");
        tokio::fs::write(&listen_path, "not a socket").await?;

        let err = remove_stale_unix_socket_if_safe(&listen_path)
            .await
            .expect_err("non-socket path should be rejected");
        let message = err.to_string();
        assert!(message.contains("is not a unix socket"));
        assert!(tokio::fs::try_exists(&listen_path).await?);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_stale_unix_socket_removes_socket_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let listen_path = tmp.path().join("daemon.sock");
        let listener = std::os::unix::net::UnixListener::bind(&listen_path)?;
        drop(listener);

        remove_stale_unix_socket_if_safe(&listen_path).await?;
        assert!(!tokio::fs::try_exists(&listen_path).await?);
        Ok(())
    }
}

#[cfg(test)]
mod response_queue_tests {
    use super::*;
    use serde_json::Value;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn run_request_loop_returns_jsonrpc_parse_error_for_invalid_json() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let (mut input_writer, input_reader) = tokio::io::duplex(1024);
        let (out_tx, mut out_rx) = mpsc::channel::<String>(1);

        let writer_task = tokio::spawn(async move {
            input_writer.write_all(b"{invalid json}\n").await?;
            input_writer.shutdown().await?;
            anyhow::Ok(())
        });

        #[cfg(unix)]
        let run_task = tokio::spawn(run_request_loop(server, input_reader, out_tx, None));
        #[cfg(not(unix))]
        let run_task = tokio::spawn(run_request_loop(server, input_reader, out_tx));

        let response_line = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
            .await
            .context("timed out waiting for parse error response")?
            .ok_or_else(|| anyhow::anyhow!("missing parse error response"))?;
        let response: Value =
            serde_json::from_str(&response_line).context("parse jsonrpc parse error response")?;
        assert_eq!(response["jsonrpc"], serde_json::json!("2.0"));
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], serde_json::json!(JSONRPC_PARSE_ERROR));
        assert_eq!(response["error"]["message"], serde_json::json!("parse error"));
        assert!(response["error"]["data"]["error"]
            .as_str()
            .is_some_and(|message| !message.is_empty()));

        run_task.await??;
        writer_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn run_request_loop_applies_backpressure_to_outbound_responses() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let (mut input_writer, input_reader) = tokio::io::duplex(1024);
        let (out_tx, mut out_rx) = mpsc::channel::<String>(1);

        let writer_task = tokio::spawn(async move {
            let initialize = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            });
            let initialized = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "initialized",
                "params": {},
            });
            input_writer
                .write_all(format!("{initialize}\n{initialized}\n").as_bytes())
                .await?;
            input_writer.shutdown().await?;
            anyhow::Ok(())
        });

        #[cfg(unix)]
        let run_task = tokio::spawn(run_request_loop(server, input_reader, out_tx, None));
        #[cfg(not(unix))]
        let run_task = tokio::spawn(run_request_loop(server, input_reader, out_tx));

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !run_task.is_finished(),
            "bounded response queue should apply backpressure until a receiver drains it"
        );

        let first = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
            .await
            .context("timed out waiting for first response")?
            .ok_or_else(|| anyhow::anyhow!("missing first response"))?;
        assert!(first.contains("\"id\":1"));

        run_task.await??;
        writer_task.await??;

        let second = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
            .await
            .context("timed out waiting for second response")?
            .ok_or_else(|| anyhow::anyhow!("missing second response"))?;
        assert!(second.contains("\"id\":2"));
        Ok(())
    }

    #[tokio::test]
    async fn run_request_loop_returns_parse_error_and_continues() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let (mut input_writer, input_reader) = tokio::io::duplex(1024);
        let (out_tx, mut out_rx) = mpsc::channel::<String>(8);

        let writer_task = tokio::spawn(async move {
            let initialize = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {},
            });
            input_writer.write_all(b"{invalid json}\n").await?;
            input_writer
                .write_all(format!("{initialize}\n").as_bytes())
                .await?;
            input_writer.shutdown().await?;
            anyhow::Ok(())
        });

        #[cfg(unix)]
        let run_task = tokio::spawn(run_request_loop(server, input_reader, out_tx, None));
        #[cfg(not(unix))]
        let run_task = tokio::spawn(run_request_loop(server, input_reader, out_tx));

        let first = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
            .await
            .context("timed out waiting for parse error response")?
            .ok_or_else(|| anyhow::anyhow!("missing parse error response"))?;
        let first: serde_json::Value =
            serde_json::from_str(&first).context("parse parse-error response")?;
        assert_eq!(first["id"], Value::Null);
        assert_eq!(first["error"]["code"], serde_json::json!(JSONRPC_PARSE_ERROR));
        assert_eq!(first["error"]["message"], serde_json::json!("parse error"));

        let second = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
            .await
            .context("timed out waiting for initialize response after parse error")?
            .ok_or_else(|| anyhow::anyhow!("missing initialize response"))?;
        let second: serde_json::Value =
            serde_json::from_str(&second).context("parse initialize response")?;
        assert_eq!(second["id"], serde_json::json!(1));
        assert!(second.get("result").is_some());

        run_task.await??;
        writer_task.await??;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod unix_socket_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test(flavor = "current_thread")]
    async fn unix_connection_does_not_forward_global_notifications_by_default()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let (client_stream, server_stream) = tokio::net::UnixStream::pair()?;
        let local = tokio::task::LocalSet::new();

        local
            .run_until(async move {
                let serve_task =
                    tokio::task::spawn_local(serve_unix_connection(server.clone(), server_stream));
                let (read_half, mut write_half) = client_stream.into_split();
                let mut lines = BufReader::new(read_half).lines();

                let initialize = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {},
                });
                write_half
                    .write_all(format!("{initialize}\n").as_bytes())
                    .await?;
                let response = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .context("timed out waiting for initialize response")??
                    .ok_or_else(|| anyhow::anyhow!("missing initialize response"))?;
                let response: serde_json::Value =
                    serde_json::from_str(&response).context("parse initialize response")?;
                assert_eq!(response["id"], serde_json::json!(1));
                assert!(response.get("result").is_some());

                let _ = server.notify_tx.send(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "thread/updated",
                        "params": { "thread_id": "ignored" },
                    })
                    .to_string(),
                );

                let next = tokio::time::timeout(Duration::from_millis(250), lines.next_line()).await;
                assert!(next.is_err(), "unexpected unsolicited notification");

                drop(write_half);
                serve_task.await??;
                Ok::<(), anyhow::Error>(())
            })
            .await?;

        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unix_connection_forwards_only_registered_thread_notifications()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(tmp.path().to_path_buf()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let (client_stream, server_stream) = tokio::net::UnixStream::pair()?;
        let local = tokio::task::LocalSet::new();

        local
            .run_until(async move {
                let serve_task =
                    tokio::task::spawn_local(serve_unix_connection(server.clone(), server_stream));
                let (read_half, mut write_half) = client_stream.into_split();
                let mut lines = BufReader::new(read_half).lines();

                let initialize = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {},
                });
                write_half
                    .write_all(format!("{initialize}\n").as_bytes())
                    .await?;
                lines
                    .next_line()
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("missing initialize response"))?;

                let state_request = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "thread/state",
                    "params": { "thread_id": thread_id },
                });
                write_half
                    .write_all(format!("{state_request}\n").as_bytes())
                    .await?;
                let response = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .context("timed out waiting for thread/state response")??
                    .ok_or_else(|| anyhow::anyhow!("missing thread/state response"))?;
                let response: serde_json::Value =
                    serde_json::from_str(&response).context("parse thread/state response")?;
                assert_eq!(response["id"], serde_json::json!(2));
                assert!(response.get("result").is_some());

                let _ = server.notify_tx.send(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "thread/event",
                        "params": {
                            "thread_id": thread_id,
                            "seq": 1,
                        },
                    })
                    .to_string(),
                );
                let forwarded = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                    .await
                    .context("timed out waiting for matching notification")??
                    .ok_or_else(|| anyhow::anyhow!("missing matching notification"))?;
                let forwarded: serde_json::Value =
                    serde_json::from_str(&forwarded).context("parse forwarded notification")?;
                assert_eq!(forwarded["method"], serde_json::json!("thread/event"));
                assert_eq!(forwarded["params"]["thread_id"], serde_json::json!(thread_id));

                let _ = server.notify_tx.send(
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "thread/event",
                        "params": {
                            "thread_id": "other-thread",
                            "seq": 2,
                        },
                    })
                    .to_string(),
                );
                let next = tokio::time::timeout(Duration::from_millis(250), lines.next_line()).await;
                assert!(next.is_err(), "unexpected unrelated notification");

                drop(write_half);
                serve_task.await??;
                Ok::<(), anyhow::Error>(())
            })
            .await?;

        Ok(())
    }
}
