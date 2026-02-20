async fn handle_mcp_list_servers(server: &Server, params: McpListServersParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone(), state.allowed_tools.clone())
    };

    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({});
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "mcp/list_servers",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }
    if !mcp_enabled() {
        return deny_mcp_disabled(&thread_rt, tool_id, params.turn_id, "mcp/list_servers", approval_params).await;
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "mcp/list_servers".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.read;
    let effective_decision = match mode.tool_overrides.get("mcp/list_servers").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "mcp/list_servers".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies mcp/list_servers".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
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
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "mcp/list_servers".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(serde_json::json!({
                            "approval_policy": approval_policy,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
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
        .map(|(name, cfg)| McpServerDescriptor {
            name: name.to_string(),
            transport: cfg.transport(),
            argv: cfg.argv().to_vec(),
            env_keys: cfg.env().keys().cloned().collect(),
        })
        .collect::<Vec<_>>();

    let result = serde_json::json!({
        "config_path": cfg.path().as_ref().map(|p| p.display().to_string()),
        "servers": servers,
    });

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "servers": servers.len(),
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
    approval_id: Option<pm_protocol::ApprovalId>,
    action: &'static str,
    tool_params: Value,
    require_prompt_strict: bool,
    mcp_method: &'static str,
    mcp_params: Option<Value>,
}

async fn handle_mcp_action(server: &Server, req: McpActionRequest) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, req.thread_id).await?;
    let (approval_policy, sandbox_policy, sandbox_network_access, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_network_access,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };

    let tool_id = pm_protocol::ToolId::new();
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

    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        req.turn_id,
        req.action,
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }
    if !mcp_enabled() {
        return deny_mcp_disabled(&thread_rt, tool_id, req.turn_id, req.action, approval_params).await;
    }

    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids mcp".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let cfg = load_mcp_config(&thread_root).await?;
    let Some(server_cfg) = cfg.servers().get(server_name) else {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Failed,
                error: Some("unknown mcp server".to_string()),
                result: Some(serde_json::json!({
                    "server": server_name,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "failed": true,
            "error": "unknown mcp server",
            "server": server_name,
        }));
    };

    if sandbox_network_access == pm_protocol::SandboxNetworkAccess::Deny
        && command_uses_network(server_cfg.argv())
    {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_network_access=deny forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_network_access": sandbox_network_access,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_network_access": sandbox_network_access,
        }));
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: req.turn_id,
                    tool: req.action.to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.command.combine(mode.permissions.artifact);
    let effective_mode_decision = match mode.tool_overrides.get(req.action).copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_mode_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some(format!("mode denies {}", req.action)),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_mode_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_mode_decision,
        }));
    }

    let exec_matches = if mode.command_execpolicy_rules.is_empty() {
        server.exec_policy.matches_for_command(server_cfg.argv(), None)
    } else {
        let mode_exec_policy =
            match load_mode_exec_policy(&thread_root, &mode.command_execpolicy_rules).await {
                Ok(policy) => policy,
                Err(err) => {
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                            tool_id,
                            turn_id: req.turn_id,
                            tool: req.action.to_string(),
                            params: Some(approval_params.clone()),
                        })
                        .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some("failed to load mode execpolicy rules".to_string()),
                            result: Some(serde_json::json!({
                                "mode": mode_name,
                                "rules": mode.command_execpolicy_rules.clone(),
                                "error": err.to_string(),
                            })),
                        })
                        .await?;
                    return Ok(serde_json::json!({
                        "tool_id": tool_id,
                        "denied": true,
                        "mode": mode_name,
                        "error": "failed to load mode execpolicy rules",
                        "details": err.to_string(),
                    }));
                }
            };

        let combined = merge_exec_policies(&server.exec_policy, &mode_exec_policy);
        combined.matches_for_command(server_cfg.argv(), None)
    };
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();
    let effective_exec_decision = match exec_decision {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };
    let exec_matches_json = serde_json::to_value(&exec_matches)?;

    if effective_exec_decision == ExecDecision::Forbidden {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: req.turn_id,
                tool: req.action.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("execpolicy forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "decision": ExecDecision::Forbidden,
                    "matched_rules": exec_matches_json,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "decision": ExecDecision::Forbidden,
            "matched_rules": exec_matches_json,
        }));
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
        || effective_mode_decision == pm_core::modes::Decision::Prompt
        || matches!(
            effective_exec_decision,
            ExecDecision::Prompt | ExecDecision::PromptStrict
        );
    if needs_approval {
        match gate_approval(
            server,
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
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: req.turn_id,
                        tool: req.action.to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(serde_json::json!({
                            "approval_policy": approval_policy,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": req.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
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
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "process_id": process_id,
                        "server": server_name,
                        "decision": effective_exec_decision,
                        "matched_rules": exec_matches_json,
                    })),
                })
                .await?;
            Ok(v)
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
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
