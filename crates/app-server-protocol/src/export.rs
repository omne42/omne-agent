use std::fs;
use std::path::Path;

use anyhow::Context;
use schemars::JsonSchema;
use schemars::schema_for;
use ts_rs::TS;

use crate::{
    ApprovalDecideResponse, ApprovalListResponse, ArtifactAllowedToolsDeniedResponse,
    ArtifactDeleteResponse, ArtifactDeniedResponse, ArtifactListResponse,
    ArtifactModeDeniedResponse, ArtifactNeedsApprovalResponse, ArtifactReadResponse,
    ArtifactUnknownModeDeniedResponse, ArtifactVersionsResponse, ClientRequest,
    FileAllowedToolsDeniedResponse, FileDeniedResponse, FileModeDeniedResponse,
    FileNeedsApprovalResponse, FileSandboxPolicyDeniedResponse, FileUnknownModeDeniedResponse,
    JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse, McpActionArtifactResponse,
    McpActionInlineResponse, McpActionResponse, McpAllowedToolsDeniedResponse, McpDeniedResponse,
    McpDisabledDeniedResponse, McpExecPolicyDeniedResponse, McpExecPolicyLoadDeniedResponse,
    McpFailedResponse, McpListServersResponse, McpModeDeniedResponse, McpNeedsApprovalResponse,
    McpSandboxNetworkDeniedResponse, McpSandboxPolicyDeniedResponse, McpServerDescriptor,
    McpUnknownModeDeniedResponse, ProcessAllowedToolsDeniedResponse, ProcessDeniedResponse,
    ProcessExecPolicyDeniedResponse, ProcessExecPolicyLoadDeniedResponse, ProcessFollowResponse,
    ProcessInfo, ProcessInspectResponse, ProcessListResponse, ProcessModeDeniedResponse,
    ProcessNeedsApprovalResponse, ProcessSandboxNetworkDeniedResponse,
    ProcessSandboxPolicyDeniedResponse, ProcessSignalResponse, ProcessStartResponse, ProcessStatus,
    ProcessTailResponse, ProcessUnknownModeDeniedResponse, RepoAllowedToolsDeniedResponse,
    RepoDeniedResponse, RepoIndexResponse, RepoModeDeniedResponse, RepoNeedsApprovalResponse,
    RepoSearchResponse, RepoSymbolsResponse, RepoUnknownModeDeniedResponse, RequestId,
    ServerNotification, ThreadArchiveResponse, ThreadAttentionResponse, ThreadAutoHookResponse,
    ThreadCheckpointCreateResponse, ThreadCheckpointListResponse,
    ThreadCheckpointRestoreDeniedResponse, ThreadCheckpointRestoreNeedsApprovalResponse,
    ThreadCheckpointRestoreResponse, ThreadClearArtifactsResponse, ThreadConfigExplainResponse,
    ThreadConfigureResponse, ThreadDeleteResponse, ThreadDiskReportResponse,
    ThreadDiskUsageResponse, ThreadEventsResponse, ThreadGitSnapshotDeniedResponse,
    ThreadGitSnapshotNeedsApprovalResponse, ThreadGitSnapshotResponse,
    ThreadGitSnapshotRpcResponse, ThreadGitSnapshotTimedOutResponse, ThreadHandleResponse,
    ThreadHookRunDeniedResponse, ThreadHookRunErrorResponse, ThreadHookRunNeedsApprovalResponse,
    ThreadHookRunResponse, ThreadHookRunRpcResponse, ThreadListMetaResponse, ThreadListResponse,
    ThreadModelsResponse, ThreadPauseResponse, ThreadStartResponse, ThreadStateResponse,
    ThreadSubscribeResponse, ThreadUnarchiveResponse, ThreadUnpauseResponse, ThreadUsageResponse,
    TurnInterruptResponse, TurnStartResponse,
};

pub fn generate_ts(out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create out dir {}", out_dir.display()))?;

    RequestId::export_all_to(out_dir).context("export RequestId typescript")?;
    JsonRpcRequest::export_all_to(out_dir).context("export JsonRpcRequest typescript")?;
    JsonRpcResponse::export_all_to(out_dir).context("export JsonRpcResponse typescript")?;
    JsonRpcErrorResponse::export_all_to(out_dir)
        .context("export JsonRpcErrorResponse typescript")?;
    JsonRpcError::export_all_to(out_dir).context("export JsonRpcError typescript")?;
    ClientRequest::export_all_to(out_dir).context("export ClientRequest typescript")?;
    ServerNotification::export_all_to(out_dir).context("export ServerNotification typescript")?;
    omne_protocol::ThreadEvent::export_all_to(out_dir).context("export ThreadEvent typescript")?;
    ThreadEventsResponse::export_all_to(out_dir)
        .context("export ThreadEventsResponse typescript")?;
    ThreadSubscribeResponse::export_all_to(out_dir)
        .context("export ThreadSubscribeResponse typescript")?;
    ThreadHandleResponse::export_all_to(out_dir)
        .context("export ThreadHandleResponse typescript")?;
    ThreadStartResponse::export_all_to(out_dir).context("export ThreadStartResponse typescript")?;
    TurnStartResponse::export_all_to(out_dir).context("export TurnStartResponse typescript")?;
    TurnInterruptResponse::export_all_to(out_dir)
        .context("export TurnInterruptResponse typescript")?;
    ThreadListResponse::export_all_to(out_dir).context("export ThreadListResponse typescript")?;
    ThreadArchiveResponse::export_all_to(out_dir)
        .context("export ThreadArchiveResponse typescript")?;
    ThreadUnarchiveResponse::export_all_to(out_dir)
        .context("export ThreadUnarchiveResponse typescript")?;
    ThreadPauseResponse::export_all_to(out_dir).context("export ThreadPauseResponse typescript")?;
    ThreadUnpauseResponse::export_all_to(out_dir)
        .context("export ThreadUnpauseResponse typescript")?;
    ThreadDeleteResponse::export_all_to(out_dir)
        .context("export ThreadDeleteResponse typescript")?;
    ThreadClearArtifactsResponse::export_all_to(out_dir)
        .context("export ThreadClearArtifactsResponse typescript")?;
    ThreadListMetaResponse::export_all_to(out_dir)
        .context("export ThreadListMetaResponse typescript")?;
    ThreadAttentionResponse::export_all_to(out_dir)
        .context("export ThreadAttentionResponse typescript")?;
    ThreadStateResponse::export_all_to(out_dir).context("export ThreadStateResponse typescript")?;
    ThreadUsageResponse::export_all_to(out_dir).context("export ThreadUsageResponse typescript")?;
    ThreadConfigureResponse::export_all_to(out_dir)
        .context("export ThreadConfigureResponse typescript")?;
    ThreadConfigExplainResponse::export_all_to(out_dir)
        .context("export ThreadConfigExplainResponse typescript")?;
    ThreadModelsResponse::export_all_to(out_dir)
        .context("export ThreadModelsResponse typescript")?;
    ThreadDiskUsageResponse::export_all_to(out_dir)
        .context("export ThreadDiskUsageResponse typescript")?;
    ThreadDiskReportResponse::export_all_to(out_dir)
        .context("export ThreadDiskReportResponse typescript")?;
    ThreadCheckpointRestoreDeniedResponse::export_all_to(out_dir)
        .context("export ThreadCheckpointRestoreDeniedResponse typescript")?;
    ThreadCheckpointRestoreNeedsApprovalResponse::export_all_to(out_dir)
        .context("export ThreadCheckpointRestoreNeedsApprovalResponse typescript")?;
    ThreadCheckpointRestoreResponse::export_all_to(out_dir)
        .context("export ThreadCheckpointRestoreResponse typescript")?;
    ThreadCheckpointCreateResponse::export_all_to(out_dir)
        .context("export ThreadCheckpointCreateResponse typescript")?;
    ThreadCheckpointListResponse::export_all_to(out_dir)
        .context("export ThreadCheckpointListResponse typescript")?;
    ThreadGitSnapshotResponse::export_all_to(out_dir)
        .context("export ThreadGitSnapshotResponse typescript")?;
    ThreadGitSnapshotNeedsApprovalResponse::export_all_to(out_dir)
        .context("export ThreadGitSnapshotNeedsApprovalResponse typescript")?;
    ThreadGitSnapshotDeniedResponse::export_all_to(out_dir)
        .context("export ThreadGitSnapshotDeniedResponse typescript")?;
    ThreadGitSnapshotTimedOutResponse::export_all_to(out_dir)
        .context("export ThreadGitSnapshotTimedOutResponse typescript")?;
    ThreadGitSnapshotRpcResponse::export_all_to(out_dir)
        .context("export ThreadGitSnapshotRpcResponse typescript")?;
    ThreadHookRunResponse::export_all_to(out_dir)
        .context("export ThreadHookRunResponse typescript")?;
    ThreadHookRunNeedsApprovalResponse::export_all_to(out_dir)
        .context("export ThreadHookRunNeedsApprovalResponse typescript")?;
    ThreadHookRunDeniedResponse::export_all_to(out_dir)
        .context("export ThreadHookRunDeniedResponse typescript")?;
    ThreadHookRunErrorResponse::export_all_to(out_dir)
        .context("export ThreadHookRunErrorResponse typescript")?;
    ThreadHookRunRpcResponse::export_all_to(out_dir)
        .context("export ThreadHookRunRpcResponse typescript")?;
    ThreadAutoHookResponse::export_all_to(out_dir)
        .context("export ThreadAutoHookResponse typescript")?;
    ArtifactListResponse::export_all_to(out_dir)
        .context("export ArtifactListResponse typescript")?;
    ArtifactVersionsResponse::export_all_to(out_dir)
        .context("export ArtifactVersionsResponse typescript")?;
    ArtifactReadResponse::export_all_to(out_dir)
        .context("export ArtifactReadResponse typescript")?;
    ArtifactDeleteResponse::export_all_to(out_dir)
        .context("export ArtifactDeleteResponse typescript")?;
    ArtifactDeniedResponse::export_all_to(out_dir)
        .context("export ArtifactDeniedResponse typescript")?;
    ArtifactNeedsApprovalResponse::export_all_to(out_dir)
        .context("export ArtifactNeedsApprovalResponse typescript")?;
    ArtifactModeDeniedResponse::export_all_to(out_dir)
        .context("export ArtifactModeDeniedResponse typescript")?;
    ArtifactUnknownModeDeniedResponse::export_all_to(out_dir)
        .context("export ArtifactUnknownModeDeniedResponse typescript")?;
    ArtifactAllowedToolsDeniedResponse::export_all_to(out_dir)
        .context("export ArtifactAllowedToolsDeniedResponse typescript")?;
    RepoDeniedResponse::export_all_to(out_dir).context("export RepoDeniedResponse typescript")?;
    RepoNeedsApprovalResponse::export_all_to(out_dir)
        .context("export RepoNeedsApprovalResponse typescript")?;
    RepoModeDeniedResponse::export_all_to(out_dir)
        .context("export RepoModeDeniedResponse typescript")?;
    RepoUnknownModeDeniedResponse::export_all_to(out_dir)
        .context("export RepoUnknownModeDeniedResponse typescript")?;
    RepoAllowedToolsDeniedResponse::export_all_to(out_dir)
        .context("export RepoAllowedToolsDeniedResponse typescript")?;
    RepoSearchResponse::export_all_to(out_dir).context("export RepoSearchResponse typescript")?;
    RepoIndexResponse::export_all_to(out_dir).context("export RepoIndexResponse typescript")?;
    RepoSymbolsResponse::export_all_to(out_dir).context("export RepoSymbolsResponse typescript")?;
    McpDeniedResponse::export_all_to(out_dir).context("export McpDeniedResponse typescript")?;
    McpNeedsApprovalResponse::export_all_to(out_dir)
        .context("export McpNeedsApprovalResponse typescript")?;
    McpModeDeniedResponse::export_all_to(out_dir)
        .context("export McpModeDeniedResponse typescript")?;
    McpUnknownModeDeniedResponse::export_all_to(out_dir)
        .context("export McpUnknownModeDeniedResponse typescript")?;
    McpAllowedToolsDeniedResponse::export_all_to(out_dir)
        .context("export McpAllowedToolsDeniedResponse typescript")?;
    McpDisabledDeniedResponse::export_all_to(out_dir)
        .context("export McpDisabledDeniedResponse typescript")?;
    McpSandboxPolicyDeniedResponse::export_all_to(out_dir)
        .context("export McpSandboxPolicyDeniedResponse typescript")?;
    McpSandboxNetworkDeniedResponse::export_all_to(out_dir)
        .context("export McpSandboxNetworkDeniedResponse typescript")?;
    McpExecPolicyDeniedResponse::export_all_to(out_dir)
        .context("export McpExecPolicyDeniedResponse typescript")?;
    McpExecPolicyLoadDeniedResponse::export_all_to(out_dir)
        .context("export McpExecPolicyLoadDeniedResponse typescript")?;
    McpFailedResponse::export_all_to(out_dir).context("export McpFailedResponse typescript")?;
    McpServerDescriptor::export_all_to(out_dir).context("export McpServerDescriptor typescript")?;
    McpListServersResponse::export_all_to(out_dir)
        .context("export McpListServersResponse typescript")?;
    McpActionInlineResponse::export_all_to(out_dir)
        .context("export McpActionInlineResponse typescript")?;
    McpActionArtifactResponse::export_all_to(out_dir)
        .context("export McpActionArtifactResponse typescript")?;
    McpActionResponse::export_all_to(out_dir).context("export McpActionResponse typescript")?;
    ProcessStatus::export_all_to(out_dir).context("export ProcessStatus typescript")?;
    ProcessInfo::export_all_to(out_dir).context("export ProcessInfo typescript")?;
    ProcessListResponse::export_all_to(out_dir).context("export ProcessListResponse typescript")?;
    ProcessStartResponse::export_all_to(out_dir)
        .context("export ProcessStartResponse typescript")?;
    ProcessInspectResponse::export_all_to(out_dir)
        .context("export ProcessInspectResponse typescript")?;
    ProcessTailResponse::export_all_to(out_dir).context("export ProcessTailResponse typescript")?;
    ProcessFollowResponse::export_all_to(out_dir)
        .context("export ProcessFollowResponse typescript")?;
    ProcessSignalResponse::export_all_to(out_dir)
        .context("export ProcessSignalResponse typescript")?;
    ProcessDeniedResponse::export_all_to(out_dir)
        .context("export ProcessDeniedResponse typescript")?;
    ProcessNeedsApprovalResponse::export_all_to(out_dir)
        .context("export ProcessNeedsApprovalResponse typescript")?;
    ProcessModeDeniedResponse::export_all_to(out_dir)
        .context("export ProcessModeDeniedResponse typescript")?;
    ProcessUnknownModeDeniedResponse::export_all_to(out_dir)
        .context("export ProcessUnknownModeDeniedResponse typescript")?;
    ProcessAllowedToolsDeniedResponse::export_all_to(out_dir)
        .context("export ProcessAllowedToolsDeniedResponse typescript")?;
    ProcessSandboxPolicyDeniedResponse::export_all_to(out_dir)
        .context("export ProcessSandboxPolicyDeniedResponse typescript")?;
    ProcessSandboxNetworkDeniedResponse::export_all_to(out_dir)
        .context("export ProcessSandboxNetworkDeniedResponse typescript")?;
    ProcessExecPolicyDeniedResponse::export_all_to(out_dir)
        .context("export ProcessExecPolicyDeniedResponse typescript")?;
    ProcessExecPolicyLoadDeniedResponse::export_all_to(out_dir)
        .context("export ProcessExecPolicyLoadDeniedResponse typescript")?;
    FileDeniedResponse::export_all_to(out_dir).context("export FileDeniedResponse typescript")?;
    FileNeedsApprovalResponse::export_all_to(out_dir)
        .context("export FileNeedsApprovalResponse typescript")?;
    FileModeDeniedResponse::export_all_to(out_dir)
        .context("export FileModeDeniedResponse typescript")?;
    FileUnknownModeDeniedResponse::export_all_to(out_dir)
        .context("export FileUnknownModeDeniedResponse typescript")?;
    FileAllowedToolsDeniedResponse::export_all_to(out_dir)
        .context("export FileAllowedToolsDeniedResponse typescript")?;
    FileSandboxPolicyDeniedResponse::export_all_to(out_dir)
        .context("export FileSandboxPolicyDeniedResponse typescript")?;
    ApprovalDecideResponse::export_all_to(out_dir)
        .context("export ApprovalDecideResponse typescript")?;
    ApprovalListResponse::export_all_to(out_dir)
        .context("export ApprovalListResponse typescript")?;

    Ok(())
}

pub fn generate_json_schema(out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create out dir {}", out_dir.display()))?;

    write_schema::<RequestId>(out_dir, "RequestId")?;
    write_schema::<JsonRpcRequest>(out_dir, "JsonRpcRequest")?;
    write_schema::<JsonRpcResponse>(out_dir, "JsonRpcResponse")?;
    write_schema::<JsonRpcErrorResponse>(out_dir, "JsonRpcErrorResponse")?;
    write_schema::<JsonRpcError>(out_dir, "JsonRpcError")?;
    write_schema::<ClientRequest>(out_dir, "ClientRequest")?;
    write_schema::<ServerNotification>(out_dir, "ServerNotification")?;
    write_schema::<omne_protocol::ThreadEvent>(out_dir, "ThreadEvent")?;
    write_schema::<ThreadEventsResponse>(out_dir, "ThreadEventsResponse")?;
    write_schema::<ThreadSubscribeResponse>(out_dir, "ThreadSubscribeResponse")?;
    write_schema::<ThreadHandleResponse>(out_dir, "ThreadHandleResponse")?;
    write_schema::<ThreadStartResponse>(out_dir, "ThreadStartResponse")?;
    write_schema::<TurnStartResponse>(out_dir, "TurnStartResponse")?;
    write_schema::<TurnInterruptResponse>(out_dir, "TurnInterruptResponse")?;
    write_schema::<ThreadListResponse>(out_dir, "ThreadListResponse")?;
    write_schema::<ThreadArchiveResponse>(out_dir, "ThreadArchiveResponse")?;
    write_schema::<ThreadUnarchiveResponse>(out_dir, "ThreadUnarchiveResponse")?;
    write_schema::<ThreadPauseResponse>(out_dir, "ThreadPauseResponse")?;
    write_schema::<ThreadUnpauseResponse>(out_dir, "ThreadUnpauseResponse")?;
    write_schema::<ThreadDeleteResponse>(out_dir, "ThreadDeleteResponse")?;
    write_schema::<ThreadClearArtifactsResponse>(out_dir, "ThreadClearArtifactsResponse")?;
    write_schema::<ThreadListMetaResponse>(out_dir, "ThreadListMetaResponse")?;
    write_schema::<ThreadAttentionResponse>(out_dir, "ThreadAttentionResponse")?;
    write_schema::<ThreadStateResponse>(out_dir, "ThreadStateResponse")?;
    write_schema::<ThreadUsageResponse>(out_dir, "ThreadUsageResponse")?;
    write_schema::<ThreadConfigureResponse>(out_dir, "ThreadConfigureResponse")?;
    write_schema::<ThreadConfigExplainResponse>(out_dir, "ThreadConfigExplainResponse")?;
    write_schema::<ThreadModelsResponse>(out_dir, "ThreadModelsResponse")?;
    write_schema::<ThreadDiskUsageResponse>(out_dir, "ThreadDiskUsageResponse")?;
    write_schema::<ThreadDiskReportResponse>(out_dir, "ThreadDiskReportResponse")?;
    write_schema::<ThreadCheckpointRestoreDeniedResponse>(
        out_dir,
        "ThreadCheckpointRestoreDeniedResponse",
    )?;
    write_schema::<ThreadCheckpointRestoreNeedsApprovalResponse>(
        out_dir,
        "ThreadCheckpointRestoreNeedsApprovalResponse",
    )?;
    write_schema::<ThreadCheckpointRestoreResponse>(out_dir, "ThreadCheckpointRestoreResponse")?;
    write_schema::<ThreadCheckpointCreateResponse>(out_dir, "ThreadCheckpointCreateResponse")?;
    write_schema::<ThreadCheckpointListResponse>(out_dir, "ThreadCheckpointListResponse")?;
    write_schema::<ThreadGitSnapshotResponse>(out_dir, "ThreadGitSnapshotResponse")?;
    write_schema::<ThreadGitSnapshotNeedsApprovalResponse>(
        out_dir,
        "ThreadGitSnapshotNeedsApprovalResponse",
    )?;
    write_schema::<ThreadGitSnapshotDeniedResponse>(out_dir, "ThreadGitSnapshotDeniedResponse")?;
    write_schema::<ThreadGitSnapshotTimedOutResponse>(
        out_dir,
        "ThreadGitSnapshotTimedOutResponse",
    )?;
    write_schema::<ThreadGitSnapshotRpcResponse>(out_dir, "ThreadGitSnapshotRpcResponse")?;
    write_schema::<ThreadHookRunResponse>(out_dir, "ThreadHookRunResponse")?;
    write_schema::<ThreadHookRunNeedsApprovalResponse>(
        out_dir,
        "ThreadHookRunNeedsApprovalResponse",
    )?;
    write_schema::<ThreadHookRunDeniedResponse>(out_dir, "ThreadHookRunDeniedResponse")?;
    write_schema::<ThreadHookRunErrorResponse>(out_dir, "ThreadHookRunErrorResponse")?;
    write_schema::<ThreadHookRunRpcResponse>(out_dir, "ThreadHookRunRpcResponse")?;
    write_schema::<ThreadAutoHookResponse>(out_dir, "ThreadAutoHookResponse")?;
    write_schema::<ArtifactListResponse>(out_dir, "ArtifactListResponse")?;
    write_schema::<ArtifactVersionsResponse>(out_dir, "ArtifactVersionsResponse")?;
    write_schema::<ArtifactReadResponse>(out_dir, "ArtifactReadResponse")?;
    write_schema::<ArtifactDeleteResponse>(out_dir, "ArtifactDeleteResponse")?;
    write_schema::<ArtifactDeniedResponse>(out_dir, "ArtifactDeniedResponse")?;
    write_schema::<ArtifactNeedsApprovalResponse>(out_dir, "ArtifactNeedsApprovalResponse")?;
    write_schema::<ArtifactModeDeniedResponse>(out_dir, "ArtifactModeDeniedResponse")?;
    write_schema::<ArtifactUnknownModeDeniedResponse>(
        out_dir,
        "ArtifactUnknownModeDeniedResponse",
    )?;
    write_schema::<ArtifactAllowedToolsDeniedResponse>(
        out_dir,
        "ArtifactAllowedToolsDeniedResponse",
    )?;
    write_schema::<RepoDeniedResponse>(out_dir, "RepoDeniedResponse")?;
    write_schema::<RepoNeedsApprovalResponse>(out_dir, "RepoNeedsApprovalResponse")?;
    write_schema::<RepoModeDeniedResponse>(out_dir, "RepoModeDeniedResponse")?;
    write_schema::<RepoUnknownModeDeniedResponse>(out_dir, "RepoUnknownModeDeniedResponse")?;
    write_schema::<RepoAllowedToolsDeniedResponse>(out_dir, "RepoAllowedToolsDeniedResponse")?;
    write_schema::<RepoSearchResponse>(out_dir, "RepoSearchResponse")?;
    write_schema::<RepoIndexResponse>(out_dir, "RepoIndexResponse")?;
    write_schema::<RepoSymbolsResponse>(out_dir, "RepoSymbolsResponse")?;
    write_schema::<McpDeniedResponse>(out_dir, "McpDeniedResponse")?;
    write_schema::<McpNeedsApprovalResponse>(out_dir, "McpNeedsApprovalResponse")?;
    write_schema::<McpModeDeniedResponse>(out_dir, "McpModeDeniedResponse")?;
    write_schema::<McpUnknownModeDeniedResponse>(out_dir, "McpUnknownModeDeniedResponse")?;
    write_schema::<McpAllowedToolsDeniedResponse>(out_dir, "McpAllowedToolsDeniedResponse")?;
    write_schema::<McpDisabledDeniedResponse>(out_dir, "McpDisabledDeniedResponse")?;
    write_schema::<McpSandboxPolicyDeniedResponse>(out_dir, "McpSandboxPolicyDeniedResponse")?;
    write_schema::<McpSandboxNetworkDeniedResponse>(out_dir, "McpSandboxNetworkDeniedResponse")?;
    write_schema::<McpExecPolicyDeniedResponse>(out_dir, "McpExecPolicyDeniedResponse")?;
    write_schema::<McpExecPolicyLoadDeniedResponse>(out_dir, "McpExecPolicyLoadDeniedResponse")?;
    write_schema::<McpFailedResponse>(out_dir, "McpFailedResponse")?;
    write_schema::<McpServerDescriptor>(out_dir, "McpServerDescriptor")?;
    write_schema::<McpListServersResponse>(out_dir, "McpListServersResponse")?;
    write_schema::<McpActionInlineResponse>(out_dir, "McpActionInlineResponse")?;
    write_schema::<McpActionArtifactResponse>(out_dir, "McpActionArtifactResponse")?;
    write_schema::<McpActionResponse>(out_dir, "McpActionResponse")?;
    write_schema::<ProcessStatus>(out_dir, "ProcessStatus")?;
    write_schema::<ProcessInfo>(out_dir, "ProcessInfo")?;
    write_schema::<ProcessListResponse>(out_dir, "ProcessListResponse")?;
    write_schema::<ProcessStartResponse>(out_dir, "ProcessStartResponse")?;
    write_schema::<ProcessInspectResponse>(out_dir, "ProcessInspectResponse")?;
    write_schema::<ProcessTailResponse>(out_dir, "ProcessTailResponse")?;
    write_schema::<ProcessFollowResponse>(out_dir, "ProcessFollowResponse")?;
    write_schema::<ProcessSignalResponse>(out_dir, "ProcessSignalResponse")?;
    write_schema::<ProcessDeniedResponse>(out_dir, "ProcessDeniedResponse")?;
    write_schema::<ProcessNeedsApprovalResponse>(out_dir, "ProcessNeedsApprovalResponse")?;
    write_schema::<ProcessModeDeniedResponse>(out_dir, "ProcessModeDeniedResponse")?;
    write_schema::<ProcessUnknownModeDeniedResponse>(out_dir, "ProcessUnknownModeDeniedResponse")?;
    write_schema::<ProcessAllowedToolsDeniedResponse>(
        out_dir,
        "ProcessAllowedToolsDeniedResponse",
    )?;
    write_schema::<ProcessSandboxPolicyDeniedResponse>(
        out_dir,
        "ProcessSandboxPolicyDeniedResponse",
    )?;
    write_schema::<ProcessSandboxNetworkDeniedResponse>(
        out_dir,
        "ProcessSandboxNetworkDeniedResponse",
    )?;
    write_schema::<ProcessExecPolicyDeniedResponse>(out_dir, "ProcessExecPolicyDeniedResponse")?;
    write_schema::<ProcessExecPolicyLoadDeniedResponse>(
        out_dir,
        "ProcessExecPolicyLoadDeniedResponse",
    )?;
    write_schema::<FileDeniedResponse>(out_dir, "FileDeniedResponse")?;
    write_schema::<FileNeedsApprovalResponse>(out_dir, "FileNeedsApprovalResponse")?;
    write_schema::<FileModeDeniedResponse>(out_dir, "FileModeDeniedResponse")?;
    write_schema::<FileUnknownModeDeniedResponse>(out_dir, "FileUnknownModeDeniedResponse")?;
    write_schema::<FileAllowedToolsDeniedResponse>(out_dir, "FileAllowedToolsDeniedResponse")?;
    write_schema::<FileSandboxPolicyDeniedResponse>(out_dir, "FileSandboxPolicyDeniedResponse")?;
    write_schema::<ApprovalDecideResponse>(out_dir, "ApprovalDecideResponse")?;
    write_schema::<ApprovalListResponse>(out_dir, "ApprovalListResponse")?;

    Ok(())
}

fn write_schema<T>(out_dir: &Path, name: &str) -> anyhow::Result<()>
where
    T: JsonSchema,
{
    let schema = schema_for!(T);
    let contents = serde_json::to_string_pretty(&schema)?;
    fs::write(out_dir.join(format!("{name}.schema.json")), contents)
        .with_context(|| format!("write schema {name}"))?;
    Ok(())
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn generate_ts_emits_thread_events_kind_enum_types() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        generate_ts(tmp.path()).expect("generate ts output");

        let thread_events_params =
            std::fs::read_to_string(tmp.path().join("ThreadEventsParams.ts"))
                .expect("read ThreadEventsParams.ts");
        assert!(
            thread_events_params.contains("import type { ThreadEventKindTag }"),
            "ThreadEventsParams.ts should import ThreadEventKindTag"
        );
        assert!(
            thread_events_params.contains("kinds?: Array<ThreadEventKindTag>"),
            "ThreadEventsParams.ts should type kinds as Array<ThreadEventKindTag>"
        );
        let thread_events_response =
            std::fs::read_to_string(tmp.path().join("ThreadEventsResponse.ts"))
                .expect("read ThreadEventsResponse.ts");
        assert!(
            thread_events_response.contains("events: Array<ThreadEvent>"),
            "ThreadEventsResponse.ts should type events as Array<ThreadEvent>"
        );
        let thread_subscribe_response =
            std::fs::read_to_string(tmp.path().join("ThreadSubscribeResponse.ts"))
                .expect("read ThreadSubscribeResponse.ts");
        assert!(
            thread_subscribe_response.contains("timed_out: boolean"),
            "ThreadSubscribeResponse.ts should include timed_out"
        );
        let thread_start_response =
            std::fs::read_to_string(tmp.path().join("ThreadStartResponse.ts"))
                .expect("read ThreadStartResponse.ts");
        assert!(
            thread_start_response.contains("auto_hook"),
            "ThreadStartResponse.ts should include auto_hook"
        );
        let thread_list_response =
            std::fs::read_to_string(tmp.path().join("ThreadListResponse.ts"))
                .expect("read ThreadListResponse.ts");
        assert!(
            thread_list_response.contains("threads: Array<ThreadId>"),
            "ThreadListResponse.ts should include typed thread ids"
        );
        let thread_archive_response =
            std::fs::read_to_string(tmp.path().join("ThreadArchiveResponse.ts"))
                .expect("read ThreadArchiveResponse.ts");
        assert!(
            thread_archive_response.contains("already_archived: boolean"),
            "ThreadArchiveResponse.ts should include already_archived"
        );
        let thread_list_meta_response =
            std::fs::read_to_string(tmp.path().join("ThreadListMetaResponse.ts"))
                .expect("read ThreadListMetaResponse.ts");
        assert!(
            thread_list_meta_response.contains("threads: Array<ThreadListMetaItem>"),
            "ThreadListMetaResponse.ts should type threads as Array<ThreadListMetaItem>"
        );
        let thread_list_meta_item =
            std::fs::read_to_string(tmp.path().join("ThreadListMetaItem.ts"))
                .expect("read ThreadListMetaItem.ts");
        assert!(
            thread_list_meta_item.contains("token_budget_limit?: number")
                || thread_list_meta_item.contains("token_budget_limit?: bigint")
                || thread_list_meta_item.contains("tokenBudgetLimit?: number")
                || thread_list_meta_item.contains("tokenBudgetLimit?: bigint"),
            "ThreadListMetaItem.ts should include optional token_budget_limit"
        );
        assert!(
            thread_list_meta_item.contains("token_budget_exceeded?: boolean")
                || thread_list_meta_item.contains("tokenBudgetExceeded?: boolean"),
            "ThreadListMetaItem.ts should include optional token_budget_exceeded"
        );
        assert!(
            thread_list_meta_item.contains("token_budget_warning_active?: boolean")
                || thread_list_meta_item.contains("tokenBudgetWarningActive?: boolean"),
            "ThreadListMetaItem.ts should include optional token_budget_warning_active"
        );
        let thread_attention_response =
            std::fs::read_to_string(tmp.path().join("ThreadAttentionResponse.ts"))
                .expect("read ThreadAttentionResponse.ts");
        assert!(
            thread_attention_response.contains("attention_markers: ThreadAttentionMarkers"),
            "ThreadAttentionResponse.ts should include typed attention_markers"
        );
        assert!(
            thread_attention_response.contains("token_budget_limit?: number")
                || thread_attention_response.contains("token_budget_limit?: bigint")
                || thread_attention_response.contains("tokenBudgetLimit?: number")
                || thread_attention_response.contains("tokenBudgetLimit?: bigint"),
            "ThreadAttentionResponse.ts should include optional token_budget_limit"
        );
        assert!(
            thread_attention_response.contains("token_budget_exceeded?: boolean")
                || thread_attention_response.contains("tokenBudgetExceeded?: boolean"),
            "ThreadAttentionResponse.ts should include optional token_budget_exceeded"
        );
        assert!(
            thread_attention_response.contains("token_budget_warning_active?: boolean")
                || thread_attention_response.contains("tokenBudgetWarningActive?: boolean"),
            "ThreadAttentionResponse.ts should include optional token_budget_warning_active"
        );
        let thread_state_response =
            std::fs::read_to_string(tmp.path().join("ThreadStateResponse.ts"))
                .expect("read ThreadStateResponse.ts");
        assert!(
            thread_state_response.contains("total_tokens_used: number")
                || thread_state_response.contains("total_tokens_used: bigint")
                || thread_state_response.contains("totalTokensUsed: number")
                || thread_state_response.contains("totalTokensUsed: bigint"),
            "ThreadStateResponse.ts should include total_tokens_used"
        );
        assert!(
            thread_state_response.contains("input_tokens_used: number")
                || thread_state_response.contains("input_tokens_used: bigint")
                || thread_state_response.contains("inputTokensUsed: number")
                || thread_state_response.contains("inputTokensUsed: bigint"),
            "ThreadStateResponse.ts should include input_tokens_used"
        );
        assert!(
            thread_state_response.contains("output_tokens_used: number")
                || thread_state_response.contains("output_tokens_used: bigint")
                || thread_state_response.contains("outputTokensUsed: number")
                || thread_state_response.contains("outputTokensUsed: bigint"),
            "ThreadStateResponse.ts should include output_tokens_used"
        );
        assert!(
            thread_state_response.contains("cache_input_tokens_used: number")
                || thread_state_response.contains("cache_input_tokens_used: bigint")
                || thread_state_response.contains("cacheInputTokensUsed: number")
                || thread_state_response.contains("cacheInputTokensUsed: bigint"),
            "ThreadStateResponse.ts should include cache_input_tokens_used"
        );
        assert!(
            thread_state_response.contains("cache_creation_input_tokens_used: number")
                || thread_state_response.contains("cache_creation_input_tokens_used: bigint")
                || thread_state_response.contains("cacheCreationInputTokensUsed: number")
                || thread_state_response.contains("cacheCreationInputTokensUsed: bigint"),
            "ThreadStateResponse.ts should include cache_creation_input_tokens_used"
        );
        let thread_usage_response =
            std::fs::read_to_string(tmp.path().join("ThreadUsageResponse.ts"))
                .expect("read ThreadUsageResponse.ts");
        assert!(
            thread_usage_response.contains("cache_input_ratio?: number")
                || thread_usage_response.contains("cacheInputRatio?: number"),
            "ThreadUsageResponse.ts should include optional cache_input_ratio"
        );
        assert!(
            thread_usage_response.contains("non_cache_input_tokens_used: number")
                || thread_usage_response.contains("non_cache_input_tokens_used: bigint")
                || thread_usage_response.contains("nonCacheInputTokensUsed: number")
                || thread_usage_response.contains("nonCacheInputTokensUsed: bigint"),
            "ThreadUsageResponse.ts should include non_cache_input_tokens_used"
        );
        assert!(
            thread_usage_response.contains("token_budget_limit?: number")
                || thread_usage_response.contains("token_budget_limit?: bigint")
                || thread_usage_response.contains("tokenBudgetLimit?: number")
                || thread_usage_response.contains("tokenBudgetLimit?: bigint"),
            "ThreadUsageResponse.ts should include optional token_budget_limit"
        );
        assert!(
            thread_usage_response.contains("token_budget_utilization?: number")
                || thread_usage_response.contains("tokenBudgetUtilization?: number"),
            "ThreadUsageResponse.ts should include optional token_budget_utilization"
        );
        assert!(
            thread_usage_response.contains("token_budget_exceeded?: boolean")
                || thread_usage_response.contains("tokenBudgetExceeded?: boolean"),
            "ThreadUsageResponse.ts should include optional token_budget_exceeded"
        );
        assert!(
            thread_usage_response.contains("token_budget_warning_active?: boolean")
                || thread_usage_response.contains("tokenBudgetWarningActive?: boolean"),
            "ThreadUsageResponse.ts should include optional token_budget_warning_active"
        );
        let thread_models_response =
            std::fs::read_to_string(tmp.path().join("ThreadModelsResponse.ts"))
                .expect("read ThreadModelsResponse.ts");
        assert!(
            thread_models_response.contains("models: Array<string>"),
            "ThreadModelsResponse.ts should type models as Array<string>"
        );

        let thread_event_kind_tag =
            std::fs::read_to_string(tmp.path().join("ThreadEventKindTag.ts"))
                .expect("read ThreadEventKindTag.ts");
        for expected in omne_protocol::THREAD_EVENT_KIND_TAGS {
            let needle = format!("\"{expected}\"");
            assert!(
                thread_event_kind_tag.contains(&needle),
                "ThreadEventKindTag.ts missing enum value: {expected}"
            );
        }
    }

    #[test]
    fn generate_ts_emits_artifact_read_response_types() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        generate_ts(tmp.path()).expect("generate ts output");

        let artifact_read_response =
            std::fs::read_to_string(tmp.path().join("ArtifactReadResponse.ts"))
                .expect("read ArtifactReadResponse.ts");
        assert!(
            artifact_read_response.contains("metadata_source: ArtifactReadMetadataSource"),
            "ArtifactReadResponse.ts should use ArtifactReadMetadataSource"
        );
        assert!(
            artifact_read_response
                .contains("metadata_fallback_reason?: ArtifactReadMetadataFallbackReason"),
            "ArtifactReadResponse.ts should include optional metadata_fallback_reason"
        );
        assert!(
            artifact_read_response.contains("fan_in_summary?: ArtifactFanInSummaryStructuredData"),
            "ArtifactReadResponse.ts should include optional fan_in_summary structured payload"
        );
        assert!(
            artifact_read_response
                .contains("fan_out_linkage_issue?: ArtifactFanOutLinkageIssueStructuredData"),
            "ArtifactReadResponse.ts should include optional fan_out_linkage_issue structured payload"
        );
        assert!(
            artifact_read_response.contains(
                "fan_out_linkage_issue_clear?: ArtifactFanOutLinkageIssueClearStructuredData"
            ),
            "ArtifactReadResponse.ts should include optional fan_out_linkage_issue_clear structured payload"
        );
        assert!(
            artifact_read_response.contains("fan_out_result?: ArtifactFanOutResultStructuredData"),
            "ArtifactReadResponse.ts should include optional fan_out_result structured payload"
        );

        let artifact_list_response =
            std::fs::read_to_string(tmp.path().join("ArtifactListResponse.ts"))
                .expect("read ArtifactListResponse.ts");
        assert!(
            artifact_list_response.contains("errors: Array<ArtifactListError>"),
            "ArtifactListResponse.ts should expose typed errors"
        );

        let artifact_versions_response =
            std::fs::read_to_string(tmp.path().join("ArtifactVersionsResponse.ts"))
                .expect("read ArtifactVersionsResponse.ts");
        assert!(
            artifact_versions_response.contains("latest_version: number"),
            "ArtifactVersionsResponse.ts should expose latest_version as required number"
        );

        let artifact_denied_response =
            std::fs::read_to_string(tmp.path().join("ArtifactDeniedResponse.ts"))
                .expect("read ArtifactDeniedResponse.ts");
        assert!(
            artifact_denied_response.contains("denied: boolean"),
            "ArtifactDeniedResponse.ts should expose denied as required boolean"
        );
        assert!(
            artifact_denied_response.contains("remembered?: boolean"),
            "ArtifactDeniedResponse.ts should expose optional remembered"
        );

        let artifact_mode_denied_response =
            std::fs::read_to_string(tmp.path().join("ArtifactModeDeniedResponse.ts"))
                .expect("read ArtifactModeDeniedResponse.ts");
        assert!(
            artifact_mode_denied_response.contains("decision_source: string"),
            "ArtifactModeDeniedResponse.ts should expose decision_source as string"
        );

        let artifact_allowed_tools_denied_response =
            std::fs::read_to_string(tmp.path().join("ArtifactAllowedToolsDeniedResponse.ts"))
                .expect("read ArtifactAllowedToolsDeniedResponse.ts");
        assert!(
            artifact_allowed_tools_denied_response.contains("allowed_tools: Array<string>"),
            "ArtifactAllowedToolsDeniedResponse.ts should expose allowed_tools as string array"
        );

        let repo_needs_approval_response =
            std::fs::read_to_string(tmp.path().join("RepoNeedsApprovalResponse.ts"))
                .expect("read RepoNeedsApprovalResponse.ts");
        assert!(
            repo_needs_approval_response.contains("needs_approval: boolean"),
            "RepoNeedsApprovalResponse.ts should expose needs_approval as required boolean"
        );

        let repo_mode_denied_response =
            std::fs::read_to_string(tmp.path().join("RepoModeDeniedResponse.ts"))
                .expect("read RepoModeDeniedResponse.ts");
        assert!(
            repo_mode_denied_response.contains("decision_source: string"),
            "RepoModeDeniedResponse.ts should expose decision_source as string"
        );

        let repo_allowed_tools_denied_response =
            std::fs::read_to_string(tmp.path().join("RepoAllowedToolsDeniedResponse.ts"))
                .expect("read RepoAllowedToolsDeniedResponse.ts");
        assert!(
            repo_allowed_tools_denied_response.contains("allowed_tools: Array<string>"),
            "RepoAllowedToolsDeniedResponse.ts should expose allowed_tools as string array"
        );

        let mcp_needs_approval_response =
            std::fs::read_to_string(tmp.path().join("McpNeedsApprovalResponse.ts"))
                .expect("read McpNeedsApprovalResponse.ts");
        assert!(
            mcp_needs_approval_response.contains("needs_approval: boolean"),
            "McpNeedsApprovalResponse.ts should expose needs_approval as required boolean"
        );

        let mcp_mode_denied_response =
            std::fs::read_to_string(tmp.path().join("McpModeDeniedResponse.ts"))
                .expect("read McpModeDeniedResponse.ts");
        assert!(
            mcp_mode_denied_response.contains("decision_source: string"),
            "McpModeDeniedResponse.ts should expose decision_source as string"
        );

        let process_needs_approval_response =
            std::fs::read_to_string(tmp.path().join("ProcessNeedsApprovalResponse.ts"))
                .expect("read ProcessNeedsApprovalResponse.ts");
        assert!(
            process_needs_approval_response.contains("needs_approval: boolean"),
            "ProcessNeedsApprovalResponse.ts should expose needs_approval as required boolean"
        );
        assert!(
            process_needs_approval_response.contains("thread_id: ThreadId"),
            "ProcessNeedsApprovalResponse.ts should expose thread_id"
        );

        let process_mode_denied_response =
            std::fs::read_to_string(tmp.path().join("ProcessModeDeniedResponse.ts"))
                .expect("read ProcessModeDeniedResponse.ts");
        assert!(
            process_mode_denied_response.contains("decision_source: string"),
            "ProcessModeDeniedResponse.ts should expose decision_source as string"
        );

        let process_allowed_tools_denied_response =
            std::fs::read_to_string(tmp.path().join("ProcessAllowedToolsDeniedResponse.ts"))
                .expect("read ProcessAllowedToolsDeniedResponse.ts");
        assert!(
            process_allowed_tools_denied_response.contains("allowed_tools: Array<string>"),
            "ProcessAllowedToolsDeniedResponse.ts should expose allowed_tools as string array"
        );

        let process_sandbox_network_denied_response =
            std::fs::read_to_string(tmp.path().join("ProcessSandboxNetworkDeniedResponse.ts"))
                .expect("read ProcessSandboxNetworkDeniedResponse.ts");
        assert!(
            process_sandbox_network_denied_response
                .contains("sandbox_network_access: SandboxNetworkAccess"),
            "ProcessSandboxNetworkDeniedResponse.ts should expose sandbox_network_access"
        );

        let process_execpolicy_load_denied_response =
            std::fs::read_to_string(tmp.path().join("ProcessExecPolicyLoadDeniedResponse.ts"))
                .expect("read ProcessExecPolicyLoadDeniedResponse.ts");
        assert!(
            process_execpolicy_load_denied_response.contains("details: string"),
            "ProcessExecPolicyLoadDeniedResponse.ts should expose details as required string"
        );

        let file_needs_approval_response =
            std::fs::read_to_string(tmp.path().join("FileNeedsApprovalResponse.ts"))
                .expect("read FileNeedsApprovalResponse.ts");
        assert!(
            file_needs_approval_response.contains("needs_approval: boolean"),
            "FileNeedsApprovalResponse.ts should expose needs_approval as required boolean"
        );

        let file_mode_denied_response =
            std::fs::read_to_string(tmp.path().join("FileModeDeniedResponse.ts"))
                .expect("read FileModeDeniedResponse.ts");
        assert!(
            file_mode_denied_response.contains("decision_source: string"),
            "FileModeDeniedResponse.ts should expose decision_source as string"
        );

        let file_allowed_tools_denied_response =
            std::fs::read_to_string(tmp.path().join("FileAllowedToolsDeniedResponse.ts"))
                .expect("read FileAllowedToolsDeniedResponse.ts");
        assert!(
            file_allowed_tools_denied_response.contains("allowed_tools: Array<string>"),
            "FileAllowedToolsDeniedResponse.ts should expose allowed_tools as string array"
        );

        let file_sandbox_policy_denied_response =
            std::fs::read_to_string(tmp.path().join("FileSandboxPolicyDeniedResponse.ts"))
                .expect("read FileSandboxPolicyDeniedResponse.ts");
        assert!(
            file_sandbox_policy_denied_response.contains("sandbox_policy: SandboxPolicy"),
            "FileSandboxPolicyDeniedResponse.ts should expose sandbox_policy"
        );

        let approval_decide_response =
            std::fs::read_to_string(tmp.path().join("ApprovalDecideResponse.ts"))
                .expect("read ApprovalDecideResponse.ts");
        assert!(
            approval_decide_response.contains("ok: boolean"),
            "ApprovalDecideResponse.ts should expose ok as required boolean"
        );
        assert!(
            approval_decide_response.contains("forwarded?: boolean"),
            "ApprovalDecideResponse.ts should expose optional forwarded"
        );
        assert!(
            approval_decide_response.contains("child_thread_id?: ThreadId"),
            "ApprovalDecideResponse.ts should expose optional child_thread_id"
        );
        assert!(
            approval_decide_response.contains("child_approval_id?: ApprovalId"),
            "ApprovalDecideResponse.ts should expose optional child_approval_id"
        );

        let approval_list_response =
            std::fs::read_to_string(tmp.path().join("ApprovalListResponse.ts"))
                .expect("read ApprovalListResponse.ts");
        assert!(
            approval_list_response.contains("approvals: Array<ApprovalListItem>"),
            "ApprovalListResponse.ts should expose approvals as ApprovalListItem array"
        );
    }

    #[test]
    fn generate_ts_emits_turn_and_empty_thread_param_types() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        generate_ts(tmp.path()).expect("generate ts output");

        let turn_start_response = std::fs::read_to_string(tmp.path().join("TurnStartResponse.ts"))
            .expect("read TurnStartResponse.ts");
        assert!(
            turn_start_response.contains("turn_id: TurnId"),
            "TurnStartResponse.ts should expose typed turn_id"
        );

        let turn_interrupt_response =
            std::fs::read_to_string(tmp.path().join("TurnInterruptResponse.ts"))
                .expect("read TurnInterruptResponse.ts");
        assert!(
            turn_interrupt_response.contains("ok: boolean"),
            "TurnInterruptResponse.ts should expose ok as required boolean"
        );

        let thread_list_params = std::fs::read_to_string(tmp.path().join("ThreadListParams.ts"))
            .expect("read ThreadListParams.ts");
        assert!(
            thread_list_params.contains("Record<string, never>"),
            "ThreadListParams.ts should encode an empty object type"
        );

        let thread_loaded_params =
            std::fs::read_to_string(tmp.path().join("ThreadLoadedParams.ts"))
                .expect("read ThreadLoadedParams.ts");
        assert!(
            thread_loaded_params.contains("Record<string, never>"),
            "ThreadLoadedParams.ts should encode an empty object type"
        );

        let client_request = std::fs::read_to_string(tmp.path().join("ClientRequest.ts"))
            .expect("read ClientRequest.ts");
        assert!(
            client_request
                .contains("\"method\": \"thread/list\", id: RequestId, params: ThreadListParams",),
            "ClientRequest.ts should type thread/list params as ThreadListParams"
        );
        assert!(
            client_request.contains(
                "\"method\": \"thread/loaded\", id: RequestId, params: ThreadLoadedParams",
            ),
            "ClientRequest.ts should type thread/loaded params as ThreadLoadedParams"
        );
        assert!(
            client_request.contains(
                "\"method\": \"thread/usage\", id: RequestId, params: ThreadUsageParams",
            ),
            "ClientRequest.ts should type thread/usage params as ThreadUsageParams"
        );
    }
}
