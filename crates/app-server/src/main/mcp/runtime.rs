const OMNE_ENABLE_MCP_ENV: &str = "OMNE_ENABLE_MCP";
const OMNE_MCP_FILE_ENV: &str = "OMNE_MCP_FILE";

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const MCP_RESULT_ARTIFACT_THRESHOLD_BYTES: usize = 256 * 1024;
const MCP_STDIO_BASELINE_ENV_VARS: [&str; 8] = [
    "PATH",
    "HOME",
    "USERPROFILE",
    "TMPDIR",
    "TEMP",
    "TMP",
    "SystemRoot",
    "SYSTEMROOT",
];

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
    omne_mcp_kit::Config::load(thread_root, override_path)
        .await
        .map_err(anyhow::Error::from)
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

#[derive(Debug)]
struct McpConnectionStartDenied {
    reason: String,
}

impl std::fmt::Display for McpConnectionStartDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for McpConnectionStartDenied {}

impl std::fmt::Debug for McpConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConnection")
            .field("process_id", &self.process_id)
            .field("config_fingerprint", &self.config_fingerprint)
            .finish_non_exhaustive()
    }
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
    let _ = write!(out, "inherit_env={:?};", server_cfg.inherit_env());
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

fn apply_mcp_server_env(cmd: &mut Command, server_cfg: &McpServerConfig) {
    if !server_cfg.inherit_env() {
        cmd.env_clear();
        for key in MCP_STDIO_BASELINE_ENV_VARS {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
    }
    cmd.envs(server_cfg.env().iter());
}

fn mcp_process_runtime_dir(thread_dir: &Path, process_id: ProcessId) -> PathBuf {
    thread_dir
        .join("runtime")
        .join("processes")
        .join(process_id.to_string())
}

async fn cleanup_untracked_mcp_process(
    child: &mut tokio::process::Child,
    process_tree_cleanup: &mut Option<omne_process_primitives::ProcessTreeCleanup>,
    stderr_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    argv: &[String],
    server_name: &str,
) {
    let needs_direct_child_kill = match process_tree_cleanup.as_mut() {
        Some(cleanup) => matches!(
            cleanup.start_termination(),
            omne_process_primitives::CleanupDisposition::DirectChildKillRequired
        ),
        None => true,
    };
    if let Some(cleanup) = process_tree_cleanup.as_ref() {
        cleanup.kill_tree();
    }
    if needs_direct_child_kill && child.start_kill().is_err() {
        tracing::warn!(?argv, server_name, "failed to kill rolled back mcp child");
    }
    if let Err(err) = child.wait().await {
        tracing::warn!(error = %err, ?argv, server_name, "failed waiting for rolled back mcp child");
    }

    if let Some(task) = stderr_task {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::warn!(error = %err, ?argv, server_name, "mcp rollback stderr capture failed");
            }
            Err(err) => {
                tracing::warn!(error = %err, ?argv, server_name, "mcp rollback stderr task panicked");
            }
        }
    }
}

async fn spawn_mcp_connection(
    server: &Server,
    thread_rt: &Arc<ThreadRuntime>,
    thread_root: &Path,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    sandbox_policy: policy_meta::WriteScope,
    server_name: &str,
    server_cfg: &McpServerConfig,
) -> anyhow::Result<McpConnection> {
    async fn capture_process_tree_cleanup(
        child: &mut tokio::process::Child,
        argv: &[String],
        server_name: &str,
    ) -> anyhow::Result<omne_process_primitives::ProcessTreeCleanup> {
        match omne_process_primitives::ProcessTreeCleanup::new(child) {
            Ok(cleanup) => Ok(cleanup),
            Err(err) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(err).with_context(|| {
                    format!("capture process tree cleanup for mcp server {argv:?} ({server_name})")
                })
            }
        }
    }

    let config_fingerprint = mcp_server_config_fingerprint(server_cfg);
    if !matches!(server_cfg.transport(), McpTransport::Stdio) {
        anyhow::bail!("unsupported mcp transport (expected stdio)");
    }
    let argv = server_cfg.argv();
    if argv.is_empty() {
        anyhow::bail!("mcp server argv must not be empty");
    }
    let env = server_cfg.env();

    let process_id = ProcessId::new();
    let thread_dir = server.thread_store.thread_dir(thread_id);
    let process_dir = mcp_process_runtime_dir(&thread_dir, process_id);
    tokio::fs::create_dir_all(&process_dir)
        .await
        .with_context(|| format!("create dir {}", process_dir.display()))?;

    let stdout_path = process_dir.join("stdout.log");
    let stderr_path = process_dir.join("stderr.log");

    let resolved_request = process_exec_gateway_request(argv, thread_root, thread_root)
        .map(|request| process_exec_gateway().resolve_request(&request));
    let mut cmd = Command::new(
        resolved_request
            .as_ref()
            .filter(|_| !Path::new(&argv[0]).is_absolute())
            .map(|request| request.program.as_os_str())
            .unwrap_or_else(|| std::ffi::OsStr::new(&argv[0])),
    );
    cmd.args(argv.iter().skip(1));
    cmd.current_dir(
        resolved_request
            .as_ref()
            .map(|request| request.cwd.as_path())
            .unwrap_or(thread_root),
    );
    cmd.stderr(std::process::Stdio::piped());
    apply_mcp_server_env(&mut cmd, server_cfg);
    if let Err(err) = prepare_process_exec_gateway_command(
        argv,
        thread_root,
        thread_root,
        sandbox_policy,
        cmd.as_std_mut(),
    ) {
        return Err(McpConnectionStartDenied {
            reason: process_exec_gateway_error_reason(&err),
        }
        .into());
    }
    let combined_env_opt = (!env.is_empty()).then_some(env);
    omne_process_primitives::configure_command_for_process_tree(&mut cmd);
    let _effective_env_summary = apply_child_process_hardening(&mut cmd, combined_env_opt)
        .context("apply child process hardening for mcp server")?;
    let max_bytes_per_part = process_log_max_bytes_per_part();

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
    .with_context(|| format!("spawn mcp server {argv:?} ({server_name})"))?;
    drop(client.take_notifications());
    let mut child = client
        .take_child()
        .ok_or_else(|| anyhow::anyhow!("mcp transport does not expose a child process"))?;
    let mut process_tree_cleanup =
        Some(capture_process_tree_cleanup(&mut child, argv, server_name).await?);
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("mcp server stderr not captured"));
    let stderr = match stderr {
        Ok(stderr) => stderr,
        Err(err) => {
            cleanup_untracked_mcp_process(
                &mut child,
                &mut process_tree_cleanup,
                None,
                argv,
                server_name,
            )
            .await;
            return Err(err);
        }
    };
    let stderr_path_for_task = stderr_path.clone();
    let mut stderr_task =
        Some(tokio::spawn(async move { capture_rotating_log(stderr, stderr_path_for_task, max_bytes_per_part).await }));

    let os_pid = child.id();
    let started = match thread_rt
        .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
            process_id,
            turn_id,
            os_pid,
            argv: argv.to_vec(),
            cwd: thread_root.display().to_string(),
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
        })
        .await
    {
        Ok(started) => started,
        Err(err) => {
            cleanup_untracked_mcp_process(
                &mut child,
                &mut process_tree_cleanup,
                stderr_task.take(),
                argv,
                server_name,
            )
            .await;
            return Err(err).context("append ProcessStarted event");
        }
    };
    let started_at = started.timestamp.format(&Rfc3339)?;

    let info = ProcessInfo {
        process_id,
        thread_id,
        turn_id,
        os_pid,
        argv: argv.to_vec(),
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
        thread_id,
        info: std::sync::Arc::new(tokio::sync::Mutex::new(info)),
        cmd_tx,
        completion: ProcessCompletion::new(),
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
        process_tree_cleanup,
        cmd_rx,
        stdout_task: None,
        stderr_task,
        execve_gate: None,
        info: entry.info.clone(),
        completion: entry.completion.clone(),
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
    let removed = {
        let mut manager = server.mcp.lock().await;
        let before = manager.connections.len();
        manager
            .connections
            .retain(|_, conn| conn.process_id != process_id);
        before.saturating_sub(manager.connections.len())
    };

    let removable = {
        let entry = {
            let entries = server.processes.lock().await;
            entries.get(&process_id).cloned()
        };
        match entry {
            Some(entry) => {
                let info = entry.info.lock().await;
                !matches!(info.status, ProcessStatus::Running)
            }
            None => false,
        }
    };
    if removable {
        server.processes.lock().await.remove(&process_id);
    }

    removed
}

async fn remove_mcp_connections_for_thread(server: &Server, thread_id: ThreadId) -> usize {
    let (removed, removed_process_ids) = {
        let mut manager = server.mcp.lock().await;
        let before = manager.connections.len();
        let mut removed_process_ids = Vec::new();
        manager.connections.retain(|(id, _), conn| {
            if *id == thread_id {
                removed_process_ids.push(conn.process_id);
                false
            } else {
                true
            }
        });
        manager.starting.retain(|(id, _), _| *id != thread_id);
        let removed = before.saturating_sub(manager.connections.len());
        (removed, removed_process_ids)
    };

    if removed == 0 {
        return 0;
    }

    if !removed_process_ids.is_empty() {
        let removed_process_ids = removed_process_ids.into_iter().collect::<HashSet<_>>();
        let matching_entries = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .filter(|(process_id, entry)| {
                    entry.thread_id == thread_id && removed_process_ids.contains(process_id)
                })
                .map(|(process_id, entry)| (*process_id, entry.clone()))
                .collect::<Vec<_>>()
        };
        let mut removable = HashSet::new();
        for (process_id, entry) in matching_entries {
            let info = entry.info.lock().await;
            if !matches!(info.status, ProcessStatus::Running) {
                removable.insert(process_id);
            }
        }
        if !removable.is_empty() {
            server.processes.lock().await.retain(|process_id, entry| {
                !(entry.thread_id == thread_id && removable.contains(process_id))
            });
        }
    }

    removed
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
    sandbox_policy: policy_meta::WriteScope,
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
            sandbox_policy,
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

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use std::process::Stdio;

    #[tokio::test]
    async fn cleanup_untracked_mcp_process_terminates_spawned_child() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let stderr_path = tmp.path().join("stderr.log");
        let argv = vec!["sh".to_string(), "-c".to_string(), "sleep 30".to_string()];

        let mut cmd = Command::new(&argv[0]);
        cmd.args(argv.iter().skip(1));
        cmd.stderr(Stdio::piped());
        omne_process_primitives::configure_command_for_process_tree(&mut cmd);

        let mut child = cmd.spawn().context("spawn rollback mcp child")?;
        let mut process_tree_cleanup = Some(
            omne_process_primitives::ProcessTreeCleanup::new(&child)
                .context("capture rollback mcp process tree cleanup")?,
        );
        let stderr = child.stderr.take().expect("stderr should be piped");
        let stderr_task = Some(tokio::spawn(async move {
            capture_rotating_log(stderr, stderr_path, process_log_max_bytes_per_part()).await
        }));

        cleanup_untracked_mcp_process(
            &mut child,
            &mut process_tree_cleanup,
            stderr_task,
            &argv,
            "test",
        )
        .await;

        assert!(child.try_wait()?.is_some(), "child should be terminated");
        Ok(())
    }
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

#[cfg(test)]
mod mcp_runtime_tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::collections::BTreeMap;

    struct LockedEnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl LockedEnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            set_locked_process_env(key, value);
            Self { key, previous }
        }
    }

    impl Drop for LockedEnvGuard {
        fn drop(&mut self) {
            restore_locked_process_env(self.key, self.previous.as_deref());
        }
    }

    fn command_env_map(cmd: &Command) -> BTreeMap<OsString, Option<OsString>> {
        cmd.as_std()
            .get_envs()
            .map(|(key, value)| (key.to_os_string(), value.map(|value| value.to_os_string())))
            .collect()
    }

    #[test]
    fn mcp_process_runtime_dir_uses_runtime_namespace() {
        let thread_dir = Path::new("/tmp/thread");
        let process_id = ProcessId::new();
        let dir = mcp_process_runtime_dir(thread_dir, process_id);

        assert!(dir.starts_with(thread_dir.join("runtime").join("processes")));
        assert!(!dir.starts_with(thread_dir.join("artifacts")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_mcp_connection_denies_non_executable_program() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let script_path = repo_dir.join("non-executable-mcp");
        tokio::fs::write(&script_path, "#!/bin/sh\nexit 0\n").await?;
        tokio::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o644)).await?;

        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "local": { "transport": "stdio", "argv": ["./non-executable-mcp"] } } }"#,
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let cfg = load_mcp_config(&repo_dir).await?;
        let server_cfg = cfg.servers().get("local").expect("local server");

        let err = spawn_mcp_connection(
            &server,
            &thread_rt,
            &repo_dir,
            thread_id,
            None,
            policy_meta::WriteScope::WorkspaceWrite,
            "local",
            server_cfg,
        )
        .await
        .expect_err("gateway should deny non-executable servers");

        assert!(
            err.downcast_ref::<McpConnectionStartDenied>().is_some(),
            "unexpected error: {err:#}"
        );
        assert!(
            err.to_string().contains("execution boundary denied command"),
            "unexpected error: {err:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn apply_mcp_server_env_respects_inherit_env_false() -> anyhow::Result<()> {
        let _env_lock = app_server_process_env_lock().lock().await;
        let _guard = LockedEnvGuard::set("OMNE_MCP_ENV_TEST_SECRET", "super-secret");

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "local": { "transport": "stdio", "argv": ["printf", "ok"], "inherit_env": false, "env": { "OMNE_MCP_EXPLICIT": "1" } } } }"#,
        )
        .await?;

        let cfg = load_mcp_config(&repo_dir).await?;
        let server_cfg = cfg.servers().get("local").expect("local server");
        let mut cmd = Command::new("printf");
        apply_mcp_server_env(&mut cmd, server_cfg);
        let env_map = command_env_map(&cmd);

        assert!(
            !env_map.contains_key(OsStr::new("OMNE_MCP_ENV_TEST_SECRET")),
            "inherit_env=false should not keep host-only env"
        );
        assert_eq!(
            env_map
                .get(OsStr::new("OMNE_MCP_EXPLICIT"))
                .and_then(|value| value.as_deref()),
            Some(OsStr::new("1"))
        );
        if let Some(path) = std::env::var_os("PATH") {
            assert_eq!(
                env_map
                    .get(OsStr::new("PATH"))
                    .and_then(|value| value.as_deref()),
                Some(path.as_os_str())
            );
        }

        Ok(())
    }
}
