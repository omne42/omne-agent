async fn init_rpc(rpc: &mut pm_jsonrpc::Client, timeout: Duration) -> anyhow::Result<()> {
    tokio::time::timeout(timeout, async {
        let _ = rpc.request("initialize", serde_json::json!({})).await?;
        let _ = rpc.request("initialized", serde_json::json!({})).await?;
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
        "pm-app-server.exe"
    } else {
        "pm-app-server"
    }
}

fn ensure_approval_and_denial_handled(action: &str, value: &Value) -> anyhow::Result<()> {
    let Some(obj) = value.as_object() else {
        return Ok(());
    };

    if obj
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let thread_id = obj
            .get("thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing thread_id>");
        let approval_id = obj
            .get("approval_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing approval_id>");
        anyhow::bail!(
            "{action} needs approval: pm approval decide {thread_id} {approval_id} --approve (then re-run with --approval-id {approval_id})"
        );
    }

    if obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false) {
        let detail = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
        anyhow::bail!("{action} denied: {detail}");
    }

    Ok(())
}
