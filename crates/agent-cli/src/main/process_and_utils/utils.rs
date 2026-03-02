async fn init_rpc(rpc: &mut omne_jsonrpc::Client, timeout: Duration) -> anyhow::Result<()> {
    tokio::time::timeout(timeout, async {
        rpc.request("initialize", Value::Null).await?;
        rpc.request("initialized", Value::Null).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("initialize timeout")?
}

fn default_app_server_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(app_server_exe_name());
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

// When the app-server binary is newer than the daemon socket, prefer refreshing daemon first.
fn should_refresh_daemon_socket(socket_path: &Path, server_path: &Path) -> bool {
    let sock_meta = match std::fs::metadata(socket_path) {
        Ok(meta) => meta,
        Err(_) => return false,
    };
    let server_meta = match std::fs::metadata(server_path) {
        Ok(meta) => meta,
        Err(_) => return false,
    };
    let sock_mtime = match sock_meta.modified() {
        Ok(time) => time,
        Err(_) => return false,
    };
    let server_mtime = match server_meta.modified() {
        Ok(time) => time,
        Err(_) => return false,
    };
    server_mtime > sock_mtime
}

fn app_server_exe_name() -> &'static str {
    if cfg!(windows) {
        "omne-app-server.exe"
    } else {
        "omne-app-server"
    }
}

fn parse_bool_token(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|value| parse_bool_token(&value))
        .unwrap_or(default)
}

async fn connect_unix_with_retry(
    socket_path: &Path,
    timeout: Duration,
) -> anyhow::Result<omne_jsonrpc::Client> {
    let started = Instant::now();
    let mut last_err: Option<String> = None;
    while started.elapsed() < timeout {
        match omne_jsonrpc::Client::connect_unix(socket_path).await {
            Ok(client) => return Ok(client),
            Err(err) => {
                last_err = Some(err.to_string());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!(
        "failed to connect to daemon socket within timeout: path={} timeout_ms={} error={}",
        socket_path.display(),
        timeout.as_millis(),
        last_err.unwrap_or_else(|| "<unknown>".to_string())
    );
}

fn spawn_daemon(
    server_path: &Path,
    omne_root: &Path,
    socket_path: &Path,
    argv: Vec<OsString>,
) -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (server_path, omne_root, socket_path, argv);
        anyhow::bail!("daemon mode is only supported on unix");
    }

    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::process::{Command, Stdio};

        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create daemon socket dir {}", parent.display()))?;
        }
        if socket_path.exists() {
            // Best-effort cleanup; app-server will also remove stale sockets, but doing it here
            // avoids an avoidable startup failure if the file is not a socket.
            let _ = std::fs::remove_file(socket_path);
        }

        // Redirect daemon logs to a file under `<omne_root>/logs/` to keep the CLI quiet but
        // preserve diagnostics when something goes wrong.
        let logs_dir = omne_root.join("logs");
        std::fs::create_dir_all(&logs_dir)
            .with_context(|| format!("create daemon logs dir {}", logs_dir.display()))?;
        let log_path = logs_dir.join("daemon.log");
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .with_context(|| format!("open daemon log {}", log_path.display()))?;

        let mut cmd = Command::new(server_path);
        cmd.args(argv);
        cmd.arg("--listen");
        cmd.arg(socket_path);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::from(
            log_file
                .try_clone()
                .context("clone daemon log file for stdout")?,
        ));
        cmd.stderr(Stdio::from(log_file));

        // Do not keep a handle to the child; the daemon is intended to outlive this CLI process.
        cmd.spawn()
            .with_context(|| format!("spawn daemon {}", server_path.display()))?;
        Ok(())
    }
}
