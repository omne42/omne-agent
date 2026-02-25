fn parse_typed_response<T>(action: &str, value: Value) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value).with_context(|| format!("parse {action} response"))
}

fn parse_serialized_response<T, R>(action: &str, kind: &str, response: R) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
    R: serde::Serialize,
{
    let value = serde_json::to_value(response)
        .with_context(|| format!("serialize {action} {kind} response"))?;
    parse_typed_response(action, value)
}

#[derive(Debug)]
enum RpcGateOutcome<T> {
    NeedsApproval {
        thread_id: omne_protocol::ThreadId,
        approval_id: omne_protocol::ApprovalId,
    },
    Denied {
        detail: Value,
    },
    Ok(T),
}

fn denied_detail_value<Response>(response: &Response) -> Value
where
    Response: serde::Serialize + ?Sized,
{
    serde_json::to_value(response).unwrap_or_else(|_| {
        Value::String("<failed to serialize denied response>".to_string())
    })
}

fn denied_result_from_value<T>(action: &str, detail: &Value) -> anyhow::Result<T> {
    let error_code = detail
        .get("error_code")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let detail = serde_json::to_string(detail).unwrap_or_else(|_| detail.to_string());
    if let Some(error_code) = error_code {
        anyhow::bail!("[rpc error_code] {error_code}; {action} denied: {detail}");
    }
    anyhow::bail!("{action} denied: {detail}")
}

fn needs_approval_outcome<T>(
    thread_id: omne_protocol::ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> RpcGateOutcome<T> {
    RpcGateOutcome::NeedsApproval {
        thread_id,
        approval_id,
    }
}

fn denied_outcome<T, Response>(response: &Response) -> RpcGateOutcome<T>
where
    Response: serde::Serialize + ?Sized,
{
    RpcGateOutcome::Denied {
        detail: denied_detail_value(response),
    }
}

fn needs_approval_result<T>(
    thread_id: omne_protocol::ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<RpcGateOutcome<T>> {
    Ok(needs_approval_outcome(thread_id, approval_id))
}

fn denied_result<T, Response>(response: &Response) -> anyhow::Result<RpcGateOutcome<T>>
where
    Response: serde::Serialize + ?Sized,
{
    Ok(denied_outcome(response))
}

fn ok_outcome<T>(response: T) -> RpcGateOutcome<T> {
    RpcGateOutcome::Ok(response)
}

fn parse_ok_outcome_from_value<T>(action: &str, value: Value) -> anyhow::Result<RpcGateOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    parse_typed_response(action, value).map(ok_outcome)
}

fn resolve_gate_outcome<T>(action: &str, outcome: RpcGateOutcome<T>) -> anyhow::Result<T> {
    match outcome {
        RpcGateOutcome::NeedsApproval {
            thread_id,
            approval_id,
        } => approval_required_result(action, &thread_id, &approval_id),
        RpcGateOutcome::Denied { detail } => denied_result_from_value(action, &detail),
        RpcGateOutcome::Ok(response) => Ok(response),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArtifactRpcResponse<T> {
    NeedsApproval(omne_app_server_protocol::ArtifactNeedsApprovalResponse),
    ModeDenied(omne_app_server_protocol::ArtifactModeDeniedResponse),
    UnknownModeDenied(omne_app_server_protocol::ArtifactUnknownModeDeniedResponse),
    AllowedToolsDenied(omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse),
    Denied(omne_app_server_protocol::ArtifactDeniedResponse),
    Ok(T),
}

fn parse_artifact_rpc_outcome<T>(action: &str, value: Value) -> anyhow::Result<RpcGateOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    let parsed: ArtifactRpcResponse<Value> = parse_typed_response(action, value.clone())?;
    match parsed {
        ArtifactRpcResponse::NeedsApproval(response) => {
            Ok(needs_approval_outcome(response.thread_id, response.approval_id))
        }
        ArtifactRpcResponse::ModeDenied(response) => Ok(denied_outcome(&response)),
        ArtifactRpcResponse::UnknownModeDenied(response) => Ok(denied_outcome(&response)),
        ArtifactRpcResponse::AllowedToolsDenied(response) => Ok(denied_outcome(&response)),
        ArtifactRpcResponse::Denied(response) if response.denied => Ok(denied_outcome(&response)),
        // ArtifactDeniedResponse has `denied` defaulting to false, so avoid false positives.
        ArtifactRpcResponse::Denied(_) | ArtifactRpcResponse::Ok(_) => {
            parse_ok_outcome_from_value(action, value)
        }
    }
}

fn parse_artifact_rpc_response_typed<T>(action: &str, value: Value) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    resolve_gate_outcome(action, parse_artifact_rpc_outcome(action, value)?)
}

fn approval_required_result<T, ThreadIdT, ApprovalIdT>(
    action: &str,
    thread_id: &ThreadIdT,
    approval_id: &ApprovalIdT,
) -> anyhow::Result<T>
where
    ThreadIdT: std::fmt::Display + ?Sized,
    ApprovalIdT: std::fmt::Display + ?Sized,
{
    anyhow::bail!(
        "{action} needs approval: omne approval decide {} {} --approve (then re-run with --approval-id {})",
        thread_id,
        approval_id,
        approval_id,
    )
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RepoRpcResponse {
    NeedsApproval(omne_app_server_protocol::RepoNeedsApprovalResponse),
    ModeDenied(omne_app_server_protocol::RepoModeDeniedResponse),
    UnknownModeDenied(omne_app_server_protocol::RepoUnknownModeDeniedResponse),
    AllowedToolsDenied(omne_app_server_protocol::RepoAllowedToolsDeniedResponse),
    Denied(omne_app_server_protocol::RepoDeniedResponse),
    Ok(Value),
}

fn parse_repo_rpc_outcome<T>(action: &str, value: Value) -> anyhow::Result<RpcGateOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    let parsed: RepoRpcResponse = parse_typed_response(action, value)?;
    match parsed {
        RepoRpcResponse::NeedsApproval(response) => {
            needs_approval_result(response.thread_id, response.approval_id)
        }
        RepoRpcResponse::ModeDenied(response) => denied_result(&response),
        RepoRpcResponse::UnknownModeDenied(response) => denied_result(&response),
        RepoRpcResponse::AllowedToolsDenied(response) => denied_result(&response),
        RepoRpcResponse::Denied(response) => denied_result(&response),
        RepoRpcResponse::Ok(response) => parse_ok_outcome_from_value(action, response),
    }
}

fn parse_repo_rpc_response_typed<T>(action: &str, value: Value) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    resolve_gate_outcome(action, parse_repo_rpc_outcome(action, value)?)
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum McpRpcResponse {
    NeedsApproval(omne_app_server_protocol::McpNeedsApprovalResponse),
    ModeDenied(omne_app_server_protocol::McpModeDeniedResponse),
    UnknownModeDenied(omne_app_server_protocol::McpUnknownModeDeniedResponse),
    AllowedToolsDenied(omne_app_server_protocol::McpAllowedToolsDeniedResponse),
    DisabledDenied(omne_app_server_protocol::McpDisabledDeniedResponse),
    SandboxPolicyDenied(omne_app_server_protocol::McpSandboxPolicyDeniedResponse),
    SandboxNetworkDenied(omne_app_server_protocol::McpSandboxNetworkDeniedResponse),
    ExecPolicyDenied(omne_app_server_protocol::McpExecPolicyDeniedResponse),
    ExecPolicyLoadDenied(omne_app_server_protocol::McpExecPolicyLoadDeniedResponse),
    Failed(omne_app_server_protocol::McpFailedResponse),
    Denied(omne_app_server_protocol::McpDeniedResponse),
    Ok(Value),
}

fn parse_mcp_rpc_response_typed<T>(action: &str, value: Value) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    resolve_gate_outcome(action, parse_mcp_rpc_outcome(action, value)?)
}

fn parse_mcp_rpc_outcome<T>(action: &str, value: Value) -> anyhow::Result<RpcGateOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    let parsed: McpRpcResponse = parse_typed_response(action, value)?;
    match parsed {
        McpRpcResponse::NeedsApproval(response) => {
            needs_approval_result(response.thread_id, response.approval_id)
        }
        McpRpcResponse::ModeDenied(response) => denied_result(&response),
        McpRpcResponse::UnknownModeDenied(response) => denied_result(&response),
        McpRpcResponse::AllowedToolsDenied(response) => denied_result(&response),
        McpRpcResponse::DisabledDenied(response) => denied_result(&response),
        McpRpcResponse::SandboxPolicyDenied(response) => denied_result(&response),
        McpRpcResponse::SandboxNetworkDenied(response) => denied_result(&response),
        McpRpcResponse::ExecPolicyDenied(response) => denied_result(&response),
        McpRpcResponse::ExecPolicyLoadDenied(response) => denied_result(&response),
        McpRpcResponse::Denied(response) => denied_result(&response),
        McpRpcResponse::Failed(response) => {
            parse_serialized_response(action, "failed", response).map(ok_outcome)
        }
        McpRpcResponse::Ok(response) => parse_ok_outcome_from_value(action, response),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ProcessRpcResponse {
    NeedsApproval(omne_app_server_protocol::ProcessNeedsApprovalResponse),
    ModeDenied(omne_app_server_protocol::ProcessModeDeniedResponse),
    UnknownModeDenied(omne_app_server_protocol::ProcessUnknownModeDeniedResponse),
    AllowedToolsDenied(omne_app_server_protocol::ProcessAllowedToolsDeniedResponse),
    SandboxPolicyDenied(omne_app_server_protocol::ProcessSandboxPolicyDeniedResponse),
    SandboxNetworkDenied(omne_app_server_protocol::ProcessSandboxNetworkDeniedResponse),
    ExecPolicyDenied(omne_app_server_protocol::ProcessExecPolicyDeniedResponse),
    ExecPolicyLoadDenied(omne_app_server_protocol::ProcessExecPolicyLoadDeniedResponse),
    Denied(omne_app_server_protocol::ProcessDeniedResponse),
    Ok(Value),
}

fn parse_process_rpc_outcome<T>(action: &str, value: Value) -> anyhow::Result<RpcGateOutcome<T>>
where
    T: serde::de::DeserializeOwned,
{
    let parsed: ProcessRpcResponse = parse_typed_response(action, value)?;
    match parsed {
        ProcessRpcResponse::NeedsApproval(response) => {
            needs_approval_result(response.thread_id, response.approval_id)
        }
        ProcessRpcResponse::ModeDenied(response) => denied_result(&response),
        ProcessRpcResponse::UnknownModeDenied(response) => denied_result(&response),
        ProcessRpcResponse::AllowedToolsDenied(response) => denied_result(&response),
        ProcessRpcResponse::SandboxPolicyDenied(response) => denied_result(&response),
        ProcessRpcResponse::SandboxNetworkDenied(response) => denied_result(&response),
        ProcessRpcResponse::ExecPolicyDenied(response) => denied_result(&response),
        ProcessRpcResponse::ExecPolicyLoadDenied(response) => denied_result(&response),
        ProcessRpcResponse::Denied(response) => denied_result(&response),
        ProcessRpcResponse::Ok(response) => parse_ok_outcome_from_value(action, response),
    }
}

fn parse_process_rpc_response_typed<T>(action: &str, value: Value) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    resolve_gate_outcome(action, parse_process_rpc_outcome(action, value)?)
}

fn parse_thread_git_snapshot_rpc_response(
    action: &str,
    value: Value,
) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse> {
    resolve_gate_outcome(action, parse_thread_git_snapshot_rpc_outcome(action, value)?)
}

fn parse_thread_git_snapshot_rpc_outcome(
    action: &str,
    value: Value,
) -> anyhow::Result<RpcGateOutcome<omne_app_server_protocol::ThreadGitSnapshotRpcResponse>> {
    let parsed: omne_app_server_protocol::ThreadGitSnapshotRpcResponse =
        parse_typed_response(action, value)?;
    match parsed {
        omne_app_server_protocol::ThreadGitSnapshotRpcResponse::NeedsApproval(response) => {
            needs_approval_result(response.thread_id, response.approval_id)
        }
        omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => {
            denied_result(&response)
        }
        response => Ok(ok_outcome(response)),
    }
}

fn parse_thread_hook_run_rpc_response(
    action: &str,
    value: Value,
) -> anyhow::Result<omne_app_server_protocol::ThreadHookRunResponse> {
    resolve_gate_outcome(action, parse_thread_hook_run_rpc_outcome(action, value)?)
}

fn parse_thread_hook_run_rpc_outcome(
    action: &str,
    value: Value,
) -> anyhow::Result<RpcGateOutcome<omne_app_server_protocol::ThreadHookRunResponse>> {
    let parsed: omne_app_server_protocol::ThreadHookRunRpcResponse =
        parse_typed_response(action, value)?;
    match parsed {
        omne_app_server_protocol::ThreadHookRunRpcResponse::NeedsApproval(response) => {
            needs_approval_result(response.thread_id, response.approval_id)
        }
        omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => {
            denied_result(&response)
        }
        omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(response) => Ok(ok_outcome(response)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CheckpointRestoreRpcResponse {
    NeedsApproval(omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse),
    Denied(omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse),
    Restored(omne_app_server_protocol::ThreadCheckpointRestoreResponse),
}

fn parse_checkpoint_restore_rpc_response(
    action: &str,
    value: Value,
) -> anyhow::Result<omne_app_server_protocol::ThreadCheckpointRestoreResponse> {
    resolve_gate_outcome(action, parse_checkpoint_restore_rpc_outcome(action, value)?)
}

fn parse_checkpoint_restore_rpc_outcome(
    action: &str,
    value: Value,
) -> anyhow::Result<RpcGateOutcome<omne_app_server_protocol::ThreadCheckpointRestoreResponse>> {
    let parsed: CheckpointRestoreRpcResponse = parse_typed_response(action, value)?;
    match parsed {
        CheckpointRestoreRpcResponse::NeedsApproval(response) => {
            needs_approval_result(response.thread_id, response.approval_id)
        }
        CheckpointRestoreRpcResponse::Denied(response) => denied_result(&response),
        CheckpointRestoreRpcResponse::Restored(response) => Ok(ok_outcome(response)),
    }
}
