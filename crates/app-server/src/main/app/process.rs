async fn handle_process_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    match method {
        "process/start" => match serde_json::from_value::<ProcessStartParams>(params) {
            Ok(params) => match handle_process_start(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "process/list" => match serde_json::from_value::<ProcessListParams>(params) {
            Ok(params) => match handle_process_list(server, params).await {
                Ok(processes) => JsonRpcResponse::ok(id, serde_json::json!({ "processes": processes })),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "process/inspect" => match serde_json::from_value::<ProcessInspectParams>(params) {
            Ok(params) => match handle_process_inspect(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "process/kill" => match serde_json::from_value::<ProcessKillParams>(params) {
            Ok(params) => match handle_process_kill(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "process/interrupt" => match serde_json::from_value::<ProcessInterruptParams>(params) {
            Ok(params) => match handle_process_interrupt(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "process/tail" => match serde_json::from_value::<ProcessTailParams>(params) {
            Ok(params) => match handle_process_tail(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "process/follow" => match serde_json::from_value::<ProcessFollowParams>(params) {
            Ok(params) => match handle_process_follow(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        _ => {
            let _ = params;
            method_not_found(id, method)
        }
    }
}

