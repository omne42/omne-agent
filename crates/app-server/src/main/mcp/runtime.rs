const OMNE_AGENT_ENABLE_MCP_ENV: &str = "OMNE_AGENT_ENABLE_MCP";
const OMNE_AGENT_MCP_FILE_ENV: &str = "OMNE_AGENT_MCP_FILE";

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const MCP_RESULT_ARTIFACT_THRESHOLD_BYTES: usize = 256 * 1024;

type McpConfig = mcp_kit::Config;
type McpServerConfig = mcp_kit::ServerConfig;
type McpTransport = mcp_kit::Transport;

fn parse_bool_env_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn mcp_enabled() -> bool {
    std::env::var(OMNE_AGENT_ENABLE_MCP_ENV)
        .ok()
        .and_then(|value| parse_bool_env_value(&value))
        .unwrap_or(false)
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
    let env_path = std::env::var(OMNE_AGENT_MCP_FILE_ENV)
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
    mcp_kit::Config::load(thread_root, override_path).await
}

#[derive(Debug, Deserialize)]
struct McpListServersParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
}

#[derive(Debug, Deserialize)]
struct McpListToolsParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    server: String,
}

#[derive(Debug, Deserialize)]
struct McpListResourcesParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    server: String,
}

#[derive(Debug, Deserialize)]
struct McpCallParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    server: String,
    tool: String,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct McpServerDescriptor {
    name: String,
    transport: McpTransport,
    argv: Vec<String>,
    env_keys: Vec<String>,
}

#[derive(Default)]
struct McpManager {
    connections: HashMap<(ThreadId, String), Arc<McpConnection>>,
}

struct McpConnection {
    process_id: ProcessId,
    client: tokio::sync::Mutex<mcp_jsonrpc::Client>,
}

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn mcp_request(
    client: &mut mcp_jsonrpc::Client,
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
    client: &mut mcp_jsonrpc::Client,
    method: &str,
    params: Option<Value>,
) -> anyhow::Result<()> {
    let outcome = tokio::time::timeout(MCP_REQUEST_TIMEOUT, client.notify(method, params)).await;
    outcome
        .with_context(|| format!("mcp notification timed out: {method}"))?
        .with_context(|| format!("mcp notification failed: {method}"))
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
    if !matches!(server_cfg.transport, McpTransport::Stdio) {
        anyhow::bail!("unsupported mcp transport (expected stdio)");
    }
    if server_cfg.argv.is_empty() {
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

    let mut cmd = Command::new(&server_cfg.argv[0]);
    cmd.args(server_cfg.argv.iter().skip(1));
    cmd.current_dir(thread_root);
    cmd.stderr(std::process::Stdio::piped());
    cmd.envs(server_cfg.env.iter());
    apply_child_process_env_defaults(&mut cmd, Some(&server_cfg.env));
    scrub_child_process_env(&mut cmd);
    let max_bytes_per_part = process_log_max_bytes_per_part();
    cmd.kill_on_drop(true);

    let stdout_log = mcp_jsonrpc::StdoutLog {
        path: stdout_path.clone(),
        max_bytes_per_part,
        max_parts: None,
    };
    let mut client = mcp_jsonrpc::Client::spawn_command_with_options(
        cmd,
        mcp_jsonrpc::SpawnOptions {
            stdout_log: Some(stdout_log),
            limits: Default::default(),
        },
    )
    .await
    .with_context(|| format!("spawn mcp server {:?} ({server_name})", server_cfg.argv))?;
    let _ = client.take_notifications();
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
        .append_event(omne_agent_protocol::ThreadEventKind::ProcessStarted {
            process_id,
            turn_id,
            argv: server_cfg.argv.clone(),
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
        argv: server_cfg.argv.clone(),
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
            "name": "omne-agent",
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
        client: tokio::sync::Mutex::new(client),
    })
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
    {
        let manager = server.mcp.lock().await;
        if let Some(conn) = manager.connections.get(&key) {
            return Ok(conn.clone());
        }
    }

    let mut manager = server.mcp.lock().await;
    if let Some(conn) = manager.connections.get(&key) {
        return Ok(conn.clone());
    }

    let conn = Arc::new(
        spawn_mcp_connection(
            server,
            thread_rt,
            thread_root,
            thread_id,
            turn_id,
            server_name,
            server_cfg,
        )
        .await?,
    );
    manager.connections.insert(key, conn.clone());
    Ok(conn)
}

fn json_value_size_bytes(value: &Value) -> usize {
    serde_json::to_string(value).map(|s| s.len()).unwrap_or(0)
}

async fn maybe_write_mcp_result_artifact(
    server: &Server,
    tool_id: omne_agent_protocol::ToolId,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    summary: String,
    result: &Value,
) -> anyhow::Result<Option<Value>> {
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

    Ok(Some(artifact_response))
}

async fn deny_mcp_disabled(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_agent_protocol::ToolId,
    turn_id: Option<TurnId>,
    tool: &str,
    params: Value,
) -> anyhow::Result<Value> {
    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: tool.to_string(),
            params: Some(params),
        })
        .await?;
    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_agent_protocol::ToolStatus::Denied,
            error: Some(format!("{OMNE_AGENT_ENABLE_MCP_ENV}=true is required")),
            result: Some(serde_json::json!({
                "reason": "mcp is disabled",
                "env": OMNE_AGENT_ENABLE_MCP_ENV,
            })),
        })
        .await?;
    Ok(serde_json::json!({
        "tool_id": tool_id,
        "denied": true,
        "reason": "mcp is disabled",
        "env": OMNE_AGENT_ENABLE_MCP_ENV,
    }))
}
