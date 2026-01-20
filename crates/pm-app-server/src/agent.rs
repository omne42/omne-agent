use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use pm_protocol::{ApprovalDecision, ApprovalId, EventSeq, ThreadEventKind, TurnId};
use serde::Deserialize;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use tokio_util::sync::CancellationToken;

use super::ProcessCommand;

const DEFAULT_MODEL: &str = "gpt-4.1";
const MAX_AGENT_STEPS: usize = 24;
const MAX_TOOL_CALLS: usize = 128;

const DEFAULT_INSTRUCTIONS: &str = r#"
You are a coding agent.

- Use tools to read/write files and run commands.
- Processes are non-interactive: you can only start/inspect/tail/follow/kill them.
- Prefer small, reviewable changes; run checks/tests when relevant.
"#;

pub async fn run_agent_turn(
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    turn_id: TurnId,
    _input: String,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let (thread_id, thread_model, thread_openai_base_url) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            handle.thread_id(),
            state.model.clone(),
            state.openai_base_url.clone(),
        )
    };

    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("CODE_PM_OPENAI_API_KEY"))
        .context("OPENAI_API_KEY is required")?;
    let model = thread_model
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let base_url = thread_openai_base_url
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "https://api.openai.com".to_string());

    let openai = pm_openai::Client::new_with_base_url(api_key, base_url)?;
    let tools = build_tools();

    let mut input_items = build_conversation(&server, thread_id).await?;

    let mut last_response_id = String::new();
    let mut last_usage: Option<Value> = None;
    let mut last_text = String::new();
    let mut tool_calls_total = 0usize;
    let mut finished = false;

    for _step in 0..MAX_AGENT_STEPS {
        if cancel.is_cancelled() {
            anyhow::bail!("turn cancelled");
        }

        let req = pm_openai::ResponsesApiRequest {
            model: &model,
            instructions: DEFAULT_INSTRUCTIONS,
            input: &input_items,
            tools: &tools,
            tool_choice: "auto",
            parallel_tool_calls: false,
            store: false,
            stream: false,
        };

        let resp = openai.create_response(&req).await?;
        last_response_id = resp.id.clone();
        last_usage = resp.usage.clone();

        let mut function_calls = Vec::new();
        last_text = extract_assistant_text(&resp.output);

        for item in resp.output {
            if let pm_openai::ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
            } = &item
            {
                function_calls.push((name.clone(), arguments.clone(), call_id.clone()));
            }
            input_items.push(item);
        }

        if function_calls.is_empty() {
            finished = true;
            break;
        }

        for (tool_name, arguments, call_id) in function_calls {
            tool_calls_total += 1;
            if tool_calls_total > MAX_TOOL_CALLS {
                anyhow::bail!("tool call budget exceeded (max_tool_calls={MAX_TOOL_CALLS})");
            }
            let args_json: Value = match serde_json::from_str(&arguments) {
                Ok(v) => v,
                Err(err) => {
                    let output = serde_json::json!({
                        "error": "invalid tool arguments",
                        "details": err.to_string(),
                        "arguments": arguments,
                    });
                    input_items.push(pm_openai::ResponseItem::FunctionCallOutput {
                        call_id,
                        output: serde_json::to_string(&output)?,
                    });
                    continue;
                }
            };

            let output_value = run_tool_call(
                &server,
                thread_id,
                Some(turn_id),
                &tool_name,
                args_json,
                cancel.clone(),
            )
            .await;
            let output_value = match output_value {
                Ok(v) => v,
                Err(err) => serde_json::json!({ "error": err.to_string() }),
            };

            input_items.push(pm_openai::ResponseItem::FunctionCallOutput {
                call_id,
                output: serde_json::to_string(&output_value)?,
            });
        }
    }

    if !finished {
        anyhow::bail!("agent step budget exceeded (max_steps={MAX_AGENT_STEPS})");
    }

    if !last_text.is_empty() {
        let _ = thread_rt
            .append_event(ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                text: last_text.clone(),
                model: Some(model.clone()),
                response_id: Some(last_response_id.clone()),
                token_usage: last_usage.clone(),
            })
            .await;
    }

    Ok(())
}

async fn build_conversation(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
) -> anyhow::Result<Vec<pm_openai::ResponseItem>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    let mut input = Vec::new();
    for event in events {
        match event.kind {
            ThreadEventKind::TurnStarted { input: text, .. } => {
                input.push(pm_openai::ResponseItem::Message {
                    role: "user".to_string(),
                    content: vec![pm_openai::ContentItem::InputText { text }],
                });
            }
            ThreadEventKind::AssistantMessage { text, .. } => {
                input.push(pm_openai::ResponseItem::Message {
                    role: "assistant".to_string(),
                    content: vec![pm_openai::ContentItem::OutputText { text }],
                });
            }
            _ => {}
        }
    }
    Ok(input)
}

fn extract_assistant_text(items: &[pm_openai::ResponseItem]) -> String {
    let mut out = String::new();
    for item in items {
        let pm_openai::ResponseItem::Message { role, content } = item else {
            continue;
        };
        if role != "assistant" {
            continue;
        }
        for c in content {
            if let pm_openai::ContentItem::OutputText { text } = c {
                out.push_str(text);
            }
        }
    }
    out
}

fn build_tools() -> Vec<Value> {
    vec![
        pm_openai::tool_function(
            "file_read",
            "Read a UTF-8 text file from the project.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "file_glob",
            "Find files by glob pattern (e.g. **/*.rs).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "max_results": { "type": "integer", "minimum": 1 },
                },
                "required": ["pattern"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "file_grep",
            "Search text across files.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "is_regex": { "type": "boolean" },
                    "include_glob": { "type": "string" },
                    "max_matches": { "type": "integer", "minimum": 1 },
                },
                "required": ["query"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "file_write",
            "Write a UTF-8 text file (overwrites).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "text": { "type": "string" },
                    "create_parent_dirs": { "type": "boolean" },
                },
                "required": ["path", "text"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "file_patch",
            "Apply a unified diff patch to a file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "patch": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["path", "patch"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "file_edit",
            "Edit a UTF-8 file by applying exact string replacements.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string" },
                                "new": { "type": "string" },
                                "expected_replacements": { "type": "integer", "minimum": 0 }
                            },
                            "required": ["old", "new"],
                            "additionalProperties": false
                        }
                    },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["path", "edits"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "file_delete",
            "Delete a file (or a directory if recursive=true).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "fs_mkdir",
            "Create a directory.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "process_start",
            "Start a background process (non-interactive).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                },
                "required": ["argv"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "process_inspect",
            "Inspect a process and read recent stdout/stderr.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "max_lines": { "type": "integer", "minimum": 1 },
                },
                "required": ["process_id"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "process_tail",
            "Read the last N lines from a process log (stdout/stderr).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "stream": { "type": "string", "enum": ["stdout", "stderr"] },
                    "max_lines": { "type": "integer", "minimum": 1 },
                },
                "required": ["process_id", "stream"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "process_follow",
            "Read the next chunk from a process log (stdout/stderr) starting at since_offset.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "stream": { "type": "string", "enum": ["stdout", "stderr"] },
                    "since_offset": { "type": "integer", "minimum": 0 },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["process_id", "stream"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "process_kill",
            "Kill a running process.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "reason": { "type": "string" },
                },
                "required": ["process_id"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "artifact_write",
            "Write a user-facing markdown artifact for this thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_type": { "type": "string" },
                    "summary": { "type": "string" },
                    "text": { "type": "string" },
                },
                "required": ["artifact_type", "summary", "text"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "artifact_list",
            "List user-facing artifacts for this thread.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "artifact_read",
            "Read a user-facing artifact by id.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["artifact_id"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "artifact_delete",
            "Delete a user-facing artifact by id.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                },
                "required": ["artifact_id"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "thread_state",
            "Read the derived state for a thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                },
                "required": ["thread_id"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "thread_events",
            "Read thread events since a given seq.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                    "since_seq": { "type": "integer", "minimum": 0 },
                    "max_events": { "type": "integer", "minimum": 1 },
                },
                "required": ["thread_id"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
            "agent_spawn",
            "Fork the current thread and start a background agent turn in the forked thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" },
                    "model": { "type": "string" },
                    "openai_base_url": { "type": "string" },
                },
                "required": ["input"],
                "additionalProperties": false,
            }),
        ),
    ]
}

#[derive(Debug, Deserialize)]
struct FileReadArgs {
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileGlobArgs {
    pattern: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileGrepArgs {
    query: String,
    #[serde(default)]
    is_regex: bool,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_matches: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileWriteArgs {
    path: String,
    text: String,
    #[serde(default)]
    create_parent_dirs: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FilePatchArgs {
    path: String,
    patch: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditArgs {
    path: String,
    edits: Vec<FileEditOpArgs>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditOpArgs {
    old: String,
    new: String,
    #[serde(default)]
    expected_replacements: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileDeleteArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct FsMkdirArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct ProcessStartArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessInspectArgs {
    process_id: String,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessTailArgs {
    process_id: String,
    stream: super::ProcessStream,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessFollowArgs {
    process_id: String,
    stream: super::ProcessStream,
    #[serde(default)]
    since_offset: u64,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ProcessKillArgs {
    process_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtifactWriteArgs {
    artifact_type: String,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactReadArgs {
    artifact_id: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ArtifactDeleteArgs {
    artifact_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadStateArgs {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadEventsArgs {
    thread_id: String,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AgentSpawnArgs {
    input: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
}

async fn run_tool_call(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: Value,
    cancel: CancellationToken,
) -> anyhow::Result<Value> {
    let mut approval_id: Option<ApprovalId> = None;

    for attempt in 0..3usize {
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled");
        }

        let output = run_tool_call_once(
            server,
            thread_id,
            turn_id,
            tool_name,
            args.clone(),
            approval_id,
        )
        .await?;

        let Some(requested) = parse_needs_approval(&output)? else {
            return Ok(redact_tool_output(output));
        };

        if attempt >= 2 {
            anyhow::bail!("too many approval cycles for tool {tool_name}");
        }

        let outcome =
            wait_for_approval_outcome(server, thread_id, requested, cancel.clone()).await?;
        match outcome.decision {
            ApprovalDecision::Approved => {
                approval_id = Some(requested);
            }
            ApprovalDecision::Denied => {
                return Ok(serde_json::json!({
                    "denied": true,
                    "approval_id": requested,
                    "decision": outcome.decision,
                    "remember": outcome.remember,
                    "reason": outcome.reason,
                }));
            }
        }
    }

    anyhow::bail!("too many attempts for tool {tool_name}");
}

async fn run_tool_call_once(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: Value,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<Value> {
    match tool_name {
        "file_read" => {
            let args: FileReadArgs = serde_json::from_value(args)?;
            super::handle_file_read(
                server,
                super::FileReadParams {
                    thread_id,
                    turn_id,
                    path: args.path,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_glob" => {
            let args: FileGlobArgs = serde_json::from_value(args)?;
            super::handle_file_glob(
                server,
                super::FileGlobParams {
                    thread_id,
                    turn_id,
                    pattern: args.pattern,
                    max_results: args.max_results,
                },
            )
            .await
        }
        "file_grep" => {
            let args: FileGrepArgs = serde_json::from_value(args)?;
            super::handle_file_grep(
                server,
                super::FileGrepParams {
                    thread_id,
                    turn_id,
                    query: args.query,
                    is_regex: args.is_regex,
                    include_glob: args.include_glob,
                    max_matches: args.max_matches,
                    max_bytes_per_file: None,
                    max_files: None,
                },
            )
            .await
        }
        "file_write" => {
            let args: FileWriteArgs = serde_json::from_value(args)?;
            super::handle_file_write(
                server,
                super::FileWriteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    text: args.text,
                    create_parent_dirs: args.create_parent_dirs,
                },
            )
            .await
        }
        "file_patch" => {
            let args: FilePatchArgs = serde_json::from_value(args)?;
            super::handle_file_patch(
                server,
                super::FilePatchParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    patch: args.patch,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_edit" => {
            let args: FileEditArgs = serde_json::from_value(args)?;
            let edits = args
                .edits
                .into_iter()
                .map(|op| super::FileEditOp {
                    old: op.old,
                    new: op.new,
                    expected_replacements: op.expected_replacements,
                })
                .collect();
            super::handle_file_edit(
                server,
                super::FileEditParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    edits,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_delete" => {
            let args: FileDeleteArgs = serde_json::from_value(args)?;
            super::handle_file_delete(
                server,
                super::FileDeleteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    recursive: args.recursive,
                },
            )
            .await
        }
        "fs_mkdir" => {
            let args: FsMkdirArgs = serde_json::from_value(args)?;
            super::handle_fs_mkdir(
                server,
                super::FsMkdirParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    recursive: args.recursive,
                },
            )
            .await
        }
        "process_start" => {
            let args: ProcessStartArgs = serde_json::from_value(args)?;
            super::handle_process_start(
                server,
                super::ProcessStartParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    argv: args.argv,
                    cwd: args.cwd,
                },
            )
            .await
        }
        "process_inspect" => {
            let args: ProcessInspectArgs = serde_json::from_value(args)?;
            super::handle_process_inspect(
                server,
                super::ProcessInspectParams {
                    process_id: args.process_id.parse()?,
                    max_lines: args.max_lines,
                },
            )
            .await
        }
        "process_tail" => {
            let args: ProcessTailArgs = serde_json::from_value(args)?;
            super::handle_process_tail(
                server,
                super::ProcessTailParams {
                    process_id: args.process_id.parse()?,
                    stream: args.stream,
                    max_lines: args.max_lines,
                },
            )
            .await
        }
        "process_follow" => {
            let args: ProcessFollowArgs = serde_json::from_value(args)?;
            super::handle_process_follow(
                server,
                super::ProcessFollowParams {
                    process_id: args.process_id.parse()?,
                    stream: args.stream,
                    since_offset: args.since_offset,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "process_kill" => {
            let args: ProcessKillArgs = serde_json::from_value(args)?;
            let process_id = args.process_id.parse()?;
            let entry = {
                let entries = server.processes.lock().await;
                entries.get(&process_id).cloned()
            };
            if let Some(entry) = entry {
                let _ = entry
                    .cmd_tx
                    .send(ProcessCommand::Kill {
                        reason: args.reason,
                    })
                    .await;
                Ok(serde_json::json!({ "ok": true }))
            } else {
                anyhow::bail!("process not found: {process_id}");
            }
        }
        "artifact_write" => {
            let args: ArtifactWriteArgs = serde_json::from_value(args)?;
            super::handle_artifact_write(
                server,
                super::ArtifactWriteParams {
                    thread_id,
                    turn_id,
                    artifact_id: None,
                    artifact_type: args.artifact_type,
                    summary: args.summary,
                    text: args.text,
                },
            )
            .await
        }
        "artifact_list" => {
            let _ = args;
            super::handle_artifact_list(server, super::ArtifactListParams { thread_id }).await
        }
        "artifact_read" => {
            let args: ArtifactReadArgs = serde_json::from_value(args)?;
            super::handle_artifact_read(
                server,
                super::ArtifactReadParams {
                    thread_id,
                    artifact_id: args.artifact_id.parse()?,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "artifact_delete" => {
            let args: ArtifactDeleteArgs = serde_json::from_value(args)?;
            super::handle_artifact_delete(
                server,
                super::ArtifactDeleteParams {
                    thread_id,
                    turn_id,
                    artifact_id: args.artifact_id.parse()?,
                },
            )
            .await
        }
        "thread_state" => {
            let args: ThreadStateArgs = serde_json::from_value(args)?;
            let thread_id: pm_protocol::ThreadId = args.thread_id.parse()?;
            let rt = server.get_or_load_thread(thread_id).await?;
            let handle = rt.handle.lock().await;
            let state = handle.state();
            let archived_at = state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok());
            Ok(serde_json::json!({
                "thread_id": handle.thread_id(),
                "cwd": state.cwd,
                "archived": state.archived,
                "archived_at": archived_at,
                "archived_reason": state.archived_reason,
                "approval_policy": state.approval_policy,
                "sandbox_policy": state.sandbox_policy,
                "model": state.model,
                "openai_base_url": state.openai_base_url,
                "last_seq": handle.last_seq().0,
                "active_turn_id": state.active_turn_id,
                "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
                "last_turn_id": state.last_turn_id,
                "last_turn_status": state.last_turn_status,
                "last_turn_reason": state.last_turn_reason,
            }))
        }
        "thread_events" => {
            let args: ThreadEventsArgs = serde_json::from_value(args)?;
            let thread_id: pm_protocol::ThreadId = args.thread_id.parse()?;
            let since = EventSeq(args.since_seq);

            let mut events = server
                .thread_store
                .read_events_since(thread_id, since)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

            let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

            let mut has_more = false;
            if let Some(max_events) = args.max_events {
                let max_events = max_events.clamp(1, 50_000);
                if events.len() > max_events {
                    events.truncate(max_events);
                    has_more = true;
                }
            }

            let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

            Ok(serde_json::json!({
                "events": events,
                "last_seq": last_seq,
                "thread_last_seq": thread_last_seq,
                "has_more": has_more,
            }))
        }
        "agent_spawn" => {
            #[derive(Debug, Deserialize)]
            struct ForkResult {
                thread_id: pm_protocol::ThreadId,
                log_path: String,
                last_seq: u64,
            }

            let args: AgentSpawnArgs = serde_json::from_value(args)?;
            if args.input.trim().is_empty() {
                anyhow::bail!("input must not be empty");
            }

            let forked =
                super::handle_thread_fork(server, super::ThreadForkParams { thread_id }).await?;
            let forked: ForkResult = serde_json::from_value(forked)?;

            if args.model.is_some() || args.openai_base_url.is_some() {
                super::handle_thread_configure(
                    server,
                    super::ThreadConfigureParams {
                        thread_id: forked.thread_id,
                        approval_policy: None,
                        sandbox_policy: None,
                        model: args.model,
                        openai_base_url: args.openai_base_url,
                    },
                )
                .await?;
            }

            let rt = server.get_or_load_thread(forked.thread_id).await?;
            let server_arc = Arc::new(server.clone());
            let turn_id = rt.start_turn(server_arc, args.input).await?;

            Ok(serde_json::json!({
                "thread_id": forked.thread_id,
                "turn_id": turn_id,
                "log_path": forked.log_path,
                "last_seq": forked.last_seq,
            }))
        }
        _ => anyhow::bail!("unknown tool: {tool_name}"),
    }
}

#[derive(Debug)]
struct ApprovalOutcome {
    decision: ApprovalDecision,
    remember: bool,
    reason: Option<String>,
}

async fn wait_for_approval_outcome(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
    approval_id: ApprovalId,
    cancel: CancellationToken,
) -> anyhow::Result<ApprovalOutcome> {
    let mut since = EventSeq::ZERO;
    loop {
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled waiting for approval");
        }

        let events = server
            .thread_store
            .read_events_since(thread_id, since)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        since = events.last().map(|e| e.seq).unwrap_or(since);

        for event in events {
            if let ThreadEventKind::ApprovalDecided {
                approval_id: id,
                decision,
                remember,
                reason,
                ..
            } = event.kind
                && id == approval_id
            {
                return Ok(ApprovalOutcome {
                    decision,
                    remember,
                    reason,
                });
            }
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn parse_needs_approval(value: &Value) -> anyhow::Result<Option<ApprovalId>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let Some(needs_approval) = obj.get("needs_approval").and_then(|v| v.as_bool()) else {
        return Ok(None);
    };
    if !needs_approval {
        return Ok(None);
    }
    let Some(approval_id) = obj.get("approval_id").and_then(|v| v.as_str()) else {
        anyhow::bail!("tool returned needs_approval without approval_id");
    };
    Ok(Some(approval_id.parse()?))
}

fn redact_tool_output(mut value: Value) -> Value {
    fn walk(value: &mut Value) {
        match value {
            Value::String(s) => {
                *s = pm_core::redact_text(s);
            }
            Value::Array(items) => {
                for item in items {
                    walk(item);
                }
            }
            Value::Object(obj) => {
                for v in obj.values_mut() {
                    walk(v);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}
