    fn parse_needs_approval(value: &Value) -> Option<(ThreadId, ApprovalId)> {
        if !value
            .get("needs_approval")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return None;
        }
        let thread_id = serde_json::from_value::<ThreadId>(value.get("thread_id")?.clone()).ok()?;
        let approval_id =
            serde_json::from_value::<ApprovalId>(value.get("approval_id")?.clone()).ok()?;
        Some((thread_id, approval_id))
    }

    fn is_denied(value: &Value) -> bool {
        value.get("denied").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    fn summarize_json(value: &Value) -> String {
        let rendered = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
        let rendered = rendered.trim().to_string();
        let mut chars = rendered.chars();
        let prefix: String = chars.by_ref().take(280).collect();
        if chars.next().is_some() {
            format!("{prefix}…")
        } else {
            prefix
        }
    }

    fn summarize_tool_kv(value: &Value) -> String {
        const MAX_ITEMS: usize = 6;
        match value {
            Value::Object(map) => {
                let mut parts = Vec::new();
                for (key, value) in map {
                    if let Some(summary) = summarize_tool_value(value) {
                        if !summary.trim().is_empty() {
                            parts.push(format!("{key}={summary}"));
                        }
                    }
                    if parts.len() >= MAX_ITEMS {
                        break;
                    }
                }
                if parts.is_empty() {
                    String::new()
                } else {
                    format!("[{}]", parts.join(", "))
                }
            }
            _ => summarize_tool_value(value).unwrap_or_default(),
        }
    }

    fn summarize_tool_value(value: &Value) -> Option<String> {
        match value {
            Value::Null => None,
            Value::Bool(value) => Some(value.to_string()),
            Value::Number(value) => Some(value.to_string()),
            Value::String(value) => {
                let normalized = normalize_single_line(value);
                if normalized.is_empty() {
                    None
                } else {
                    Some(truncate_tool_text(&normalized))
                }
            }
            Value::Array(value) => Some(format!("[{}]", value.len())),
            Value::Object(value) => Some(format!("{{{}}}", value.len())),
        }
    }

    fn truncate_tool_text(value: &str) -> String {
        const MAX_CHARS: usize = 120;
        if value.chars().count() <= MAX_CHARS {
            return value.to_string();
        }
        let mut out: String = value.chars().take(MAX_CHARS).collect();
        out.push('…');
        out
    }

    fn normalize_agent_tool_name(tool: &str) -> String {
        if tool.contains('/') {
            return tool.to_string();
        }
        let mut parts = tool.splitn(2, '_');
        let group = parts.next().unwrap_or(tool);
        let action = parts.next().unwrap_or("");
        if action.is_empty() {
            tool.to_string()
        } else {
            format!("{group}/{action}")
        }
    }

    fn truncate_multiline_output(text: &str, max_lines: usize, max_chars: usize) -> String {
        let text = text.trim_end_matches('\n');
        let max_lines = max_lines.max(1);
        let max_chars = max_chars.max(1);

        let mut out = String::new();
        let mut chars_used = 0usize;
        let mut lines_used = 0usize;

        for line in text.split('\n') {
            if lines_used >= max_lines {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push('…');
                return out;
            }
            if lines_used > 0 {
                out.push('\n');
                chars_used = chars_used.saturating_add(1);
            }
            for ch in line.chars() {
                if chars_used >= max_chars {
                    out.push('…');
                    return out;
                }
                out.push(ch);
                chars_used = chars_used.saturating_add(1);
            }
            lines_used = lines_used.saturating_add(1);
        }
        out
    }

    fn extract_primary_tool_text(value: &Value) -> Option<String> {
        match value {
            Value::String(value) => {
                let value = value.to_string();
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value)
                }
            }
            Value::Object(map) => {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    return Some(text.to_string());
                }
                let stdout = map
                    .get("stdout")
                    .or_else(|| map.get("stdout_tail"))
                    .and_then(Value::as_str);
                let stderr = map
                    .get("stderr")
                    .or_else(|| map.get("stderr_tail"))
                    .and_then(Value::as_str);
                if stdout.is_some() || stderr.is_some() {
                    let mut out = String::new();
                    if let Some(stdout) = stdout.filter(|s| !s.trim().is_empty()) {
                        out.push_str(stdout.trim_end());
                    }
                    if let Some(stderr) = stderr.filter(|s| !s.trim().is_empty()) {
                        if !out.is_empty() {
                            out.push_str("\n\n");
                            out.push_str("[stderr]\n");
                        }
                        out.push_str(stderr.trim_end());
                    }
                    if out.trim().is_empty() {
                        None
                    } else {
                        Some(out)
                    }
                } else {
                    for key in ["output", "content", "diff", "patch"] {
                        if let Some(text) = map.get(key).and_then(Value::as_str) {
                            if !text.trim().is_empty() {
                                return Some(text.to_string());
                            }
                        }
                    }
                    None
                }
            }
            _ => None,
        }
    }

    fn tool_meta_brief(value: &Value) -> String {
        let Some(map) = value.as_object() else {
            return String::new();
        };
        let mut parts = Vec::<String>::new();
        if let Some(process_id) = map
            .get("process_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            parts.push(format!("process_id={}", normalize_single_line(process_id)));
        }
        if let Some(path) = map
            .get("resolved_path")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            parts.push(format!("path={}", normalize_single_line(path)));
        } else if let Some(path) = map
            .get("path")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            parts.push(format!("path={}", normalize_single_line(path)));
        }
        if let Some(exit_code) = map.get("exit_code").and_then(Value::as_i64) {
            parts.push(format!("exit_code={exit_code}"));
        }
        if map.get("truncated").and_then(Value::as_bool) == Some(true) {
            parts.push("truncated=true".to_string());
        }
        if map.get("denied").and_then(Value::as_bool) == Some(true) {
            parts.push("denied=true".to_string());
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" [{}]", parts.join(", "))
        }
    }

    fn format_agent_step_tool_result(tool: &str, output: &str) -> Option<String> {
        const MAX_OUTPUT_LINES: usize = 80;
        const MAX_OUTPUT_CHARS: usize = 8_000;

        let tool = normalize_agent_tool_name(tool);
        let output = output.trim();
        if output.is_empty() {
            return Some(format!("{tool} → (empty)"));
        }
        let parsed = serde_json::from_str::<Value>(output);
        if tool == "process/start" {
            match parsed {
                Ok(value) => {
                    if value.get("needs_approval").and_then(Value::as_bool) == Some(true) {
                        let approval_id = value
                            .get("approval_id")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let mut body = "needs_approval=true".to_string();
                        let approval_id = normalize_single_line(approval_id);
                        if approval_id != "-" {
                            body.push_str(&format!(" approval_id={approval_id}"));
                        }
                        return Some(format!("{tool} → {body}"));
                    }

                    if value.get("denied").and_then(Value::as_bool) == Some(true) {
                        return Some(format!("{tool} → denied=true"));
                    }

                    // process/start success is already rendered via ThreadEventKind::ProcessStarted.
                    return None;
                }
                Err(_) => {
                    // Avoid dumping raw JSON (often includes long stdout/stderr paths).
                    return Some(format!("{tool} → (started)"));
                }
            }
        }

        let (meta, body) = match parsed {
            Ok(value) => match tool.as_str() {
                "process/inspect" => {
                    if value.get("needs_approval").and_then(Value::as_bool) == Some(true) {
                        let approval_id = value
                            .get("approval_id")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let mut body = "needs_approval=true".to_string();
                        let approval_id = normalize_single_line(approval_id);
                        if approval_id != "-" {
                            body.push_str(&format!(" approval_id={approval_id}"));
                        }
                        return Some(format!("{tool} → {body}"));
                    }

                    if value.get("denied").and_then(Value::as_bool) == Some(true) {
                        return Some(format!("{tool} → denied=true"));
                    }

                    let process = value.get("process").and_then(Value::as_object);
                    let exit_code = process
                        .and_then(|p| p.get("exit_code"))
                        .and_then(Value::as_i64);
                    let meta = exit_code
                        .map(|code| format!(" [exit_code={code}]"))
                        .unwrap_or_default();

                    let cmd_line = process
                        .and_then(|p| p.get("argv"))
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .and_then(|argv| {
                            let cwd = process
                                .and_then(|p| p.get("cwd"))
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            format_process_started_line(&argv, cwd, None)
                        })
                        .unwrap_or_default();

                    let mut body = extract_primary_tool_text(&value).unwrap_or_default();
                    body = truncate_multiline_output(&body, MAX_OUTPUT_LINES, MAX_OUTPUT_CHARS);
                    let body = if !cmd_line.trim().is_empty() && !body.trim().is_empty() {
                        format!("{cmd_line}\n{body}")
                    } else if !cmd_line.trim().is_empty() {
                        cmd_line
                    } else if !body.trim().is_empty() {
                        body
                    } else {
                        "(no output)".to_string()
                    };
                    (meta, body)
                }
                _ => {
                    let meta = tool_meta_brief(&value);
                    if let Some(text) = extract_primary_tool_text(&value) {
                        (
                            meta,
                            truncate_multiline_output(&text, MAX_OUTPUT_LINES, MAX_OUTPUT_CHARS),
                        )
                    } else {
                        let summary = summarize_json(&value);
                        (
                            meta,
                            truncate_multiline_output(&summary, MAX_OUTPUT_LINES, MAX_OUTPUT_CHARS),
                        )
                    }
                }
            },
            Err(_) => {
                if tool == "process/inspect" {
                    // Avoid dumping raw JSON (often includes long stdout/stderr paths).
                    return Some(format!("{tool} → (output unavailable)"));
                }

                // The agent event log stores output as a JSON string preview which may be truncated
                // with an ellipsis. If parsing fails, show a slightly-unescaped preview.
                let display = output.replace("\\n", "\n").replace("\\t", "\t");
                (
                    String::new(),
                    truncate_multiline_output(&display, MAX_OUTPUT_LINES, MAX_OUTPUT_CHARS),
                )
            }
        };
        if body.contains('\n') {
            Some(format!("{tool}{meta} →\n{body}"))
        } else {
            Some(format!("{tool}{meta} → {body}"))
        }
    }

    fn should_suppress_tool_started(tool: &str) -> bool {
        matches!(
            tool,
            "process/start" | "process/inspect" | "process/tail" | "process/follow"
        )
    }

    fn should_suppress_tool_completed(tool: &str) -> bool {
        matches!(tool, "process/inspect" | "process/tail" | "process/follow")
    }

    fn format_tool_started_line(tool: &str, params: Option<&Value>) -> Option<String> {
        let line = match tool {
            "file/read" => format_file_action("read", params),
            "file/write" => format_file_action("write", params),
            "file/edit" => format_file_action("edit", params),
            "file/patch" => format_file_action("patch", params),
            "file/delete" => format_file_action("delete", params),
            "file/glob" => format_file_glob(params),
            "file/grep" => format_file_grep(params),
            "repo/search" => format_repo_search(params),
            "repo/index" => format_repo_index(params),
            "repo/symbols" => format_repo_symbols(params),
            "fs/mkdir" => format_fs_mkdir(params),
            "process/kill" => format_process_kill(params),
            "process/interrupt" => format_process_interrupt(params),
            "artifact/list" => Some("artifact list".to_string()),
            "artifact/read" => format_artifact_read(params),
            "artifact/write" => format_artifact_write(params),
            "artifact/delete" => format_artifact_delete(params),
            "mcp/list_servers" => Some("mcp list servers".to_string()),
            "mcp/list_tools" => format_mcp_list_tools(params),
            "mcp/list_resources" => format_mcp_list_resources(params),
            "mcp/call" => format_mcp_call(params),
            "subagent/spawn" => Some("subagent spawn".to_string()),
            _ => None,
        };
        if let Some(line) = line {
            return Some(line);
        }
        let summary = params.map(summarize_tool_kv).unwrap_or_default();
        if summary.trim().is_empty() {
            Some(tool.to_string())
        } else {
            Some(format!("{tool} {summary}"))
        }
    }

    fn format_tool_result_line(tool: &str, result: &Value) -> Option<String> {
        if should_suppress_tool_completed(tool) || tool == "process/start" {
            return None;
        }
        let summary = summarize_tool_kv(result);
        if summary.trim().is_empty() {
            None
        } else {
            Some(format!("{tool} → {summary}"))
        }
    }

    fn format_process_started_line(
        argv: &[String],
        cwd: &str,
        thread_cwd: Option<&str>,
    ) -> Option<String> {
        let cmd = extract_shell_command(argv).unwrap_or_else(|| format_argv(argv));
        if cmd.is_empty() {
            return None;
        }
        let mut line = format!("$ {cmd}");
        if let Some(cwd_display) = format_cwd_display(cwd, thread_cwd) {
            line.push_str(&format!(" (cwd={cwd_display})"));
        }
        Some(line)
    }

    fn format_argv(argv: &[String]) -> String {
        argv.iter()
            .map(|arg| format_shell_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn extract_shell_command(argv: &[String]) -> Option<String> {
        if argv.len() >= 3 && is_shell_wrapper(&argv[0]) && argv[1] == "-lc" {
            let cmd = argv[2].trim();
            return if cmd.is_empty() {
                None
            } else {
                Some(cmd.to_string())
            };
        }
        if argv.len() >= 4
            && argv[0] == "/usr/bin/env"
            && is_shell_wrapper(&argv[1])
            && argv[2] == "-lc"
        {
            let cmd = argv[3].trim();
            return if cmd.is_empty() {
                None
            } else {
                Some(cmd.to_string())
            };
        }
        None
    }

    fn is_shell_wrapper(cmd: &str) -> bool {
        matches!(
            cmd,
            "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/sh" | "/bin/zsh"
        )
    }

    fn format_shell_arg(value: &str) -> String {
        if value.is_empty() {
            return "\"\"".to_string();
        }
        let safe = value.chars().all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '/' | ':' ));
        if safe {
            value.to_string()
        } else {
            format!("{value:?}")
        }
    }

    fn format_cwd_display(cwd: &str, thread_cwd: Option<&str>) -> Option<String> {
        let trimmed = cwd.trim();
        if trimmed.is_empty() || trimmed == "." {
            return None;
        }
        if let Some(thread_cwd) = thread_cwd.map(str::trim).filter(|s| !s.is_empty()) {
            if trimmed == thread_cwd {
                return None;
            }
        }
        Some(trimmed.to_string())
    }

    fn format_file_action(action: &str, params: Option<&Value>) -> Option<String> {
        let path = param_str(params, "path")?;
        let root = param_str(params, "root");
        let path = format_path_with_root(root, path);
        Some(format!("{action} {path}"))
    }

    fn format_file_glob(params: Option<&Value>) -> Option<String> {
        let pattern = param_str(params, "pattern")?;
        let root = root_tag(param_str(params, "root"));
        let mut line = format!("glob \"{pattern}\"");
        if let Some(root) = root {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_file_grep(params: Option<&Value>) -> Option<String> {
        let query = param_str(params, "query")?;
        let mut line = format!("grep \"{query}\"");
        if let Some(include) = param_str(params, "include_glob") {
            line.push_str(&format!(" in {include}"));
        }
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_repo_search(params: Option<&Value>) -> Option<String> {
        let query = param_str(params, "query")?;
        let mut line = format!("search \"{query}\"");
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_repo_index(params: Option<&Value>) -> Option<String> {
        let mut line = "repo index".to_string();
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_repo_symbols(params: Option<&Value>) -> Option<String> {
        let mut line = "repo symbols".to_string();
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_fs_mkdir(params: Option<&Value>) -> Option<String> {
        let path = param_str(params, "path")?;
        let recursive = param_bool(params, "recursive").unwrap_or(false);
        if recursive {
            Some(format!("mkdir -p {path}"))
        } else {
            Some(format!("mkdir {path}"))
        }
    }

    fn format_process_kill(params: Option<&Value>) -> Option<String> {
        let process_id = param_str(params, "process_id")?;
        Some(format!("kill {process_id}"))
    }

    fn format_process_interrupt(params: Option<&Value>) -> Option<String> {
        let process_id = param_str(params, "process_id")?;
        Some(format!("interrupt {process_id}"))
    }

    fn format_artifact_read(params: Option<&Value>) -> Option<String> {
        let artifact_id = param_str(params, "artifact_id")?;
        Some(format!("artifact read {artifact_id}"))
    }

    fn format_artifact_write(params: Option<&Value>) -> Option<String> {
        let mut line = "artifact write".to_string();
        if let Some(artifact_type) = param_str(params, "artifact_type") {
            line.push(' ');
            line.push_str(artifact_type);
        }
        if let Some(summary) = param_str(params, "summary") {
            let summary = truncate_tool_text(summary);
            if !summary.is_empty() {
                line.push_str(&format!(" \"{summary}\""));
            }
        }
        Some(line)
    }

    fn format_artifact_delete(params: Option<&Value>) -> Option<String> {
        let artifact_id = param_str(params, "artifact_id")?;
        Some(format!("artifact delete {artifact_id}"))
    }

    fn format_mcp_list_tools(params: Option<&Value>) -> Option<String> {
        let server = param_str(params, "server")?;
        Some(format!("mcp list_tools {server}"))
    }

    fn format_mcp_list_resources(params: Option<&Value>) -> Option<String> {
        let server = param_str(params, "server")?;
        Some(format!("mcp list_resources {server}"))
    }

    fn format_mcp_call(params: Option<&Value>) -> Option<String> {
        let server = param_str(params, "server").unwrap_or("-");
        let tool = param_str(params, "tool").unwrap_or("-");
        Some(format!("mcp {server}.{tool}"))
    }

    fn param_str<'a>(params: Option<&'a Value>, key: &str) -> Option<&'a str> {
        params
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn param_bool(params: Option<&Value>, key: &str) -> Option<bool> {
        params.and_then(|value| value.get(key)).and_then(Value::as_bool)
    }

    fn root_tag(root: Option<&str>) -> Option<&'static str> {
        if matches!(root, Some("reference")) {
            Some("ref")
        } else {
            None
        }
    }

    fn format_path_with_root(root: Option<&str>, path: &str) -> String {
        if matches!(root, Some("reference")) {
            format!("ref:{path}")
        } else {
            path.to_string()
        }
    }
