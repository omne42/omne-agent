async fn emit_repo_tool_denied(
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

struct RepoModeApprovalContext<'a> {
    thread_rt: &'a Arc<ThreadRuntime>,
    thread_root: &'a Path,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<omne_protocol::ApprovalId>,
    approval_policy: omne_protocol::ApprovalPolicy,
    mode_name: &'a str,
    action: &'static str,
    tool_id: omne_protocol::ToolId,
    approval_params: &'a Value,
}

async fn enforce_repo_mode_and_approval(
    server: &Server,
    ctx: RepoModeApprovalContext<'_>,
) -> anyhow::Result<Option<Value>> {
    let catalog = omne_core::modes::ModeCatalog::load(ctx.thread_root).await;
    let mode = match catalog.mode(ctx.mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = repo_unknown_mode_denied_response(
                ctx.tool_id,
                ctx.mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_repo_tool_denied(
                ctx.thread_rt,
                ctx.tool_id,
                ctx.turn_id,
                ctx.action,
                ctx.approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(Some(result));
        }
    };

    let mode_decision = resolve_mode_decision_audit(
        mode,
        ctx.action,
        mode.permissions.read.combine(mode.permissions.artifact),
    );
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = repo_mode_denied_response(ctx.tool_id, ctx.mode_name, mode_decision)?;
        emit_repo_tool_denied(
            ctx.thread_rt,
            ctx.tool_id,
            ctx.turn_id,
            ctx.action,
            ctx.approval_params,
            format!("mode denies {}", ctx.action),
            result.clone(),
        )
        .await?;
        return Ok(Some(result));
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            ctx.thread_rt,
            ctx.thread_id,
            ctx.turn_id,
            ctx.approval_policy,
            ApprovalRequest {
                approval_id: ctx.approval_id,
                action: ctx.action,
                params: ctx.approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = repo_denied_response(ctx.tool_id, Some(remembered))?;
                emit_repo_tool_denied(
                    ctx.thread_rt,
                    ctx.tool_id,
                    ctx.turn_id,
                    ctx.action,
                    ctx.approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(Some(result));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                let result = repo_needs_approval_response(ctx.thread_id, approval_id)?;
                return Ok(Some(result));
            }
        }
    }

    Ok(None)
}

fn repo_denied_response(
    tool_id: omne_protocol::ToolId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::RepoDeniedResponse {
        tool_id,
        denied: true,
        remembered,
        error_code: Some("approval_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize repo denied response")
}

fn repo_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::RepoAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
        error_code: Some("allowed_tools_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize repo allowed_tools denied response")
}

fn repo_needs_approval_response(
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::RepoNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    };
    serde_json::to_value(response).context("serialize repo needs_approval response")
}

fn repo_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::RepoModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: repo_mode_decision(mode_decision.decision),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
        error_code: Some("mode_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize repo mode denied response")
}

fn repo_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::RepoUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: omne_app_server_protocol::RepoModeDecision::Deny,
        available,
        load_error,
        error_code: Some("mode_unknown".to_string()),
    };
    serde_json::to_value(response).context("serialize repo unknown mode denied response")
}

fn repo_mode_decision(decision: omne_core::modes::Decision) -> omne_app_server_protocol::RepoModeDecision {
    match decision {
        omne_core::modes::Decision::Allow => omne_app_server_protocol::RepoModeDecision::Allow,
        omne_core::modes::Decision::Prompt => omne_app_server_protocol::RepoModeDecision::Prompt,
        omne_core::modes::Decision::Deny => omne_app_server_protocol::RepoModeDecision::Deny,
    }
}
