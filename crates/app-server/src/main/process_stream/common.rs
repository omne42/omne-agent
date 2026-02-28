fn stream_label(stream: ProcessStream) -> &'static str {
    match stream {
        ProcessStream::Stdout => "stdout",
        ProcessStream::Stderr => "stderr",
    }
}

async fn resolve_process_info(server: &Server, process_id: ProcessId) -> anyhow::Result<ProcessInfo> {
    let entry = server.processes.lock().await.get(&process_id).cloned();

    if let Some(entry) = entry {
        let info = entry.info.lock().await;
        return Ok(info.clone());
    }

    let processes = handle_process_list(server, ProcessListParams { thread_id: None }).await?;
    processes
        .into_iter()
        .find(|p| p.process_id == process_id)
        .ok_or_else(|| anyhow::anyhow!("process not found: {}", process_id))
}

enum ProcessModeGate {
    Allowed {
        mode: omne_core::modes::ModeDef,
        mode_decision: ModeDecisionAudit,
    },
    Denied(Value),
}

struct ProcessModeApprovalContext<'a> {
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

async fn enforce_process_mode_gate<F>(
    ctx: &ProcessModeApprovalContext<'_>,
    base_decision_for_mode: F,
) -> anyhow::Result<ProcessModeGate>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
{
    let catalog = omne_core::modes::ModeCatalog::load(ctx.thread_root).await;
    let mode = match catalog.mode(ctx.mode_name).cloned() {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = process_unknown_mode_denied_response(
                ctx.tool_id,
                ctx.thread_id,
                ctx.mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_process_tool_denied(
                ctx.thread_rt,
                ctx.tool_id,
                ctx.turn_id,
                ctx.action,
                ctx.approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(ProcessModeGate::Denied(result));
        }
    };

    let mode_decision = resolve_mode_decision_audit(&mode, ctx.action, base_decision_for_mode(&mode));
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result =
            process_mode_denied_response(ctx.tool_id, ctx.thread_id, ctx.mode_name, mode_decision)?;
        emit_process_tool_denied(
            ctx.thread_rt,
            ctx.tool_id,
            ctx.turn_id,
            ctx.action,
            ctx.approval_params,
            format!("mode denies {}", ctx.action),
            result.clone(),
        )
        .await?;
        return Ok(ProcessModeGate::Denied(result));
    }

    Ok(ProcessModeGate::Allowed {
        mode,
        mode_decision,
    })
}

async fn enforce_process_mode_and_approval<F>(
    server: &Server,
    ctx: ProcessModeApprovalContext<'_>,
    base_decision_for_mode: F,
) -> anyhow::Result<Option<Value>>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
{
    let mode_decision = match enforce_process_mode_gate(&ctx, base_decision_for_mode).await? {
        ProcessModeGate::Denied(result) => return Ok(Some(result)),
        ProcessModeGate::Allowed { mode_decision, .. } => mode_decision,
    };

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
                let result = process_denied_response(ctx.tool_id, ctx.thread_id, Some(remembered))?;
                emit_process_tool_denied(
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
                let result = process_needs_approval_response(ctx.thread_id, approval_id)?;
                return Ok(Some(result));
            }
        }
    }

    Ok(None)
}

async fn emit_process_tool_denied(
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

fn process_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    denied_response_with_remembered(
        tool_id,
        remembered,
        "serialize process denied response",
        |tool_id, remembered, error_code| omne_app_server_protocol::ProcessDeniedResponse {
            tool_id,
            denied: true,
            thread_id,
            remembered,
            error_code,
        },
    )
}

fn process_needs_approval_response(
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Value> {
    needs_approval_response_json(
        approval_id,
        "serialize process needs_approval response",
        |approval_id| omne_app_server_protocol::ProcessNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        },
    )
}

fn process_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
        error_code: Some("allowed_tools_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process allowed_tools denied response")
}

fn process_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    mode_name: &str,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessModeDeniedResponse {
        tool_id,
        denied: true,
        thread_id,
        mode: mode_name.to_string(),
        decision: map_mode_decision_for_protocol!(
            mode_decision.decision,
            omne_app_server_protocol::ProcessModeDecision
        ),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
        error_code: Some("mode_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process mode denied response")
}

fn process_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    mode_name: &str,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        thread_id,
        mode: mode_name.to_string(),
        decision: omne_app_server_protocol::ProcessModeDecision::Deny,
        available,
        load_error,
        error_code: Some("mode_unknown".to_string()),
    };
    serde_json::to_value(response).context("serialize process unknown mode denied response")
}

fn process_sandbox_policy_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_policy: omne_protocol::SandboxPolicy,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessSandboxPolicyDeniedResponse {
        tool_id,
        denied: true,
        sandbox_policy,
        error_code: Some("sandbox_policy_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process sandbox_policy denied response")
}

fn process_sandbox_network_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_network_access: omne_protocol::SandboxNetworkAccess,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessSandboxNetworkDeniedResponse {
        tool_id,
        denied: true,
        sandbox_network_access,
        error_code: Some("sandbox_network_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process sandbox_network denied response")
}

fn process_execpolicy_denied_response(
    tool_id: omne_protocol::ToolId,
    decision: ExecDecision,
    matched_rules: &[ExecRuleMatch],
    justification: Option<String>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessExecPolicyDeniedResponse {
        tool_id,
        denied: true,
        decision: to_protocol_execpolicy_decision(decision),
        matched_rules: to_protocol_execpolicy_matches(matched_rules),
        justification,
        error_code: Some("execpolicy_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process execpolicy denied response")
}

fn process_execpolicy_load_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    error: &str,
    details: String,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessExecPolicyLoadDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        error: error.to_string(),
        details,
        error_code: Some("execpolicy_load_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process execpolicy load denied response")
}

fn to_protocol_execpolicy_decision(
    decision: ExecDecision,
) -> omne_app_server_protocol::ExecPolicyDecision {
    match decision {
        ExecDecision::Allow => omne_app_server_protocol::ExecPolicyDecision::Allow,
        ExecDecision::Prompt => omne_app_server_protocol::ExecPolicyDecision::Prompt,
        ExecDecision::PromptStrict => omne_app_server_protocol::ExecPolicyDecision::PromptStrict,
        ExecDecision::Forbidden => omne_app_server_protocol::ExecPolicyDecision::Forbidden,
    }
}

fn to_protocol_execpolicy_matches(
    matched_rules: &[ExecRuleMatch],
) -> Vec<omne_app_server_protocol::ExecPolicyRuleMatch> {
    matched_rules
        .iter()
        .map(|rule| match rule {
            ExecRuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision,
                justification,
            } => omne_app_server_protocol::ExecPolicyRuleMatch::PrefixRuleMatch {
                matched_prefix: matched_prefix.clone(),
                decision: to_protocol_execpolicy_decision(*decision),
                justification: justification.clone(),
            },
            ExecRuleMatch::HeuristicsRuleMatch { command, decision } => {
                omne_app_server_protocol::ExecPolicyRuleMatch::HeuristicsRuleMatch {
                    command: command.clone(),
                    decision: to_protocol_execpolicy_decision(*decision),
                }
            }
        })
        .collect()
}
