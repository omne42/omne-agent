fn approval_action_label_from_parts(
    action_id: Option<omne_app_server_protocol::ThreadApprovalActionId>,
    action: Option<&str>,
) -> String {
    if let Some(action_id) = action_id {
        if !matches!(
            action_id,
            omne_app_server_protocol::ThreadApprovalActionId::Unknown
        ) {
            if let Ok(raw) = serde_json::to_string(&action_id) {
                return raw.trim_matches('"').to_string();
            }
        }
    }
    action
        .filter(|raw| !raw.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn approval_action_id_from_action(
    action: &str,
) -> Option<omne_app_server_protocol::ThreadApprovalActionId> {
    Some(match action {
        "artifact/write" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactWrite,
        "artifact/list" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactList,
        "artifact/read" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactRead,
        "artifact/versions" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactVersions,
        "artifact/delete" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactDelete,
        "file/read" => omne_app_server_protocol::ThreadApprovalActionId::FileRead,
        "file/write" => omne_app_server_protocol::ThreadApprovalActionId::FileWrite,
        "file/edit" => omne_app_server_protocol::ThreadApprovalActionId::FileEdit,
        "file/patch" => omne_app_server_protocol::ThreadApprovalActionId::FilePatch,
        "file/delete" => omne_app_server_protocol::ThreadApprovalActionId::FileDelete,
        "file/glob" => omne_app_server_protocol::ThreadApprovalActionId::FileGlob,
        "file/grep" => omne_app_server_protocol::ThreadApprovalActionId::FileGrep,
        "fs/mkdir" => omne_app_server_protocol::ThreadApprovalActionId::FsMkdir,
        "process/start" => omne_app_server_protocol::ThreadApprovalActionId::ProcessStart,
        "process/kill" => omne_app_server_protocol::ThreadApprovalActionId::ProcessKill,
        "process/interrupt" => omne_app_server_protocol::ThreadApprovalActionId::ProcessInterrupt,
        "process/tail" => omne_app_server_protocol::ThreadApprovalActionId::ProcessTail,
        "process/follow" => omne_app_server_protocol::ThreadApprovalActionId::ProcessFollow,
        "process/inspect" => omne_app_server_protocol::ThreadApprovalActionId::ProcessInspect,
        "process/execve" => omne_app_server_protocol::ThreadApprovalActionId::ProcessExecve,
        "repo/search" => omne_app_server_protocol::ThreadApprovalActionId::RepoSearch,
        "repo/index" => omne_app_server_protocol::ThreadApprovalActionId::RepoIndex,
        "repo/symbols" => omne_app_server_protocol::ThreadApprovalActionId::RepoSymbols,
        "mcp/list_servers" => omne_app_server_protocol::ThreadApprovalActionId::McpListServers,
        "mcp/list_tools" => omne_app_server_protocol::ThreadApprovalActionId::McpListTools,
        "mcp/list_resources" => {
            omne_app_server_protocol::ThreadApprovalActionId::McpListResources
        }
        "mcp/call" => omne_app_server_protocol::ThreadApprovalActionId::McpCall,
        "thread/checkpoint/restore" => {
            omne_app_server_protocol::ThreadApprovalActionId::ThreadCheckpointRestore
        }
        "subagent/proxy_approval" => {
            omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval
        }
        _ => return None,
    })
}

fn approval_action_label_from_action(action: &str) -> String {
    approval_action_label_from_parts(approval_action_id_from_action(action), Some(action))
}

fn approval_summary_from_params(
    params: &serde_json::Value,
) -> Option<omne_app_server_protocol::ThreadAttentionPendingApprovalSummary> {
    approval_summary_from_params_with_context(None, None, None, params)
}

fn approval_summary_from_params_with_context(
    thread_id: Option<ThreadId>,
    approval_id: Option<ApprovalId>,
    action: Option<&str>,
    params: &serde_json::Value,
) -> Option<omne_app_server_protocol::ThreadAttentionPendingApprovalSummary> {
    let obj = params.as_object()?;
    let child_request = obj
        .get("child_request")
        .and_then(serde_json::Value::as_object);
    let child_params = child_request
        .and_then(|child| child.get("params"))
        .and_then(serde_json::Value::as_object);
    let proxy = obj
        .get("subagent_proxy")
        .and_then(serde_json::Value::as_object);
    let source = child_params.unwrap_or(obj);

    let requirement = source
        .get("approval")
        .and_then(|value| value.get("requirement"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let argv = source
        .get("argv")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty());
    let cwd = source
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let process_id = source
        .get("process_id")
        .and_then(|value| serde_json::from_value::<ProcessId>(value.clone()).ok());
    let artifact_type = source
        .get("artifact_type")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let path = source
        .get("path")
        .or_else(|| source.get("target_path"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let server = source
        .get("server")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let tool = source
        .get("tool")
        .or_else(|| child_request.and_then(|child| child.get("action")))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let hook = source
        .get("hook")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let child_thread_id = proxy
        .and_then(|value| value.get("child_thread_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<ThreadId>().ok());
    let child_turn_id = proxy
        .and_then(|value| value.get("child_turn_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<TurnId>().ok());
    let child_approval_id = proxy
        .and_then(|value| value.get("child_approval_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<ApprovalId>().ok());
    let approve_cmd = match (thread_id, approval_id, action) {
        (Some(thread_id), Some(approval_id), Some("subagent/proxy_approval")) => Some(format!(
            "omne approval decide {thread_id} {approval_id} --approve"
        )),
        _ => None,
    };
    let deny_cmd = approve_cmd
        .as_deref()
        .and_then(|command| command.strip_suffix(" --approve"))
        .map(|base| format!("{base} --deny"));

    let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
        requirement,
        argv,
        cwd,
        process_id,
        artifact_type,
        path,
        server,
        tool,
        hook,
        child_thread_id,
        child_turn_id,
        child_approval_id,
        child_attention_state: None,
        child_last_turn_status: None,
        approve_cmd,
        deny_cmd,
    };

    if summary.requirement.is_none()
        && summary.argv.is_none()
        && summary.cwd.is_none()
        && summary.process_id.is_none()
        && summary.artifact_type.is_none()
        && summary.path.is_none()
        && summary.server.is_none()
        && summary.tool.is_none()
        && summary.hook.is_none()
        && summary.child_thread_id.is_none()
        && summary.child_turn_id.is_none()
        && summary.child_approval_id.is_none()
        && summary.child_attention_state.is_none()
        && summary.child_last_turn_status.is_none()
        && summary.approve_cmd.is_none()
        && summary.deny_cmd.is_none()
    {
        None
    } else {
        Some(summary)
    }
}

fn approval_summary_hint_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Option<String> {
    if let Some(child_thread_id) = summary.child_thread_id {
        if let Some(child_approval_id) = summary.child_approval_id {
            return Some(format!(
                "subagent={child_thread_id} approval={child_approval_id}"
            ));
        }
        return Some(format!("subagent={child_thread_id}"));
    }
    if let Some(child_approval_id) = summary.child_approval_id {
        return Some(format!("subagent_approval={child_approval_id}"));
    }
    approval_summary_context_hint_from_summary(summary)
}

fn approval_summary_context_hint_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Option<String> {
    if let Some(path) = summary.path.as_deref().filter(|v| !v.is_empty()) {
        return Some(format!("path={path}"));
    }
    if let Some(artifact_type) = summary.artifact_type.as_deref().filter(|v| !v.is_empty()) {
        return Some(format!("artifact_type={artifact_type}"));
    }
    if let Some(process_id) = summary.process_id {
        return Some(format!("process_id={process_id}"));
    }
    if let Some(server) = summary.server.as_deref().filter(|v| !v.is_empty()) {
        if let Some(tool) = summary.tool.as_deref().filter(|v| !v.is_empty()) {
            return Some(format!("mcp={server}/{tool}"));
        }
        return Some(format!("mcp_server={server}"));
    }
    if let Some(tool) = summary.tool.as_deref().filter(|v| !v.is_empty()) {
        return Some(format!("tool={tool}"));
    }
    if let Some(hook) = summary.hook.as_deref().filter(|v| !v.is_empty()) {
        return Some(format!("hook={hook}"));
    }
    if let Some(requirement) = summary.requirement.as_deref().filter(|v| !v.is_empty()) {
        return Some(format!("requirement={requirement}"));
    }
    if let Some(argv) = summary.argv.as_ref().filter(|v| !v.is_empty()) {
        return Some(format!("argv={}", argv.join(" ")));
    }
    if let Some(cwd) = summary.cwd.as_deref().filter(|v| !v.is_empty()) {
        return Some(format!("cwd={cwd}"));
    }
    None
}

fn approval_subagent_link_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Option<String> {
    let mut segments = Vec::<String>::new();
    if let Some(child_thread_id) = summary.child_thread_id {
        segments.push(format!("child_thread_id={child_thread_id}"));
    }
    if let Some(child_turn_id) = summary.child_turn_id {
        segments.push(format!("child_turn_id={child_turn_id}"));
    }
    if let Some(child_approval_id) = summary.child_approval_id {
        segments.push(format!("child_approval_id={child_approval_id}"));
    }
    if let Some(child_attention_state) = summary
        .child_attention_state
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        segments.push(format!("child_attention_state={child_attention_state}"));
    }
    if let Some(child_last_turn_status) = summary.child_last_turn_status {
        segments.push(format!(
            "child_last_turn_status={}",
            turn_status_label(child_last_turn_status)
        ));
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments.join(" "))
    }
}

fn approval_approve_cmd_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Option<String> {
    summary
        .approve_cmd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn approval_deny_cmd_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Option<String> {
    if let Some(deny_cmd) = summary
        .deny_cmd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(deny_cmd.to_string());
    }
    let approve_cmd = approval_approve_cmd_from_summary(summary)?;
    let base = approve_cmd.strip_suffix(" --approve")?;
    Some(format!("{base} --deny"))
}

fn is_subagent_summary_hint(hint: &str) -> bool {
    hint.starts_with("subagent=") || hint.starts_with("subagent_approval=")
}

fn approval_summary_display_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Option<String> {
    let mut parts = Vec::<String>::new();
    if let Some(subagent_link) = approval_subagent_link_from_summary(summary) {
        parts.push(subagent_link);
    }
    if let Some(context_hint) = approval_summary_context_hint_from_summary(summary) {
        parts.push(context_hint);
    }
    if let Some(approve_cmd) = summary.approve_cmd.as_deref().filter(|v| !v.is_empty()) {
        parts.push(format!("approve_cmd={approve_cmd}"));
    }
    if let Some(deny_cmd) = summary.deny_cmd.as_deref().filter(|v| !v.is_empty()) {
        parts.push(format!("deny_cmd={deny_cmd}"));
    }
    if parts.is_empty() {
        if let Some(hint) = approval_summary_hint_from_summary(summary) {
            if !is_subagent_summary_hint(&hint) {
                parts.push(hint);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

fn approval_summary_lines_from_summary(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    if let Some(requirement) = summary.requirement.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("requirement: {requirement}"));
    }
    if let Some(argv) = summary.argv.as_ref().filter(|v| !v.is_empty()) {
        lines.push(format!("argv: {}", argv.join(" ")));
    }
    if let Some(cwd) = summary.cwd.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("cwd: {cwd}"));
    }
    if let Some(process_id) = summary.process_id {
        lines.push(format!("process_id: {process_id}"));
    }
    if let Some(artifact_type) = summary.artifact_type.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("artifact_type: {artifact_type}"));
    }
    if let Some(path) = summary.path.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("path: {path}"));
    }
    if let Some(server) = summary.server.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("server: {server}"));
    }
    if let Some(tool) = summary.tool.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("tool: {tool}"));
    }
    if let Some(hook) = summary.hook.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("hook: {hook}"));
    }
    if let Some(child_thread_id) = summary.child_thread_id {
        lines.push(format!("child_thread_id: {child_thread_id}"));
    }
    if let Some(child_turn_id) = summary.child_turn_id {
        lines.push(format!("child_turn_id: {child_turn_id}"));
    }
    if let Some(child_approval_id) = summary.child_approval_id {
        lines.push(format!("child_approval_id: {child_approval_id}"));
    }
    if let Some(child_attention_state) = summary
        .child_attention_state
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("child_attention_state: {child_attention_state}"));
    }
    if let Some(child_last_turn_status) = summary.child_last_turn_status {
        lines.push(format!(
            "child_last_turn_status: {}",
            turn_status_label(child_last_turn_status)
        ));
    }
    if let Some(approve_cmd) = summary.approve_cmd.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("approve_cmd: {approve_cmd}"));
    }
    if let Some(deny_cmd) = summary.deny_cmd.as_deref().filter(|v| !v.is_empty()) {
        lines.push(format!("deny_cmd: {deny_cmd}"));
    }
    lines
}

fn turn_status_label(status: TurnStatus) -> &'static str {
    match status {
        TurnStatus::Completed => "completed",
        TurnStatus::Interrupted => "interrupted",
        TurnStatus::Failed => "failed",
        TurnStatus::Cancelled => "cancelled",
        TurnStatus::Stuck => "stuck",
    }
}
