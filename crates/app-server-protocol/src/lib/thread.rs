use super::*;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStartParams {
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadResumeParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadForkParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadHandleResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub log_path: String,
    pub last_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStartResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub log_path: String,
    pub last_seq: u64,
    pub auto_hook: ThreadAutoHookResponse,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListResponse {
    #[serde(default)]
    pub threads: Vec<omne_protocol::ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListParams {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadLoadedParams {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadArchiveParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadArchiveResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub archived: bool,
    pub already_archived: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub force: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub killed_processes: Option<Vec<omne_protocol::ProcessId>>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub auto_hook: Option<ThreadAutoHookResponse>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnarchiveParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnarchiveResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub archived: bool,
    pub already_unarchived: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadPauseParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadPauseResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub paused: bool,
    pub already_paused: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub interrupted_turn_id: Option<omne_protocol::TurnId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnpauseParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnpauseResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub paused: bool,
    pub already_unpaused: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDeleteParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDeleteResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub deleted: bool,
    pub thread_dir: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadClearArtifactsParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadClearArtifactsResponse {
    pub tool_id: omne_protocol::ToolId,
    pub removed: bool,
    pub artifacts_dir: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStateParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStateResponse {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
    pub archived: bool,
    #[serde(default)]
    #[ts(optional)]
    pub archived_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub archived_reason: Option<String>,
    pub paused: bool,
    #[serde(default)]
    #[ts(optional)]
    pub paused_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub paused_reason: Option<String>,
    pub approval_policy: omne_protocol::ApprovalPolicy,
    pub sandbox_policy: omne_protocol::SandboxPolicy,
    #[serde(default)]
    pub sandbox_writable_roots: Vec<String>,
    pub sandbox_network_access: omne_protocol::SandboxNetworkAccess,
    pub mode: String,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub openai_base_url: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub allowed_tools: Option<Vec<String>>,
    pub last_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub active_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    pub active_turn_interrupt_requested: bool,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_status: Option<omne_protocol::TurnStatus>,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_reason: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_remaining: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_utilization: Option<f64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_exceeded: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_warning_active: Option<bool>,
    pub total_tokens_used: u64,
    #[serde(default)]
    pub input_tokens_used: u64,
    #[serde(default)]
    pub output_tokens_used: u64,
    #[serde(default)]
    pub cache_input_tokens_used: u64,
    #[serde(default)]
    pub cache_creation_input_tokens_used: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUsageParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUsageResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub last_seq: u64,
    pub total_tokens_used: u64,
    #[serde(default)]
    pub input_tokens_used: u64,
    #[serde(default)]
    pub output_tokens_used: u64,
    #[serde(default)]
    pub cache_input_tokens_used: u64,
    #[serde(default)]
    pub cache_creation_input_tokens_used: u64,
    #[serde(default)]
    pub non_cache_input_tokens_used: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cache_input_ratio: Option<f64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub output_ratio: Option<f64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_remaining: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_utilization: Option<f64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_exceeded: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_warning_active: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionArtifactMarker {
    pub set_at: String,
    pub artifact_id: omne_protocol::ArtifactId,
    pub artifact_type: String,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionTestFailedMarker {
    pub set_at: String,
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    #[ts(optional)]
    pub command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionStateMarker {
    pub set_at: String,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionMarkers {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub plan_ready: Option<ThreadAttentionArtifactMarker>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diff_ready: Option<ThreadAttentionArtifactMarker>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_linkage_issue: Option<ThreadAttentionArtifactMarker>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_auto_apply_error: Option<ThreadAttentionArtifactMarker>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub test_failed: Option<ThreadAttentionTestFailedMarker>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_warning: Option<ThreadAttentionStateMarker>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_exceeded: Option<ThreadAttentionStateMarker>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionPendingApproval {
    pub approval_id: omne_protocol::ApprovalId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub action: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub action_id: Option<ThreadApprovalActionId>,
    #[serde(default)]
    #[ts(optional)]
    pub params: Option<serde_json::Value>,
    #[serde(default)]
    #[ts(optional)]
    pub summary: Option<ThreadAttentionPendingApprovalSummary>,
    #[serde(default)]
    #[ts(optional)]
    pub requested_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub enum ThreadApprovalActionId {
    #[serde(rename = "artifact/write")]
    ArtifactWrite,
    #[serde(rename = "artifact/list")]
    ArtifactList,
    #[serde(rename = "artifact/read")]
    ArtifactRead,
    #[serde(rename = "artifact/versions")]
    ArtifactVersions,
    #[serde(rename = "artifact/delete")]
    ArtifactDelete,
    #[serde(rename = "file/read")]
    FileRead,
    #[serde(rename = "file/write")]
    FileWrite,
    #[serde(rename = "file/edit")]
    FileEdit,
    #[serde(rename = "file/patch")]
    FilePatch,
    #[serde(rename = "file/delete")]
    FileDelete,
    #[serde(rename = "file/glob")]
    FileGlob,
    #[serde(rename = "file/grep")]
    FileGrep,
    #[serde(rename = "fs/mkdir")]
    FsMkdir,
    #[serde(rename = "process/start")]
    ProcessStart,
    #[serde(rename = "process/kill")]
    ProcessKill,
    #[serde(rename = "process/interrupt")]
    ProcessInterrupt,
    #[serde(rename = "process/tail")]
    ProcessTail,
    #[serde(rename = "process/follow")]
    ProcessFollow,
    #[serde(rename = "process/inspect")]
    ProcessInspect,
    #[serde(rename = "process/execve")]
    ProcessExecve,
    #[serde(rename = "repo/search")]
    RepoSearch,
    #[serde(rename = "repo/index")]
    RepoIndex,
    #[serde(rename = "repo/symbols")]
    RepoSymbols,
    #[serde(rename = "mcp/list_servers")]
    McpListServers,
    #[serde(rename = "mcp/list_tools")]
    McpListTools,
    #[serde(rename = "mcp/list_resources")]
    McpListResources,
    #[serde(rename = "mcp/call")]
    McpCall,
    #[serde(rename = "thread/checkpoint/restore")]
    ThreadCheckpointRestore,
    #[serde(rename = "subagent/proxy_approval")]
    SubagentProxyApproval,
    #[serde(rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionPendingApprovalSummary {
    #[serde(default)]
    #[ts(optional)]
    pub requirement: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub argv: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub process_id: Option<omne_protocol::ProcessId>,
    #[serde(default)]
    #[ts(optional)]
    pub artifact_type: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub path: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub server: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub tool: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub hook: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub child_thread_id: Option<omne_protocol::ThreadId>,
    #[serde(default)]
    #[ts(optional)]
    pub child_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub child_approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub child_attention_state: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub child_last_turn_status: Option<omne_protocol::TurnStatus>,
    #[serde(default)]
    #[ts(optional)]
    pub approve_cmd: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub deny_cmd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionRunningProcess {
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionStaleProcess {
    pub process_id: omne_protocol::ProcessId,
    pub idle_seconds: u64,
    pub last_update_at: String,
    pub stdout_path: String,
    pub stderr_path: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadFanInDependencyBlockedSummary {
    pub task_id: String,
    pub status: String,
    pub dependency_blocked_count: usize,
    pub task_count: usize,
    pub dependency_blocked_ratio: f64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diagnostics_tasks: Option<usize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diagnostics_matched_completion_total: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diagnostics_pending_matching_tool_ids_total: Option<usize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub diagnostics_scan_last_seq_max: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub blocker_task_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub blocker_status: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadFanInResultDiagnosticsSummary {
    pub task_count: usize,
    pub diagnostics_tasks: usize,
    pub diagnostics_matched_completion_total: u64,
    pub diagnostics_pending_matching_tool_ids_total: usize,
    pub diagnostics_scan_last_seq_max: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadFanOutAutoApplySummary {
    pub task_id: String,
    pub status: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stage: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub patch_artifact_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recovery_commands: Option<usize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recovery_1: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionResponse {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
    pub archived: bool,
    #[serde(default)]
    #[ts(optional)]
    pub archived_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub archived_reason: Option<String>,
    pub paused: bool,
    #[serde(default)]
    #[ts(optional)]
    pub paused_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub paused_reason: Option<String>,
    #[serde(default)]
    pub failed_processes: Vec<omne_protocol::ProcessId>,
    pub approval_policy: omne_protocol::ApprovalPolicy,
    pub sandbox_policy: omne_protocol::SandboxPolicy,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub openai_base_url: Option<String>,
    pub last_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub active_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    pub active_turn_interrupt_requested: bool,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_status: Option<omne_protocol::TurnStatus>,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_reason: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_remaining: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_utilization: Option<f64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_exceeded: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_warning_active: Option<bool>,
    pub attention_state: String,
    #[serde(default)]
    pub pending_approvals: Vec<ThreadAttentionPendingApproval>,
    #[serde(default)]
    pub running_processes: Vec<ThreadAttentionRunningProcess>,
    #[serde(default)]
    pub stale_processes: Vec<ThreadAttentionStaleProcess>,
    pub attention_markers: ThreadAttentionMarkers,
    #[serde(default)]
    pub has_plan_ready: bool,
    #[serde(default)]
    pub has_diff_ready: bool,
    #[serde(default)]
    pub has_fan_out_linkage_issue: bool,
    #[serde(default)]
    pub has_fan_out_auto_apply_error: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_auto_apply: Option<ThreadFanOutAutoApplySummary>,
    #[serde(default)]
    pub has_fan_in_dependency_blocked: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_in_dependency_blocker: Option<ThreadFanInDependencyBlockedSummary>,
    #[serde(default)]
    pub has_fan_in_result_diagnostics: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_in_result_diagnostics: Option<ThreadFanInResultDiagnosticsSummary>,
    #[serde(default)]
    pub has_test_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListMetaParams {
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub include_attention_markers: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListMetaItem {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
    pub archived: bool,
    #[serde(default)]
    #[ts(optional)]
    pub archived_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub archived_reason: Option<String>,
    pub approval_policy: omne_protocol::ApprovalPolicy,
    pub sandbox_policy: omne_protocol::SandboxPolicy,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub openai_base_url: Option<String>,
    pub last_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub active_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    pub active_turn_interrupt_requested: bool,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_status: Option<omne_protocol::TurnStatus>,
    #[serde(default)]
    #[ts(optional)]
    pub last_turn_reason: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_limit: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_remaining: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_utilization: Option<f64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_exceeded: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub token_budget_warning_active: Option<bool>,
    pub attention_state: String,
    #[serde(default)]
    pub has_plan_ready: bool,
    #[serde(default)]
    pub has_diff_ready: bool,
    #[serde(default)]
    pub has_fan_out_linkage_issue: bool,
    #[serde(default)]
    pub has_fan_out_auto_apply_error: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_out_auto_apply: Option<ThreadFanOutAutoApplySummary>,
    #[serde(default)]
    pub has_fan_in_dependency_blocked: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_in_dependency_blocker: Option<ThreadFanInDependencyBlockedSummary>,
    #[serde(default)]
    pub has_fan_in_result_diagnostics: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fan_in_result_diagnostics: Option<ThreadFanInResultDiagnosticsSummary>,
    #[serde(default)]
    pub pending_subagent_proxy_approvals: usize,
    #[serde(default)]
    pub has_test_failed: bool,
    #[serde(default)]
    #[ts(optional)]
    pub created_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub updated_at: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub first_message: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attention_markers: Option<ThreadAttentionMarkers>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListMetaResponse {
    #[serde(default)]
    pub threads: Vec<ThreadListMetaItem>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskUsageParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskUsageResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub thread_dir: String,
    pub events_log_path: String,
    pub events_log_bytes: u64,
    pub artifacts_bytes: u64,
    pub total_bytes: u64,
    pub file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskReportParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub top_files: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskUsageSummary {
    pub events_log_bytes: u64,
    pub artifacts_bytes: u64,
    pub total_bytes: u64,
    pub file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskReportResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub disk_usage: ThreadDiskUsageSummary,
    pub artifact: ArtifactWriteResponse,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiffParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadPatchParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadGitSnapshotResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub process_id: omne_protocol::ProcessId,
    pub stdout_path: String,
    pub stderr_path: String,
    #[serde(default)]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    pub truncated: bool,
    pub max_bytes: u64,
    pub artifact: ArtifactWriteResponse,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadGitSnapshotNeedsApprovalResponse {
    pub needs_approval: bool,
    pub thread_id: omne_protocol::ThreadId,
    pub approval_id: omne_protocol::ApprovalId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadGitSnapshotDeniedResponse {
    pub denied: bool,
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    pub detail: ThreadGitSnapshotDeniedDetail,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadGitSnapshotTimedOutResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub process_id: omne_protocol::ProcessId,
    pub stdout_path: String,
    pub stderr_path: String,
    pub timed_out: bool,
    pub wait_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ThreadGitSnapshotRpcResponse {
    NeedsApproval(ThreadGitSnapshotNeedsApprovalResponse),
    Denied(ThreadGitSnapshotDeniedResponse),
    TimedOut(ThreadGitSnapshotTimedOutResponse),
    Ok(ThreadGitSnapshotResponse),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ThreadProcessDeniedDetail {
    AllowedToolsDenied(ProcessAllowedToolsDeniedResponse),
    ModeDenied(ProcessModeDeniedResponse),
    UnknownModeDenied(ProcessUnknownModeDeniedResponse),
    SandboxPolicyDenied(ProcessSandboxPolicyDeniedResponse),
    SandboxNetworkDenied(ProcessSandboxNetworkDeniedResponse),
    ExecPolicyDenied(ProcessExecPolicyDeniedResponse),
    ExecPolicyLoadDenied(ProcessExecPolicyLoadDeniedResponse),
    Denied(ProcessDeniedResponse),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ThreadArtifactDeniedDetail {
    AllowedToolsDenied(ArtifactAllowedToolsDeniedResponse),
    ModeDenied(ArtifactModeDeniedResponse),
    UnknownModeDenied(ArtifactUnknownModeDeniedResponse),
    Denied(ArtifactDeniedResponse),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ThreadGitSnapshotDeniedDetail {
    Process(ThreadProcessDeniedDetail),
    Artifact(ThreadArtifactDeniedDetail),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointCreateParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointListParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointRestoreParams {
    pub thread_id: omne_protocol::ThreadId,
    pub checkpoint_id: omne_protocol::CheckpointId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointRestoreDeniedResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub checkpoint_id: omne_protocol::CheckpointId,
    pub denied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_policy: Option<omne_protocol::SandboxPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub mode: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub decision: Option<ThreadCheckpointDecision>,
    #[serde(default)]
    #[ts(optional)]
    pub available: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub load_error: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_writable_roots: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ThreadCheckpointDecision {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointRestoreNeedsApprovalResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub checkpoint_id: omne_protocol::CheckpointId,
    pub needs_approval: bool,
    pub approval_id: omne_protocol::ApprovalId,
    pub plan: ThreadCheckpointPlan,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointRestoreResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub checkpoint_id: omne_protocol::CheckpointId,
    pub restored: bool,
    pub plan: ThreadCheckpointPlan,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointPlan {
    pub create: u64,
    pub modify: u64,
    pub delete: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointStats {
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointExcluded {
    pub symlink_count: u64,
    pub oversize_count: u64,
    pub secret_count: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointSizeLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointSummary {
    pub checkpoint_id: omne_protocol::CheckpointId,
    pub created_at: String,
    #[serde(default)]
    #[ts(optional)]
    pub label: Option<String>,
    pub snapshot_ref: String,
    pub manifest_path: String,
    pub stats: ThreadCheckpointStats,
    pub excluded: ThreadCheckpointExcluded,
    pub size_limits: ThreadCheckpointSizeLimits,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointCreateResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub checkpoint_id: omne_protocol::CheckpointId,
    #[serde(default)]
    #[ts(optional)]
    pub label: Option<String>,
    pub created_at: String,
    pub checkpoint_dir: String,
    pub snapshot_ref: String,
    pub manifest_path: String,
    pub stats: ThreadCheckpointStats,
    pub excluded: ThreadCheckpointExcluded,
    pub size_limits: ThreadCheckpointSizeLimits,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointListResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub checkpoints_dir: String,
    #[serde(default)]
    pub checkpoints: Vec<ThreadCheckpointSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceHookName {
    Setup,
    Run,
    Archive,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadHookRunParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub hook: WorkspaceHookName,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadHookRunResponse {
    pub ok: bool,
    #[serde(default)]
    pub skipped: bool,
    pub hook: String,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub searched: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub config_path: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub argv: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub process_id: Option<omne_protocol::ProcessId>,
    #[serde(default)]
    #[ts(optional)]
    pub stdout_path: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub stderr_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadHookRunNeedsApprovalResponse {
    pub needs_approval: bool,
    pub thread_id: omne_protocol::ThreadId,
    pub approval_id: omne_protocol::ApprovalId,
    pub hook: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadHookRunDeniedResponse {
    pub denied: bool,
    pub thread_id: omne_protocol::ThreadId,
    pub hook: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub config_path: Option<String>,
    pub detail: ThreadProcessDeniedDetail,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadHookRunErrorResponse {
    pub ok: bool,
    pub hook: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ThreadHookRunRpcResponse {
    NeedsApproval(ThreadHookRunNeedsApprovalResponse),
    Denied(ThreadHookRunDeniedResponse),
    Ok(ThreadHookRunResponse),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ThreadAutoHookResponse {
    NeedsApproval(ThreadHookRunNeedsApprovalResponse),
    Denied(ThreadHookRunDeniedResponse),
    Ok(ThreadHookRunResponse),
    Error(ThreadHookRunErrorResponse),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigureParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub approval_policy: Option<omne_protocol::ApprovalPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_policy: Option<omne_protocol::SandboxPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_writable_roots: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_network_access: Option<omne_protocol::SandboxNetworkAccess>,
    #[serde(default)]
    #[ts(optional)]
    pub mode: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub thinking: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub openai_base_url: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub allowed_tools: Option<Option<Vec<String>>>,
    #[serde(default)]
    #[ts(optional)]
    pub execpolicy_rules: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigureResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigExplainParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigExplainResponse {
    pub thread_id: omne_protocol::ThreadId,
    pub effective: ThreadConfigExplainEffective,
    pub mode_catalog: ThreadConfigExplainModeCatalog,
    #[serde(default)]
    #[ts(optional)]
    pub effective_mode_def: Option<serde_json::Value>,
    #[serde(default)]
    pub layers: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigExplainEffective {
    pub approval_policy: omne_protocol::ApprovalPolicy,
    pub sandbox_policy: omne_protocol::SandboxPolicy,
    #[serde(default)]
    pub sandbox_writable_roots: Vec<String>,
    pub sandbox_network_access: omne_protocol::SandboxNetworkAccess,
    pub mode: String,
    pub model: String,
    pub thinking: String,
    pub openai_base_url: String,
    #[serde(default)]
    #[ts(optional)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub execpolicy_rules: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub model_context_window: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub auto_compact_token_limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigExplainModeCatalog {
    pub source: String,
    #[serde(default)]
    #[ts(optional)]
    pub path: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub load_error: Option<String>,
    #[serde(default)]
    pub modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadModelsParams {
    pub thread_id: omne_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadModelsResponse {
    pub provider: String,
    pub base_url: String,
    pub current_model: String,
    pub thinking: String,
    #[serde(default)]
    #[ts(optional)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_whitelist: Vec<String>,
    pub capabilities: ThreadModelCapabilities,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadModelCapabilities {
    pub tools: bool,
    pub vision: bool,
    pub reasoning: bool,
    pub json_schema: bool,
    pub streaming: bool,
    pub prompt_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadEventsParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_events: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub kinds: Option<Vec<omne_protocol::ThreadEventKindTag>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadEventsResponse {
    pub events: Vec<omne_protocol::ThreadEvent>,
    pub last_seq: u64,
    pub thread_last_seq: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadSubscribeParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_events: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub kinds: Option<Vec<omne_protocol::ThreadEventKindTag>>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadSubscribeResponse {
    pub events: Vec<omne_protocol::ThreadEvent>,
    pub last_seq: u64,
    pub thread_last_seq: u64,
    pub has_more: bool,
    pub timed_out: bool,
}

fn truncate_summary_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub fn fan_out_auto_apply_summary_from_payload(
    payload: &crate::ArtifactFanOutResultStructuredData,
    truncate_len: usize,
) -> Option<ThreadFanOutAutoApplySummary> {
    let auto_apply = payload.isolated_write_auto_apply.as_ref()?;
    if auto_apply.applied {
        return None;
    }

    let status = if auto_apply
        .error
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        "error".to_string()
    } else if auto_apply.attempted {
        "attempted_not_applied".to_string()
    } else if auto_apply.enabled {
        "enabled_not_attempted".to_string()
    } else {
        "disabled".to_string()
    };

    let stage = auto_apply
        .failure_stage
        .as_ref()
        .map(|value| value.as_str().to_string());
    let patch_artifact_id = auto_apply
        .patch_artifact_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let recovery_commands =
        (!auto_apply.recovery_commands.is_empty()).then_some(auto_apply.recovery_commands.len());
    let recovery_1 = auto_apply
        .recovery_commands
        .first()
        .map(format_fan_out_recovery_command_preview)
        .map(|value| truncate_summary_chars(value.as_str(), truncate_len));
    let error = auto_apply
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_summary_chars(value, truncate_len));

    Some(ThreadFanOutAutoApplySummary {
        task_id: payload.task_id.clone(),
        status,
        stage,
        patch_artifact_id,
        recovery_commands,
        recovery_1,
        error,
    })
}

pub fn fan_in_result_diagnostics_summary_from_payload(
    payload: &crate::ArtifactFanInSummaryStructuredData,
) -> Option<ThreadFanInResultDiagnosticsSummary> {
    let mut diagnostics_tasks = 0usize;
    let mut diagnostics_matched_completion_total = 0u64;
    let mut diagnostics_pending_matching_tool_ids_total = 0usize;
    let mut diagnostics_scan_last_seq_max = 0u64;
    for item in &payload.tasks {
        if let Some(diagnostics) = item.result_artifact_diagnostics.as_ref() {
            diagnostics_tasks = diagnostics_tasks.saturating_add(1);
            diagnostics_matched_completion_total = diagnostics_matched_completion_total
                .saturating_add(diagnostics.matched_completion_count);
            diagnostics_pending_matching_tool_ids_total =
                diagnostics_pending_matching_tool_ids_total
                    .saturating_add(diagnostics.pending_matching_tool_ids);
            diagnostics_scan_last_seq_max =
                diagnostics_scan_last_seq_max.max(diagnostics.scan_last_seq);
        }
    }

    (diagnostics_tasks > 0).then_some(ThreadFanInResultDiagnosticsSummary {
        task_count: payload.task_count,
        diagnostics_tasks,
        diagnostics_matched_completion_total,
        diagnostics_pending_matching_tool_ids_total,
        diagnostics_scan_last_seq_max,
    })
}

pub fn fan_in_dependency_blocked_summary_from_payload(
    payload: &crate::ArtifactFanInSummaryStructuredData,
    reason_truncate_len: usize,
) -> Option<ThreadFanInDependencyBlockedSummary> {
    let dependency_blocked_count = payload
        .tasks
        .iter()
        .filter(|task| task.dependency_blocked)
        .count();
    let task = payload.tasks.iter().find(|task| {
        task.dependency_blocked
            || task
                .dependency_blocker_task_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || task
                .dependency_blocker_status
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    })?;
    let diagnostics = fan_in_result_diagnostics_summary_from_payload(payload);

    let blocker_task_id = task
        .dependency_blocker_task_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let blocker_status = task
        .dependency_blocker_status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let reason = task
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_summary_chars(value, reason_truncate_len));

    Some(ThreadFanInDependencyBlockedSummary {
        task_id: task.task_id.clone(),
        status: task.status.clone(),
        dependency_blocked_count,
        task_count: payload.task_count,
        dependency_blocked_ratio: if payload.task_count == 0 {
            0.0
        } else {
            dependency_blocked_count as f64 / payload.task_count as f64
        },
        diagnostics_tasks: diagnostics.as_ref().map(|value| value.diagnostics_tasks),
        diagnostics_matched_completion_total: diagnostics
            .as_ref()
            .map(|value| value.diagnostics_matched_completion_total),
        diagnostics_pending_matching_tool_ids_total: diagnostics
            .as_ref()
            .map(|value| value.diagnostics_pending_matching_tool_ids_total),
        diagnostics_scan_last_seq_max: diagnostics
            .as_ref()
            .map(|value| value.diagnostics_scan_last_seq_max),
        blocker_task_id,
        blocker_status,
        reason,
    })
}

fn format_fan_out_recovery_command_preview(
    command: &crate::ArtifactFanOutResultRecoveryCommandStructuredData,
) -> String {
    if command.argv.is_empty() {
        return command.label.clone();
    }
    format!("{}: {}", command.label, command.argv.join(" "))
}

#[cfg(test)]
mod thread_schema_tests {
    use super::*;
    use schemars::schema_for;

    #[test]
    fn thread_events_params_schema_uses_kind_enum_ref() {
        let schema = schema_for!(ThreadEventsParams);
        let value = serde_json::to_value(schema).expect("serialize schema");

        assert_eq!(
            value["properties"]["kinds"]["items"]["$ref"].as_str(),
            Some("#/definitions/ThreadEventKindTag")
        );

        let enum_values = value["definitions"]["ThreadEventKindTag"]["enum"]
            .as_array()
            .expect("ThreadEventKindTag enum values");
        assert_eq!(
            enum_values.len(),
            omne_protocol::THREAD_EVENT_KIND_TAGS.len()
        );

        let enum_set = enum_values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<std::collections::HashSet<_>>();
        for expected in omne_protocol::THREAD_EVENT_KIND_TAGS {
            assert!(
                enum_set.contains(expected),
                "missing enum value in schema: {expected}"
            );
        }
    }

    #[test]
    fn thread_subscribe_params_schema_uses_kind_enum_ref() {
        let schema = schema_for!(ThreadSubscribeParams);
        let value = serde_json::to_value(schema).expect("serialize schema");

        assert_eq!(
            value["properties"]["kinds"]["items"]["$ref"].as_str(),
            Some("#/definitions/ThreadEventKindTag")
        );
    }
}
