const CODE_PM_ENABLE_MCP_ENV: &str = "CODE_PM_ENABLE_MCP";
const CODE_PM_MCP_FILE_ENV: &str = "CODE_PM_MCP_FILE";

const MCP_CONFIG_VERSION: u32 = 1;
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const MCP_RESULT_ARTIFACT_THRESHOLD_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpConfigFile {
    version: u32,
    servers: BTreeMap<String, McpServerConfigFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerConfigFile {
    transport: McpTransport,
    argv: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum McpTransport {
    Stdio,
}

#[derive(Debug, Clone)]
struct McpConfig {
    path: Option<PathBuf>,
    servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone)]
struct McpServerConfig {
    transport: McpTransport,
    argv: Vec<String>,
    env: BTreeMap<String, String>,
}

fn parse_bool_env_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn mcp_enabled() -> bool {
    std::env::var(CODE_PM_ENABLE_MCP_ENV)
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
    let env_path = std::env::var(CODE_PM_MCP_FILE_ENV)
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
    let from_env = override_path.is_some();
    let path = match override_path {
        Some(path) if path.is_absolute() => path,
        Some(path) => thread_root.join(path),
        None => thread_root
            .join(".codepm_data")
            .join("spec")
            .join("mcp.json"),
    };

    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !from_env => {
            return Ok(McpConfig {
                path: None,
                servers: BTreeMap::new(),
            });
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };

    let cfg: McpConfigFile = serde_json::from_str(&contents)
        .with_context(|| format!("parse {}", path.display()))?;
    if cfg.version != MCP_CONFIG_VERSION {
        anyhow::bail!(
            "unsupported mcp.json version {} (expected {})",
            cfg.version,
            MCP_CONFIG_VERSION
        );
    }

    let mut servers = BTreeMap::<String, McpServerConfig>::new();
    for (name, server) in cfg.servers {
        if !is_valid_mcp_server_name(&name) {
            anyhow::bail!("invalid mcp server name: {name}");
        }
        if server.argv.is_empty() {
            anyhow::bail!("mcp server {name}: argv must not be empty");
        }
        for (idx, arg) in server.argv.iter().enumerate() {
            if arg.trim().is_empty() {
                anyhow::bail!("mcp server {name}: argv[{idx}] must not be empty");
            }
        }
        servers.insert(
            name,
            McpServerConfig {
                transport: server.transport,
                argv: server.argv,
                env: server.env,
            },
        );
    }

    Ok(McpConfig {
        path: Some(path),
        servers,
    })
}

#[derive(Debug, Deserialize)]
struct McpListServersParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
}

#[derive(Debug, Deserialize)]
struct McpListToolsParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    server: String,
}

#[derive(Debug, Deserialize)]
struct McpListResourcesParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
    server: String,
}

#[derive(Debug, Deserialize)]
struct McpCallParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<pm_protocol::ApprovalId>,
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
    client: tokio::sync::Mutex<McpJsonRpcClient>,
}

#[derive(Debug, thiserror::Error)]
enum McpRpcError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("json-rpc error {code}: {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("protocol error: {0}")]
    Protocol(String),
}

type McpPendingRequests =
    std::sync::Arc<
        tokio::sync::Mutex<
            HashMap<u64, tokio::sync::oneshot::Sender<Result<Value, McpRpcError>>>,
        >,
    >;

struct McpJsonRpcClient {
    stdin: tokio::process::ChildStdin,
    next_id: u64,
    pending: McpPendingRequests,
}

impl McpJsonRpcClient {
    async fn request(&mut self, method: &str, params: Option<Value>) -> Result<Value, McpRpcError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let (tx, rx) = tokio::sync::oneshot::channel::<Result<Value, McpRpcError>>();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        if let Err(err) = self.write_line(&line).await {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
            return Err(err);
        }

        match rx.await {
            Ok(result) => result,
            Err(_) => Err(McpRpcError::Protocol("response channel closed".to_string())),
        }
    }

    async fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), McpRpcError> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        self.write_line(&line).await
    }

    async fn write_line(&mut self, line: &str) -> Result<(), McpRpcError> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct McpJsonRpcResponse {
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<McpJsonRpcError>,
}

#[derive(Debug, serde::Deserialize)]
struct McpJsonRpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

async fn drain_mcp_pending(pending: &McpPendingRequests, err: McpRpcError) {
    let pending = {
        let mut pending = pending.lock().await;
        std::mem::take(&mut *pending)
    };

    for (_id, tx) in pending {
        let _ = tx.send(Err(McpRpcError::Protocol(err.to_string())));
    }
}

fn spawn_mcp_stdout_reader_task<R>(
    mut reader: R,
    log_path: PathBuf,
    max_bytes_per_part: u64,
    pending: McpPendingRequests,
) -> tokio::task::JoinHandle<anyhow::Result<()>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let max_bytes_per_part = max_bytes_per_part.max(1);
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create dir {}", parent.display()))?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .with_context(|| format!("open {}", log_path.display()))?;
        let mut current_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
        let mut next_part = next_rotating_log_part(&log_path).await?;

        let mut buf = vec![0u8; 8192];
        let mut pending_line = Vec::<u8>::new();
        loop {
            let n = reader
                .read(&mut buf)
                .await
                .with_context(|| format!("read mcp stdout into {}", log_path.display()))?;
            if n == 0 {
                drain_mcp_pending(
                    &pending,
                    McpRpcError::Protocol("server closed connection".to_string()),
                )
                .await;
                return Ok(());
            }

            // Write raw bytes to rotating log first.
            let mut offset = 0usize;
            while offset < n {
                let remaining = max_bytes_per_part.saturating_sub(current_len);
                if remaining == 0 {
                    file.flush()
                        .await
                        .with_context(|| format!("flush {}", log_path.display()))?;
                    drop(file);
                    next_part = rotate_log_file(&log_path, next_part).await?;
                    file = tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log_path)
                        .await
                        .with_context(|| format!("open {}", log_path.display()))?;
                    current_len = 0;
                    continue;
                }

                let take = usize::try_from(remaining.min((n - offset) as u64)).unwrap_or(n - offset);
                file.write_all(&buf[offset..(offset + take)])
                    .await
                    .with_context(|| format!("write {}", log_path.display()))?;
                current_len = current_len.saturating_add(take as u64);
                offset = offset.saturating_add(take);
            }

            pending_line.extend_from_slice(&buf[..n]);
            loop {
                let Some(pos) = pending_line.iter().position(|b| *b == b'\n') else {
                    break;
                };

                let line = pending_line.drain(..=pos).collect::<Vec<_>>();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let value: Value = match serde_json::from_str(line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };

                let Some(method) = value.get("method").and_then(|v| v.as_str()) else {
                    if value.get("id").is_none() {
                        continue;
                    }

                    let response: McpJsonRpcResponse = match serde_json::from_value(value) {
                        Ok(resp) => resp,
                        Err(err) => {
                            drain_mcp_pending(&pending, McpRpcError::Protocol(err.to_string()))
                                .await;
                            return Ok(());
                        }
                    };

                    let Some(id) = response.id.as_u64() else {
                        continue;
                    };

                    let tx = {
                        let mut pending = pending.lock().await;
                        pending.remove(&id)
                    };
                    let Some(tx) = tx else {
                        continue;
                    };

                    if let Some(err) = response.error {
                        let _ = tx.send(Err(McpRpcError::Rpc {
                            code: err.code,
                            message: err.message,
                            data: err.data,
                        }));
                        continue;
                    }

                    let Some(result) = response.result else {
                        let _ = tx.send(Err(McpRpcError::Protocol("missing result".to_string())));
                        continue;
                    };
                    let _ = tx.send(Ok(result));
                    continue;
                };

                let _ = method;
                // notifications are ignored for now
            }
        }
    })
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
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.envs(server_cfg.env.iter());
    apply_child_process_env_defaults(&mut cmd, Some(&server_cfg.env));
    scrub_child_process_env(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn mcp server {:?} ({server_name})", server_cfg.argv))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("mcp server stdin not captured"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("mcp server stdout not captured"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("mcp server stderr not captured"))?;

    let max_bytes_per_part = process_log_max_bytes_per_part();
    let pending: McpPendingRequests = std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let stdout_task =
        spawn_mcp_stdout_reader_task(stdout, stdout_path.clone(), max_bytes_per_part, pending.clone());
    let stderr_path_for_task = stderr_path.clone();
    let stderr_task = tokio::spawn(async move {
        capture_rotating_log(stderr, stderr_path_for_task, max_bytes_per_part).await
    });

    let started = thread_rt
        .append_event(pm_protocol::ThreadEventKind::ProcessStarted {
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

    tokio::spawn(run_process_actor(
        thread_rt.clone(),
        process_id,
        child,
        cmd_rx,
        Some(stdout_task),
        Some(stderr_task),
        entry.info.clone(),
    ));

    let mut client = McpJsonRpcClient {
        stdin,
        next_id: 1,
        pending,
    };

    let initialize_params = serde_json::json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "clientInfo": {
            "name": "codepm",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {},
    });
    if let Err(err) = client.request("initialize", Some(initialize_params)).await {
        let _ = entry
            .cmd_tx
            .send(ProcessCommand::Kill {
                reason: Some("mcp initialize failed".to_string()),
            })
            .await;
        return Err(anyhow::anyhow!("mcp initialize failed: {err}"));
    }
    if let Err(err) = client.notify("notifications/initialized", None).await {
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
    tool_id: pm_protocol::ToolId,
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
    tool_id: pm_protocol::ToolId,
    turn_id: Option<TurnId>,
    tool: &str,
    params: Value,
) -> anyhow::Result<Value> {
    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: tool.to_string(),
            params: Some(params),
        })
        .await?;
    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Denied,
            error: Some(format!("{CODE_PM_ENABLE_MCP_ENV}=true is required")),
            result: Some(serde_json::json!({
                "reason": "mcp is disabled",
                "env": CODE_PM_ENABLE_MCP_ENV,
            })),
        })
        .await?;
    Ok(serde_json::json!({
        "tool_id": tool_id,
        "denied": true,
        "reason": "mcp is disabled",
        "env": CODE_PM_ENABLE_MCP_ENV,
    }))
}

async fn handle_mcp_list_servers(server: &Server, params: McpListServersParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone(), state.allowed_tools.clone())
    };

    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({});
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "mcp/list_servers",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }
    if !mcp_enabled() {
        return deny_mcp_disabled(&thread_rt, tool_id, params.turn_id, "mcp/list_servers", approval_params).await;
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "mcp/list_servers".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.read;
    let effective_decision = match mode.tool_overrides.get("mcp/list_servers").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "mcp/list_servers".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies mcp/list_servers".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "mcp/list_servers",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "mcp/list_servers".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(serde_json::json!({
                            "approval_policy": approval_policy,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "mcp/list_servers".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let cfg = load_mcp_config(&thread_root).await?;
    let servers = cfg
        .servers
        .iter()
        .map(|(name, cfg)| McpServerDescriptor {
            name: name.clone(),
            transport: cfg.transport,
            argv: cfg.argv.clone(),
            env_keys: cfg.env.keys().cloned().collect(),
        })
        .collect::<Vec<_>>();

    let result = serde_json::json!({
        "config_path": cfg.path.as_ref().map(|p| p.display().to_string()),
        "servers": servers,
    });

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "servers": servers.len(),
            })),
        })
        .await?;

    Ok(result)
}

async fn handle_mcp_list_tools(server: &Server, params: McpListToolsParams) -> anyhow::Result<Value> {
    handle_mcp_action(
        server,
        McpActionRequest {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            action: "mcp/list_tools",
            tool_params: serde_json::json!({ "server": params.server }),
            require_prompt_strict: false,
            mcp_method: "tools/list",
            mcp_params: None,
        },
    )
    .await
}

async fn handle_mcp_list_resources(
    server: &Server,
    params: McpListResourcesParams,
) -> anyhow::Result<Value> {
    handle_mcp_action(
        server,
        McpActionRequest {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            action: "mcp/list_resources",
            tool_params: serde_json::json!({ "server": params.server }),
            require_prompt_strict: false,
            mcp_method: "resources/list",
            mcp_params: None,
        },
    )
    .await
}

async fn handle_mcp_call(server: &Server, params: McpCallParams) -> anyhow::Result<Value> {
    let mut mcp_params = serde_json::json!({ "name": params.tool.clone() });
    if let Some(arguments) = params.arguments.clone() {
        mcp_params["arguments"] = arguments;
    }
    let tool_params = serde_json::json!({
        "server": params.server.clone(),
        "tool": params.tool.clone(),
        "arguments": params.arguments.clone(),
    });
    handle_mcp_action(
        server,
        McpActionRequest {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            action: "mcp/call",
            tool_params,
            require_prompt_strict: true,
            mcp_method: "tools/call",
            mcp_params: Some(mcp_params),
        },
    )
    .await
}

struct McpActionRequest {
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<pm_protocol::ApprovalId>,
    action: &'static str,
    tool_params: Value,
    require_prompt_strict: bool,
    mcp_method: &'static str,
    mcp_params: Option<Value>,
}

async fn handle_mcp_action(server: &Server, req: McpActionRequest) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, req.thread_id).await?;
    let (approval_policy, sandbox_policy, sandbox_network_access, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_network_access,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };

    let tool_id = pm_protocol::ToolId::new();
    let Some(server_name) = req.tool_params.get("server").and_then(|v| v.as_str()) else {
        anyhow::bail!("server is required");
    };
    let server_name = server_name.trim();
    if !is_valid_mcp_server_name(server_name) {
        anyhow::bail!("invalid mcp server name: {server_name}");
    }

    let approval_params = {
        let mut params = req.tool_params.clone();
        if let Some(obj) = params.as_object_mut() {
            if req.require_prompt_strict {
                obj.insert(
                    "approval".to_string(),
                    serde_json::json!({ "requirement": "prompt_strict", "source": "mcp" }),
                );
            }
        }
        params
    };

    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        req.turn_id,
        req.action,
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }
    if !mcp_enabled() {
        return deny_mcp_disabled(&thread_rt, tool_id, req.turn_id, req.action, approval_params).await;
    }

    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids mcp".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let cfg = load_mcp_config(&thread_root).await?;
    let Some(server_cfg) = cfg.servers.get(server_name) else {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Failed,
                error: Some("unknown mcp server".to_string()),
                result: Some(serde_json::json!({
                    "server": server_name,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "failed": true,
            "error": "unknown mcp server",
            "server": server_name,
        }));
    };

    if sandbox_network_access == pm_protocol::SandboxNetworkAccess::Deny
        && command_uses_network(&server_cfg.argv)
    {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_network_access=deny forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_network_access": sandbox_network_access,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_network_access": sandbox_network_access,
        }));
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: req.turn_id,
                    tool: req.action.to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.command.combine(mode.permissions.artifact);
    let effective_mode_decision = match mode.tool_overrides.get(req.action).copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_mode_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some(format!("mode denies {}", req.action)),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_mode_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_mode_decision,
        }));
    }

    let exec_matches = if mode.command_execpolicy_rules.is_empty() {
        server.exec_policy.matches_for_command(&server_cfg.argv, None)
    } else {
        let mode_exec_policy =
            match load_mode_exec_policy(&thread_root, &mode.command_execpolicy_rules).await {
                Ok(policy) => policy,
                Err(err) => {
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                            tool_id,
                            turn_id: req.turn_id,
                            tool: req.action.to_string(),
                            params: Some(approval_params.clone()),
                        })
                        .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some("failed to load mode execpolicy rules".to_string()),
                            result: Some(serde_json::json!({
                                "mode": mode_name,
                                "rules": mode.command_execpolicy_rules.clone(),
                                "error": err.to_string(),
                            })),
                        })
                        .await?;
                    return Ok(serde_json::json!({
                        "tool_id": tool_id,
                        "denied": true,
                        "mode": mode_name,
                        "error": "failed to load mode execpolicy rules",
                        "details": err.to_string(),
                    }));
                }
            };

        let combined = merge_exec_policies(&server.exec_policy, &mode_exec_policy);
        combined.matches_for_command(&server_cfg.argv, None)
    };
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();
    let effective_exec_decision = match exec_decision {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };
    let exec_matches_json = serde_json::to_value(&exec_matches)?;

    if effective_exec_decision == ExecDecision::Forbidden {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("execpolicy forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "decision": ExecDecision::Forbidden,
                    "matched_rules": exec_matches_json,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "decision": ExecDecision::Forbidden,
            "matched_rules": exec_matches_json,
        }));
    }

    let mut approval_params = approval_params;
    if effective_exec_decision == ExecDecision::PromptStrict {
        if let Some(obj) = approval_params.as_object_mut() {
            obj.insert(
                "approval".to_string(),
                serde_json::json!({ "requirement": "prompt_strict", "source": "execpolicy" }),
            );
        }
    }

    let needs_approval = req.require_prompt_strict
        || effective_mode_decision == pm_core::modes::Decision::Prompt
        || matches!(
            effective_exec_decision,
            ExecDecision::Prompt | ExecDecision::PromptStrict
        );
    if needs_approval {
        match gate_approval(
            server,
            &thread_rt,
            req.thread_id,
            req.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: req.approval_id,
                action: req.action,
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: req.turn_id,
                        tool: req.action.to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(serde_json::json!({
                            "approval_policy": approval_policy,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": req.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: req.turn_id,
            tool: req.action.to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let conn = get_or_start_mcp_connection(
        server,
        &thread_rt,
        &thread_root,
        req.thread_id,
        req.turn_id,
        server_name,
        server_cfg,
    )
    .await?;
    let process_id = conn.process_id;

    let result: anyhow::Result<Value> = async {
        let v = {
            let mut client = conn.client.lock().await;
            client
                .request(req.mcp_method, req.mcp_params)
                .await
                .map_err(|err| anyhow::anyhow!(err.to_string()))?
        };
        if let Some(artifact) = maybe_write_mcp_result_artifact(
            server,
            tool_id,
            req.thread_id,
            req.turn_id,
            format!("{}: {server_name}", req.action),
            &v,
        )
        .await?
        {
            return Ok(serde_json::json!({
                "process_id": process_id,
                "artifact": artifact,
                "truncated": true,
                "bytes": json_value_size_bytes(&v),
            }));
        }
        Ok(serde_json::json!({
            "process_id": process_id,
            "result": v,
        }))
    }
    .await;

    match result {
        Ok(v) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "process_id": process_id,
                        "server": server_name,
                        "decision": effective_exec_decision,
                        "matched_rules": exec_matches_json,
                    })),
                })
                .await?;
            Ok(v)
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: Some(serde_json::json!({
                        "process_id": process_id,
                        "server": server_name,
                        "decision": effective_exec_decision,
                        "matched_rules": exec_matches_json,
                    })),
                })
                .await?;
            Err(err)
        }
    }
}
