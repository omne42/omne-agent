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

fn should_bypass_daemon(socket_path: &Path, server_path: &Path) -> bool {
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
