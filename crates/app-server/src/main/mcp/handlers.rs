async fn handle_mcp_list_servers(server: &Server, params: McpListServersParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone(), state.allowed_tools.clone())
    };

    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({});
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "mcp/list_servers",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return mcp_allowed_tools_denied_response(tool_id, "mcp/list_servers", &allowed_tools);
    }
    if !mcp_enabled() {
        return deny_mcp_disabled(&thread_rt, tool_id, params.turn_id, "mcp/list_servers", approval_params).await;
    }

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = mcp_unknown_mode_denied_response(
                tool_id,
                &mode_name,
                available,
                catalog.load_error.clone(),
            )?;

            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "mcp/list_servers".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(result.clone()),
                })
                .await?;
            return Ok(result);
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(mode, "mcp/list_servers", mode.permissions.read);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = mcp_mode_denied_response(tool_id, &mode_name, mode_decision)?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "mcp/list_servers".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("mode denies mcp/list_servers".to_string()),
                result: Some(result.clone()),
            })
            .await?;
        return Ok(result);
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "mcp/list_servers",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = mcp_denied_response(tool_id, Some(remembered))?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "mcp/list_servers".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(result.clone()),
                    })
                    .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return mcp_needs_approval_response(params.thread_id, approval_id);
            }
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "mcp/list_servers".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let cfg = load_mcp_config(&thread_root).await?;
    let servers = cfg
        .servers()
        .iter()
        .map(|(name, cfg)| {
            let transport = serde_json::to_value(cfg.transport())
                .ok()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| format!("{:?}", cfg.transport()).to_ascii_lowercase());
            omne_app_server_protocol::McpServerDescriptor {
                name: name.to_string(),
                transport,
                argv: cfg.argv().to_vec(),
                env_keys: cfg.env().keys().cloned().collect(),
            }
        })
        .collect::<Vec<_>>();

    let server_count = servers.len();
    let response = omne_app_server_protocol::McpListServersResponse {
        config_path: cfg.path().as_ref().map(|p| p.display().to_string()),
        servers,
    };
    let result = serde_json::to_value(response).context("serialize mcp/list_servers response")?;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "servers": server_count,
            })),
        })
        .await?;

    Ok(result)
}

async fn handle_mcp_list_tools(server: &Server, params: McpListToolsParams) -> anyhow::Result<Value> {
    handle_mcp_action(
        server,
        McpActionRequest {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            action: "mcp/list_tools",
            tool_params: serde_json::json!({ "server": params.server }),
            require_prompt_strict: false,
            mcp_method: "tools/list",
            mcp_params: None,
        },
    )
    .await
}

async fn handle_mcp_list_resources(
    server: &Server,
    params: McpListResourcesParams,
) -> anyhow::Result<Value> {
    handle_mcp_action(
        server,
        McpActionRequest {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            action: "mcp/list_resources",
            tool_params: serde_json::json!({ "server": params.server }),
            require_prompt_strict: false,
            mcp_method: "resources/list",
            mcp_params: None,
        },
    )
    .await
}

async fn handle_mcp_call(server: &Server, params: McpCallParams) -> anyhow::Result<Value> {
    let mut mcp_params = serde_json::json!({ "name": params.tool.clone() });
    if let Some(arguments) = params.arguments.clone() {
        mcp_params["arguments"] = arguments;
    }
    let tool_params = serde_json::json!({
        "server": params.server.clone(),
        "tool": params.tool.clone(),
        "arguments": params.arguments.clone(),
    });
    handle_mcp_action(
        server,
        McpActionRequest {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            action: "mcp/call",
            tool_params,
            require_prompt_strict: true,
            mcp_method: "tools/call",
            mcp_params: Some(mcp_params),
        },
    )
    .await
}

struct McpActionRequest {
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<omne_protocol::ApprovalId>,
    action: &'static str,
    tool_params: Value,
    require_prompt_strict: bool,
    mcp_method: &'static str,
    mcp_params: Option<Value>,
}

async fn handle_mcp_action(server: &Server, req: McpActionRequest) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, req.thread_id).await?;
    let (
        approval_policy,
        sandbox_policy,
        sandbox_network_access,
        mode_name,
        allowed_tools,
        thread_execpolicy_rules,
    ) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_network_access,
            state.mode.clone(),
            state.allowed_tools.clone(),
            state.execpolicy_rules.clone(),
        )
    };

    let tool_id = omne_protocol::ToolId::new();
    let Some(server_name) = req.tool_params.get("server").and_then(|v| v.as_str()) else {
        anyhow::bail!("server is required");
    };
    let server_name = server_name.trim();
    if !is_valid_mcp_server_name(server_name) {
        anyhow::bail!("invalid mcp server name: {server_name}");
    }

    let approval_params = {
        let mut params = req.tool_params.clone();
        if let Some(obj) = params.as_object_mut() {
            if req.require_prompt_strict {
                obj.insert(
                    "approval".to_string(),
                    serde_json::json!({ "requirement": "prompt_strict", "source": "mcp" }),
                );
            }
        }
        params
    };

    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        req.turn_id,
        req.action,
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return mcp_allowed_tools_denied_response(tool_id, req.action, &allowed_tools);
    }
    if !mcp_enabled() {
        return deny_mcp_disabled(&thread_rt, tool_id, req.turn_id, req.action, approval_params).await;
    }

    if sandbox_policy == omne_protocol::SandboxPolicy::ReadOnly {
        let result = mcp_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids mcp".to_string()),
                result: Some(result.clone()),
            })
            .await?;
        return Ok(result);
    }

    let cfg = load_mcp_config(&thread_root).await?;
    let Some(server_cfg) = cfg.servers().get(server_name) else {
        let result = mcp_failed_response(tool_id, "unknown mcp server", server_name)?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("unknown mcp server".to_string()),
                result: Some(result.clone()),
            })
            .await?;
        return Ok(result);
    };

    if sandbox_network_access == omne_protocol::SandboxNetworkAccess::Deny
        && omne_process_runtime::command_uses_network(server_cfg.argv())
    {
        let result = mcp_sandbox_network_denied_response(tool_id, sandbox_network_access)?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("sandbox_network_access=deny forbids this command".to_string()),
                result: Some(result.clone()),
            })
            .await?;
        return Ok(result);
    }

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = mcp_unknown_mode_denied_response(
                tool_id,
                &mode_name,
                available,
                catalog.load_error.clone(),
            )?;

            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: req.turn_id,
                    tool: req.action.to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(result.clone()),
                })
                .await?;
            return Ok(result);
        }
    };

    let mode_decision = resolve_mode_decision_audit(
        mode,
        req.action,
        mode.permissions.command.combine(mode.permissions.artifact),
    );
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = mcp_mode_denied_response(tool_id, &mode_name, mode_decision)?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some(format!("mode denies {}", req.action)),
                result: Some(result.clone()),
            })
            .await?;
        return Ok(result);
    }

    let mut effective_exec_policy = server.exec_policy.clone();
    if !mode.command_execpolicy_rules.is_empty() {
        let mode_exec_policy =
            match load_mode_exec_policy(&thread_root, &mode.command_execpolicy_rules).await {
                Ok(policy) => policy,
                Err(err) => {
                    let result = mcp_execpolicy_load_denied_response(
                        tool_id,
                        &mode_name,
                        "failed to load mode execpolicy rules",
                        err.to_string(),
                    )?;
                    thread_rt
                        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                            tool_id,
                            turn_id: req.turn_id,
                            tool: req.action.to_string(),
                            params: Some(approval_params.clone()),
                        })
                        .await?;
                    thread_rt
                        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: omne_protocol::ToolStatus::Denied,
                            error: Some("failed to load mode execpolicy rules".to_string()),
                            result: Some(result.clone()),
                        })
                        .await?;
                    return Ok(result);
                }
            };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &mode_exec_policy);
    }
    if !thread_execpolicy_rules.is_empty() {
        let thread_exec_policy = match load_mode_exec_policy(&thread_root, &thread_execpolicy_rules).await {
            Ok(policy) => policy,
            Err(err) => {
                let result = mcp_execpolicy_load_denied_response(
                    tool_id,
                    &mode_name,
                    "failed to load thread execpolicy rules",
                    err.to_string(),
                )?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: req.turn_id,
                        tool: req.action.to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        error: Some("failed to load thread execpolicy rules".to_string()),
                        result: Some(result.clone()),
                    })
                    .await?;
                return Ok(result);
            }
        };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &thread_exec_policy);
    }
    let exec_matches = effective_exec_policy.matches_for_command(server_cfg.argv(), None);
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();
    let effective_exec_decision = match exec_decision {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };
    let exec_matches_json = serde_json::to_value(&exec_matches)?;

    if effective_exec_decision == ExecDecision::Forbidden {
        let result =
            mcp_execpolicy_denied_response(tool_id, ExecDecision::Forbidden, &exec_matches)?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("execpolicy forbids this command".to_string()),
                result: Some(result.clone()),
            })
            .await?;
        return Ok(result);
    }

    let mut approval_params = approval_params;
    if effective_exec_decision == ExecDecision::PromptStrict {
        if let Some(obj) = approval_params.as_object_mut() {
            obj.insert(
                "approval".to_string(),
                serde_json::json!({ "requirement": "prompt_strict", "source": "execpolicy" }),
            );
        }
    }

    let needs_approval = req.require_prompt_strict
        || mode_decision.decision == omne_core::modes::Decision::Prompt
        || matches!(
            effective_exec_decision,
            ExecDecision::Prompt | ExecDecision::PromptStrict
        );
    if needs_approval {
        match gate_approval_with_deps(
            &server.thread_store,
            &effective_exec_policy,
            &thread_rt,
            req.thread_id,
            req.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: req.approval_id,
                action: req.action,
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = mcp_denied_response(tool_id, Some(remembered))?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: req.turn_id,
                        tool: req.action.to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(result.clone()),
                    })
                    .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return mcp_needs_approval_response(req.thread_id, approval_id);
            }
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: req.turn_id,
            tool: req.action.to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let conn = get_or_start_mcp_connection(
        server,
        &thread_rt,
        &thread_root,
        req.thread_id,
        req.turn_id,
        server_name,
        server_cfg,
    )
    .await?;
    let process_id = conn.process_id;

    let result: anyhow::Result<Value> = async {
        let v = {
            let mut client = conn.client.lock().await;
            mcp_request(&mut client, req.mcp_method, req.mcp_params).await?
        };
        if let Some(artifact) = maybe_write_mcp_result_artifact(
            server,
            tool_id,
            req.thread_id,
            req.turn_id,
            format!("{}: {server_name}", req.action),
            &v,
        )
        .await?
        {
            return Ok(serde_json::json!({
                "process_id": process_id,
                "artifact": artifact,
                "truncated": true,
                "bytes": json_value_size_bytes(&v),
            }));
        }
        Ok(serde_json::json!({
            "process_id": process_id,
            "result": v,
        }))
    }
    .await;

    match result {
        Ok(v) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "process_id": process_id,
                        "server": server_name,
                        "decision": effective_exec_decision,
                        "matched_rules": exec_matches_json,
                    })),
                })
                .await?;
            let response: omne_app_server_protocol::McpActionResponse =
                serde_json::from_value(v).context("parse mcp action response")?;
            let response_value =
                serde_json::to_value(response).context("serialize mcp action response")?;
            Ok(response_value)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: Some(serde_json::json!({
                        "process_id": process_id,
                        "server": server_name,
                        "decision": effective_exec_decision,
                        "matched_rules": exec_matches_json,
                    })),
                })
                .await?;
            Err(err)
        }
    }
}

fn mcp_denied_response(
    tool_id: omne_protocol::ToolId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpDeniedResponse {
        tool_id,
        denied: true,
        remembered,
        error_code: Some("approval_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp denied response")
}

fn mcp_needs_approval_response(
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    };
    serde_json::to_value(response).context("serialize mcp needs_approval response")
}

fn mcp_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: mcp_mode_decision(mode_decision.decision),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
        error_code: Some("mode_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp mode denied response")
}

fn mcp_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        decision: omne_app_server_protocol::McpModeDecision::Deny,
        available,
        load_error,
        error_code: Some("mode_unknown".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp unknown mode denied response")
}

fn mcp_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
        error_code: Some("allowed_tools_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp allowed_tools denied response")
}

fn mcp_sandbox_policy_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_policy: omne_protocol::SandboxPolicy,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpSandboxPolicyDeniedResponse {
        tool_id,
        denied: true,
        sandbox_policy,
        error_code: Some("sandbox_policy_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp sandbox_policy denied response")
}

fn mcp_sandbox_network_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_network_access: omne_protocol::SandboxNetworkAccess,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpSandboxNetworkDeniedResponse {
        tool_id,
        denied: true,
        sandbox_network_access,
        error_code: Some("sandbox_network_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp sandbox_network_access denied response")
}

fn mcp_execpolicy_denied_response(
    tool_id: omne_protocol::ToolId,
    decision: ExecDecision,
    matched_rules: &[ExecRuleMatch],
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpExecPolicyDeniedResponse {
        tool_id,
        denied: true,
        decision: to_protocol_execpolicy_decision(decision),
        matched_rules: to_protocol_execpolicy_matches(matched_rules),
        error_code: Some("execpolicy_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp execpolicy denied response")
}

fn mcp_execpolicy_load_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    error: &str,
    details: String,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpExecPolicyLoadDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        error: error.to_string(),
        details,
        error_code: Some("execpolicy_load_denied".to_string()),
    };
    serde_json::to_value(response).context("serialize mcp execpolicy load denied response")
}

fn mcp_failed_response(
    tool_id: omne_protocol::ToolId,
    error: &str,
    server: &str,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::McpFailedResponse {
        tool_id,
        failed: true,
        error: error.to_string(),
        server: server.to_string(),
    };
    serde_json::to_value(response).context("serialize mcp failed response")
}

fn mcp_mode_decision(decision: omne_core::modes::Decision) -> omne_app_server_protocol::McpModeDecision {
    match decision {
        omne_core::modes::Decision::Allow => omne_app_server_protocol::McpModeDecision::Allow,
        omne_core::modes::Decision::Prompt => omne_app_server_protocol::McpModeDecision::Prompt,
        omne_core::modes::Decision::Deny => omne_app_server_protocol::McpModeDecision::Deny,
    }
}
