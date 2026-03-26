use super::*;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactWriteParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub artifact_id: Option<omne_protocol::ArtifactId>,
    pub artifact_type: String,
    pub summary: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactListParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactReadParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub artifact_id: omne_protocol::ArtifactId,
    #[serde(default)]
    #[ts(optional)]
    pub version: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactVersionsParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub artifact_id: omne_protocol::ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactDeleteParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub artifact_id: omne_protocol::ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactPruneReportVersionDetail {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactPruneReportReadPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source_artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub retained_history_versions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pruned_count: Option<usize>,
    #[serde(default)]
    pub pruned_version_details: Vec<ArtifactPruneReportVersionDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanInSummaryPendingApproval {
    pub approval_id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub approve_cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deny_cmd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanInSummaryResultArtifactDiagnostics {
    pub scan_last_seq: u64,
    pub matched_completion_count: u64,
    pub pending_matching_tool_ids: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanInSummaryTask {
    pub task_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    #[serde(default)]
    pub dependency_blocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dependency_blocker_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dependency_blocker_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result_artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result_artifact_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result_artifact_structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result_artifact_error_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result_artifact_diagnostics: Option<ArtifactFanInSummaryResultArtifactDiagnostics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pending_approval: Option<ArtifactFanInSummaryPendingApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanInSummaryScheduling {
    pub env_max_concurrent_subagents: usize,
    pub effective_concurrency_limit: usize,
    pub priority_aging_rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanInSummaryStructuredData {
    pub schema_version: String,
    pub thread_id: String,
    pub task_count: usize,
    pub scheduling: ArtifactFanInSummaryScheduling,
    #[serde(default)]
    pub tasks: Vec<ArtifactFanInSummaryTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutLinkageIssueStructuredData {
    pub schema_version: String,
    pub fan_in_summary_artifact_id: String,
    pub issue: String,
    #[serde(default)]
    pub issue_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutLinkageIssueClearStructuredData {
    pub schema_version: String,
    pub fan_in_summary_artifact_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutResultIsolatedWritePatchStructuredData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub artifact_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub read_cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutResultIsolatedWriteHandoffStructuredData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_cwd: Option<String>,
    #[serde(default)]
    pub status_argv: Vec<String>,
    #[serde(default)]
    pub diff_argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub apply_patch_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub patch: Option<ArtifactFanOutResultIsolatedWritePatchStructuredData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage {
    Precondition,
    CapturePatch,
    CheckPatch,
    ApplyPatch,
    #[serde(other)]
    Unknown,
}

impl ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Precondition => "precondition",
            Self::CapturePatch => "capture_patch",
            Self::CheckPatch => "check_patch",
            Self::ApplyPatch => "apply_patch",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub attempted: bool,
    #[serde(default)]
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_workspace_cwd: Option<String>,
    #[serde(default)]
    pub check_argv: Vec<String>,
    #[serde(default)]
    pub apply_argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub patch_artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub patch_read_cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub failure_stage: Option<ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recovery_hint: Option<String>,
    #[serde(default)]
    pub recovery_commands: Vec<ArtifactFanOutResultRecoveryCommandStructuredData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutResultRecoveryCommandStructuredData {
    pub label: String,
    #[serde(default)]
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactFanOutResultStructuredData {
    pub schema_version: String,
    pub task_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub workspace_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub isolated_write_patch: Option<ArtifactFanOutResultIsolatedWritePatchStructuredData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub isolated_write_handoff: Option<ArtifactFanOutResultIsolatedWriteHandoffStructuredData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub isolated_write_auto_apply: Option<ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactReadResponse {
    pub tool_id: omne_protocol::ToolId,
    pub metadata: omne_protocol::ArtifactMetadata,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub bytes: u64,
    pub version: u32,
    pub latest_version: u32,
    #[serde(default)]
    pub historical: bool,
    pub metadata_source: omne_protocol::ArtifactReadMetadataSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub metadata_fallback_reason: Option<omne_protocol::ArtifactReadMetadataFallbackReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prune_report: Option<ArtifactPruneReportReadPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_in_summary: Option<ArtifactFanInSummaryStructuredData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_linkage_issue: Option<ArtifactFanOutLinkageIssueStructuredData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_linkage_issue_clear: Option<ArtifactFanOutLinkageIssueClearStructuredData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_result: Option<ArtifactFanOutResultStructuredData>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactListError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactListResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub artifacts: Vec<omne_protocol::ArtifactMetadata>,
    #[serde(default)]
    pub errors: Vec<ArtifactListError>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactVersionsResponse {
    pub tool_id: omne_protocol::ToolId,
    pub artifact_id: omne_protocol::ArtifactId,
    pub latest_version: u32,
    #[serde(default)]
    pub versions: Vec<u32>,
    #[serde(default)]
    pub history_versions: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactDeleteResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactWriteHistory {
    pub max_versions: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snapshotted_version: Option<u32>,
    #[serde(default)]
    pub pruned_versions: Vec<u32>,
    #[serde(default)]
    pub pruned_version_details: Vec<ArtifactPruneReportVersionDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prune_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prune_report_artifact_id: Option<omne_protocol::ArtifactId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prune_report_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactWriteResponse {
    pub tool_id: omne_protocol::ToolId,
    pub artifact_id: omne_protocol::ArtifactId,
    #[serde(default)]
    pub created: bool,
    pub content_path: String,
    pub metadata_path: String,
    pub metadata: omne_protocol::ArtifactMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub history: Option<ArtifactWriteHistory>,
}

define_tool_denied_response_skip_none!(ArtifactDeniedResponse {});

define_tool_needs_approval_response!(ArtifactNeedsApprovalResponse {
    pub thread_id: omne_protocol::ThreadId,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactModeDecision {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    pub mode: String,
    pub decision: ArtifactModeDecision,
    pub decision_source: String,
    #[serde(default)]
    pub tool_override_hit: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactUnknownModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    pub mode: String,
    pub decision: ArtifactModeDecision,
    pub available: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub load_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactAllowedToolsDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}
