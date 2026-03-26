#[derive(Debug)]
enum RpcActionOutcome<T> {
    NeedsApproval {
        thread_id: omne_protocol::ThreadId,
        approval_id: omne_protocol::ApprovalId,
    },
    Denied {
        summary: String,
    },
    Ok(T),
}

fn gate_to_tui_outcome<T>(outcome: super::RpcGateOutcome<T>) -> RpcActionOutcome<T> {
    match outcome {
        super::RpcGateOutcome::NeedsApproval {
            thread_id,
            approval_id,
        } => RpcActionOutcome::NeedsApproval {
            thread_id,
            approval_id,
        },
        super::RpcGateOutcome::Denied { detail } => RpcActionOutcome::Denied {
            summary: super::summarize_rpc_detail(&detail),
        },
        super::RpcGateOutcome::Ok(value) => RpcActionOutcome::Ok(value),
    }
}

fn parse_process_tui_outcome<T>(action: &str, value: Value) -> anyhow::Result<RpcActionOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    let outcome = super::parse_process_rpc_outcome(action, value)?;
    Ok(gate_to_tui_outcome(outcome))
}

fn parse_artifact_tui_outcome<T>(action: &str, value: Value) -> anyhow::Result<RpcActionOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    let outcome = super::parse_artifact_rpc_outcome(action, value)?;
    Ok(gate_to_tui_outcome(outcome))
}

async fn rpc_artifact_list_tui_outcome(
    app: &mut super::App,
    params: omne_app_server_protocol::ArtifactListParams,
) -> anyhow::Result<RpcActionOutcome<omne_app_server_protocol::ArtifactListResponse>> {
    let value = app.rpc_artifact_list_value(params).await?;
    parse_artifact_tui_outcome("artifact/list", value)
}

async fn rpc_artifact_read_tui_outcome(
    app: &mut super::App,
    params: omne_app_server_protocol::ArtifactReadParams,
) -> anyhow::Result<RpcActionOutcome<omne_app_server_protocol::ArtifactReadResponse>> {
    let value = app.rpc_artifact_read_value(params).await?;
    parse_artifact_tui_outcome("artifact/read", value)
}

async fn rpc_artifact_versions_tui_outcome(
    app: &mut super::App,
    params: omne_app_server_protocol::ArtifactVersionsParams,
) -> anyhow::Result<RpcActionOutcome<omne_app_server_protocol::ArtifactVersionsResponse>> {
    let value = app.rpc_artifact_versions_value(params).await?;
    parse_artifact_tui_outcome("artifact/versions", value)
}

async fn rpc_process_inspect_tui_outcome(
    app: &mut super::App,
    params: omne_app_server_protocol::ProcessInspectParams,
) -> anyhow::Result<RpcActionOutcome<omne_app_server_protocol::ProcessInspectResponse>> {
    let value = app.rpc_process_inspect_value(params).await?;
    parse_process_tui_outcome("process/inspect", value)
}

async fn rpc_process_kill_tui_outcome(
    app: &mut super::App,
    params: omne_app_server_protocol::ProcessKillParams,
) -> anyhow::Result<RpcActionOutcome<omne_app_server_protocol::ProcessSignalResponse>> {
    let value = app.rpc_process_kill_value(params).await?;
    parse_process_tui_outcome("process/kill", value)
}

async fn rpc_process_interrupt_tui_outcome(
    app: &mut super::App,
    params: omne_app_server_protocol::ProcessInterruptParams,
) -> anyhow::Result<RpcActionOutcome<omne_app_server_protocol::ProcessSignalResponse>> {
    let value = app.rpc_process_interrupt_value(params).await?;
    parse_process_tui_outcome("process/interrupt", value)
}
