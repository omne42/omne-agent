async fn handle_initialized_request(server: &Arc<Server>, request: JsonRpcRequest) -> JsonRpcResponse {
    let JsonRpcRequest { id, method, params } = request;

    match method.as_str() {
        method if method.starts_with("thread/") => handle_thread_request(server, id, method, params).await,
        method if method.starts_with("turn/") => handle_turn_request(server, id, method, params).await,
        method if method.starts_with("process/") => handle_process_request(server, id, method, params).await,
        method if method.starts_with("file/") => handle_file_request(server, id, method, params).await,
        method if method.starts_with("fs/") => handle_fs_request(server, id, method, params).await,
        method if method.starts_with("artifact/") => handle_artifact_request(server, id, method, params).await,
        method if method.starts_with("approval/") => handle_approval_request(server, id, method, params).await,
        _ => JsonRpcResponse::err(
            id,
            JSONRPC_METHOD_NOT_FOUND,
            "method not found",
            Some(serde_json::json!({ "method": method })),
        ),
    }
}

fn invalid_params(id: serde_json::Value, err: serde_json::Error) -> JsonRpcResponse {
    JsonRpcResponse::err(
        id,
        JSONRPC_INVALID_PARAMS,
        "invalid params",
        Some(serde_json::json!({ "error": err.to_string() })),
    )
}

fn method_not_found(id: serde_json::Value, method: &str) -> JsonRpcResponse {
    JsonRpcResponse::err(
        id,
        JSONRPC_METHOD_NOT_FOUND,
        "method not found",
        Some(serde_json::json!({ "method": method })),
    )
}

