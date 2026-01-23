async fn handle_turn_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    match method {
        "turn/start" => match serde_json::from_value::<TurnStartParams>(params) {
            Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                Ok(rt) => match rt
                    .start_turn(server.clone(), params.input, params.context_refs, params.attachments)
                    .await
                {
                    Ok(turn_id) => JsonRpcResponse::ok(id, serde_json::json!({ "turn_id": turn_id })),
                    Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
                },
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "turn/interrupt" => match serde_json::from_value::<TurnInterruptParams>(params) {
            Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                Ok(rt) => {
                    let kill_reason = params
                        .reason
                        .clone()
                        .or_else(|| Some("turn interrupted".to_string()));
                    match rt.interrupt_turn(params.turn_id, kill_reason.clone()).await {
                        Ok(()) => {
                            interrupt_processes_for_turn(
                                server,
                                params.thread_id,
                                params.turn_id,
                                kill_reason.clone(),
                            )
                            .await;
                            let server = server.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                kill_processes_for_turn(
                                    &server,
                                    params.thread_id,
                                    params.turn_id,
                                    kill_reason,
                                )
                                .await;
                            });
                            JsonRpcResponse::ok(id, serde_json::json!({ "ok": true }))
                        }
                        Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
                    }
                }
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
