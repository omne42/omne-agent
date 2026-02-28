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

macro_rules! for_each_export_type {
    ($m:ident) => {
        $m!(RequestId, "RequestId");
        $m!(JsonRpcRequest, "JsonRpcRequest");
        $m!(JsonRpcResponse, "JsonRpcResponse");
        $m!(JsonRpcErrorResponse, "JsonRpcErrorResponse");
        $m!(JsonRpcError, "JsonRpcError");
        $m!(ClientRequest, "ClientRequest");
        $m!(ServerNotification, "ServerNotification");
        $m!(omne_protocol::ThreadEvent, "ThreadEvent");
        $m!(ThreadEventsResponse, "ThreadEventsResponse");
        $m!(ThreadSubscribeResponse, "ThreadSubscribeResponse");
        $m!(ThreadHandleResponse, "ThreadHandleResponse");
        $m!(ThreadStartResponse, "ThreadStartResponse");
        $m!(TurnStartResponse, "TurnStartResponse");
        $m!(TurnInterruptResponse, "TurnInterruptResponse");
        $m!(ThreadListResponse, "ThreadListResponse");
        $m!(ThreadArchiveResponse, "ThreadArchiveResponse");
        $m!(ThreadUnarchiveResponse, "ThreadUnarchiveResponse");
        $m!(ThreadPauseResponse, "ThreadPauseResponse");
        $m!(ThreadUnpauseResponse, "ThreadUnpauseResponse");
        $m!(ThreadDeleteResponse, "ThreadDeleteResponse");
        $m!(ThreadClearArtifactsResponse, "ThreadClearArtifactsResponse");
        $m!(ThreadListMetaResponse, "ThreadListMetaResponse");
        $m!(ThreadAttentionResponse, "ThreadAttentionResponse");
        $m!(ThreadStateResponse, "ThreadStateResponse");
        $m!(ThreadUsageResponse, "ThreadUsageResponse");
        $m!(ThreadConfigureResponse, "ThreadConfigureResponse");
        $m!(ThreadConfigExplainResponse, "ThreadConfigExplainResponse");
        $m!(ThreadModelsResponse, "ThreadModelsResponse");
        $m!(ThreadDiskUsageResponse, "ThreadDiskUsageResponse");
        $m!(ThreadDiskReportResponse, "ThreadDiskReportResponse");
        $m!(
            ThreadCheckpointRestoreDeniedResponse,
            "ThreadCheckpointRestoreDeniedResponse"
        );
        $m!(
            ThreadCheckpointRestoreNeedsApprovalResponse,
            "ThreadCheckpointRestoreNeedsApprovalResponse"
        );
        $m!(
            ThreadCheckpointRestoreResponse,
            "ThreadCheckpointRestoreResponse"
        );
        $m!(
            ThreadCheckpointCreateResponse,
            "ThreadCheckpointCreateResponse"
        );
        $m!(ThreadCheckpointListResponse, "ThreadCheckpointListResponse");
        $m!(ThreadGitSnapshotResponse, "ThreadGitSnapshotResponse");
        $m!(
            ThreadGitSnapshotNeedsApprovalResponse,
            "ThreadGitSnapshotNeedsApprovalResponse"
        );
        $m!(
            ThreadGitSnapshotDeniedResponse,
            "ThreadGitSnapshotDeniedResponse"
        );
        $m!(
            ThreadGitSnapshotTimedOutResponse,
            "ThreadGitSnapshotTimedOutResponse"
        );
        $m!(ThreadGitSnapshotRpcResponse, "ThreadGitSnapshotRpcResponse");
        $m!(ThreadHookRunResponse, "ThreadHookRunResponse");
        $m!(
            ThreadHookRunNeedsApprovalResponse,
            "ThreadHookRunNeedsApprovalResponse"
        );
        $m!(ThreadHookRunDeniedResponse, "ThreadHookRunDeniedResponse");
        $m!(ThreadHookRunErrorResponse, "ThreadHookRunErrorResponse");
        $m!(ThreadHookRunRpcResponse, "ThreadHookRunRpcResponse");
        $m!(ThreadAutoHookResponse, "ThreadAutoHookResponse");
        $m!(ArtifactListResponse, "ArtifactListResponse");
        $m!(ArtifactVersionsResponse, "ArtifactVersionsResponse");
        $m!(ArtifactReadResponse, "ArtifactReadResponse");
        $m!(ArtifactDeleteResponse, "ArtifactDeleteResponse");
        $m!(ArtifactDeniedResponse, "ArtifactDeniedResponse");
        $m!(
            ArtifactNeedsApprovalResponse,
            "ArtifactNeedsApprovalResponse"
        );
        $m!(ArtifactModeDeniedResponse, "ArtifactModeDeniedResponse");
        $m!(
            ArtifactUnknownModeDeniedResponse,
            "ArtifactUnknownModeDeniedResponse"
        );
        $m!(
            ArtifactAllowedToolsDeniedResponse,
            "ArtifactAllowedToolsDeniedResponse"
        );
        $m!(RepoDeniedResponse, "RepoDeniedResponse");
        $m!(RepoNeedsApprovalResponse, "RepoNeedsApprovalResponse");
        $m!(RepoModeDeniedResponse, "RepoModeDeniedResponse");
        $m!(
            RepoUnknownModeDeniedResponse,
            "RepoUnknownModeDeniedResponse"
        );
        $m!(
            RepoAllowedToolsDeniedResponse,
            "RepoAllowedToolsDeniedResponse"
        );
        $m!(RepoSearchResponse, "RepoSearchResponse");
        $m!(RepoIndexResponse, "RepoIndexResponse");
        $m!(RepoSymbolsResponse, "RepoSymbolsResponse");
        $m!(McpDeniedResponse, "McpDeniedResponse");
        $m!(McpNeedsApprovalResponse, "McpNeedsApprovalResponse");
        $m!(McpModeDeniedResponse, "McpModeDeniedResponse");
        $m!(McpUnknownModeDeniedResponse, "McpUnknownModeDeniedResponse");
        $m!(
            McpAllowedToolsDeniedResponse,
            "McpAllowedToolsDeniedResponse"
        );
        $m!(McpDisabledDeniedResponse, "McpDisabledDeniedResponse");
        $m!(
            McpSandboxPolicyDeniedResponse,
            "McpSandboxPolicyDeniedResponse"
        );
        $m!(
            McpSandboxNetworkDeniedResponse,
            "McpSandboxNetworkDeniedResponse"
        );
        $m!(McpExecPolicyDeniedResponse, "McpExecPolicyDeniedResponse");
        $m!(
            McpExecPolicyLoadDeniedResponse,
            "McpExecPolicyLoadDeniedResponse"
        );
        $m!(McpFailedResponse, "McpFailedResponse");
        $m!(McpServerDescriptor, "McpServerDescriptor");
        $m!(McpListServersResponse, "McpListServersResponse");
        $m!(McpActionInlineResponse, "McpActionInlineResponse");
        $m!(McpActionArtifactResponse, "McpActionArtifactResponse");
        $m!(McpActionResponse, "McpActionResponse");
        $m!(ProcessStatus, "ProcessStatus");
        $m!(ProcessInfo, "ProcessInfo");
        $m!(ProcessListResponse, "ProcessListResponse");
        $m!(ProcessStartResponse, "ProcessStartResponse");
        $m!(ProcessInspectResponse, "ProcessInspectResponse");
        $m!(ProcessTailResponse, "ProcessTailResponse");
        $m!(ProcessFollowResponse, "ProcessFollowResponse");
        $m!(ProcessSignalResponse, "ProcessSignalResponse");
        $m!(ProcessDeniedResponse, "ProcessDeniedResponse");
        $m!(ProcessNeedsApprovalResponse, "ProcessNeedsApprovalResponse");
        $m!(ProcessModeDeniedResponse, "ProcessModeDeniedResponse");
        $m!(
            ProcessUnknownModeDeniedResponse,
            "ProcessUnknownModeDeniedResponse"
        );
        $m!(
            ProcessAllowedToolsDeniedResponse,
            "ProcessAllowedToolsDeniedResponse"
        );
        $m!(
            ProcessSandboxPolicyDeniedResponse,
            "ProcessSandboxPolicyDeniedResponse"
        );
        $m!(
            ProcessSandboxNetworkDeniedResponse,
            "ProcessSandboxNetworkDeniedResponse"
        );
        $m!(
            ProcessExecPolicyDeniedResponse,
            "ProcessExecPolicyDeniedResponse"
        );
        $m!(
            ProcessExecPolicyLoadDeniedResponse,
            "ProcessExecPolicyLoadDeniedResponse"
        );
        $m!(FileDeniedResponse, "FileDeniedResponse");
        $m!(FileNeedsApprovalResponse, "FileNeedsApprovalResponse");
        $m!(FileModeDeniedResponse, "FileModeDeniedResponse");
        $m!(
            FileUnknownModeDeniedResponse,
            "FileUnknownModeDeniedResponse"
        );
        $m!(
            FileAllowedToolsDeniedResponse,
            "FileAllowedToolsDeniedResponse"
        );
        $m!(
            FileSandboxPolicyDeniedResponse,
            "FileSandboxPolicyDeniedResponse"
        );
        $m!(ApprovalDecideResponse, "ApprovalDecideResponse");
        $m!(ApprovalListResponse, "ApprovalListResponse");
    };
}

pub fn generate_ts(out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create out dir {}", out_dir.display()))?;

    macro_rules! export_ts_type {
        ($ty:ty, $name:literal) => {
            <$ty>::export_all_to(out_dir).context(concat!("export ", $name, " typescript"))?;
        };
    }
    for_each_export_type!(export_ts_type);

    Ok(())
}

pub fn generate_json_schema(out_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create out dir {}", out_dir.display()))?;

    macro_rules! write_schema_type {
        ($ty:ty, $name:literal) => {
            write_schema::<$ty>(out_dir, $name)?;
        };
    }
    for_each_export_type!(write_schema_type);

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
