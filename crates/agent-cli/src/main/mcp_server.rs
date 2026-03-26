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
    audit_thread_id: Option<omne_protocol::ThreadId>,
}

impl McpServeState {
    fn new(audit_thread_id: Option<omne_protocol::ThreadId>) -> Self {
        Self {
            init: McpInitState::NotInitialized,
            audit_thread_id,
        }
    }
}

async fn run_mcp_serve(app: &mut App, args: McpServeArgs) -> anyhow::Result<()> {
    let (audit_thread_id, configure_audit_thread) = match (args.no_audit, args.audit_thread_id) {
        (true, _) => (None, false),
        (false, Some(thread_id)) => (Some(thread_id), false),
        (false, None) => {
            let audit_cwd = args
                .audit_cwd
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let started = app.thread_start(Some(audit_cwd.display().to_string())).await?;
            ensure_thread_start_auto_hook_ready("mcp/serve", &started)?;
            (Some(started.thread_id), true)
        }
    };

    if configure_audit_thread {
        if let Some(thread_id) = audit_thread_id {
            app.thread_configure_rpc(omne_app_server_protocol::ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: Some(policy_meta::WriteScope::ReadOnly),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("reviewer".to_string()),
                role: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;
        }
    }

    let mut state = McpServeState::new(audit_thread_id);

    let stdin = tokio::io::stdin();
    let mut stdin = tokio::io::BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = stdin.next_line().await? {
        let message = match omne_core::jsonrpc_line::parse_jsonrpc_line(&line) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(err) => {
                let resp = jsonrpc_error(err.id, err.code, err.message, err.data);
                write_json_line(&mut stdout, &resp).await?;
                continue;
            }
        };

        let (id, method, params) = match message {
            omne_core::jsonrpc_line::JsonRpcLine::Notification(notification) => {
                if notification.method == "notifications/initialized"
                    && state.init == McpInitState::InitializeResponded
                {
                    state.init = McpInitState::Ready;
                }
                continue;
            }
            omne_core::jsonrpc_line::JsonRpcLine::Request(request) => {
                (request.id, request.method, request.params)
            }
        };

        let resp = match method.as_str() {
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
    #[derive(Serialize)]
    struct JsonRpcOk<T> {
        jsonrpc: &'static str,
        id: Value,
        result: T,
    }

    serde_json::to_value(JsonRpcOk {
        jsonrpc: "2.0",
        id,
        result,
    })
    .unwrap_or_else(|_| {
        let mut response = serde_json::Map::new();
        response.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
        response.insert("id".to_string(), Value::Null);
        response.insert("result".to_string(), Value::Null);
        Value::Object(response)
    })
}

fn jsonrpc_error<T: serde::Serialize>(
    id: Value,
    code: i64,
    message: &str,
    data: Option<T>,
) -> Value {
    #[derive(Serialize)]
    struct JsonRpcErrorData<T> {
        code: i64,
        message: String,
        data: Option<T>,
    }

    #[derive(Serialize)]
    struct JsonRpcError<T> {
        jsonrpc: &'static str,
        id: Value,
        error: JsonRpcErrorData<T>,
    }

    serde_json::to_value(JsonRpcError {
        jsonrpc: "2.0",
        id,
        error: JsonRpcErrorData {
            code,
            message: message.to_string(),
            data,
        },
    })
    .unwrap_or_else(|_| {
        let mut error = serde_json::Map::new();
        error.insert("code".to_string(), Value::from(code));
        error.insert("message".to_string(), Value::String(message.to_string()));
        error.insert("data".to_string(), Value::Null);

        let mut response = serde_json::Map::new();
        response.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
        response.insert("id".to_string(), Value::Null);
        response.insert("error".to_string(), Value::Object(error));
        Value::Object(response)
    })
}

async fn handle_mcp_initialize(state: &mut McpServeState, id: Value) -> Value {
    if state.init != McpInitState::NotInitialized {
        return jsonrpc_error(id, -32000, "already initialized", None::<String>);
    }

    state.init = McpInitState::InitializeResponded;
    #[derive(Serialize)]
    struct InitializeServerInfo {
        name: &'static str,
        version: &'static str,
    }

    #[derive(Serialize)]
    struct InitializeToolsCapability {
        #[serde(rename = "listChanged")]
        list_changed: bool,
    }

    #[derive(Serialize)]
    struct InitializeResourcesCapability {
        subscribe: bool,
        #[serde(rename = "listChanged")]
        list_changed: bool,
    }

    #[derive(Serialize)]
    struct InitializePromptsCapability {
        #[serde(rename = "listChanged")]
        list_changed: bool,
    }

    #[derive(Serialize)]
    struct InitializeCapabilities {
        tools: InitializeToolsCapability,
        resources: InitializeResourcesCapability,
        prompts: InitializePromptsCapability,
    }

    #[derive(Serialize)]
    struct InitializeResult {
        #[serde(rename = "protocolVersion")]
        protocol_version: &'static str,
        #[serde(rename = "serverInfo")]
        server_info: InitializeServerInfo,
        capabilities: InitializeCapabilities,
    }

    let result = serde_json::to_value(InitializeResult {
        protocol_version: MCP_PROTOCOL_VERSION,
        server_info: InitializeServerInfo {
            name: "omneagent",
            version: env!("CARGO_PKG_VERSION"),
        },
        capabilities: InitializeCapabilities {
            tools: InitializeToolsCapability { list_changed: false },
            resources: InitializeResourcesCapability {
                subscribe: false,
                list_changed: false,
            },
            prompts: InitializePromptsCapability { list_changed: false },
        },
    })
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    jsonrpc_ok(id, result)
}

async fn handle_mcp_tools_list(state: &mut McpServeState, id: Value) -> Value {
    if state.init == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    #[derive(Serialize)]
    struct ToolsListResult {
        tools: Vec<Value>,
    }
    let result = serde_json::to_value(ToolsListResult { tools: mcp_tools() })
        .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    jsonrpc_ok(id, result)
}

async fn handle_mcp_resources_list(state: &mut McpServeState, id: Value) -> Value {
    if state.init == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    #[derive(Serialize)]
    struct ResourcesListResult {
        resources: Vec<Value>,
    }
    let result = serde_json::to_value(ResourcesListResult {
        resources: Vec::new(),
    })
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    jsonrpc_ok(id, result)
}

async fn handle_mcp_prompts_list(state: &mut McpServeState, id: Value) -> Value {
    if state.init == McpInitState::NotInitialized {
        return jsonrpc_error(id, -32001, "not initialized", None::<String>);
    }
    #[derive(Serialize)]
    struct PromptsListResult {
        prompts: Vec<Value>,
    }
    let result = serde_json::to_value(PromptsListResult {
        prompts: Vec::new(),
    })
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    jsonrpc_ok(id, result)
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
    let parsed = match parse_mcp_tools_call_params(params) {
        Ok(parsed) => parsed,
        Err(err) => {
            return jsonrpc_error(id, -32602, "invalid params", Some(err.to_string()));
        }
    };

    let outcome = dispatch_pm_mcp_tool(app, &parsed.name, &parsed.arguments).await;
    if let Some(thread_id) = state.audit_thread_id {
        let audit =
            write_mcp_audit_artifact(app, thread_id, &parsed.name, &parsed.arguments, &outcome)
                .await;
        if let Err(err) = audit {
            eprintln!("mcp serve: audit artifact write failed: {err}");
        }
    }

    match outcome {
        Ok(payload) => jsonrpc_ok(id, mcp_tool_ok(payload)),
        Err(err) => jsonrpc_ok(id, mcp_tool_err(err.to_string())),
    }
}

fn default_json_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpToolsCallParams {
    name: String,
    #[serde(default = "default_json_object")]
    arguments: Value,
}

fn parse_mcp_tools_call_params(params: Value) -> anyhow::Result<McpToolsCallParams> {
    if !params.is_object() {
        anyhow::bail!("params must be an object");
    }
    serde_json::from_value(params).context("parse tools/call params")
}

#[derive(Serialize)]
struct McpToolContentText {
    #[serde(rename = "type")]
    content_type: &'static str,
    text: String,
}

#[derive(Serialize)]
struct McpToolCallResult {
    content: Vec<McpToolContentText>,
    #[serde(rename = "isError")]
    is_error: bool,
}

fn mcp_tool_ok(payload: Value) -> Value {
    let result = McpToolCallResult {
        content: vec![McpToolContentText {
            content_type: "text",
            text: serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
        }],
        is_error: false,
    };
    serde_json::to_value(result).unwrap_or_else(|_| {
        let mut content_item = serde_json::Map::new();
        content_item.insert("type".to_string(), Value::String("text".to_string()));
        content_item.insert("text".to_string(), Value::String(payload.to_string()));

        let mut fallback = serde_json::Map::new();
        fallback.insert(
            "content".to_string(),
            Value::Array(vec![Value::Object(content_item)]),
        );
        fallback.insert("isError".to_string(), Value::Bool(false));
        Value::Object(fallback)
    })
}

fn mcp_tool_err(message: String) -> Value {
    let result = McpToolCallResult {
        content: vec![McpToolContentText {
            content_type: "text",
            text: message,
        }],
        is_error: true,
    };
    serde_json::to_value(result).unwrap_or_else(|_| {
        let mut content_item = serde_json::Map::new();
        content_item.insert("type".to_string(), Value::String("text".to_string()));
        content_item.insert("text".to_string(), Value::String(String::new()));

        let mut fallback = serde_json::Map::new();
        fallback.insert(
            "content".to_string(),
            Value::Array(vec![Value::Object(content_item)]),
        );
        fallback.insert("isError".to_string(), Value::Bool(true));
        Value::Object(fallback)
    })
}

async fn write_mcp_audit_artifact(
    app: &mut App,
    thread_id: omne_protocol::ThreadId,
    tool_name: &str,
    arguments: &Value,
    outcome: &anyhow::Result<Value>,
) -> anyhow::Result<()> {
    let summary = format!("mcp serve tool: {tool_name}");
    #[derive(Serialize)]
    struct McpAuditRecord {
        tool: String,
        ok: bool,
        arguments: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    }
    let record = match outcome {
        Ok(_) => McpAuditRecord {
            tool: tool_name.to_string(),
            ok: true,
            arguments: arguments.clone(),
            error: None,
        },
        Err(err) => McpAuditRecord {
            tool: tool_name.to_string(),
            ok: false,
            arguments: arguments.clone(),
            error: Some(err.to_string()),
        },
    };
    let record = serde_json::to_value(record).context("serialize mcp serve audit record")?;

    let text = format!(
        "# MCP Server Call\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(&record).unwrap_or_else(|_| record.to_string())
    );

    let _ = app
        .artifact_write(omne_app_server_protocol::ArtifactWriteParams {
            thread_id,
            turn_id: None,
            approval_id: None,
            artifact_id: None,
            artifact_type: "mcp_server_call".to_string(),
            summary,
            text,
        })
        .await?;
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadListMetaToolArgs {
    #[serde(default)]
    include_archived: bool,
    #[serde(default)]
    include_attention_markers: bool,
}

fn parse_thread_list_meta_tool_args(args: &Value) -> anyhow::Result<ThreadListMetaToolArgs> {
    serde_json::from_value(args.clone()).context("parse tool arguments: omne.thread.list_meta")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadAttentionToolArgs {
    thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadStateToolArgs {
    thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactListToolArgs {
    thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReadToolArgs {
    thread_id: omne_protocol::ThreadId,
    artifact_id: omne_protocol::ArtifactId,
    #[serde(default)]
    version: Option<std::num::NonZeroU32>,
    #[serde(default)]
    max_bytes: Option<std::num::NonZeroU64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactVersionsToolArgs {
    thread_id: omne_protocol::ThreadId,
    artifact_id: omne_protocol::ArtifactId,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessListToolArgs {
    #[serde(default)]
    thread_id: Option<omne_protocol::ThreadId>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessInspectToolArgs {
    process_id: omne_protocol::ProcessId,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessTailToolArgs {
    process_id: omne_protocol::ProcessId,
    #[serde(default)]
    stderr: bool,
    #[serde(default)]
    max_lines: Option<std::num::NonZeroUsize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessFollowToolArgs {
    process_id: omne_protocol::ProcessId,
    #[serde(default)]
    stderr: bool,
    #[serde(default)]
    since_offset: u64,
    #[serde(default)]
    max_bytes: Option<std::num::NonZeroU64>,
}

#[derive(Serialize)]
struct ProcessTailToolResult {
    text: String,
}

#[derive(Serialize)]
struct ProcessFollowToolResult {
    text: String,
    next_offset: u64,
    eof: bool,
}

fn parse_tool_args<T>(args: &Value, tool_name: &str) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(args.clone())
        .with_context(|| format!("parse tool arguments: {tool_name}"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadEventsToolArgsRaw {
    thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<u64>,
    #[serde(default)]
    kinds: Option<Vec<String>>,
}

#[derive(Debug)]
struct ThreadEventsToolArgs {
    thread_id: omne_protocol::ThreadId,
    since_seq: u64,
    max_events: Option<usize>,
    kinds: Option<Vec<omne_protocol::ThreadEventKindTag>>,
}

fn parse_thread_events_tool_args(args: &Value) -> anyhow::Result<ThreadEventsToolArgs> {
    let raw: ThreadEventsToolArgsRaw =
        serde_json::from_value(args.clone()).context("parse tool arguments: omne.thread.events")?;
    let kinds = raw
        .kinds
        .map(|kinds| -> anyhow::Result<Vec<omne_protocol::ThreadEventKindTag>> {
            let normalized = omne_protocol::normalize_thread_event_kind_filter(&kinds).map_err(
                |invalid| {
                    anyhow::anyhow!(
                        "parse tool arguments: omne.thread.events unsupported kinds: {}; supported kinds: {}",
                        invalid.join(", "),
                        omne_protocol::THREAD_EVENT_KIND_TAGS.join(", "),
                    )
                },
            )?;
            let mut normalized = normalized.into_iter().collect::<Vec<_>>();
            normalized.sort_by_key(|kind| kind.as_str());
            Ok(normalized)
        })
        .transpose()?;
    let max_events = raw
        .max_events
        .map(|value| {
            usize::try_from(value).with_context(|| {
                format!("parse tool arguments: omne.thread.events.max_events out of range: {value}")
            })
        })
        .transpose()?;
    Ok(ThreadEventsToolArgs {
        thread_id: raw.thread_id,
        since_seq: raw.since_seq,
        max_events,
        kinds,
    })
}

fn mcp_tool(name: &str, description: &str, input_schema: Value) -> Value {
    let mut tool = serde_json::Map::new();
    tool.insert("name".to_string(), Value::String(name.to_string()));
    tool.insert(
        "description".to_string(),
        Value::String(description.to_string()),
    );
    tool.insert("inputSchema".to_string(), input_schema);
    Value::Object(tool)
}

fn object_schema(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let mut properties_map = serde_json::Map::new();
    for (key, value) in properties {
        properties_map.insert((*key).to_string(), value.clone());
    }

    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(properties_map));
    if !required.is_empty() {
        schema.insert(
            "required".to_string(),
            Value::Array(
                required
                    .iter()
                    .map(|name| Value::String((*name).to_string()))
                    .collect(),
            ),
        );
    }
    schema.insert("additionalProperties".to_string(), Value::Bool(false));
    Value::Object(schema)
}

fn schema_boolean() -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), Value::String("boolean".to_string()));
    Value::Object(schema)
}

fn schema_string() -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), Value::String("string".to_string()));
    Value::Object(schema)
}

fn schema_integer_with_minimum(minimum: u64) -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), Value::String("integer".to_string()));
    schema.insert("minimum".to_string(), Value::from(minimum));
    Value::Object(schema)
}

fn schema_array(items: Value) -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), Value::String("array".to_string()));
    schema.insert("items".to_string(), items);
    Value::Object(schema)
}

fn schema_string_enum(values: &[&str]) -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert("type".to_string(), Value::String("string".to_string()));
    schema.insert(
        "enum".to_string(),
        Value::Array(
            values
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    );
    Value::Object(schema)
}

async fn dispatch_pm_mcp_tool(app: &mut App, name: &str, args: &Value) -> anyhow::Result<Value> {
    match name {
        "omne.thread.list_meta" => {
            let parsed = parse_thread_list_meta_tool_args(args)?;
            let response = app
                .thread_list_meta(parsed.include_archived, parsed.include_attention_markers)
                .await?;
            serde_json::to_value(response).context("serialize thread/list_meta response")
        }
        "omne.thread.attention" => {
            let parsed = parse_tool_args::<ThreadAttentionToolArgs>(args, "omne.thread.attention")?;
            let response = app.thread_attention(parsed.thread_id).await?;
            serde_json::to_value(response).context("serialize thread/attention response")
        }
        "omne.thread.state" => {
            let parsed = parse_tool_args::<ThreadStateToolArgs>(args, "omne.thread.state")?;
            let response = app.thread_state(parsed.thread_id).await?;
            serde_json::to_value(response).context("serialize thread/state response")
        }
        "omne.thread.events" => {
            let parsed = parse_thread_events_tool_args(args)?;
            let response = app
                .thread_events(
                    parsed.thread_id,
                    parsed.since_seq,
                    parsed.max_events,
                    parsed.kinds,
                )
                .await?;
            serde_json::to_value(response).context("serialize thread/events response")
        }
        "omne.artifact.list" => {
            let parsed = parse_tool_args::<ArtifactListToolArgs>(args, "omne.artifact.list")?;
            let response = app.artifact_list(parsed.thread_id, None).await?;
            serde_json::to_value(response).context("serialize artifact/list response")
        }
        "omne.artifact.read" => {
            let parsed = parse_tool_args::<ArtifactReadToolArgs>(args, "omne.artifact.read")?;
            let response = app
                .artifact_read(
                    parsed.thread_id,
                    parsed.artifact_id,
                    parsed.version.map(std::num::NonZeroU32::get),
                    parsed.max_bytes.map(std::num::NonZeroU64::get),
                    None,
                )
                .await?;
            serde_json::to_value(response).context("serialize artifact/read response")
        }
        "omne.artifact.versions" => {
            let parsed =
                parse_tool_args::<ArtifactVersionsToolArgs>(args, "omne.artifact.versions")?;
            let response = app
                .artifact_versions(parsed.thread_id, parsed.artifact_id, None)
                .await?;
            serde_json::to_value(response).context("serialize artifact/versions response")
        }
        "omne.process.list" => {
            let parsed = parse_tool_args::<ProcessListToolArgs>(args, "omne.process.list")?;
            let response = app.process_list(parsed.thread_id).await?;
            serde_json::to_value(response).context("serialize process/list response")
        }
        "omne.process.inspect" => {
            let parsed = parse_tool_args::<ProcessInspectToolArgs>(args, "omne.process.inspect")?;
            let response = app
                .process_inspect(parsed.process_id, parsed.max_lines, None)
                .await?;
            serde_json::to_value(response).context("serialize process/inspect response")
        }
        "omne.process.tail" => {
            let parsed = parse_tool_args::<ProcessTailToolArgs>(args, "omne.process.tail")?;
            let text = app
                .process_tail(
                    parsed.process_id,
                    parsed.stderr,
                    parsed.max_lines.map(std::num::NonZeroUsize::get),
                    None,
                )
                .await?;
            serde_json::to_value(ProcessTailToolResult { text })
                .context("serialize process/tail tool response")
        }
        "omne.process.follow" => {
            let parsed = parse_tool_args::<ProcessFollowToolArgs>(args, "omne.process.follow")?;
            let (text, next_offset, eof) = app
                .process_follow(
                    parsed.process_id,
                    parsed.stderr,
                    parsed.since_offset,
                    parsed.max_bytes.map(std::num::NonZeroU64::get),
                    None,
                )
                .await?;
            serde_json::to_value(ProcessFollowToolResult {
                text,
                next_offset,
                eof,
            })
            .context("serialize process/follow tool response")
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    }
}

fn mcp_tools() -> Vec<Value> {
    vec![
        mcp_tool(
            "omne.thread.list_meta",
            "List OmneAgent threads (metadata, includes attention_state and has_plan_ready/has_diff_ready/has_fan_out_linkage_issue/has_fan_out_auto_apply_error/has_test_failed booleans).",
            object_schema(
                &[
                    ("include_archived", schema_boolean()),
                    ("include_attention_markers", schema_boolean()),
                ],
                &[],
            ),
        ),
        mcp_tool(
            "omne.thread.attention",
            "Get a thread's attention details (attention_state, marker booleans including has_fan_out_linkage_issue and has_fan_out_auto_apply_error, and optional attention_markers metadata).",
            object_schema(&[("thread_id", schema_string())], &["thread_id"]),
        ),
        mcp_tool(
            "omne.thread.state",
            "Get a thread's current state (includes approvals/processes summary).",
            object_schema(&[("thread_id", schema_string())], &["thread_id"]),
        ),
        mcp_tool(
            "omne.thread.events",
            "Poll thread events since a sequence number (read-only, for clients that don't use subscribe).",
            object_schema(
                &[
                    ("thread_id", schema_string()),
                    ("since_seq", schema_integer_with_minimum(0)),
                    ("max_events", schema_integer_with_minimum(1)),
                    (
                        "kinds",
                        schema_array(schema_string_enum(omne_protocol::THREAD_EVENT_KIND_TAGS)),
                    ),
                ],
                &["thread_id"],
            ),
        ),
        mcp_tool(
            "omne.artifact.list",
            "List artifacts in a thread.",
            object_schema(&[("thread_id", schema_string())], &["thread_id"]),
        ),
        mcp_tool(
            "omne.artifact.read",
            "Read an artifact (content is redacted on the server side).",
            object_schema(
                &[
                    ("thread_id", schema_string()),
                    ("artifact_id", schema_string()),
                    ("version", schema_integer_with_minimum(1)),
                    ("max_bytes", schema_integer_with_minimum(1)),
                ],
                &["thread_id", "artifact_id"],
            ),
        ),
        mcp_tool(
            "omne.artifact.versions",
            "List available versions for an artifact (latest + retained history versions).",
            object_schema(
                &[("thread_id", schema_string()), ("artifact_id", schema_string())],
                &["thread_id", "artifact_id"],
            ),
        ),
        mcp_tool(
            "omne.process.list",
            "List processes (optionally filtered by thread).",
            object_schema(&[("thread_id", schema_string())], &[]),
        ),
        mcp_tool(
            "omne.process.inspect",
            "Inspect a process (includes redacted stdout/stderr tail).",
            object_schema(
                &[
                    ("process_id", schema_string()),
                    ("max_lines", schema_integer_with_minimum(0)),
                ],
                &["process_id"],
            ),
        ),
        mcp_tool(
            "omne.process.tail",
            "Tail a process log (redacted on the server side).",
            object_schema(
                &[
                    ("process_id", schema_string()),
                    ("stderr", schema_boolean()),
                    ("max_lines", schema_integer_with_minimum(1)),
                ],
                &["process_id"],
            ),
        ),
        mcp_tool(
            "omne.process.follow",
            "Follow a process log from an offset (redacted on the server side).",
            object_schema(
                &[
                    ("process_id", schema_string()),
                    ("stderr", schema_boolean()),
                    ("since_offset", schema_integer_with_minimum(0)),
                    ("max_bytes", schema_integer_with_minimum(1)),
                ],
                &["process_id"],
            ),
        ),
    ]
}

#[cfg(test)]
mod mcp_server_tests {
    use super::*;

    #[test]
    fn thread_list_meta_tool_args_defaults_to_false() {
        let parsed = match parse_thread_list_meta_tool_args(&serde_json::json!({})) {
            Ok(parsed) => parsed,
            Err(err) => panic!("expected defaults parse to succeed: {err}"),
        };
        assert!(!parsed.include_archived);
        assert!(!parsed.include_attention_markers);
    }

    #[test]
    fn thread_list_meta_tool_args_parses_attention_markers_flag() {
        let parsed = match parse_thread_list_meta_tool_args(&serde_json::json!({
            "include_archived": true,
            "include_attention_markers": true
        })) {
            Ok(parsed) => parsed,
            Err(err) => panic!("expected explicit booleans parse to succeed: {err}"),
        };
        assert!(parsed.include_archived);
        assert!(parsed.include_attention_markers);
    }

    #[test]
    fn thread_list_meta_tool_args_rejects_non_boolean_marker_flag() {
        let err = match parse_thread_list_meta_tool_args(&serde_json::json!({
            "include_attention_markers": "true"
        })) {
            Ok(_) => panic!("expected parse failure for non-boolean include_attention_markers"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("parse tool arguments: omne.thread.list_meta"));
    }

    #[test]
    fn thread_events_tool_args_parses_kinds() {
        let thread_id = omne_protocol::ThreadId::new();
        let parsed = match parse_thread_events_tool_args(&serde_json::json!({
            "thread_id": thread_id,
            "since_seq": 42,
            "max_events": 8,
            "kinds": ["attention_marker_set", "attention_marker_cleared"],
        })) {
            Ok(parsed) => parsed,
            Err(err) => panic!("expected thread events args parse to succeed: {err}"),
        };

        assert_eq!(parsed.thread_id, thread_id);
        assert_eq!(parsed.since_seq, 42);
        assert_eq!(parsed.max_events, Some(8));
        assert_eq!(
            parsed.kinds,
            Some(vec![
                omne_protocol::ThreadEventKindTag::AttentionMarkerCleared,
                omne_protocol::ThreadEventKindTag::AttentionMarkerSet,
            ])
        );
    }

    #[test]
    fn thread_events_tool_args_rejects_unknown_kinds() {
        let thread_id = omne_protocol::ThreadId::new();
        let err = match parse_thread_events_tool_args(&serde_json::json!({
            "thread_id": thread_id,
            "kinds": ["attention_marker_set", "not_a_real_event_kind"]
        })) {
            Ok(_) => panic!("expected parse failure for unknown thread.events kind"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("unsupported kinds"));
        assert!(err.contains("not_a_real_event_kind"));
    }

    #[test]
    fn thread_events_tool_args_rejects_unknown_fields() {
        let thread_id = omne_protocol::ThreadId::new();
        let err = match parse_thread_events_tool_args(&serde_json::json!({
            "thread_id": thread_id,
            "unexpected": true
        })) {
            Ok(_) => panic!("expected parse failure for unknown thread.events field"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("parse tool arguments: omne.thread.events"));
    }

    #[test]
    fn artifact_read_tool_args_rejects_zero_version() {
        let thread_id = omne_protocol::ThreadId::new();
        let artifact_id = omne_protocol::ArtifactId::new();
        let err = match parse_tool_args::<ArtifactReadToolArgs>(
            &serde_json::json!({
                "thread_id": thread_id,
                "artifact_id": artifact_id,
                "version": 0
            }),
            "omne.artifact.read",
        ) {
            Ok(_) => panic!("expected parse failure for version=0"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("parse tool arguments: omne.artifact.read"));
    }

    #[test]
    fn process_tail_tool_args_rejects_zero_max_lines() {
        let process_id = omne_protocol::ProcessId::new();
        let err = match parse_tool_args::<ProcessTailToolArgs>(
            &serde_json::json!({
                "process_id": process_id,
                "max_lines": 0
            }),
            "omne.process.tail",
        ) {
            Ok(_) => panic!("expected parse failure for max_lines=0"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("parse tool arguments: omne.process.tail"));
    }

    #[test]
    fn mcp_tools_call_params_defaults_arguments_to_object() {
        let parsed = match parse_mcp_tools_call_params(serde_json::json!({
            "name": "omne.thread.state"
        })) {
            Ok(parsed) => parsed,
            Err(err) => panic!("expected tools/call params parse to succeed: {err}"),
        };
        assert_eq!(parsed.name, "omne.thread.state");
        assert!(parsed.arguments.is_object());
        assert_eq!(parsed.arguments, serde_json::json!({}));
    }

    #[test]
    fn mcp_tools_call_params_rejects_unknown_fields() {
        let err = match parse_mcp_tools_call_params(serde_json::json!({
            "name": "omne.thread.state",
            "extra": true
        })) {
            Ok(_) => panic!("expected tools/call params parse failure for unknown field"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("parse tools/call params"));
    }

    #[test]
    fn mcp_tool_list_contains_expected_entries() {
        let tools = mcp_tools();
        let names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|v| v.as_str()))
            .collect::<Vec<_>>();
        assert!(names.contains(&"omne.thread.list_meta"));
        assert!(names.contains(&"omne.thread.events"));
        assert!(names.contains(&"omne.artifact.read"));
        assert!(names.contains(&"omne.artifact.versions"));
        assert!(names.contains(&"omne.process.follow"));
    }

    #[test]
    fn mcp_thread_events_schema_exposes_kind_enum() {
        let tools = mcp_tools();
        let tool = tools
            .iter()
            .find(|tool| tool.get("name").and_then(|v| v.as_str()) == Some("omne.thread.events"))
            .expect("expected omne.thread.events schema");
        let enum_values = tool
            .get("inputSchema")
            .and_then(|v| v.get("properties"))
            .and_then(|v| v.get("kinds"))
            .and_then(|v| v.get("items"))
            .and_then(|v| v.get("enum"))
            .and_then(|v| v.as_array())
            .expect("expected kinds.items.enum");
        assert!(enum_values
            .iter()
            .any(|v| v.as_str() == Some("attention_marker_set")));
        assert_eq!(enum_values.len(), omne_protocol::THREAD_EVENT_KIND_TAGS.len());
    }

    #[test]
    fn jsonrpc_error_includes_code_and_message() {
        let v = jsonrpc_error(Value::from(1), -32601, "method not found", None::<String>);
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 1);
        assert_eq!(v["error"]["code"], -32601);
        assert_eq!(v["error"]["message"], "method not found");
    }
}
