const CODE_PM_ENABLE_MCP_ENV: &str = "CODE_PM_ENABLE_MCP";
const CODE_PM_MCP_FILE_ENV: &str = "CODE_PM_MCP_FILE";

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const MCP_RESULT_ARTIFACT_THRESHOLD_BYTES: usize = 256 * 1024;

type McpConfig = pm_mcp_kit::Config;
type McpServerConfig = pm_mcp_kit::ServerConfig;
type McpTransport = pm_mcp_kit::Transport;

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
    pm_mcp_kit::Config::load(thread_root, override_path).await
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
    client: tokio::sync::Mutex<pm_jsonrpc::Client>,
}

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn mcp_request(
    client: &mut pm_jsonrpc::Client,
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
    client: &mut pm_jsonrpc::Client,
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

    let stdout_log = pm_jsonrpc::StdoutLog {
        path: stdout_path.clone(),
        max_bytes_per_part,
        max_parts: None,
    };
    let mut client = pm_jsonrpc::Client::spawn_command_with_options(
        cmd,
        pm_jsonrpc::SpawnOptions {
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
            "name": "codepm",
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
            mcp_request(&mut client, req.mcp_method, req.mcp_params).await?
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
