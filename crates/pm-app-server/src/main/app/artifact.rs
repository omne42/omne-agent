async fn handle_artifact_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    match method {
        "artifact/write" => match serde_json::from_value::<ArtifactWriteParams>(params) {
            Ok(params) => match handle_artifact_write(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "artifact/list" => match serde_json::from_value::<ArtifactListParams>(params) {
            Ok(params) => match handle_artifact_list(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "artifact/read" => match serde_json::from_value::<ArtifactReadParams>(params) {
            Ok(params) => match handle_artifact_read(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "artifact/delete" => match serde_json::from_value::<ArtifactDeleteParams>(params) {
            Ok(params) => match handle_artifact_delete(server, params).await {
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

