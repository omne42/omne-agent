use super::*;

pub(super) async fn handle_initialized_request(
    server: &Arc<Server>,
    request: JsonRpcRequest,
) -> JsonRpcResponse {
    let JsonRpcRequest { id, method, params } = request;

    match method.as_str() {
        method if method.starts_with("thread/") => {
            handle_thread_request(server, id, method, params).await
        }
        method if method.starts_with("turn/") => {
            handle_turn_request(server, id, method, params).await
        }
        method if method.starts_with("process/") => {
            handle_process_request(server, id, method, params).await
        }
        method if method.starts_with("file/") => {
            handle_file_request(server, id, method, params).await
        }
        method if method.starts_with("repo/") => {
            handle_repo_request(server, id, method, params).await
        }
        method if method.starts_with("mcp/") => {
            handle_mcp_request(server, id, method, params).await
        }
        method if method.starts_with("fs/") => handle_fs_request(server, id, method, params).await,
        method if method.starts_with("artifact/") => {
            handle_artifact_request(server, id, method, params).await
        }
        method if method.starts_with("approval/") => {
            handle_approval_request(server, id, method, params).await
        }
        _ => JsonRpcResponse::err(
            id,
            JSONRPC_METHOD_NOT_FOUND,
            "method not found",
            Some(serde_json::json!({ "method": method })),
        ),
    }
}

pub(super) fn invalid_params(id: serde_json::Value, err: serde_json::Error) -> JsonRpcResponse {
    JsonRpcResponse::err(
        id,
        JSONRPC_INVALID_PARAMS,
        "invalid params",
        Some(serde_json::json!({ "error": err.to_string() })),
    )
}

fn collect_unknown_json_fields(
    original: &serde_json::Value,
    normalized: &serde_json::Value,
    path: &str,
    out: &mut Vec<String>,
) {
    match (original, normalized) {
        (serde_json::Value::Object(original), serde_json::Value::Object(normalized)) => {
            for (key, value) in original {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if let Some(normalized_value) = normalized.get(key) {
                    collect_unknown_json_fields(value, normalized_value, &child_path, out);
                } else {
                    out.push(child_path);
                }
            }
        }
        (serde_json::Value::Array(original), serde_json::Value::Array(normalized)) => {
            for (index, (value, normalized_value)) in original.iter().zip(normalized).enumerate() {
                let child_path = format!("{path}[{index}]");
                collect_unknown_json_fields(value, normalized_value, &child_path, out);
            }
        }
        _ => {}
    }
}

fn strict_from_value<T>(params: serde_json::Value) -> Result<T, serde_json::Error>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let parsed = serde_json::from_value::<T>(params.clone())?;
    let normalized = serde_json::to_value(&parsed)
        .map_err(|err| serde_json::Error::io(std::io::Error::other(err)))?;
    let mut unknown = Vec::new();
    collect_unknown_json_fields(&params, &normalized, "", &mut unknown);
    if unknown.is_empty() {
        return Ok(parsed);
    }

    Err(serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("unknown fields: {}", unknown.join(", ")),
    )))
}

fn jsonrpc_internal_error_data(err: &anyhow::Error) -> Option<serde_json::Value> {
    thread_configure_error_code(err)
        .map(|error_code| serde_json::json!({ "error_code": error_code }))
}

pub(super) fn jsonrpc_internal_error(
    id: &serde_json::Value,
    err: impl Into<anyhow::Error>,
) -> JsonRpcResponse {
    let err = err.into();
    JsonRpcResponse::err(
        id.clone(),
        JSONRPC_INTERNAL_ERROR,
        err.to_string(),
        jsonrpc_internal_error_data(&err),
    )
}

fn jsonrpc_ok_serialized(
    id: &serde_json::Value,
    payload: impl serde::Serialize,
) -> JsonRpcResponse {
    match serde_json::to_value(payload) {
        Ok(value) => JsonRpcResponse::ok(id.clone(), value),
        Err(err) => jsonrpc_internal_error(id, err),
    }
}

pub(super) fn jsonrpc_ok_or_internal<T: serde::Serialize>(
    id: &serde_json::Value,
    result: anyhow::Result<T>,
) -> JsonRpcResponse {
    match result {
        Ok(payload) => jsonrpc_ok_serialized(id, payload),
        Err(err) => jsonrpc_internal_error(id, err),
    }
}

pub(super) fn parse_jsonrpc_params<T: serde::de::DeserializeOwned + serde::Serialize>(
    id: &serde_json::Value,
    params: serde_json::Value,
) -> Result<T, Box<JsonRpcResponse>> {
    strict_from_value(params).map_err(|err| Box::new(invalid_params(id.clone(), err)))
}

pub(super) async fn dispatch_jsonrpc_request<P, R, F, Fut>(
    id: &serde_json::Value,
    params: serde_json::Value,
    handler: F,
) -> JsonRpcResponse
where
    P: serde::de::DeserializeOwned + serde::Serialize,
    R: serde::Serialize,
    F: FnOnce(P) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<R>>,
{
    match parse_jsonrpc_params::<P>(id, params) {
        Ok(params) => jsonrpc_ok_or_internal(id, handler(params).await),
        Err(response) => *response,
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    #[test]
    fn parse_jsonrpc_params_rejects_unknown_fields() {
        let id = serde_json::json!(1);
        let response = parse_jsonrpc_params::<omne_app_server_protocol::RepoSearchParams>(
            &id,
            serde_json::json!({
                "thread_id": omne_protocol::ThreadId::new(),
                "query": "needle",
                "unexpected": true
            }),
        )
        .expect_err("unknown field should be rejected");

        assert_eq!(response.error.code, JSONRPC_INVALID_PARAMS);
        let message = response.error.data.unwrap_or_default().to_string();
        assert!(message.contains("unexpected"));
    }
}

pub(super) fn method_not_found(id: serde_json::Value, method: &str) -> JsonRpcResponse {
    JsonRpcResponse::err(
        id,
        JSONRPC_METHOD_NOT_FOUND,
        "method not found",
        Some(serde_json::json!({ "method": method })),
    )
}
