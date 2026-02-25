fn rel_path_is_secret(rel_path: &Path) -> bool {
    omne_fs_policy::is_secret_rel_path(rel_path)
}

async fn resolve_reference_repo_root(thread_root: &Path) -> anyhow::Result<PathBuf> {
    let rel = Path::new(".omne_data/reference/repo");
    omne_core::resolve_dir(thread_root, rel)
        .await
        .with_context(|| format!("resolve reference repo root {}", thread_root.join(rel).display()))
}

async fn emit_file_tool_denied(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
    turn_id: Option<TurnId>,
    action: &str,
    params: &Value,
    error: String,
    result: Value,
) -> anyhow::Result<()> {
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: action.to_string(),
            params: Some(params.clone()),
        })
        .await?;
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Denied,
            error: Some(error),
            result: Some(result),
        })
        .await?;
    Ok(())
}

fn file_denied_response(
    tool_id: omne_protocol::ToolId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::FileDeniedResponse {
        tool_id,
        denied: true,
        remembered,
        error_code: Some("approval_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize file denied response")
}

fn file_needs_approval_response(approval_id: omne_protocol::ApprovalId) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::FileNeedsApprovalResponse {
        needs_approval: true,
        approval_id,
    };
    serde_json::to_value(response).context("serialize file needs_approval response")
}

fn file_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::FileAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
        error_code: Some("allowed_tools_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize file allowed_tools denied response")
}

fn file_sandbox_policy_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_policy: omne_protocol::SandboxPolicy,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::FileSandboxPolicyDeniedResponse {
        tool_id,
        denied: true,
        sandbox_policy,
        error_code: Some("sandbox_policy_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize file sandbox_policy denied response")
}

fn file_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::FileModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: file_mode_decision(mode_decision.decision),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
        error_code: Some("mode_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize file mode denied response")
}

fn file_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::FileUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: omne_app_server_protocol::FileModeDecision::Deny,
        available,
        load_error,
        error_code: Some("mode_unknown".to_string()),
    };
    serde_json::to_value(response).context("serialize file unknown mode denied response")
}

fn file_mode_decision(decision: omne_core::modes::Decision) -> omne_app_server_protocol::FileModeDecision {
    match decision {
        omne_core::modes::Decision::Allow => omne_app_server_protocol::FileModeDecision::Allow,
        omne_core::modes::Decision::Prompt => omne_app_server_protocol::FileModeDecision::Prompt,
        omne_core::modes::Decision::Deny => omne_app_server_protocol::FileModeDecision::Deny,
    }
}
