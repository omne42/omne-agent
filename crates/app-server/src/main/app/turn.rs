use super::*;

async fn handle_turn_start_request(
    server: &Arc<Server>,
    params: TurnStartParams,
) -> anyhow::Result<omne_app_server_protocol::TurnStartResponse> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    let turn_id = rt
        .start_turn(
            server.clone(),
            params.input,
            params.context_refs,
            params.attachments,
            params.directives,
            params.priority.unwrap_or_default(),
        )
        .await?;
    Ok(omne_app_server_protocol::TurnStartResponse { turn_id })
}

async fn handle_turn_interrupt_request(
    server: &Arc<Server>,
    params: TurnInterruptParams,
) -> anyhow::Result<omne_app_server_protocol::TurnInterruptResponse> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    let kill_reason = params
        .reason
        .clone()
        .or_else(|| Some("turn interrupted".to_string()));
    rt.interrupt_turn(params.turn_id, kill_reason.clone())
        .await?;
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
        kill_processes_for_turn(&server, params.thread_id, params.turn_id, kill_reason).await;
    });
    Ok(omne_app_server_protocol::TurnInterruptResponse { ok: true })
}

pub(super) async fn handle_turn_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    dispatch_typed_routes!(id, method, params, {
        "turn/start" => TurnStartParams => |params| handle_turn_start_request(server, params),
        "turn/interrupt" => TurnInterruptParams => |params| handle_turn_interrupt_request(server, params),
    })
}
