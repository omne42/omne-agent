use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpInitState {
    NotInitialized,
    InitializeResponded,
    Ready,
}

struct McpServeState {
    init: McpInitState,
    audit_thread_id: Option<omne_agent_protocol::ThreadId>,
}

impl McpServeState {
    fn new(audit_thread_id: Option<omne_agent_protocol::ThreadId>) -> Self {
        Self {
            init: McpInitState::NotInitialized,
            audit_thread_id,
        }
    }
}

async fn run_mcp_serve(app: &mut App, args: McpServeArgs) -> anyhow::Result<()> {
    #[derive(Debug, Deserialize)]
    struct ThreadStartResult {
        thread_id: omne_agent_protocol::ThreadId,
    }

    let (audit_thread_id, configure_audit_thread) = match (args.no_audit, args.audit_thread_id) {
        (true, _) => (None, false),
        (false, Some(thread_id)) => (Some(thread_id), false),
        (false, None) => {
            let audit_cwd = args
                .audit_cwd
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let started = app.thread_start(Some(audit_cwd.display().to_string())).await?;
            let started: ThreadStartResult =
                serde_json::from_value(started).context("parse thread/start for mcp audit thread")?;
            (Some(started.thread_id), true)
        }
    };

    if configure_audit_thread {
        if let Some(thread_id) = audit_thread_id {
            let _ = app
                .rpc(
                    "thread/configure",
                    serde_json::json!({
                        "thread_id": thread_id,
                        "approval_policy": null,
                        "sandbox_policy": omne_agent_protocol::SandboxPolicy::ReadOnly,
                        "sandbox_writable_roots": null,
                        "sandbox_network_access": null,
                        "mode": "reviewer",
                        "model": null,
                        "openai_base_url": null,
                    }),
                )
                .await?;
        }
    }

    let mut state = McpServeState::new(audit_thread_id);

    let stdin = tokio::io::stdin();
    let mut stdin = tokio::io::BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = stdin.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                let resp = jsonrpc_error(Value::Null, -32700, "parse error", Some(err.to_string()));
                write_json_line(&mut stdout, &resp).await?;
                continue;
            }
        };

        let Some(obj) = msg.as_object() else {
            let resp = jsonrpc_error(Value::Null, -32600, "invalid request", None::<String>);
            write_json_line(&mut stdout, &resp).await?;
            continue;
        };

        let method = obj.get("method").and_then(|v| v.as_str());
        let id = obj.get("id").cloned();
        let params = obj.get("params").cloned();

        let Some(method) = method else {
            let resp = jsonrpc_error(Value::Null, -32600, "invalid request", None::<String>);
            write_json_line(&mut stdout, &resp).await?;
            continue;
        };

        // Notifications (no id) are best-effort and never responded to.
        let Some(id) = id else {
            if method == "notifications/initialized" && state.init == McpInitState::InitializeResponded {
                state.init = McpInitState::Ready;
            }
            continue;
        };
        if id.is_null() {
            continue;
        }

        let resp = match method {
            "initialize" => handle_mcp_initialize(&mut state, id).await,
            "tools/list" => handle_mcp_tools_list(&mut state, id).await,
            "tools/call" => handle_mcp_tools_call(app, &mut state, id, params).await,
            "resources/list" => handle_mcp_resources_list(&mut state, id).await,
            "prompts/list" => handle_mcp_prompts_list(&mut state, id).await,
            _ => jsonrpc_error(id, -32601, "method not found", None::<String>),
        };

        write_json_line(&mut stdout, &resp).await?;
    }

    Ok(())
}

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

fn jsonrpc_ok(id: Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

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

async fn handle_mcp_initialize(state: &mut McpServeState, id: Value) -> Value {
    if state.init != McpInitState::NotInitialized {
        return jsonrpc_error(id, -32000, "already initialized", None::<String>);
    }

    state.init = McpInitState::InitializeResponded;
    jsonrpc_ok(
        id,
        serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "serverInfo": {
                "name": "omne-agent",
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

async fn handle_mcp_tools_list(state: &mut McpServeState, id: Value) -> Value {
    if state.init == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    jsonrpc_ok(
        id,
        serde_json::json!({
            "tools": mcp_tools(),
        }),
    )
}

async fn handle_mcp_resources_list(state: &mut McpServeState, id: Value) -> Value {
    if state.init == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    jsonrpc_ok(
        id,
        serde_json::json!({
            "resources": [],
        }),
    )
}

async fn handle_mcp_prompts_list(state: &mut McpServeState, id: Value) -> Value {
    if state.init == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    jsonrpc_ok(
        id,
        serde_json::json!({
            "prompts": [],
        }),
    )
}

async fn handle_mcp_tools_call(
    app: &mut App,
    state: &mut McpServeState,
    id: Value,
    params: Option<Value>,
) -> Value {
    if state.init != McpInitState::Ready {
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

    let outcome = dispatch_mcp_tool(app, &name, &arguments).await;
    if let Some(thread_id) = state.audit_thread_id {
        let audit = write_mcp_audit_artifact(app, thread_id, &name, &arguments, &outcome).await;
        if let Err(err) = audit {
            eprintln!("mcp serve: audit artifact write failed: {err}");
        }
    }

    match outcome {
        Ok(payload) => jsonrpc_ok(id, mcp_tool_ok(payload)),
        Err(err) => jsonrpc_ok(id, mcp_tool_err(err.to_string())),
    }
}

fn mcp_tool_ok(payload: Value) -> Value {
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
        }],
        "isError": false,
    })
}

fn mcp_tool_err(message: String) -> Value {
    serde_json::json!({
        "content": [{
            "type": "text",
            "text": message,
        }],
        "isError": true,
    })
}

async fn write_mcp_audit_artifact(
    app: &mut App,
    thread_id: omne_agent_protocol::ThreadId,
    tool_name: &str,
    arguments: &Value,
    outcome: &anyhow::Result<Value>,
) -> anyhow::Result<()> {
    let summary = format!("mcp serve tool: {tool_name}");
    let record = match outcome {
        Ok(_) => serde_json::json!({ "tool": tool_name, "ok": true, "arguments": arguments }),
        Err(err) => serde_json::json!({
            "tool": tool_name,
            "ok": false,
            "arguments": arguments,
            "error": err.to_string(),
        }),
    };

    let text = format!(
        "# MCP Server Call\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(&record).unwrap_or_else(|_| record.to_string())
    );

    let _ = app
        .rpc(
            "artifact/write",
            serde_json::json!({
                "thread_id": thread_id,
                "turn_id": null,
                "approval_id": null,
                "artifact_id": null,
                "artifact_type": "mcp_server_call",
                "summary": summary,
                "text": text,
            }),
        )
        .await?;
    Ok(())
}

async fn dispatch_mcp_tool(app: &mut App, name: &str, args: &Value) -> anyhow::Result<Value> {
    match name {
        "omne_agent.thread.list_meta" => {
            let include_archived = args
                .get("include_archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            app.thread_list_meta(include_archived).await
        }
        "omne_agent.thread.attention" => {
            let thread_id = parse_required::<omne_agent_protocol::ThreadId>(args, "thread_id")?;
            app.thread_attention(thread_id).await
        }
        "omne_agent.thread.state" => {
            let thread_id = parse_required::<omne_agent_protocol::ThreadId>(args, "thread_id")?;
            app.thread_state(thread_id).await
        }
        "omne_agent.thread.events" => {
            let thread_id = parse_required::<omne_agent_protocol::ThreadId>(args, "thread_id")?;
            let since_seq = args.get("since_seq").and_then(|v| v.as_u64()).unwrap_or(0);
            let max_events = args
                .get("max_events")
                .and_then(|v| v.as_u64())
                .and_then(|v| usize::try_from(v).ok());
            app.thread_events(thread_id, since_seq, max_events).await
        }
        "omne_agent.artifact.list" => {
            let thread_id = parse_required::<omne_agent_protocol::ThreadId>(args, "thread_id")?;
            app.artifact_list(thread_id, None).await
        }
        "omne_agent.artifact.read" => {
            let thread_id = parse_required::<omne_agent_protocol::ThreadId>(args, "thread_id")?;
            let artifact_id = parse_required::<omne_agent_protocol::ArtifactId>(args, "artifact_id")?;
            let max_bytes = args.get("max_bytes").and_then(|v| v.as_u64());
            app.artifact_read(thread_id, artifact_id, max_bytes, None)
                .await
        }
        "omne_agent.process.list" => {
            let thread_id = parse_optional::<omne_agent_protocol::ThreadId>(args, "thread_id")?;
            app.process_list(thread_id).await
        }
        "omne_agent.process.inspect" => {
            let process_id = parse_required::<omne_agent_protocol::ProcessId>(args, "process_id")?;
            let max_lines = args
                .get("max_lines")
                .and_then(|v| v.as_u64())
                .and_then(|v| usize::try_from(v).ok());
            app.process_inspect(process_id, max_lines, None).await
        }
        "omne_agent.process.tail" => {
            let process_id = parse_required::<omne_agent_protocol::ProcessId>(args, "process_id")?;
            let stderr = args.get("stderr").and_then(|v| v.as_bool()).unwrap_or(false);
            let max_lines = args
                .get("max_lines")
                .and_then(|v| v.as_u64())
                .and_then(|v| usize::try_from(v).ok());
            let text = app.process_tail(process_id, stderr, max_lines, None).await?;
            Ok(serde_json::json!({ "text": text }))
        }
        "omne_agent.process.follow" => {
            let process_id = parse_required::<omne_agent_protocol::ProcessId>(args, "process_id")?;
            let stderr = args.get("stderr").and_then(|v| v.as_bool()).unwrap_or(false);
            let since_offset = args.get("since_offset").and_then(|v| v.as_u64()).unwrap_or(0);
            let max_bytes = args.get("max_bytes").and_then(|v| v.as_u64());
            let (text, next_offset, eof) = app
                .process_follow(process_id, stderr, since_offset, max_bytes, None)
                .await?;
            Ok(serde_json::json!({ "text": text, "next_offset": next_offset, "eof": eof }))
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn parse_required<T>(args: &Value, key: &str) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let Some(value) = args.get(key) else {
        anyhow::bail!("missing required argument: {key}");
    };
    serde_json::from_value(value.clone()).with_context(|| format!("parse argument: {key}"))
}

fn parse_optional<T>(args: &Value, key: &str) -> anyhow::Result<Option<T>>
where
    T: serde::de::DeserializeOwned,
{
    match args.get(key) {
        Some(v) if !v.is_null() => Ok(Some(
            serde_json::from_value(v.clone()).with_context(|| format!("parse argument: {key}"))?,
        )),
        _ => Ok(None),
    }
}

fn mcp_tools() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "omne_agent.thread.list_meta",
            "description": "List omne-agent threads (metadata).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_archived": { "type": "boolean" }
                },
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.thread.attention",
            "description": "Get a thread's attention state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" }
                },
                "required": ["thread_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.thread.state",
            "description": "Get a thread's current state (includes approvals/processes summary).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" }
                },
                "required": ["thread_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.thread.events",
            "description": "Poll thread events since a sequence number (read-only, for clients that don't use subscribe).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                    "since_seq": { "type": "integer", "minimum": 0 },
                    "max_events": { "type": "integer", "minimum": 1 }
                },
                "required": ["thread_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.artifact.list",
            "description": "List artifacts in a thread.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" }
                },
                "required": ["thread_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.artifact.read",
            "description": "Read an artifact (content is redacted on the server side).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                    "artifact_id": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 }
                },
                "required": ["thread_id", "artifact_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.process.list",
            "description": "List processes (optionally filtered by thread).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" }
                },
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.process.inspect",
            "description": "Inspect a process (includes redacted stdout/stderr tail).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "max_lines": { "type": "integer", "minimum": 0 }
                },
                "required": ["process_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.process.tail",
            "description": "Tail a process log (redacted on the server side).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "stderr": { "type": "boolean" },
                    "max_lines": { "type": "integer", "minimum": 1 }
                },
                "required": ["process_id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "omne_agent.process.follow",
            "description": "Follow a process log from an offset (redacted on the server side).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "stderr": { "type": "boolean" },
                    "since_offset": { "type": "integer", "minimum": 0 },
                    "max_bytes": { "type": "integer", "minimum": 1 }
                },
                "required": ["process_id"],
                "additionalProperties": false
            }
        }),
    ]
}

#[cfg(test)]
mod mcp_server_tests {
    use super::*;

    #[test]
    fn mcp_tool_list_contains_expected_entries() {
        let tools = mcp_tools();
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
            .collect::<Vec<_>>();
        assert!(names.contains(&"omne_agent.thread.list_meta"));
        assert!(names.contains(&"omne_agent.thread.events"));
        assert!(names.contains(&"omne_agent.artifact.read"));
        assert!(names.contains(&"omne_agent.process.follow"));
    }

    #[test]
    fn jsonrpc_error_includes_number_and_message() {
        let v = jsonrpc_error(Value::from(1), -32601, "method not found", None::<String>);
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        assert_eq!(v["error"]["code"], -32601);
        assert_eq!(v["error"]["message"], "method not found");
    }
}
