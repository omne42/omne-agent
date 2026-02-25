include!("app/dispatch.rs");
include!("app/thread.rs");
include!("app/turn.rs");
include!("app/process.rs");
include!("app/file.rs");
include!("app/repo.rs");
include!("app/mcp.rs");
include!("app/fs.rs");
include!("app/artifact.rs");
include!("app/approval.rs");

const NOTIFY_CHANNEL_CAPACITY: usize = 1024;

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
            CliCommand::GenerateTs(output) => omne_app_server_protocol::generate_ts(&output.out_dir)?,
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
        omne_execpolicy::execpolicycheck::load_policies(&args.execpolicy_rules)?
    };

    let (notify_tx, _notify_rx) = broadcast::channel::<String>(NOTIFY_CHANNEL_CAPACITY);

    let server = Arc::new(Server {
        cwd,
        notify_tx,
        thread_store: ThreadStore::new(PmPaths::new(omne_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
        disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
    let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();
    tokio::task::spawn_local(spawn_stdio_writer(out_rx));
    tokio::task::spawn_local(spawn_notification_forwarder(
        server.notify_tx.subscribe(),
        out_tx.clone(),
    ));
    run_request_loop(server.clone(), tokio::io::stdin(), out_tx).await?;
    shutdown_running_processes(&server).await;
    Ok(())
}

async fn run_request_loop<R>(
    server: Arc<Server>,
    reader: R,
    out_tx: mpsc::UnboundedSender<String>,
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
                continue;
            }
        };

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
            _ if !initialized => JsonRpcResponse::err(id, OMNE_NOT_INITIALIZED, "not initialized", None),
            _ => handle_initialized_request(&server, request).await,
        };

        let line = serde_json::to_string(&response)?;
        if out_tx.send(line).is_err() {
            break;
        }
    }

    Ok(())
}

async fn spawn_stdio_writer(mut out_rx: mpsc::UnboundedReceiver<String>) {
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
    out_tx: mpsc::UnboundedSender<String>,
) {
    loop {
        match notify_rx.recv().await {
            Ok(line) => {
                if out_tx.send(line).is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
}

#[cfg(unix)]
async fn serve_unix_socket(server: Arc<Server>, listen_path: PathBuf) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::{UnixListener, UnixStream};

    if let Some(parent) = listen_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    if tokio::fs::try_exists(&listen_path).await? {
        if UnixStream::connect(&listen_path).await.is_ok() {
            anyhow::bail!("daemon already running: {}", listen_path.display());
        }
        tokio::fs::remove_file(&listen_path)
            .await
            .with_context(|| format!("remove stale unix socket {}", listen_path.display()))?;
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
async fn serve_unix_connection(server: Arc<Server>, stream: tokio::net::UnixStream) -> anyhow::Result<()> {
    let (read_half, write_half) = stream.into_split();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<String>();

    let notify_task = tokio::task::spawn_local(spawn_notification_forwarder(
        server.notify_tx.subscribe(),
        out_tx.clone(),
    ));

    let writer_task = tokio::task::spawn_local(async move {
        write_lines_to_socket(write_half, out_rx).await;
    });

    let result = run_request_loop(server, read_half, out_tx).await;

    notify_task.abort();
    writer_task.abort();
    result
}

#[cfg(unix)]
async fn write_lines_to_socket(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut out_rx: mpsc::UnboundedReceiver<String>,
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
