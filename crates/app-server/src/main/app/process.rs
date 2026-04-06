use super::*;

fn thread_allows_process_list(allowed_tools: &Option<Vec<String>>) -> bool {
    allowed_tools
        .as_ref()
        .is_none_or(|tools| tools.iter().any(|tool| tool == "process/list"))
}

async fn thread_mode_allows_process_list(
    thread_root: &Path,
    mode_name: &str,
    role_name: &str,
) -> Option<ModeDecisionAudit> {
    let catalog = omne_core::modes::ModeCatalog::load(thread_root).await;
    let mode = catalog.mode(mode_name)?;
    let mode_decision = resolve_mode_decision_audit(
        mode,
        "process/list",
        mode.permissions.process.inspect,
    );

    let role_catalog = omne_core::roles::RoleCatalog::builtin();
    let permission_mode_name = role_catalog
        .permission_mode_name(role_name)
        .unwrap_or(mode_name);
    let role_permission_mode = catalog.mode(permission_mode_name).unwrap_or(mode);
    let role_decision = omne_core::allowed_tools::effective_mode_decision_for_tool(
        role_permission_mode,
        "process/list",
    )?;
    let role_override_hit = role_permission_mode.tool_overrides.contains_key("process/list");
    let combined = mode_decision.decision.combine(role_decision);
    let role_tightened = combined != mode_decision.decision;

    Some(ModeDecisionAudit {
        decision: combined,
        decision_source: if role_tightened {
            "role_permission_mode"
        } else {
            mode_decision.decision_source
        },
        tool_override_hit: mode_decision.tool_override_hit || role_override_hit,
    })
}

async fn thread_access_allows_global_process_list(
    thread_root: &Path,
    mode_name: &str,
    role_name: &str,
) -> bool {
    let Some(mode_decision) = thread_mode_allows_process_list(thread_root, mode_name, role_name).await
    else {
        return false;
    };
    mode_decision.decision == omne_core::modes::Decision::Allow
}

pub(super) async fn handle_process_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    if method == "process/list" {
        return handle_process_list_request(server, &id, params).await;
    }

    dispatch_typed_routes!(id, method, params, {
        "process/start" => ProcessStartParams => |params| handle_process_start(server, params),
        "process/inspect" => ProcessInspectParams => |params| handle_process_inspect(server, params),
        "process/kill" => ProcessKillParams => |params| handle_process_kill(server, params),
        "process/interrupt" => ProcessInterruptParams => |params| handle_process_interrupt(server, params),
        "process/tail" => ProcessTailParams => |params| handle_process_tail(server, params),
        "process/follow" => ProcessFollowParams => |params| handle_process_follow(server, params),
    })
}

async fn handle_process_list_request(
    server: &Arc<Server>,
    id: &serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params = match parse_jsonrpc_params::<ProcessListParams>(id, params) {
        Ok(params) => params,
        Err(response) => return *response,
    };

    let visible_thread_ids = if let Some(thread_id) = params.thread_id {
        let (thread_rt, thread_root) = match load_thread_root_without_recovery(server, thread_id).await {
            Ok(values) => values,
            Err(err) => return jsonrpc_internal_error(id, err),
        };
        let (approval_policy, mode_name, role_name, allowed_tools) = {
            let handle = thread_rt.handle.lock().await;
            let state = handle.state();
            (
                state.approval_policy,
                state.mode.clone(),
                state.role.clone(),
                state.allowed_tools.clone(),
            )
        };
        let tool_id = omne_protocol::ToolId::new();
        let approval_params = serde_json::json!({
            "thread_id": thread_id,
        });
        match enforce_thread_allowed_tools(
            &thread_rt,
            tool_id,
            None,
            "process/list",
            Some(approval_params.clone()),
            &allowed_tools,
        )
        .await
        {
            Ok(Some(_result)) => {
                return jsonrpc_ok_or_internal(
                    id,
                    process_allowed_tools_denied_response(tool_id, "process/list", &allowed_tools),
                );
            }
            Ok(None) => {}
            Err(err) => return jsonrpc_internal_error(id, err),
        }
        let Some(mode_decision) =
            thread_mode_allows_process_list(&thread_root, &mode_name, &role_name).await
        else {
            return jsonrpc_ok_or_internal(
                id,
                process_unknown_mode_denied_response(tool_id, thread_id, &mode_name, String::new(), None),
            );
        };
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            return jsonrpc_ok_or_internal(
                id,
                process_mode_denied_response(tool_id, thread_id, &mode_name, mode_decision),
            );
        }
        if mode_decision.decision == omne_core::modes::Decision::Prompt {
            match gate_approval(
                server,
                &thread_rt,
                thread_id,
                None,
                approval_policy,
                ApprovalRequest {
                    approval_id: None,
                    action: "process/list",
                    params: &approval_params,
                },
            )
            .await
            {
                Ok(ApprovalGate::Approved) => {}
                Ok(ApprovalGate::Denied { remembered }) => {
                    return jsonrpc_ok_or_internal(
                        id,
                        process_denied_response(tool_id, thread_id, Some(remembered)),
                    );
                }
                Ok(ApprovalGate::NeedsApproval { approval_id }) => {
                    return jsonrpc_ok_or_internal(
                        id,
                        process_needs_approval_response(thread_id, approval_id),
                    );
                }
                Err(err) => return jsonrpc_internal_error(id, err),
            }
        }
        Some(std::iter::once(thread_id).collect::<std::collections::HashSet<_>>())
    } else {
        let thread_ids = match server.thread_store.list_threads().await {
            Ok(thread_ids) => thread_ids,
            Err(err) => return jsonrpc_internal_error(id, err),
        };
        let mut visible = std::collections::HashSet::new();
        for thread_id in thread_ids {
            let (thread_rt, thread_root) = match load_thread_root_without_recovery(server, thread_id).await {
                Ok(values) => values,
                Err(err) => return jsonrpc_internal_error(id, err),
            };
            let (mode_name, role_name, allowed_tools) = {
                let handle = thread_rt.handle.lock().await;
                let state = handle.state();
                (
                    state.mode.clone(),
                    state.role.clone(),
                    state.allowed_tools.clone(),
                )
            };
            if thread_allows_process_list(&allowed_tools)
                && thread_access_allows_global_process_list(&thread_root, &mode_name, &role_name).await
            {
                visible.insert(thread_id);
            }
        }
        Some(visible)
    };

    let result = handle_process_list(server, params).await.map(|processes| {
        let processes = processes
            .into_iter()
            .filter(|process| {
                visible_thread_ids
                    .as_ref()
                    .is_none_or(|visible| visible.contains(&process.thread_id))
            })
            .map(into_protocol_process_info)
            .collect::<Vec<_>>();
        omne_app_server_protocol::ProcessListResponse { processes }
    });
    jsonrpc_ok_or_internal(id, result)
}
