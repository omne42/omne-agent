async fn handle_mcp_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    match method {
        "mcp/list_servers" => match serde_json::from_value::<McpListServersParams>(params) {
            Ok(params) => match handle_mcp_list_servers(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "mcp/list_tools" => match serde_json::from_value::<McpListToolsParams>(params) {
            Ok(params) => match handle_mcp_list_tools(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "mcp/list_resources" => match serde_json::from_value::<McpListResourcesParams>(params) {
            Ok(params) => match handle_mcp_list_resources(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "mcp/call" => match serde_json::from_value::<McpCallParams>(params) {
            Ok(params) => match handle_mcp_call(server, params).await {
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

