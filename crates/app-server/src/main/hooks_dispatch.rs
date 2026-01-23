use pm_protocol::{ThreadEventKind, ToolId, ToolStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HookPoint {
    SessionStart,
    PreToolUse,
    PostToolUse,
    Stop,
}

impl HookPoint {
    fn as_str(self) -> &'static str {
        match self {
            HookPoint::SessionStart => "session_start",
            HookPoint::PreToolUse => "pre_tool_use",
            HookPoint::PostToolUse => "post_tool_use",
            HookPoint::Stop => "stop",
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

fn hooks_process_timeout() -> Duration {
    let value = std::env::var("CODE_PM_HOOK_PROCESS_TIMEOUT_SECS")
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
        .join(".codepm_data")
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
    let _ = write_file_atomic(&path, text.as_bytes()).await;
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
    truncate_owned_chars(pm_core::redact_text(s), max_chars)
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
                .map(|(k, v)| (k.clone(), redact_and_truncate_value(v, max_string_chars)))
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
    Some(match tool_name {
        "file_read" => "file/read",
        "file_glob" => "file/glob",
        "file_grep" => "file/grep",
        "repo_search" => "repo/search",
        "repo_index" => "repo/index",
        "file_write" => "file/write",
        "file_patch" => "file/patch",
        "file_edit" => "file/edit",
        "file_delete" => "file/delete",
        "fs_mkdir" => "fs/mkdir",
        "process_start" => "process/start",
        "process_inspect" => "process/inspect",
        "process_tail" => "process/tail",
        "process_follow" => "process/follow",
        "process_kill" => "process/kill",
        "artifact_write" => "artifact/write",
        "artifact_list" => "artifact/list",
        "artifact_read" => "artifact/read",
        "artifact_delete" => "artifact/delete",
        "thread_state" => "thread/state",
        "thread_events" => "thread/events",
        "thread_diff" => "thread/diff",
        "thread_hook_run" => "thread/hook_run",
        "agent_spawn" => "subagent/spawn",
        _ => return None,
    })
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
                .join(".codepm_data")
                .join("spec")
                .join("hooks.yaml");
            record_hooks_config_error(server, thread_id, &expected_path, &err).await;
            return Vec::new();
        }
    };

    let commands = match hook_point {
        HookPoint::SessionStart => &cfg.hooks.session_start,
        HookPoint::PreToolUse => &cfg.hooks.pre_tool_use,
        HookPoint::PostToolUse => &cfg.hooks.post_tool_use,
        HookPoint::Stop => &cfg.hooks.stop,
    };
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
        let _ = write_file_atomic(&output_path, b"{}").await;

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
        let _ = thread_rt
            .append_event(ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: "hook/run".to_string(),
                params: Some(tool_params.clone()),
            })
            .await;

        let mut env = BTreeMap::<String, String>::new();
        env.insert(
            "CODE_PM_HOOK_INPUT_PATH".to_string(),
            input_path.display().to_string(),
        );
        env.insert(
            "CODE_PM_HOOK_OUTPUT_PATH".to_string(),
            output_path.display().to_string(),
        );
        env.insert(
            "CODE_PM_HOOK_POINT".to_string(),
            hook_point.as_str().to_string(),
        );
        if let Some(tool) = ctx.tool {
            env.insert("CODE_PM_HOOK_TOOL".to_string(), tool.to_string());
        }

        let started = handle_process_start_with_env(
            server,
            ProcessStartParams {
                thread_id,
                turn_id,
                approval_id: None,
                argv: command.argv.clone(),
                cwd: None,
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

        let summary = hook_output
            .as_ref()
            .and_then(|v| v.get("summary"))
            .and_then(Value::as_str)
            .map(|s| redact_and_truncate_string(s, 512))
            .filter(|s| !s.trim().is_empty());

        if command.emit_additional_context {
            if let Some(text) = additional_context.clone() {
                let _ = write_file_atomic(context_path.as_path(), text.as_bytes()).await;
                out.push(HookAdditionalContext {
                    hook_id,
                    hook_point,
                    context_path: context_path.clone(),
                    text,
                    summary: summary.clone(),
                });
            }
        }

        let _ = thread_rt
            .append_event(ThreadEventKind::ToolCompleted {
                tool_id,
                status: ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "process_id": process_id,
                    "exit_code": exit_code,
                    "input_path": input_path.display().to_string(),
                    "output_path": output_path.display().to_string(),
                    "additional_context_path": if command.emit_additional_context && additional_context.is_some() {
                        Some(context_path.display().to_string())
                    } else {
                        None
                    },
                })),
            })
            .await;
    }

    out
}

async fn run_session_start_hooks(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> Vec<HookAdditionalContext> {
    run_hook_commands(
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
    run_hook_commands(
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
    run_hook_commands(
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
    run_hook_commands(
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

#[derive(Clone, Copy, Debug, Default)]
struct HookDispatchContext<'a> {
    tool: Option<&'a str>,
    tool_args: Option<&'a Value>,
    tool_result: Option<&'a Value>,
    turn_status: Option<TurnStatus>,
    turn_reason: Option<&'a str>,
}
