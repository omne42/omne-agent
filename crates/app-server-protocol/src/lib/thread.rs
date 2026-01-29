#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStartParams {
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadResumeParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadForkParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadArchiveParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnarchiveParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadPauseParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnpauseParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDeleteParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadClearArtifactsParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStateParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListMetaParams {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskUsageParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskReportParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub top_files: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiffParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadPatchParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointCreateParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointListParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadCheckpointRestoreParams {
    pub thread_id: pm_protocol::ThreadId,
    pub checkpoint_id: pm_protocol::CheckpointId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
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
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub hook: WorkspaceHookName,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigureParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub approval_policy: Option<pm_protocol::ApprovalPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_policy: Option<pm_protocol::SandboxPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_writable_roots: Option<Vec<String>>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_network_access: Option<pm_protocol::SandboxNetworkAccess>,
    #[serde(default)]
    #[ts(optional)]
    pub mode: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub openai_base_url: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub allowed_tools: Option<Option<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigExplainParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadModelsParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadEventsParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_events: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadSubscribeParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_events: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_ms: Option<u64>,
}
