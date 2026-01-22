include!("app/dispatch.rs");
include!("app/thread.rs");
include!("app/turn.rs");
include!("app/process.rs");
include!("app/file.rs");
include!("app/fs.rs");
include!("app/artifact.rs");
include!("app/approval.rs");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    if let Some(command) = args.command {
        match command {
            CliCommand::GenerateTs(output) => pm_app_server_protocol::generate_ts(&output.out_dir)?,
            CliCommand::GenerateJsonSchema(output) => {
                pm_app_server_protocol::generate_json_schema(&output.out_dir)?
            }
        }
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let pm_root = args
        .pm_root
        .or_else(|| std::env::var_os("CODE_PM_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| cwd.join(".codepm_data"));

    let exec_policy = if args.execpolicy_rules.is_empty() {
        pm_execpolicy::Policy::empty()
    } else {
        pm_execpolicy::execpolicycheck::load_policies(&args.execpolicy_rules)?
    };

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    let server = Arc::new(Server {
        cwd,
        out_tx: out_tx.clone(),
        thread_store: ThreadStore::new(PmPaths::new(pm_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        exec_policy,
    });

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            tokio::task::spawn_local(async move {
                let mut stdout = tokio::io::stdout();
                while let Some(line) = out_rx.recv().await {
                    if stdout.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    if stdout.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
            });

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
                    _ => handle_initialized_request(&server, request).await,
                };

                let line = serde_json::to_string(&response)?;
                let _ = server.out_tx.send(line);
            }

            shutdown_running_processes(&server).await;
            Ok(())
        })
        .await
}
