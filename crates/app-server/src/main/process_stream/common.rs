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

async fn emit_process_tool_denied(
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

fn process_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessDeniedResponse {
        tool_id,
        denied: true,
        thread_id,
        remembered,
        error_code: Some("approval_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize process denied response")
}

fn process_needs_approval_response(
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ProcessNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    };
    serde_json::to_value(response).context("serialize process needs_approval response")
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
        decision: process_mode_decision(mode_decision.decision),
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

fn process_mode_decision(
    decision: omne_core::modes::Decision,
) -> omne_app_server_protocol::ProcessModeDecision {
    match decision {
        omne_core::modes::Decision::Allow => omne_app_server_protocol::ProcessModeDecision::Allow,
        omne_core::modes::Decision::Prompt => {
            omne_app_server_protocol::ProcessModeDecision::Prompt
        }
        omne_core::modes::Decision::Deny => omne_app_server_protocol::ProcessModeDecision::Deny,
    }
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
