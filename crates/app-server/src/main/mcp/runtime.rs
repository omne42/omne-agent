const OMNE_ENABLE_MCP_ENV: &str = "OMNE_ENABLE_MCP";
const OMNE_MCP_FILE_ENV: &str = "OMNE_MCP_FILE";

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const MCP_RESULT_ARTIFACT_THRESHOLD_BYTES: usize = 256 * 1024;

type McpConfig = omne_mcp_kit::Config;
type McpServerConfig = omne_mcp_kit::ServerConfig;
type McpTransport = omne_mcp_kit::Transport;

fn parse_bool_env_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn mcp_enabled() -> bool {
    #[cfg(test)]
    match MCP_ENABLED_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => return false,
        1 => return true,
        _ => {}
    }

    std::env::var(OMNE_ENABLE_MCP_ENV)
        .ok()
        .and_then(|value| parse_bool_env_value(&value))
        .unwrap_or(false)
}

#[cfg(test)]
static MCP_ENABLED_OVERRIDE: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

#[cfg(test)]
fn set_mcp_enabled_override_for_tests(value: Option<bool>) {
    let encoded = match value {
        Some(true) => 1,
        Some(false) => 0,
        None => -1,
    };
    MCP_ENABLED_OVERRIDE.store(encoded, std::sync::atomic::Ordering::Relaxed);
}

fn is_valid_mcp_server_name(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

async fn load_mcp_config(thread_root: &Path) -> anyhow::Result<McpConfig> {
    let env_path = std::env::var(OMNE_MCP_FILE_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let override_path = env_path.map(PathBuf::from);
    load_mcp_config_inner(thread_root, override_path).await
}

async fn load_mcp_config_inner(
    thread_root: &Path,
    override_path: Option<PathBuf>,
) -> anyhow::Result<McpConfig> {
    omne_mcp_kit::Config::load(thread_root, override_path).await
}

type McpListServersParams = omne_app_server_protocol::McpListServersParams;
type McpListToolsParams = omne_app_server_protocol::McpListToolsParams;
type McpListResourcesParams = omne_app_server_protocol::McpListResourcesParams;
type McpCallParams = omne_app_server_protocol::McpCallParams;

#[derive(Default)]
struct McpManager {
    connections: HashMap<(ThreadId, String), Arc<McpConnection>>,
    starting: HashMap<(ThreadId, String), Arc<tokio::sync::Notify>>,
}

struct McpConnection {
    process_id: ProcessId,
    config_fingerprint: String,
    client: tokio::sync::Mutex<omne_jsonrpc::Client>,
}

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn mcp_request(
    client: &mut omne_jsonrpc::Client,
    method: &str,
    params: Option<Value>,
) -> anyhow::Result<Value> {
    let params = params.unwrap_or(Value::Null);
    let outcome = tokio::time::timeout(MCP_REQUEST_TIMEOUT, client.request(method, params)).await;
    outcome
        .with_context(|| format!("mcp request timed out: {method}"))?
        .with_context(|| format!("mcp request failed: {method}"))
}

async fn mcp_notify(
    client: &mut omne_jsonrpc::Client,
    method: &str,
    params: Option<Value>,
) -> anyhow::Result<()> {
    let outcome = tokio::time::timeout(MCP_REQUEST_TIMEOUT, client.notify(method, params)).await;
    outcome
        .with_context(|| format!("mcp notification timed out: {method}"))?
        .with_context(|| format!("mcp notification failed: {method}"))
}

fn mcp_server_config_fingerprint(server_cfg: &McpServerConfig) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = write!(out, "transport={:?};", server_cfg.transport());
    let _ = write!(out, "argv={:?};", server_cfg.argv());
    let _ = write!(out, "inherit_env={};", server_cfg.inherit_env());
    let _ = write!(out, "env={:?};", server_cfg.env());
    let _ = write!(out, "unix_path={:?};", server_cfg.unix_path());
    let _ = write!(out, "url={:?};", server_cfg.url());
    let _ = write!(out, "sse_url={:?};", server_cfg.sse_url());
    let _ = write!(out, "http_url={:?};", server_cfg.http_url());
    let _ = write!(
        out,
        "bearer_token_env_var={:?};",
        server_cfg.bearer_token_env_var()
    );
    let _ = write!(out, "http_headers={:?};", server_cfg.http_headers());
    let _ = write!(out, "env_http_headers={:?};", server_cfg.env_http_headers());
    let _ = write!(out, "stdout_log={:?};", server_cfg.stdout_log());
    out
}

async fn spawn_mcp_connection(
    server: &Server,
    thread_rt: &Arc<ThreadRuntime>,
    thread_root: &Path,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    server_name: &str,
    server_cfg: &McpServerConfig,
) -> anyhow::Result<McpConnection> {
    let config_fingerprint = mcp_server_config_fingerprint(server_cfg);
    if !matches!(server_cfg.transport(), McpTransport::Stdio) {
        anyhow::bail!("unsupported mcp transport (expected stdio)");
    }
    if server_cfg.argv().is_empty() {
        anyhow::bail!("mcp server argv must not be empty");
    }

    let process_id = ProcessId::new();
    let thread_dir = server.thread_store.thread_dir(thread_id);
    let process_dir = thread_dir
        .join("artifacts")
        .join("processes")
        .join(process_id.to_string());
    tokio::fs::create_dir_all(&process_dir)
        .await
        .with_context(|| format!("create dir {}", process_dir.display()))?;

    let stdout_path = process_dir.join("stdout.log");
    let stderr_path = process_dir.join("stderr.log");

    let mut cmd = Command::new(&server_cfg.argv()[0]);
    cmd.args(server_cfg.argv().iter().skip(1));
    cmd.current_dir(thread_root);
    cmd.stderr(std::process::Stdio::piped());
    cmd.envs(server_cfg.env().iter());
    let _effective_env_summary = apply_child_process_hardening(&mut cmd, Some(server_cfg.env()))
        .context("apply child process hardening for mcp server")?;
    let max_bytes_per_part = process_log_max_bytes_per_part();
    cmd.kill_on_drop(true);

    let stdout_log = omne_jsonrpc::StdoutLog {
        path: stdout_path.clone(),
        max_bytes_per_part,
        max_parts: None,
    };
    let mut client = omne_jsonrpc::Client::spawn_command_with_options(
        cmd,
        omne_jsonrpc::SpawnOptions {
            stdout_log: Some(stdout_log),
            limits: Default::default(),
            ..Default::default()
        },
    )
    .await
    .with_context(|| format!("spawn mcp server {:?} ({server_name})", server_cfg.argv()))?;
    drop(client.take_notifications());
    let mut child = client
        .take_child()
        .ok_or_else(|| anyhow::anyhow!("mcp transport does not expose a child process"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("mcp server stderr not captured"))?;
    let stderr_path_for_task = stderr_path.clone();
    let stderr_task =
        tokio::spawn(async move { capture_rotating_log(stderr, stderr_path_for_task, max_bytes_per_part).await });

    let started = thread_rt
        .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
            process_id,
            turn_id,
            argv: server_cfg.argv().to_vec(),
            cwd: thread_root.display().to_string(),
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
        })
        .await?;
    let started_at = started.timestamp.format(&Rfc3339)?;

    let info = ProcessInfo {
        process_id,
        thread_id,
        turn_id,
        argv: server_cfg.argv().to_vec(),
        cwd: thread_root.display().to_string(),
        started_at: started_at.clone(),
        status: ProcessStatus::Running,
        exit_code: None,
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        last_update_at: started_at,
    };

    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let entry = ProcessEntry {
        info: std::sync::Arc::new(tokio::sync::Mutex::new(info)),
        cmd_tx,
    };
    server
        .processes
        .lock()
        .await
        .insert(process_id, entry.clone());

    tokio::spawn(run_process_actor(ProcessActorArgs {
        server: server.clone(),
        thread_rt: thread_rt.clone(),
        process_id,
        child,
        cmd_rx,
        stdout_task: None,
        stderr_task: Some(stderr_task),
        execve_gate: None,
        info: entry.info.clone(),
    }));

    let initialize_params = serde_json::json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "clientInfo": {
            "name": "omneagent",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {},
    });
    if let Err(err) = mcp_request(&mut client, "initialize", Some(initialize_params)).await {
        let _ = entry
            .cmd_tx
            .send(ProcessCommand::Kill {
                reason: Some("mcp initialize failed".to_string()),
            })
            .await;
        return Err(anyhow::anyhow!("mcp initialize failed: {err}"));
    }
    if let Err(err) = mcp_notify(&mut client, "notifications/initialized", None).await {
        let _ = entry
            .cmd_tx
            .send(ProcessCommand::Kill {
                reason: Some("mcp initialized notification failed".to_string()),
            })
            .await;
        return Err(anyhow::anyhow!("mcp initialized notification failed: {err}"));
    }

    Ok(McpConnection {
        process_id,
        config_fingerprint,
        client: tokio::sync::Mutex::new(client),
    })
}

async fn remove_mcp_connections_for_process(server: &Server, process_id: ProcessId) -> usize {
    let mut manager = server.mcp.lock().await;
    let before = manager.connections.len();
    manager
        .connections
        .retain(|_, conn| conn.process_id != process_id);
    before.saturating_sub(manager.connections.len())
}

async fn remove_mcp_connections_for_thread(server: &Server, thread_id: ThreadId) -> usize {
    let mut manager = server.mcp.lock().await;
    let before = manager.connections.len();
    manager.connections.retain(|(id, _), _| *id != thread_id);
    manager.starting.retain(|(id, _), _| *id != thread_id);
    before.saturating_sub(manager.connections.len())
}

async fn invalidate_mcp_connection(
    server: &Server,
    thread_id: ThreadId,
    server_name: &str,
    process_id: ProcessId,
    reason: &str,
) -> bool {
    let key = (thread_id, server_name.to_string());
    let removed = {
        let mut manager = server.mcp.lock().await;
        match manager.connections.get(&key) {
            Some(conn) if conn.process_id == process_id => {
                manager.connections.remove(&key);
                true
            }
            _ => false,
        }
    };
    if !removed {
        return false;
    }

    let entry = {
        let entries = server.processes.lock().await;
        entries.get(&process_id).cloned()
    };
    if let Some(entry) = entry {
        let _ = entry
            .cmd_tx
            .send(ProcessCommand::Kill {
                reason: Some(reason.to_string()),
            })
            .await;
    }
    true
}

async fn process_is_running(server: &Server, process_id: ProcessId) -> bool {
    let entry = {
        let entries = server.processes.lock().await;
        entries.get(&process_id).cloned()
    };
    let Some(entry) = entry else {
        return false;
    };
    let info = entry.info.lock().await;
    matches!(info.status, ProcessStatus::Running)
}

async fn get_or_start_mcp_connection(
    server: &Server,
    thread_rt: &Arc<ThreadRuntime>,
    thread_root: &Path,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    server_name: &str,
    server_cfg: &McpServerConfig,
) -> anyhow::Result<Arc<McpConnection>> {
    let key = (thread_id, server_name.to_string());
    let config_fingerprint = mcp_server_config_fingerprint(server_cfg);
    loop {
        if let Some(conn) = {
            let manager = server.mcp.lock().await;
            manager.connections.get(&key).cloned()
        } {
            if conn.config_fingerprint != config_fingerprint {
                let _ = invalidate_mcp_connection(
                    server,
                    thread_id,
                    server_name,
                    conn.process_id,
                    "mcp config changed",
                )
                .await;
                continue;
            }
            if process_is_running(server, conn.process_id).await {
                return Ok(conn);
            }
            let _ = remove_mcp_connections_for_process(server, conn.process_id).await;
            continue;
        }

        let waiter = {
            let mut manager = server.mcp.lock().await;
            if let Some(conn) = manager.connections.get(&key).cloned() {
                Some(Err(conn))
            } else if let Some(waiter) = manager.starting.get(&key).cloned() {
                Some(Ok(waiter))
            } else {
                manager
                    .starting
                    .insert(key.clone(), Arc::new(tokio::sync::Notify::new()));
                None
            }
        };

        match waiter {
            Some(Err(conn)) => {
                if conn.config_fingerprint != config_fingerprint {
                    let _ = invalidate_mcp_connection(
                        server,
                        thread_id,
                        server_name,
                        conn.process_id,
                        "mcp config changed",
                    )
                    .await;
                    continue;
                }
                if process_is_running(server, conn.process_id).await {
                    return Ok(conn);
                }
                let _ = remove_mcp_connections_for_process(server, conn.process_id).await;
                continue;
            }
            Some(Ok(waiter)) => {
                waiter.notified().await;
                continue;
            }
            None => {}
        }

        let result = spawn_mcp_connection(
            server,
            thread_rt,
            thread_root,
            thread_id,
            turn_id,
            server_name,
            server_cfg,
        )
        .await
        .map(Arc::new);

        let mut manager = server.mcp.lock().await;
        let notify = manager.starting.remove(&key);
        if let Ok(conn) = &result {
            manager.connections.insert(key.clone(), conn.clone());
        }
        drop(manager);
        if let Some(notify) = notify {
            notify.notify_waiters();
        }

        return result;
    }
}

fn json_value_size_bytes(value: &Value) -> usize {
    serde_json::to_string(value).map(|s| s.len()).unwrap_or(0)
}

async fn maybe_write_mcp_result_artifact(
    server: &Server,
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    summary: String,
    result: &Value,
) -> anyhow::Result<Option<omne_app_server_protocol::ArtifactWriteResponse>> {
    let size = json_value_size_bytes(result);
    if size <= MCP_RESULT_ARTIFACT_THRESHOLD_BYTES {
        return Ok(None);
    }

    let text = format!(
        "# MCP Result\n\n_summary: {summary}_\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(result).unwrap_or_else(|_| "<invalid-json>".to_string())
    );

    let (artifact_response, _completed) = write_user_artifact(
        server,
        UserArtifactWriteRequest {
            tool_id,
            thread_id,
            turn_id,
            artifact_id: None,
            artifact_type: "mcp_result".to_string(),
            summary,
            text,
        },
    )
    .await?;
    let artifact_response =
        serde_json::from_value::<omne_app_server_protocol::ArtifactWriteResponse>(
            artifact_response,
        )
        .context("parse artifact/write response for mcp result artifact")?;

    Ok(Some(artifact_response))
}

async fn deny_mcp_disabled(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
    turn_id: Option<TurnId>,
    tool: &str,
    params: Value,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("mcp_disabled", |message| {
        message.try_with_value_arg("env", OMNE_ENABLE_MCP_ENV)?;
        Ok(())
    })?;
    let result = serde_json::to_value(omne_app_server_protocol::McpDisabledDeniedResponse {
        tool_id,
        denied: true,
        reason: "mcp is disabled".to_string(),
        env: OMNE_ENABLE_MCP_ENV.to_string(),
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    })
    .context("serialize mcp disabled denied response")?;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: tool.to_string(),
            params: Some(params),
        })
        .await?;
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Denied,
            structured_error: structured_error_from_result_value(&result),
            error: Some(format!("{OMNE_ENABLE_MCP_ENV}=true is required")),
            result: Some(result.clone()),
        })
        .await?;
    Ok(result)
}
