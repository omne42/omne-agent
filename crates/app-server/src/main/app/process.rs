use super::*;

fn thread_allows_process_list(allowed_tools: &Option<Vec<String>>) -> bool {
    allowed_tools
        .as_ref()
        .is_none_or(|tools| tools.iter().any(|tool| tool == "process/list"))
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
        let thread_rt = match server.get_or_load_thread(thread_id).await {
            Ok(thread_rt) => thread_rt,
            Err(err) => return jsonrpc_internal_error(id, err),
        };
        let allowed_tools = {
            let handle = thread_rt.handle.lock().await;
            handle.state().allowed_tools.clone()
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
            Some(approval_params),
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
            Ok(None) => Some(std::iter::once(thread_id).collect::<std::collections::HashSet<_>>()),
            Err(err) => return jsonrpc_internal_error(id, err),
        }
    } else {
        let thread_ids = match server.thread_store.list_threads().await {
            Ok(thread_ids) => thread_ids,
            Err(err) => return jsonrpc_internal_error(id, err),
        };
        let mut visible = std::collections::HashSet::new();
        for thread_id in thread_ids {
            let thread_rt = match server.get_or_load_thread(thread_id).await {
                Ok(thread_rt) => thread_rt,
                Err(err) => return jsonrpc_internal_error(id, err),
            };
            let allowed_tools = {
                let handle = thread_rt.handle.lock().await;
                handle.state().allowed_tools.clone()
            };
            if thread_allows_process_list(&allowed_tools) {
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
