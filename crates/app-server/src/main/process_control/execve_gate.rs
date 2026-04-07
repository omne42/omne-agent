use super::*;

#[cfg(unix)]
const EXECVE_GATE_MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[cfg(unix)]
const EXECVE_GATE_TOOL_DECIDE: &str = "omne.execve.decide";
#[cfg(unix)]
const EXECVE_GATE_TOOL_WAIT: &str = "omne.execve.wait";

#[cfg(unix)]
pub(super) struct ExecveGateHandle {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    socket_path: PathBuf,
}

#[cfg(not(unix))]
pub(super) struct ExecveGateHandle;

#[cfg(unix)]
#[derive(Clone)]
pub(super) struct ExecveGateContext {
    pub(super) thread_id: ThreadId,
    pub(super) turn_id: Option<TurnId>,
    pub(super) token: String,
    pub(super) thread_root: PathBuf,
    pub(super) thread_store: ThreadStore,
    pub(super) exec_policy: omne_execpolicy::Policy,
    pub(super) thread_rt: Arc<ThreadRuntime>,
}

#[cfg(unix)]
pub(super) async fn spawn_execve_gate(
    ctx: ExecveGateContext,
    socket_path: PathBuf,
) -> anyhow::Result<ExecveGateHandle> {
    use std::os::unix::fs::PermissionsExt;

    match tokio::fs::remove_file(&socket_path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!("remove stale execve gate socket {}", socket_path.display())
            });
        }
    }
    let listener = tokio::net::UnixListener::bind(&socket_path)
        .with_context(|| format!("bind execve gate socket {}", socket_path.display()))?;
    tokio::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .await
        .with_context(|| format!("chmod execve gate socket {}", socket_path.display()))?;

    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();
    let socket_path_for_task = socket_path.clone();

    let task = tokio::spawn(async move {
        if let Err(err) = run_execve_gate_server(ctx, listener, cancel_for_task).await {
            eprintln!(
                "execve gate failed: socket={} err={err:#}",
                socket_path_for_task.display()
            );
        }
    });

    Ok(ExecveGateHandle {
        cancel,
        task,
        socket_path,
    })
}

#[cfg(unix)]
pub(super) async fn shutdown_execve_gate(gate: ExecveGateHandle) {
    gate.cancel.cancel();
    if let Err(err) = gate.task.await {
        tracing::warn!(path = %gate.socket_path.display(), error = %err, "execve gate task join failed");
    }
    match tokio::fs::remove_file(&gate.socket_path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(path = %gate.socket_path.display(), error = %err, "failed to remove execve gate socket");
        }
    }
}

#[cfg(not(unix))]
pub(super) async fn shutdown_execve_gate(_: ExecveGateHandle) {}

#[cfg(unix)]
async fn run_execve_gate_server(
    ctx: ExecveGateContext,
    listener: tokio::net::UnixListener,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let ctx = Arc::new(ctx);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let ctx = ctx.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_execve_gate_connection(ctx, stream, cancel).await {
                        eprintln!("execve gate connection error: {err:#}");
                    }
                });
            }
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpInitState {
    NotInitialized,
    InitializeResponded,
    Ready,
}

#[cfg(unix)]
async fn handle_execve_gate_connection(
    ctx: Arc<ExecveGateContext>,
    stream: tokio::net::UnixStream,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(read_half).lines();
    let mut init_state = McpInitState::NotInitialized;

    while let Some(line) = lines.next_line().await? {
        if cancel.is_cancelled() {
            break;
        }

        let message = match omne_core::jsonrpc_line::parse_jsonrpc_line(&line) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(err) => {
                let resp = jsonrpc_error(err.id, err.code, err.message, err.data);
                write_json_line(&mut write_half, &resp).await?;
                continue;
            }
        };

        let (id, method, params) = match message {
            omne_core::jsonrpc_line::JsonRpcLine::Notification(notification) => {
                if notification.method == "notifications/initialized"
                    && init_state == McpInitState::InitializeResponded
                {
                    init_state = McpInitState::Ready;
                }
                continue;
            }
            omne_core::jsonrpc_line::JsonRpcLine::Request(request) => {
                (request.id, request.method, request.params)
            }
        };

        let resp = match method.as_str() {
            "initialize" => handle_mcp_initialize(&mut init_state, id).await,
            "tools/list" => handle_mcp_tools_list(&init_state, id).await,
            "tools/call" => {
                handle_mcp_tools_call(ctx.clone(), &mut init_state, id, params, cancel.clone())
                    .await
            }
            "resources/list" => handle_mcp_resources_list(&init_state, id).await,
            "prompts/list" => handle_mcp_prompts_list(&init_state, id).await,
            _ => jsonrpc_error(id, -32601, "method not found", None::<String>),
        };

        write_json_line(&mut write_half, &resp).await?;
    }

    Ok(())
}

#[cfg(unix)]
async fn write_json_line<W: tokio::io::AsyncWrite + Unpin>(
    out: &mut W,
    value: &Value,
) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    out.write_all(line.as_bytes()).await?;
    out.flush().await?;
    Ok(())
}

#[cfg(unix)]
fn jsonrpc_ok(id: Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

#[cfg(unix)]
fn jsonrpc_error<T: serde::Serialize>(
    id: Value,
    code: i64,
    message: &str,
    data: Option<T>,
) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": data,
        }
    })
}

#[cfg(unix)]
async fn handle_mcp_initialize(state: &mut McpInitState, id: Value) -> Value {
    if *state != McpInitState::NotInitialized {
        return jsonrpc_error(id, -32000, "already initialized", None::<String>);
    }

    *state = McpInitState::InitializeResponded;
    jsonrpc_ok(
        id,
        serde_json::json!({
            "protocolVersion": EXECVE_GATE_MCP_PROTOCOL_VERSION,
            "serverInfo": {
                "name": "omne-execve-gate",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false },
            }
        }),
    )
}

#[cfg(unix)]
async fn handle_mcp_tools_list(state: &McpInitState, id: Value) -> Value {
    if *state == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    jsonrpc_ok(
        id,
        serde_json::json!({
            "tools": execve_gate_tools(),
        }),
    )
}

#[cfg(unix)]
async fn handle_mcp_resources_list(state: &McpInitState, id: Value) -> Value {
    if *state == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    jsonrpc_ok(
        id,
        serde_json::json!({
            "resources": [],
        }),
    )
}

#[cfg(unix)]
async fn handle_mcp_prompts_list(state: &McpInitState, id: Value) -> Value {
    if *state == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    jsonrpc_ok(
        id,
        serde_json::json!({
            "prompts": [],
        }),
    )
}

#[cfg(unix)]
async fn handle_mcp_tools_call(
    ctx: Arc<ExecveGateContext>,
    init_state: &mut McpInitState,
    id: Value,
    params: Option<Value>,
    cancel: CancellationToken,
) -> Value {
    if *init_state != McpInitState::Ready {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }

    let params = match params {
        Some(v) => v,
        None => return jsonrpc_error(id, -32602, "invalid params", Some("missing params")),
    };
    let Some(obj) = params.as_object() else {
        return jsonrpc_error(
            id,
            -32602,
            "invalid params",
            Some("params must be an object"),
        );
    };
    let Some(name) = obj.get("name").and_then(|v| v.as_str()).map(str::to_string) else {
        return jsonrpc_error(id, -32602, "invalid params", Some("missing tool name"));
    };
    let arguments = obj
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let outcome = dispatch_execve_gate_tool(ctx, &name, &arguments, cancel).await;
    match outcome {
        Ok(payload) => jsonrpc_ok(id, mcp_tool_ok(payload)),
        Err(err) => jsonrpc_ok(id, mcp_tool_err(err.to_string())),
    }
}

#[cfg(unix)]
fn mcp_tool_ok(payload: Value) -> Value {
    let text = serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string());
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": text,
        }],
        "isError": false,
    })
}

#[cfg(unix)]
fn mcp_tool_err(message: String) -> Value {
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": message,
        }],
        "isError": true,
    })
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecveGateDecideArgs {
    token: String,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecveGateWaitArgs {
    token: String,
    approval_id: omne_protocol::ApprovalId,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[cfg(unix)]
async fn dispatch_execve_gate_tool(
    ctx: Arc<ExecveGateContext>,
    name: &str,
    args: &Value,
    cancel: CancellationToken,
) -> anyhow::Result<Value> {
    match name {
        EXECVE_GATE_TOOL_DECIDE => {
            let args: ExecveGateDecideArgs =
                serde_json::from_value(args.clone()).context("parse decide arguments")?;
            handle_execve_gate_decide(ctx, args).await
        }
        EXECVE_GATE_TOOL_WAIT => {
            let args: ExecveGateWaitArgs =
                serde_json::from_value(args.clone()).context("parse wait arguments")?;
            handle_execve_gate_wait(ctx, args, cancel).await
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

#[cfg(unix)]
async fn handle_execve_gate_decide(
    ctx: Arc<ExecveGateContext>,
    args: ExecveGateDecideArgs,
) -> anyhow::Result<Value> {
    if args.token != ctx.token {
        anyhow::bail!("unauthorized");
    }
    if args.argv.is_empty() {
        anyhow::bail!("argv must not be empty");
    }

    let snapshot = load_thread_process_exec_snapshot(&ctx.thread_rt).await;
    let gateway_cwd = args
        .cwd
        .as_deref()
        .map(Path::new)
        .unwrap_or(ctx.thread_root.as_path());
    let approval_cwd = args
        .cwd
        .clone()
        .unwrap_or_else(|| ctx.thread_root.display().to_string());
    let exec_governance = evaluate_thread_process_exec_governance(
        &ctx.thread_store,
        &ctx.exec_policy,
        &ctx.thread_rt,
        &ctx.thread_root,
        &snapshot,
        ctx.thread_id,
        ctx.turn_id,
        None,
        gateway_cwd,
        "process/execve",
        &args.argv,
        |mode| mode.permissions.command,
        |approval_requirement| {
            build_process_exec_approval_params(
                &args.argv,
                &approval_cwd,
                None,
                Some((
                    ProcessExecApprovalSource::ExecveWrapper,
                    approval_requirement,
                )),
            )
        },
    )
    .await?;

    match exec_governance {
        ProcessExecGovernance::Allowed => {
            Ok(serde_json::json!({ "decision": "run" }))
        }
        ProcessExecGovernance::NeedsApproval { approval_id } => Ok(serde_json::json!({
            "decision": "escalate",
            "approval_id": approval_id,
        })),
        ProcessExecGovernance::Denied(denied) => {
            let reason = process_exec_governance_denied_reason(&denied, "process/execve");

            match denied {
                ProcessExecGovernanceDenied::UnknownMode {
                    available,
                    load_error,
                } => Ok(serde_json::json!({
                    "decision": "deny",
                    "reason": reason,
                    "mode": snapshot.mode_name,
                    "available": available,
                    "load_error": load_error,
                })),
                ProcessExecGovernanceDenied::ModeDenied { mode_decision } => {
                    Ok(serde_json::json!({
                        "decision": "deny",
                        "reason": reason,
                        "mode": snapshot.mode_name,
                        "mode_decision": format!("{:?}", mode_decision.decision).to_lowercase(),
                        "decision_source": mode_decision.decision_source,
                        "tool_override_hit": mode_decision.tool_override_hit,
                    }))
                }
                ProcessExecGovernanceDenied::ExecPolicyLoad { details, rules, .. } => {
                    Ok(serde_json::json!({
                        "decision": "deny",
                        "reason": reason,
                        "mode": snapshot.mode_name,
                        "rules": rules,
                        "details": details,
                    }))
                }
                ProcessExecGovernanceDenied::ExecPolicyForbidden {
                    matches,
                    justification,
                } => Ok(serde_json::json!({
                    "decision": "deny",
                    "reason": reason,
                    "matched_rules": serde_json::to_value(&matches)?,
                    "justification": justification,
                })),
                ProcessExecGovernanceDenied::SandboxPolicyReadOnly
                | ProcessExecGovernanceDenied::SandboxNetworkDenied
                | ProcessExecGovernanceDenied::GatewayDenied(_)
                | ProcessExecGovernanceDenied::ApprovalDenied { .. } => Ok(serde_json::json!({
                    "decision": "deny",
                    "reason": reason,
                })),
            }
        }
    }
}

#[cfg(unix)]
async fn handle_execve_gate_wait(
    ctx: Arc<ExecveGateContext>,
    args: ExecveGateWaitArgs,
    cancel: CancellationToken,
) -> anyhow::Result<Value> {
    if args.token != ctx.token {
        anyhow::bail!("unauthorized");
    }

    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(15 * 60_000));
    let deadline = tokio::time::Instant::now() + timeout;

    let mut since = EventSeq::ZERO;
    loop {
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled waiting for approval");
        }

        if tokio::time::Instant::now() > deadline {
            return Ok(serde_json::json!({
                "decision": "deny",
                "reason": "approval wait timed out",
            }));
        }

        let events = ctx
            .thread_store
            .read_events_since(ctx.thread_id, since)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {}", ctx.thread_id))?;
        since = events.last().map(|e| e.seq).unwrap_or(since);

        for event in events {
            if let omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember,
                reason,
            } = event.kind
                && approval_id == args.approval_id
            {
                return Ok(serde_json::json!({
                    "decision": match decision {
                        omne_protocol::ApprovalDecision::Approved => "run",
                        omne_protocol::ApprovalDecision::Denied => "deny",
                    },
                    "remember": remember,
                    "reason": reason,
                }));
            }
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(unix)]
fn execve_gate_tools() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": EXECVE_GATE_TOOL_DECIDE,
            "description": "Gate an execve attempt (Run/Escalate/Deny).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "token": { "type": "string" },
                    "argv": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" }
                },
                "required": ["token", "argv"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": EXECVE_GATE_TOOL_WAIT,
            "description": "Wait for an execve approval decision (returns Run/Deny).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "token": { "type": "string" },
                    "approval_id": { "type": "string" },
                    "timeout_ms": { "type": "integer", "minimum": 1 }
                },
                "required": ["token", "approval_id"],
                "additionalProperties": false
            }
        }),
    ]
}

#[cfg(all(test, unix))]
mod execve_gate_tests {
    use super::*;

    fn unix_socket_bind_not_permitted(err: &anyhow::Error) -> bool {
        err.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .and_then(|err| err.raw_os_error())
                == Some(1)
        })
    }

    async fn mcp_initialize(
        lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>,
        write_half: &mut tokio::net::unix::OwnedWriteHalf,
    ) -> anyhow::Result<()> {
        let init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {},
        });
        let mut line = serde_json::to_string(&init)?;
        line.push('\n');
        write_half.write_all(line.as_bytes()).await?;
        let _ = lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing initialize response"))?;

        let notify = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {},
        });
        let mut line = serde_json::to_string(&notify)?;
        line.push('\n');
        write_half.write_all(line.as_bytes()).await?;
        Ok(())
    }

    async fn mcp_tools_call(
        lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>,
        write_half: &mut tokio::net::unix::OwnedWriteHalf,
        id: u64,
        name: &str,
        arguments: Value,
    ) -> anyhow::Result<Value> {
        let call = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let mut line = serde_json::to_string(&call)?;
        line.push('\n');
        write_half.write_all(line.as_bytes()).await?;

        let resp_line = lines
            .next_line()
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing tools/call response"))?;
        Ok(serde_json::from_str(&resp_line)?)
    }

    fn mcp_payload(resp: &Value) -> anyhow::Result<Value> {
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing mcp response text"))?;
        Ok(serde_json::from_str(text)?)
    }

    #[tokio::test]
    async fn execve_gate_denies_forbidden_execpolicy() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let mut exec_policy = omne_execpolicy::Policy::empty();
        exec_policy.add_prefix_rule(&["git".to_string()], ExecDecision::Forbidden)?;

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy: exec_policy.clone(),
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({ "token": token, "argv": ["git", "status"] }),
        )
        .await?;

        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_escalates_unmatched_commands_instead_of_running_them()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({ "token": token, "argv": ["echo", "ok"] }),
        )
        .await?;

        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "escalate");
        assert!(payload["approval_id"].as_str().is_some_and(|id| !id.is_empty()));

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_denies_read_only_sandbox_policy() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: Some(policy_meta::WriteScope::ReadOnly),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({ "token": token, "argv": ["echo", "ok"] }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");
        assert!(
            payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("sandbox_policy=read_only"))
        );

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_denies_network_commands_when_network_access_is_denied()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["curl", "https://example.com"],
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");
        assert!(
            payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("sandbox_network_access=deny"))
        );

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_hard_boundary_precedes_mode_and_execpolicy() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let mut exec_policy = omne_execpolicy::Policy::empty();
        exec_policy.add_prefix_rule(&["curl".to_string()], ExecDecision::Forbidden)?;

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: Some("coder".to_string()),
                role: Some("chat".to_string()),
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["curl", "https://example.com"],
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");
        assert!(
            payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("sandbox_network_access=deny"))
        );
        assert!(payload.get("mode").is_none());
        assert!(payload.get("decision_source").is_none());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        assert!(
            !events.iter().any(|event| matches!(
                event.kind,
                omne_protocol::ThreadEventKind::ApprovalRequested { .. }
            )),
            "hard boundary denial should not emit ApprovalRequested"
        );

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_denies_wrapped_network_commands_when_network_access_is_denied()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        for argv in [
            serde_json::json!(["env", "FOO=bar", "curl", "https://example.com"]),
            serde_json::json!(["python", "-c", "import requests; requests.get('https://example.com')"]),
            serde_json::json!(["bash", "-lc", "echo ok && curl https://example.com"]),
            serde_json::json!(["npm", "install"]),
            serde_json::json!(["python", "-m", "pip", "install", "requests"]),
            serde_json::json!(["cargo", "install", "ripgrep"]),
            serde_json::json!(["go", "get", "example.com/x"]),
            serde_json::json!(["git", "--git-dir", "/tmp/repo.git", "fetch"]),
            serde_json::json!(["git", "--attr-source", "HEAD", "push"]),
        ] {
            let resp = mcp_tools_call(
                &mut lines,
                &mut write_half,
                2,
                EXECVE_GATE_TOOL_DECIDE,
                serde_json::json!({
                    "token": token,
                    "argv": argv,
                }),
            )
            .await?;
            let payload = mcp_payload(&resp)?;
            assert_eq!(payload["decision"], "deny");
            assert!(
                payload["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("sandbox_network_access=deny"))
            );
        }

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_denies_generic_launchers_when_network_access_is_denied()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_rt,
                thread_store: server.thread_store.clone(),
                exec_policy,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if err.to_string().contains("Operation not permitted") => {
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        for argv in [
            serde_json::json!(["python", "-m", "http.server"]),
            serde_json::json!(["python", "server.py"]),
            serde_json::json!(["node", "server.js"]),
            serde_json::json!(["bash", "script.sh"]),
        ] {
            let resp = mcp_tools_call(
                &mut lines,
                &mut write_half,
                2,
                EXECVE_GATE_TOOL_DECIDE,
                serde_json::json!({
                    "token": token,
                    "argv": argv,
                }),
            )
            .await?;
            let payload = mcp_payload(&resp)?;
            assert_eq!(payload["decision"], "deny");
            assert!(
                payload["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("sandbox_network_access=deny"))
            );
        }

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_denies_cwd_outside_workspace() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let outside_dir = tmp.path().join("outside");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::create_dir_all(&outside_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["echo", "ok"],
                "cwd": outside_dir.display().to_string(),
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");
        assert!(
            payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("cwd_outside_workspace"))
        );

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_allows_cwd_outside_workspace_with_full_access() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let outside_dir = tmp.path().join("outside");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::create_dir_all(&outside_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: Some(policy_meta::WriteScope::FullAccess),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["echo", "ok"],
                "cwd": outside_dir.display().to_string(),
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "run");

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_full_access_still_enforces_network_policy() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let mut exec_policy = omne_execpolicy::Policy::empty();
        exec_policy.add_prefix_rule(&["curl".to_string()], ExecDecision::Forbidden)?;

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoDeny),
                sandbox_policy: Some(policy_meta::WriteScope::FullAccess),
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["curl", "https://example.com"],
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");
        assert!(
            payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("sandbox_network_access=deny"))
        );

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_denies_when_mode_disallows_commands() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("chat".to_string()),
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["echo", "ok"],
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "deny");
        assert_eq!(payload["reason"], "mode denies process/execve");
        assert_eq!(payload["mode"], "chat");
        assert_eq!(payload["mode_decision"], "deny");
        assert_eq!(payload["decision_source"], "mode_permission");
        assert_eq!(payload["tool_override_hit"].as_bool(), Some(false));

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_mode_prompt_can_request_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("default".to_string()),
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["echo", "ok"],
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "escalate");
        let approval_id: omne_protocol::ApprovalId = payload["approval_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing approval_id"))?
            .parse()?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let approval_event = events.into_iter().find_map(|event| match event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: got,
                action,
                params,
                ..
            } if got == approval_id => Some((action, params)),
            _ => None,
        });
        let (action, params) =
            approval_event.ok_or_else(|| anyhow::anyhow!("missing approval request event"))?;
        assert_eq!(action, "process/execve");
        assert_eq!(params["approval"]["source"], "execve-wrapper");
        assert_eq!(params["approval"]["requirement"], "prompt");
        assert_eq!(params["argv"], serde_json::json!(["echo", "ok"]));

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_unmatched_execpolicy_matches_process_start_fallback() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  mode-x:
    description: "mode x"
    permissions:
      command:
        decision: allow
"#,
        )
        .await?;

        let exec_policy = omne_execpolicy::Policy::empty();

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("mode-x".to_string()),
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy,
                thread_rt,
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({
                "token": token,
                "argv": ["echo", "ok"],
            }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "escalate");
        let approval_id: omne_protocol::ApprovalId = payload["approval_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing approval_id"))?
            .parse()?;
        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let approval_event = events.into_iter().find_map(|event| match event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: got,
                action,
                params,
                ..
            } if got == approval_id => Some((action, params)),
            _ => None,
        });
        let (action, params) =
            approval_event.ok_or_else(|| anyhow::anyhow!("missing approval request event"))?;
        assert_eq!(action, "process/execve");
        assert_eq!(params["approval"]["source"], "execve-wrapper");
        assert_eq!(params["approval"]["requirement"], "prompt");
        assert_eq!(params["argv"], serde_json::json!(["echo", "ok"]));

        shutdown_execve_gate(gate).await;
        Ok(())
    }

    #[tokio::test]
    async fn execve_gate_escalate_and_wait_roundtrip() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let mut exec_policy = omne_execpolicy::Policy::empty();
        exec_policy.add_prefix_rule(&["git".to_string()], ExecDecision::PromptStrict)?;

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy.clone();
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = match spawn_execve_gate(
            ExecveGateContext {
                thread_id,
                turn_id: None,
                token: token.clone(),
                thread_root: repo_dir,
                thread_store: server.thread_store.clone(),
                exec_policy: exec_policy.clone(),
                thread_rt: thread_rt.clone(),
            },
            socket_path.clone(),
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) if unix_socket_bind_not_permitted(&err) => {
                eprintln!("skipping execve gate test: unix socket bind not permitted");
                return Ok(());
            }
            Err(err) => return Err(err),
        };

        let stream = tokio::net::UnixStream::connect(&socket_path).await?;
        let (read_half, mut write_half) = stream.into_split();
        let mut lines = tokio::io::BufReader::new(read_half).lines();
        mcp_initialize(&mut lines, &mut write_half).await?;

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            2,
            EXECVE_GATE_TOOL_DECIDE,
            serde_json::json!({ "token": token.clone(), "argv": ["git", "status"] }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "escalate");
        let approval_id: omne_protocol::ApprovalId = payload["approval_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing approval_id"))?
            .parse()?;

        tokio::spawn({
            let thread_rt = thread_rt.clone();
            async move {
                let _ = thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_protocol::ApprovalDecision::Approved,
                        remember: false,
                        reason: Some("ok".to_string()),
                    })
                    .await;
            }
        });

        let resp = mcp_tools_call(
            &mut lines,
            &mut write_half,
            3,
            EXECVE_GATE_TOOL_WAIT,
            serde_json::json!({ "token": token, "approval_id": approval_id }),
        )
        .await?;
        let payload = mcp_payload(&resp)?;
        assert_eq!(payload["decision"], "run");

        shutdown_execve_gate(gate).await;
        Ok(())
    }
}
