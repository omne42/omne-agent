#[cfg(unix)]
const EXECVE_GATE_MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[cfg(unix)]
const EXECVE_GATE_TOOL_DECIDE: &str = "omne.execve.decide";
#[cfg(unix)]
const EXECVE_GATE_TOOL_WAIT: &str = "omne.execve.wait";

#[cfg(unix)]
struct ExecveGateHandle {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    socket_path: PathBuf,
}

#[cfg(not(unix))]
struct ExecveGateHandle;

#[cfg(unix)]
#[derive(Clone)]
struct ExecveGateContext {
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    token: String,
    thread_root: PathBuf,
    thread_store: ThreadStore,
    exec_policy: omne_execpolicy::Policy,
    thread_rt: Arc<ThreadRuntime>,
}

#[cfg(unix)]
async fn spawn_execve_gate(ctx: ExecveGateContext, socket_path: PathBuf) -> anyhow::Result<ExecveGateHandle> {
    use std::os::unix::fs::PermissionsExt;

    match tokio::fs::remove_file(&socket_path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err)
                .with_context(|| format!("remove stale execve gate socket {}", socket_path.display()));
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
async fn shutdown_execve_gate(gate: ExecveGateHandle) {
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
async fn shutdown_execve_gate(_: ExecveGateHandle) {}

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

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                let resp = jsonrpc_error(Value::Null, -32700, "parse error", Some(err.to_string()));
                write_json_line(&mut write_half, &resp).await?;
                continue;
            }
        };

        let Some(obj) = msg.as_object() else {
            let resp = jsonrpc_error(Value::Null, -32600, "invalid request", None::<String>);
            write_json_line(&mut write_half, &resp).await?;
            continue;
        };

        let method = obj.get("method").and_then(|v| v.as_str());
        let id = obj.get("id").cloned();
        let params = obj.get("params").cloned();

        let Some(method) = method else {
            let resp = jsonrpc_error(Value::Null, -32600, "invalid request", None::<String>);
            write_json_line(&mut write_half, &resp).await?;
            continue;
        };

        // Notifications (no id) are best-effort and never responded to.
        let Some(id) = id else {
            if method == "notifications/initialized" && init_state == McpInitState::InitializeResponded
            {
                init_state = McpInitState::Ready;
            }
            continue;
        };
        if id.is_null() {
            continue;
        }

        let resp = match method {
            "initialize" => handle_mcp_initialize(&mut init_state, id).await,
            "tools/list" => handle_mcp_tools_list(&init_state, id).await,
            "tools/call" => handle_mcp_tools_call(ctx.clone(), &mut init_state, id, params, cancel.clone()).await,
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
        return jsonrpc_error(id, -32602, "invalid params", Some("params must be an object"));
    };
    let Some(name) = obj.get("name").and_then(|v| v.as_str()).map(str::to_string) else {
        return jsonrpc_error(id, -32602, "invalid params", Some("missing tool name"));
    };
    let arguments = obj.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));

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
fn execpolicy_allow_fallback(_: &[String]) -> ExecDecision {
    ExecDecision::Allow
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

    let (
        approval_policy,
        sandbox_policy,
        sandbox_network_access,
        mode_name,
        thread_execpolicy_rules,
    ) = {
        let handle = ctx.thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_network_access,
            state.mode.clone(),
            state.execpolicy_rules.clone(),
        )
    };

    if sandbox_policy == omne_protocol::SandboxPolicy::ReadOnly {
        return Ok(serde_json::json!({
            "decision": "deny",
            "reason": "sandbox_policy=read_only forbids execve",
        }));
    }

    if sandbox_network_access == omne_protocol::SandboxNetworkAccess::Deny
        && omne_process_runtime::command_uses_network(&args.argv)
    {
        return Ok(serde_json::json!({
            "decision": "deny",
            "reason": "sandbox_network_access=deny forbids this command",
        }));
    }

    let catalog = omne_core::modes::ModeCatalog::load(&ctx.thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            return Ok(serde_json::json!({
                "decision": "deny",
                "reason": "unknown mode",
                "mode": mode_name,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let mut effective_exec_policy = ctx.exec_policy.clone();
    if !mode.command_execpolicy_rules.is_empty() {
        let mode_exec_policy = match load_mode_exec_policy(&ctx.thread_root, &mode.command_execpolicy_rules).await
        {
            Ok(policy) => policy,
            Err(err) => {
                return Ok(serde_json::json!({
                    "decision": "deny",
                    "reason": "failed to load mode execpolicy rules",
                    "mode": mode_name,
                    "rules": mode.command_execpolicy_rules.clone(),
                    "details": err.to_string(),
                }));
            }
        };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &mode_exec_policy);
    }
    if !thread_execpolicy_rules.is_empty() {
        let thread_exec_policy = match load_mode_exec_policy(&ctx.thread_root, &thread_execpolicy_rules).await {
            Ok(policy) => policy,
            Err(err) => {
                return Ok(serde_json::json!({
                    "decision": "deny",
                    "reason": "failed to load thread execpolicy rules",
                    "mode": mode_name,
                    "rules": thread_execpolicy_rules.clone(),
                    "details": err.to_string(),
                }));
            }
        };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &thread_exec_policy);
    }
    let exec_matches = effective_exec_policy
        .matches_for_command(&args.argv, Some(&execpolicy_allow_fallback));
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();

    let effective_exec_decision = match exec_decision {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };

    if effective_exec_decision == ExecDecision::Forbidden {
        let exec_matches_json = serde_json::to_value(&exec_matches)?;
        let justification = exec_matches.iter().find_map(|m| match m {
            ExecRuleMatch::PrefixRuleMatch {
                decision: ExecDecision::Forbidden,
                justification,
                ..
            } => justification.clone(),
            _ => None,
        });

        return Ok(serde_json::json!({
            "decision": "deny",
            "reason": "execpolicy forbids this command",
            "matched_rules": exec_matches_json,
            "justification": justification,
        }));
    }

    if effective_exec_decision == ExecDecision::Allow {
        return Ok(serde_json::json!({ "decision": "run" }));
    }

    let approval_params = serde_json::json!({
        "argv": args.argv.clone(),
        "cwd": args
            .cwd
            .unwrap_or_else(|| ctx.thread_root.display().to_string()),
        "approval": {
            "source": "execve-wrapper",
            "requirement": match effective_exec_decision {
                ExecDecision::PromptStrict => "prompt_strict",
                _ => "prompt",
            }
        }
    });

    match gate_approval_with_deps(
        &ctx.thread_store,
        &effective_exec_policy,
        &ctx.thread_rt,
        ctx.thread_id,
        ctx.turn_id,
        approval_policy,
        ApprovalRequest {
            approval_id: None,
            action: "process/execve",
            params: &approval_params,
        },
    )
    .await?
    {
        ApprovalGate::Approved => Ok(serde_json::json!({ "decision": "run" })),
        ApprovalGate::Denied { remembered } => Ok(serde_json::json!({
            "decision": "deny",
            "reason": approval_denied_error(remembered),
        })),
        ApprovalGate::NeedsApproval { approval_id } => Ok(serde_json::json!({
            "decision": "escalate",
            "approval_id": approval_id,
        })),
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

    fn build_test_server(omne_root: PathBuf, exec_policy: omne_execpolicy::Policy) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: omne_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(omne_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy,
        }
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

        let server = build_test_server(tmp.path().join(".omne_data"), exec_policy.clone());
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = spawn_execve_gate(
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
        .await?;

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
    async fn execve_gate_escalate_and_wait_roundtrip() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let mut exec_policy = omne_execpolicy::Policy::empty();
        exec_policy.add_prefix_rule(&["git".to_string()], ExecDecision::PromptStrict)?;

        let server = build_test_server(tmp.path().join(".omne_data"), exec_policy.clone());
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let socket_path = tmp.path().join("execve.sock");
        let token = "test-token".to_string();

        let gate = spawn_execve_gate(
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
        .await?;

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
