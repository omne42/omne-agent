fn rel_path_is_secret(rel_path: &Path) -> bool {
    omne_fs_policy::is_secret_rel_path(rel_path)
}

fn rel_path_is_read_blocked(rel_path: &Path) -> bool {
    omne_fs_policy::is_read_blocked_rel_path(rel_path)
}

enum FileModeApprovalGate {
    Allowed(Box<omne_core::modes::ModeDef>),
    Denied(Box<Value>),
}

struct FileModeApprovalContext<'a> {
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

async fn enforce_file_mode_and_approval<F>(
    server: &Server,
    ctx: FileModeApprovalContext<'_>,
    base_decision_for_mode: F,
) -> anyhow::Result<FileModeApprovalGate>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
{
    let catalog = omne_core::modes::ModeCatalog::load(ctx.thread_root).await;
    let mode = match catalog.mode(ctx.mode_name).cloned() {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = file_unknown_mode_denied_response(
                ctx.tool_id,
                ctx.mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_file_tool_denied(
                ctx.thread_rt,
                ctx.tool_id,
                ctx.turn_id,
                ctx.action,
                ctx.approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(FileModeApprovalGate::Denied(Box::new(result)));
        }
    };

    let mode_decision = resolve_mode_decision_audit(&mode, ctx.action, base_decision_for_mode(&mode));
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = file_mode_denied_response(ctx.tool_id, ctx.mode_name, mode_decision)?;
        emit_file_tool_denied(
            ctx.thread_rt,
            ctx.tool_id,
            ctx.turn_id,
            ctx.action,
            ctx.approval_params,
            format!("mode denies {}", ctx.action),
            result.clone(),
        )
        .await?;
        return Ok(FileModeApprovalGate::Denied(Box::new(result)));
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
                let result = file_denied_response(ctx.tool_id, Some(remembered))?;
                emit_file_tool_denied(
                    ctx.thread_rt,
                    ctx.tool_id,
                    ctx.turn_id,
                    ctx.action,
                    ctx.approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(FileModeApprovalGate::Denied(Box::new(result)));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                let result = file_needs_approval_response(approval_id)?;
                return Ok(FileModeApprovalGate::Denied(Box::new(result)));
            }
        }
    }

    Ok(FileModeApprovalGate::Allowed(Box::new(mode)))
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
    emit_tool_denied(
        thread_rt,
        tool_id,
        turn_id,
        action,
        Some(params.clone()),
        error,
        result,
    )
    .await
}

fn file_denied_response(
    tool_id: omne_protocol::ToolId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    denied_response_with_remembered(
        tool_id,
        remembered,
        "serialize file denied response",
        |tool_id, remembered, structured_error, error_code| omne_app_server_protocol::FileDeniedResponse {
            tool_id,
            denied: true,
            remembered,
            structured_error,
            error_code,
        },
    )
}

fn file_needs_approval_response(approval_id: omne_protocol::ApprovalId) -> anyhow::Result<Value> {
    needs_approval_response_json(
        approval_id,
        "serialize file needs_approval response",
        |approval_id| omne_app_server_protocol::FileNeedsApprovalResponse {
            needs_approval: true,
            approval_id,
        },
    )
}

fn file_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("allowed_tools_denied", |message| {
        message.try_with_value_arg("tool", tool)?;
        Ok(())
    })?;
    let response = omne_app_server_protocol::FileAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize file allowed_tools denied response")
}

fn file_sandbox_policy_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_policy: policy_meta::WriteScope,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error("sandbox_policy_denied")?;
    let response = omne_app_server_protocol::FileSandboxPolicyDeniedResponse {
        tool_id,
        denied: true,
        sandbox_policy,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize file sandbox_policy denied response")
}

fn file_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("mode_denied", |message| {
        message.try_with_value_arg("mode", mode_name)?;
        message.try_with_value_arg("decision_source", mode_decision.decision_source)?;
        message.try_with_value_arg("tool_override_hit", mode_decision.tool_override_hit)?;
        Ok(())
    })?;
    let response = omne_app_server_protocol::FileModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: map_mode_decision_for_protocol!(
            mode_decision.decision,
            omne_app_server_protocol::FileModeDecision
        ),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize file mode denied response")
}

fn file_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("mode_unknown", |message| {
        message.try_with_value_arg("mode", mode_name)?;
        message.try_with_value_arg("available", available.as_str())?;
        if let Some(load_error) = load_error.as_deref() {
            message.try_with_value_arg("load_error", load_error)?;
        }
        Ok(())
    })?;
    let response = omne_app_server_protocol::FileUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: omne_app_server_protocol::FileModeDecision::Deny,
        available,
        load_error,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize file unknown mode denied response")
}
