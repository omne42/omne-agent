use omne_protocol::{ThreadEventKind, ToolId, ToolStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookPoint {
    SessionStart,
    PreToolUse,
    PostToolUse,
    Stop,
    SubagentStart,
    SubagentStop,
}

impl HookPoint {
    fn as_str(self) -> &'static str {
        match self {
            HookPoint::SessionStart => "session_start",
            HookPoint::PreToolUse => "pre_tool_use",
            HookPoint::PostToolUse => "post_tool_use",
            HookPoint::Stop => "stop",
            HookPoint::SubagentStart => "subagent_start",
            HookPoint::SubagentStop => "subagent_stop",
        }
    }

    fn commands(self, hooks: &HooksConfigHooks) -> &[HookCommandConfig] {
        match self {
            HookPoint::SessionStart => &hooks.session_start,
            HookPoint::PreToolUse => &hooks.pre_tool_use,
            HookPoint::PostToolUse => &hooks.post_tool_use,
            HookPoint::Stop => &hooks.stop,
            HookPoint::SubagentStart => &hooks.subagent_start,
            HookPoint::SubagentStop => &hooks.subagent_stop,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HooksConfigFile {
    version: u32,
    hooks: HooksConfigHooks,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HooksConfigHooks {
    #[serde(default)]
    session_start: Vec<HookCommandConfig>,
    #[serde(default)]
    pre_tool_use: Vec<HookCommandConfig>,
    #[serde(default)]
    post_tool_use: Vec<HookCommandConfig>,
    #[serde(default)]
    stop: Vec<HookCommandConfig>,
    #[serde(default)]
    subagent_start: Vec<HookCommandConfig>,
    #[serde(default)]
    subagent_stop: Vec<HookCommandConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookCommandConfig {
    argv: Vec<String>,
    #[serde(default)]
    ok_exit_codes: Vec<i32>,
    #[serde(default)]
    when_tools: Vec<String>,
    #[serde(default)]
    when_turn_status: Vec<TurnStatus>,
    #[serde(default)]
    emit_additional_context: bool,
}

#[derive(Clone, Debug)]
struct HookAdditionalContext {
    hook_id: ToolId,
    hook_point: HookPoint,
    context_path: PathBuf,
    text: String,
    summary: Option<String>,
}

const DEFAULT_HOOK_OK_EXIT_CODES: &[i32] = &[0];
const DEFAULT_HOOK_PROCESS_TIMEOUT_SECS: u64 = 3;
const MAX_HOOK_PROCESS_TIMEOUT_SECS: u64 = 60;
const MAX_HOOK_CONTEXT_CHARS: usize = 16 * 1024;
const REDACTED_VALUE: &str = "<REDACTED>";

fn hooks_process_timeout() -> Duration {
    let value = std::env::var("OMNE_HOOK_PROCESS_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_HOOK_PROCESS_TIMEOUT_SECS);
    Duration::from_secs(value.clamp(1, MAX_HOOK_PROCESS_TIMEOUT_SECS))
}

fn hook_artifacts_dir_for_thread(server: &Server, thread_id: ThreadId) -> PathBuf {
    server
        .thread_store
        .thread_dir(thread_id)
        .join("artifacts")
        .join("hooks")
}

async fn load_hooks_config(
    thread_root: &Path,
) -> anyhow::Result<Option<(PathBuf, HooksConfigFile)>> {
    let path = thread_root
        .join(".omne_data")
        .join("spec")
        .join("hooks.yaml");

    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };

    let cfg: HooksConfigFile =
        serde_yaml::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
    if cfg.version != 1 {
        anyhow::bail!("unsupported hooks.yaml version {} (expected 1)", cfg.version);
    }
    Ok(Some((path, cfg)))
}

async fn record_hooks_config_error(
    server: &Server,
    thread_id: ThreadId,
    config_path: &Path,
    err: &anyhow::Error,
) {
    let dir = hook_artifacts_dir_for_thread(server, thread_id);
    let path = dir.join("hooks_config_error.txt");
    let text = format!(
        "hooks config error\n\npath: {}\nerror: {}\n",
        config_path.display(),
        err
    );
    if let Err(write_err) = write_file_atomic(&path, text.as_bytes()).await {
        tracing::warn!(
            path = %path.display(),
            error = %write_err,
            "failed to write hooks config error artifact"
        );
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct HookInput {
    hook_point: String,
    thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn_id: Option<TurnId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "tool_params")]
    tool_params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "tool_result")]
    tool_result: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn_status: Option<TurnStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn_reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    subagent: Option<HookInputSubagent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct HookInputSubagent {
    parent_thread_id: ThreadId,
    task_id: String,
    child_thread_id: ThreadId,
    child_turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<TurnStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

fn truncate_owned_chars(mut s: String, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s;
    }
    s = s.chars().take(max_chars).collect::<String>();
    s.push('…');
    s
}

fn redact_and_truncate_string(s: &str, max_chars: usize) -> String {
    truncate_owned_chars(omne_core::redact_text(s), max_chars)
}

fn redact_and_truncate_value(value: &Value, max_string_chars: usize) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(s) => Value::String(redact_and_truncate_string(s, max_string_chars)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(64)
                .map(|v| redact_and_truncate_value(v, max_string_chars))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .take(128)
                .map(|(k, v)| {
                    if omne_core::redaction::is_sensitive_key(k) {
                        (k.clone(), Value::String(REDACTED_VALUE.to_string()))
                    } else {
                        (k.clone(), redact_and_truncate_value(v, max_string_chars))
                    }
                })
                .collect(),
        ),
    }
}

fn hook_tool_params_for_action(action: &str, tool_args: &Value) -> Value {
    match action {
        "file/write" => {
            let path = tool_args.get("path").and_then(Value::as_str).unwrap_or("");
            let bytes = tool_args
                .get("text")
                .and_then(Value::as_str)
                .map(|s| s.len())
                .unwrap_or(0);
            let create_parent_dirs = tool_args
                .get("create_parent_dirs")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            serde_json::json!({
                "path": path,
                "bytes": bytes,
                "create_parent_dirs": create_parent_dirs,
            })
        }
        "file/patch" => {
            let path = tool_args.get("path").and_then(Value::as_str).unwrap_or("");
            let patch_bytes = tool_args
                .get("patch")
                .and_then(Value::as_str)
                .map(|s| s.len())
                .unwrap_or(0);
            let max_bytes = tool_args.get("max_bytes").and_then(Value::as_u64);
            serde_json::json!({
                "path": path,
                "patch_bytes": patch_bytes,
                "max_bytes": max_bytes,
            })
        }
        "file/edit" => {
            let path = tool_args.get("path").and_then(Value::as_str).unwrap_or("");
            let edits_len = tool_args
                .get("edits")
                .and_then(Value::as_array)
                .map(|v| v.len())
                .unwrap_or(0);
            let max_bytes = tool_args.get("max_bytes").and_then(Value::as_u64);
            serde_json::json!({
                "path": path,
                "edits": edits_len,
                "max_bytes": max_bytes,
            })
        }
        "process/start" => {
            let argv = tool_args.get("argv").cloned().unwrap_or(Value::Null);
            let cwd = tool_args.get("cwd").cloned().unwrap_or(Value::Null);
            serde_json::json!({
                "argv": redact_and_truncate_value(&argv, 256),
                "cwd": redact_and_truncate_value(&cwd, 256),
            })
        }
        "artifact/write" => {
            let artifact_type = tool_args
                .get("artifact_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            let summary = tool_args
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("");
            let bytes = tool_args
                .get("text")
                .and_then(Value::as_str)
                .map(|s| s.len())
                .unwrap_or(0);
            serde_json::json!({
                "artifact_type": artifact_type,
                "summary": redact_and_truncate_string(summary, 256),
                "bytes": bytes,
            })
        }
        _ => redact_and_truncate_value(tool_args, 256),
    }
}

fn hook_tool_name_from_agent_tool(tool_name: &str) -> Option<&'static str> {
    crate::agent::agent_tool_action(tool_name)
}

async fn wait_for_process_exit(
    server: &Server,
    process_id: ProcessId,
    timeout: Duration,
) -> anyhow::Result<Option<i32>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let entry = {
            let processes = server.processes.lock().await;
            processes.get(&process_id).cloned()
        };
        let Some(entry) = entry else {
            return Ok(None);
        };

        let (status, exit_code) = {
            let info = entry.info.lock().await;
            (info.status.clone(), info.exit_code)
        };

        if matches!(status, ProcessStatus::Exited | ProcessStatus::Abandoned) {
            return Ok(exit_code);
        }

        if tokio::time::Instant::now() >= deadline {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("hook timeout".to_string()),
                })
                .await;
            anyhow::bail!("hook process timed out: {}", process_id);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn run_hook_commands(
    server: &Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    hook_point: HookPoint,
    ctx: HookDispatchContext<'_>,
) -> Vec<HookAdditionalContext> {
    let (_thread_rt, thread_root) = match load_thread_root(server, thread_id).await {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let loaded = load_hooks_config(&thread_root).await;
    let (config_path, cfg) = match loaded {
        Ok(Some(v)) => v,
        Ok(None) => return Vec::new(),
        Err(err) => {
            let expected_path = thread_root
                .join(".omne_data")
                .join("spec")
                .join("hooks.yaml");
            record_hooks_config_error(server, thread_id, &expected_path, &err).await;
            return Vec::new();
        }
    };

    let commands = hook_point.commands(&cfg.hooks);
    if commands.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for command in commands {
        if command.argv.is_empty() {
            continue;
        }

        if !command.when_tools.is_empty() {
            let Some(tool) = ctx.tool else {
                continue;
            };
            if !command
                .when_tools
                .iter()
                .any(|t| t.trim() == tool.trim())
            {
                continue;
            }
        }

        if !command.when_turn_status.is_empty() {
            let Some(status) = ctx.turn_status else {
                continue;
            };
            if !command.when_turn_status.contains(&status) {
                continue;
            }
        }

        let hook_id = ToolId::new();
        let hooks_dir = hook_artifacts_dir_for_thread(server, thread_id);
        let input_path = hooks_dir.join(format!("{hook_id}.input.json"));
        let output_path = hooks_dir.join(format!("{hook_id}.output.json"));
        let context_path = hooks_dir.join(format!("{hook_id}.additional_context.md"));

        let mut input = HookInput {
            hook_point: hook_point.as_str().to_string(),
            thread_id,
            turn_id,
            tool: ctx.tool.map(ToString::to_string),
            tool_params: ctx
                .tool
                .and_then(|tool| ctx.tool_args.map(|args| hook_tool_params_for_action(tool, args))),
            tool_result: ctx.tool_result.map(|v| redact_and_truncate_value(v, 512)),
            turn_status: ctx.turn_status,
            turn_reason: ctx.turn_reason.map(|s| redact_and_truncate_string(s, 512)),
            subagent: ctx.subagent.map(|subagent| HookInputSubagent {
                parent_thread_id: subagent.parent_thread_id,
                task_id: subagent.task_id.to_string(),
                child_thread_id: subagent.child_thread_id,
                child_turn_id: subagent.child_turn_id,
                status: subagent.status,
                reason: subagent
                    .reason
                    .map(|s| redact_and_truncate_string(s, 512))
                    .filter(|s| !s.trim().is_empty()),
            }),
        };

        if input.tool_params.as_ref().is_some_and(|v| v == &Value::Null) {
            input.tool_params = None;
        }

        let input_json = match serde_json::to_vec_pretty(&input) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };

        if write_file_atomic(&input_path, &input_json).await.is_err() {
            continue;
        }
        if let Err(err) = write_file_atomic(&output_path, b"{}").await {
            tracing::warn!(
                path = %output_path.display(),
                error = %err,
                "failed to initialize hook output file"
            );
            continue;
        }

        let tool_id = hook_id;
        let tool_params = serde_json::json!({
            "hook_point": hook_point.as_str(),
            "config_path": config_path.display().to_string(),
            "argv": command.argv.clone(),
            "input_path": input_path.display().to_string(),
            "output_path": output_path.display().to_string(),
        });

        let thread_rt = match server.get_or_load_thread(thread_id).await {
            Ok(rt) => rt,
            Err(_) => continue,
        };
        if let Err(err) = thread_rt
            .append_event(ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: "hook/run".to_string(),
                params: Some(tool_params.clone()),
            })
            .await
        {
            tracing::warn!(
                thread_id = %thread_id,
                tool_id = %tool_id,
                error = %err,
                "failed to append hook ToolStarted event"
            );
        }

        let mut env = BTreeMap::<String, String>::new();
        env.insert(
            "OMNE_HOOK_INPUT_PATH".to_string(),
            input_path.display().to_string(),
        );
        env.insert(
            "OMNE_HOOK_OUTPUT_PATH".to_string(),
            output_path.display().to_string(),
        );
        env.insert(
            "OMNE_HOOK_POINT".to_string(),
            hook_point.as_str().to_string(),
        );
        if let Some(tool) = ctx.tool {
            env.insert("OMNE_HOOK_TOOL".to_string(), tool.to_string());
        }

        let started = handle_process_start_with_env(
            server,
            ProcessStartParams {
                thread_id,
                turn_id,
                approval_id: None,
                argv: command.argv.clone(),
                cwd: None,
                timeout_ms: None,
            },
            &env,
        )
        .await;

        let output = match started {
            Ok(v) => v,
            Err(err) => {
                let _ = thread_rt
                    .append_event(ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: None,
                    })
                    .await;
                continue;
            }
        };

        let obj = output.as_object();
        if obj
            .and_then(|o| o.get("needs_approval"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let approval_id = obj.and_then(|o| o.get("approval_id")).cloned();
            let _ = thread_rt
                .append_event(ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: ToolStatus::Denied,
                    error: Some("hook process/start needs approval; skipped".to_string()),
                    result: Some(serde_json::json!({
                        "needs_approval": true,
                        "approval_id": approval_id,
                    })),
                })
                .await;
            continue;
        }

        if obj
            .and_then(|o| o.get("denied"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let _ = thread_rt
                .append_event(ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: ToolStatus::Denied,
                    error: Some("hook process/start denied".to_string()),
                    result: Some(redact_and_truncate_value(&output, 512)),
                })
                .await;
            continue;
        }

        let Some(process_id) = obj
            .and_then(|o| o.get("process_id"))
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<ProcessId>().ok())
        else {
            let _ = thread_rt
                .append_event(ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: ToolStatus::Failed,
                    error: Some("hook process/start returned unexpected response".to_string()),
                    result: Some(redact_and_truncate_value(&output, 512)),
                })
                .await;
            continue;
        };

        let exit_code = match wait_for_process_exit(server, process_id, hooks_process_timeout()).await
        {
            Ok(code) => code,
            Err(err) => {
                let _ = thread_rt
                    .append_event(ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: Some(serde_json::json!({
                            "process_id": process_id,
                            "output": redact_and_truncate_value(&output, 512),
                        })),
                    })
                    .await;
                continue;
            }
        };

        let ok_exit_codes = if command.ok_exit_codes.is_empty() {
            DEFAULT_HOOK_OK_EXIT_CODES
        } else {
            command.ok_exit_codes.as_slice()
        };

        let exit_ok = exit_code.is_some_and(|code| ok_exit_codes.contains(&code));
        if !exit_ok {
            let _ = thread_rt
                .append_event(ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: ToolStatus::Failed,
                    error: Some("hook process exited with unexpected status".to_string()),
                    result: Some(serde_json::json!({
                        "process_id": process_id,
                        "exit_code": exit_code,
                        "ok_exit_codes": ok_exit_codes,
                        "output": redact_and_truncate_value(&output, 512),
                    })),
                })
                .await;
            continue;
        }

        let mut hook_output: Option<Value> = None;
        if let Ok(bytes) = tokio::fs::read(&output_path).await {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&bytes) {
                hook_output = Some(parsed);
            }
        }

        let additional_context = hook_output
            .as_ref()
            .and_then(|v| v.get("additional_context"))
            .and_then(Value::as_str)
            .map(|s| redact_and_truncate_string(s, MAX_HOOK_CONTEXT_CHARS))
            .filter(|s| !s.trim().is_empty());
        let has_additional_context = additional_context.is_some();

        let summary = hook_output
            .as_ref()
            .and_then(|v| v.get("summary"))
            .and_then(Value::as_str)
            .map(|s| redact_and_truncate_string(s, 512))
            .filter(|s| !s.trim().is_empty());

        if command.emit_additional_context {
            if let Some(text) = additional_context {
                if let Err(err) = write_file_atomic(context_path.as_path(), text.as_bytes()).await {
                    tracing::warn!(
                        path = %context_path.display(),
                        error = %err,
                        "failed to persist hook additional context"
                    );
                }
                out.push(HookAdditionalContext {
                    hook_id,
                    hook_point,
                    context_path: context_path.clone(),
                    text,
                    summary,
                });
            }
        }

        if let Err(err) = thread_rt
            .append_event(ThreadEventKind::ToolCompleted {
                tool_id,
                status: ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "process_id": process_id,
                    "exit_code": exit_code,
                    "input_path": input_path.display().to_string(),
                    "output_path": output_path.display().to_string(),
                    "additional_context_path": if command.emit_additional_context && has_additional_context {
                        Some(context_path.display().to_string())
                    } else {
                        None
                    },
                })),
            })
            .await
        {
            tracing::warn!(
                thread_id = %thread_id,
                tool_id = %tool_id,
                error = %err,
                "failed to append hook ToolCompleted event"
            );
        }
    }

    out
}

async fn run_hooks_for_point(
    server: &Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    hook_point: HookPoint,
    ctx: HookDispatchContext<'_>,
) -> Vec<HookAdditionalContext> {
    run_hook_commands(server, thread_id, turn_id, hook_point, ctx).await
}

async fn run_session_start_hooks(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> Vec<HookAdditionalContext> {
    run_hooks_for_point(
        server,
        thread_id,
        Some(turn_id),
        HookPoint::SessionStart,
        HookDispatchContext::default(),
    )
    .await
}

async fn run_pre_tool_use_hooks(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    tool: &str,
    tool_args: &Value,
) -> Vec<HookAdditionalContext> {
    run_hooks_for_point(
        server,
        thread_id,
        Some(turn_id),
        HookPoint::PreToolUse,
        HookDispatchContext {
            tool: Some(tool),
            tool_args: Some(tool_args),
            ..HookDispatchContext::default()
        },
    )
    .await
}

async fn run_post_tool_use_hooks(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    tool: &str,
    tool_args: &Value,
    tool_result: &Value,
) -> Vec<HookAdditionalContext> {
    run_hooks_for_point(
        server,
        thread_id,
        Some(turn_id),
        HookPoint::PostToolUse,
        HookDispatchContext {
            tool: Some(tool),
            tool_args: Some(tool_args),
            tool_result: Some(tool_result),
            ..HookDispatchContext::default()
        },
    )
    .await
}

async fn run_stop_hooks(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
) -> Vec<HookAdditionalContext> {
    run_hooks_for_point(
        server,
        thread_id,
        Some(turn_id),
        HookPoint::Stop,
        HookDispatchContext {
            turn_status: Some(status),
            turn_reason: reason,
            ..HookDispatchContext::default()
        },
    )
    .await
}

async fn run_subagent_start_hooks(
    server: &Server,
    parent_thread_id: ThreadId,
    task_id: &str,
    child_thread_id: ThreadId,
    child_turn_id: TurnId,
) -> Vec<HookAdditionalContext> {
    run_hooks_for_point(
        server,
        parent_thread_id,
        None,
        HookPoint::SubagentStart,
        HookDispatchContext {
            subagent: Some(HookSubagentContext {
                parent_thread_id,
                task_id,
                child_thread_id,
                child_turn_id,
                status: None,
                reason: None,
            }),
            ..HookDispatchContext::default()
        },
    )
    .await
}

async fn run_subagent_stop_hooks(
    server: &Server,
    parent_thread_id: ThreadId,
    task_id: &str,
    child_thread_id: ThreadId,
    child_turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
) -> Vec<HookAdditionalContext> {
    run_hooks_for_point(
        server,
        parent_thread_id,
        None,
        HookPoint::SubagentStop,
        HookDispatchContext {
            turn_status: Some(status),
            turn_reason: reason,
            subagent: Some(HookSubagentContext {
                parent_thread_id,
                task_id,
                child_thread_id,
                child_turn_id,
                status: Some(status),
                reason,
            }),
            ..HookDispatchContext::default()
        },
    )
    .await
}

#[derive(Clone, Copy, Debug, Default)]
struct HookDispatchContext<'a> {
    tool: Option<&'a str>,
    tool_args: Option<&'a Value>,
    tool_result: Option<&'a Value>,
    turn_status: Option<TurnStatus>,
    turn_reason: Option<&'a str>,
    subagent: Option<HookSubagentContext<'a>>,
}

#[derive(Clone, Copy, Debug)]
struct HookSubagentContext<'a> {
    parent_thread_id: ThreadId,
    task_id: &'a str,
    child_thread_id: ThreadId,
    child_turn_id: TurnId,
    status: Option<TurnStatus>,
    reason: Option<&'a str>,
}

#[cfg(test)]
mod hooks_dispatch_tests {
    use super::*;
    use omne_protocol::{ApprovalPolicy, SandboxPolicy};

    #[tokio::test]
    async fn load_hooks_config_parses_valid_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let thread_root = tmp.path().join("repo");
        let spec_dir = thread_root.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        let config_path = spec_dir.join("hooks.yaml");
        tokio::fs::write(
            &config_path,
            r#"
version: 1
hooks:
  stop:
    - argv: ["sh", "-c", "exit 0"]
  subagent_start:
    - argv: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let loaded = load_hooks_config(&thread_root).await?;
        let (path, cfg) = loaded.ok_or_else(|| anyhow::anyhow!("hooks config not loaded"))?;
        assert_eq!(path, config_path);
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.hooks.stop.len(), 1);
        assert_eq!(cfg.hooks.stop[0].argv, vec!["sh", "-c", "exit 0"]);
        assert_eq!(cfg.hooks.subagent_start.len(), 1);
        assert_eq!(cfg.hooks.subagent_start[0].argv, vec!["sh", "-c", "exit 0"]);
        Ok(())
    }

    #[tokio::test]
    async fn load_hooks_config_rejects_unsupported_version() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let thread_root = tmp.path().join("repo");
        let spec_dir = thread_root.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 2
hooks: {}
"#,
        )
        .await?;

        let err = load_hooks_config(&thread_root)
            .await
            .expect_err("unsupported hooks version should fail");
        assert!(err.to_string().contains("unsupported hooks.yaml version 2"));
        Ok(())
    }

    #[tokio::test]
    async fn record_hooks_config_error_writes_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let config_path = repo_dir.join(".omne_data").join("spec").join("hooks.yaml");
        let err = anyhow::anyhow!("hooks parse failed");
        record_hooks_config_error(&server, thread_id, &config_path, &err).await;

        let artifact_path = hook_artifacts_dir_for_thread(&server, thread_id).join("hooks_config_error.txt");
        let text = tokio::fs::read_to_string(&artifact_path).await?;
        assert!(text.contains("hooks config error"));
        assert!(text.contains(&config_path.display().to_string()));
        assert!(text.contains("hooks parse failed"));
        Ok(())
    }

    #[tokio::test]
    async fn pre_tool_use_hook_emits_additional_context_once_without_recursion() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  pre_tool_use:
    - when_tools: ["process/start"]
      argv:
        [
          "sh",
          "-c",
          "printf '{\"additional_context\":\"safety reminder\",\"summary\":\"prehook\"}' > \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "argv": ["echo", "hello"],
            "cwd": repo_dir.display().to_string(),
        });

        let contexts =
            run_pre_tool_use_hooks(&server, thread_id, turn_id, "process/start", &tool_args).await;
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].text, "safety reminder");
        assert_eq!(contexts[0].summary.as_deref(), Some("prehook"));
        assert_eq!(contexts[0].hook_point, HookPoint::PreToolUse);
        assert!(contexts[0].context_path.exists());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_started = events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    ThreadEventKind::ToolStarted { tool, .. } if tool == "hook/run"
                )
            })
            .count();
        assert_eq!(hook_started, 1);

        Ok(())
    }

    #[tokio::test]
    async fn post_tool_use_hook_emits_additional_context_once_without_recursion() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  post_tool_use:
    - when_tools: ["artifact/write"]
      argv:
        [
          "sh",
          "-c",
          "printf '{\"additional_context\":\"post safety reminder\",\"summary\":\"posthook\"}' > \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "artifact_type": "note",
            "summary": "created by test",
            "text": "hello",
        });
        let tool_result = serde_json::json!({
            "ok": true,
            "bytes": 5,
        });

        let contexts = run_post_tool_use_hooks(
            &server,
            thread_id,
            turn_id,
            "artifact/write",
            &tool_args,
            &tool_result,
        )
        .await;
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].text, "post safety reminder");
        assert_eq!(contexts[0].summary.as_deref(), Some("posthook"));
        assert_eq!(contexts[0].hook_point, HookPoint::PostToolUse);
        assert!(contexts[0].context_path.exists());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_started = events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    ThreadEventKind::ToolStarted { tool, .. } if tool == "hook/run"
                )
            })
            .count();
        assert_eq!(hook_started, 1);

        Ok(())
    }

    #[tokio::test]
    async fn stop_hook_filters_commands_by_turn_status() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  stop:
    - when_turn_status: ["failed"]
      argv:
        [
          "sh",
          "-c",
          "printf '{\"additional_context\":\"failed reminder\",\"summary\":\"failed-hook\"}' > \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: true
    - when_turn_status: ["completed"]
      argv:
        [
          "sh",
          "-c",
          "printf '{\"additional_context\":\"completed reminder\",\"summary\":\"completed-hook\"}' > \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let completed_contexts = run_stop_hooks(
            &server,
            thread_id,
            turn_id,
            TurnStatus::Completed,
            Some("done"),
        )
        .await;
        assert_eq!(completed_contexts.len(), 1);
        assert_eq!(completed_contexts[0].text, "completed reminder");
        assert_eq!(
            completed_contexts[0].summary.as_deref(),
            Some("completed-hook")
        );
        assert_eq!(completed_contexts[0].hook_point, HookPoint::Stop);

        let failed_contexts =
            run_stop_hooks(&server, thread_id, turn_id, TurnStatus::Failed, Some("failed")).await;
        assert_eq!(failed_contexts.len(), 1);
        assert_eq!(failed_contexts[0].text, "failed reminder");
        assert_eq!(failed_contexts[0].summary.as_deref(), Some("failed-hook"));
        assert_eq!(failed_contexts[0].hook_point, HookPoint::Stop);

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_started = events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    ThreadEventKind::ToolStarted { tool, .. } if tool == "hook/run"
                )
            })
            .count();
        assert_eq!(hook_started, 2);

        Ok(())
    }

    #[tokio::test]
    async fn stop_hook_skips_non_matching_turn_status() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  stop:
    - when_turn_status: ["failed"]
      argv:
        [
          "sh",
          "-c",
          "printf '{\"additional_context\":\"failed reminder\"}' > \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let contexts = run_stop_hooks(
            &server,
            thread_id,
            turn_id,
            TurnStatus::Completed,
            Some("done"),
        )
        .await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_started = events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    ThreadEventKind::ToolStarted { tool, .. } if tool == "hook/run"
                )
            })
            .count();
        assert_eq!(hook_started, 0);

        Ok(())
    }

    #[tokio::test]
    async fn post_tool_use_hook_input_artifact_contains_redacted_tool_result() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  post_tool_use:
    - when_tools: ["artifact/write"]
      argv:
        [
          "sh",
          "-c",
          "printf '{}' > \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "artifact_type": "note",
            "summary": "safe summary",
            "text": "hello",
        });
        let tool_result = serde_json::json!({
            "authorization": "Bearer super-secret-token",
            "nested": {
                "api_key": "sk-live-secret",
            },
            "ok": true,
        });

        let contexts = run_post_tool_use_hooks(
            &server,
            thread_id,
            turn_id,
            "artifact/write",
            &tool_args,
            &tool_result,
        )
        .await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                status: ToolStatus::Completed,
                result: Some(result),
                ..
            } => Some(result.clone()),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;

        let input_path = completed
            .get("input_path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;
        let input_bytes = tokio::fs::read(input_path).await?;
        let hook_input: Value = serde_json::from_slice(&input_bytes)?;

        assert_eq!(
            hook_input.get("hook_point").and_then(Value::as_str),
            Some("post_tool_use")
        );
        assert_eq!(
            hook_input.get("tool").and_then(Value::as_str),
            Some("artifact/write")
        );
        assert_eq!(
            hook_input
                .get("tool_result")
                .and_then(|v| v.get("authorization"))
                .and_then(Value::as_str),
            Some("<REDACTED>")
        );
        assert_eq!(
            hook_input
                .get("tool_result")
                .and_then(|v| v.get("nested"))
                .and_then(|v| v.get("api_key"))
                .and_then(Value::as_str),
            Some("<REDACTED>")
        );

        Ok(())
    }

    #[tokio::test]
    async fn hook_additional_context_is_truncated_to_max_chars() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        let oversized_text = "x".repeat(MAX_HOOK_CONTEXT_CHARS + 1024);
        let payload = serde_json::json!({
            "additional_context": oversized_text,
            "summary": "oversized",
        });
        let payload_path = repo_dir.join("hook_output_payload.json");
        tokio::fs::write(&payload_path, serde_json::to_vec(&payload)?).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            format!(
                r#"
version: 1
hooks:
  pre_tool_use:
    - when_tools: ["process/start"]
      argv:
        [
          "sh",
          "-c",
          "cp '{}' \"$OMNE_HOOK_OUTPUT_PATH\"",
        ]
      emit_additional_context: true
"#,
                payload_path.display()
            ),
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "argv": ["echo", "hello"],
            "cwd": repo_dir.display().to_string(),
        });

        let contexts =
            run_pre_tool_use_hooks(&server, thread_id, turn_id, "process/start", &tool_args).await;
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].summary.as_deref(), Some("oversized"));
        assert_eq!(contexts[0].hook_point, HookPoint::PreToolUse);
        assert!(contexts[0].context_path.exists());
        assert_eq!(contexts[0].text.chars().count(), MAX_HOOK_CONTEXT_CHARS + 1);
        assert!(contexts[0].text.ends_with('…'));

        Ok(())
    }

    #[tokio::test]
    async fn hook_process_start_needs_approval_is_recorded_as_skipped() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  pre_tool_use:
    - when_tools: ["process/start"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "argv": ["echo", "hello"],
            "cwd": repo_dir.display().to_string(),
        });

        let contexts =
            run_pre_tool_use_hooks(&server, thread_id, turn_id, "process/start", &tool_args).await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status,
                error,
                result,
                ..
            } if *tool_id == hook_tool_id => Some((status, error.as_deref(), result.clone())),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        assert_eq!(*completed.0, ToolStatus::Denied);
        assert_eq!(completed.1, Some("hook process/start needs approval; skipped"));
        let result = completed
            .2
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted result"))?;
        assert_eq!(
            result.get("needs_approval").and_then(Value::as_bool),
            Some(true)
        );
        assert!(result.get("approval_id").is_some());

        Ok(())
    }

    #[tokio::test]
    async fn hook_process_start_denied_is_recorded() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  pre_tool_use:
    - when_tools: ["process/start"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(ApprovalPolicy::AutoApprove),
                sandbox_policy: Some(SandboxPolicy::ReadOnly),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "argv": ["echo", "hello"],
            "cwd": repo_dir.display().to_string(),
        });

        let contexts =
            run_pre_tool_use_hooks(&server, thread_id, turn_id, "process/start", &tool_args).await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status,
                error,
                result,
                ..
            } if *tool_id == hook_tool_id => Some((status, error.as_deref(), result.clone())),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        assert_eq!(*completed.0, ToolStatus::Denied);
        assert_eq!(completed.1, Some("hook process/start denied"));
        let result = completed
            .2
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted result"))?;
        assert_eq!(result.get("denied").and_then(Value::as_bool), Some(true));

        Ok(())
    }

    #[tokio::test]
    async fn hook_process_timeout_is_recorded_as_failed() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  pre_tool_use:
    - when_tools: ["process/start"]
      argv: ["sh", "-c", "sleep 4"]
      emit_additional_context: true
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "argv": ["echo", "hello"],
            "cwd": repo_dir.display().to_string(),
        });

        let contexts =
            run_pre_tool_use_hooks(&server, thread_id, turn_id, "process/start", &tool_args).await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status,
                error,
                result,
                ..
            } if *tool_id == hook_tool_id => Some((status, error.as_deref(), result.clone())),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        assert_eq!(*completed.0, ToolStatus::Failed);
        let error = completed
            .1
            .ok_or_else(|| anyhow::anyhow!("missing hook timeout error"))?;
        assert!(error.contains("hook process timed out"));
        let result = completed
            .2
            .ok_or_else(|| anyhow::anyhow!("missing hook timeout result"))?;
        assert!(result.get("process_id").is_some());
        assert!(result.get("output").is_some());

        Ok(())
    }

    #[tokio::test]
    async fn stop_hook_input_artifact_redacts_and_truncates_turn_reason() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  stop:
    - when_turn_status: ["failed"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let long_reason = format!(
            "Bearer super-secret-token-abcdefghijklmnopqrstuvwxyz {}",
            "x".repeat(800)
        );
        let turn_id = TurnId::new();
        let contexts = run_stop_hooks(
            &server,
            thread_id,
            turn_id,
            TurnStatus::Failed,
            Some(&long_reason),
        )
        .await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status: ToolStatus::Completed,
                result: Some(result),
                ..
            } if *tool_id == hook_tool_id => Some(result.clone()),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        let input_path = completed
            .get("input_path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;

        let input_bytes = tokio::fs::read(input_path).await?;
        let hook_input: Value = serde_json::from_slice(&input_bytes)?;
        let turn_reason = hook_input
            .get("turn_reason")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing turn_reason in hook input"))?;
        assert!(turn_reason.contains("Bearer <REDACTED>"));
        assert_eq!(turn_reason.chars().count(), 513);
        assert!(turn_reason.ends_with('…'));

        Ok(())
    }

    #[tokio::test]
    async fn subagent_stop_hook_input_artifact_includes_subagent_context() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  subagent_stop:
    - when_turn_status: ["failed"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = handle.thread_id();
        drop(handle);

        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let parent_thread_id_text = parent_thread_id.to_string();
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let reason = "Bearer secret-subagent-reason";
        let contexts = run_subagent_stop_hooks(
            &server,
            parent_thread_id,
            "task-1",
            child_thread_id,
            child_turn_id,
            TurnStatus::Failed,
            Some(reason),
        )
        .await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing parent thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status: ToolStatus::Completed,
                result: Some(result),
                ..
            } if *tool_id == hook_tool_id => Some(result.clone()),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        let input_path = completed
            .get("input_path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;

        let input_bytes = tokio::fs::read(input_path).await?;
        let hook_input: Value = serde_json::from_slice(&input_bytes)?;
        assert_eq!(
            hook_input.get("hook_point").and_then(Value::as_str),
            Some("subagent_stop")
        );
        assert_eq!(
            hook_input.get("thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert!(hook_input.get("turn_id").is_none());
        assert_eq!(
            hook_input.get("turn_status").and_then(Value::as_str),
            Some("failed")
        );

        let subagent = hook_input
            .get("subagent")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing subagent context in hook input"))?;
        assert_eq!(
            subagent.get("task_id").and_then(Value::as_str),
            Some("task-1")
        );
        assert_eq!(
            subagent.get("parent_thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert_eq!(subagent.get("status").and_then(Value::as_str), Some("failed"));
        let redacted_reason = subagent
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing subagent reason in hook input"))?;
        assert!(redacted_reason.contains("Bearer <REDACTED>"));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_start_hook_input_artifact_includes_subagent_context() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  subagent_start:
    - argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = handle.thread_id();
        drop(handle);

        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let parent_thread_id_text = parent_thread_id.to_string();
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let contexts = run_subagent_start_hooks(
            &server,
            parent_thread_id,
            "task-2",
            child_thread_id,
            child_turn_id,
        )
        .await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing parent thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status: ToolStatus::Completed,
                result: Some(result),
                ..
            } if *tool_id == hook_tool_id => Some(result.clone()),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        let input_path = completed
            .get("input_path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;

        let input_bytes = tokio::fs::read(input_path).await?;
        let hook_input: Value = serde_json::from_slice(&input_bytes)?;
        assert_eq!(
            hook_input.get("hook_point").and_then(Value::as_str),
            Some("subagent_start")
        );
        assert_eq!(
            hook_input.get("thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert!(hook_input.get("turn_id").is_none());
        assert!(hook_input.get("turn_status").is_none());
        assert!(hook_input.get("turn_reason").is_none());

        let subagent = hook_input
            .get("subagent")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing subagent context in hook input"))?;
        assert_eq!(
            subagent.get("task_id").and_then(Value::as_str),
            Some("task-2")
        );
        assert_eq!(
            subagent.get("parent_thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert!(subagent.get("status").is_none());
        assert!(subagent.get("reason").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn pre_tool_use_hook_input_artifact_uses_sanitized_artifact_write_params()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  pre_tool_use:
    - when_tools: ["artifact/write"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let tool_args = serde_json::json!({
            "artifact_type": "note",
            "summary": "Bearer hidden-token-abcdefghijklmnopqrstuvwxyz",
            "text": "abc",
        });
        let contexts =
            run_pre_tool_use_hooks(&server, thread_id, turn_id, "artifact/write", &tool_args).await;
        assert!(contexts.is_empty());

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing thread events"))?;

        let hook_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                ThreadEventKind::ToolStarted { tool_id, tool, .. } if tool == "hook/run" => {
                    Some(*tool_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing hook ToolStarted event"))?;

        let completed = events.iter().find_map(|event| match &event.kind {
            ThreadEventKind::ToolCompleted {
                tool_id,
                status: ToolStatus::Completed,
                result: Some(result),
                ..
            } if *tool_id == hook_tool_id => Some(result.clone()),
            _ => None,
        });
        let completed =
            completed.ok_or_else(|| anyhow::anyhow!("missing hook ToolCompleted event"))?;
        let input_path = completed
            .get("input_path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;

        let input_bytes = tokio::fs::read(input_path).await?;
        let hook_input: Value = serde_json::from_slice(&input_bytes)?;
        let tool_params = hook_input
            .get("tool_params")
            .ok_or_else(|| anyhow::anyhow!("missing tool_params in hook input"))?;

        assert_eq!(
            tool_params.get("artifact_type").and_then(Value::as_str),
            Some("note")
        );
        assert_eq!(tool_params.get("bytes").and_then(Value::as_u64), Some(3));
        assert!(tool_params.get("text").is_none());
        let summary = tool_params
            .get("summary")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing summary in tool_params"))?;
        assert!(summary.contains("Bearer <REDACTED>"));

        Ok(())
    }
}
