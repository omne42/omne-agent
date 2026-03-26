use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use structured_text_protocol::StructuredTextData;
use time::OffsetDateTime;
use ts_rs::TS;
use uuid::Uuid;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct ThreadId(pub Uuid);

impl ThreadId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ThreadId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ThreadId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct TurnId(pub Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TurnId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct ProcessId(pub Uuid);

impl ProcessId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ProcessId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ProcessId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct ToolId(pub Uuid);

impl ToolId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ToolId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ToolId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct ApprovalId(pub Uuid);

impl ApprovalId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ApprovalId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ApprovalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ApprovalId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct ArtifactId(pub Uuid);

impl ArtifactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ArtifactId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "string")]
pub struct CheckpointId(pub Uuid);

impl CheckpointId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for CheckpointId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(transparent)]
#[ts(type = "number")]
pub struct EventSeq(pub u64);

impl EventSeq {
    pub const ZERO: Self = Self(0);
}

impl fmt::Display for EventSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    Cancelled,
    Stuck,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS, Default)]
#[serde(rename_all = "snake_case")]
pub enum TurnPriority {
    #[default]
    Foreground,
    Background,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Completed,
    Failed,
    Denied,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointRestoreStatus {
    Ok,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AttentionMarkerKind {
    PlanReady,
    DiffReady,
    TestFailed,
    FanOutLinkageIssue,
    FanOutAutoApplyError,
    TokenBudgetWarning,
    TokenBudgetExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    AutoApprove,
    Manual,
    UnlessTrusted,
    AutoDeny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum SandboxNetworkAccess {
    Deny,
    Allow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelRoutingRuleSource {
    Subagent,
    ProjectOverride,
    KeywordRule,
    RoleDefault,
    GlobalDefault,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ArtifactProvenance {
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<ToolId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<ProcessId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPreviewKind {
    Markdown,
    DiffUnified,
    PatchUnified,
    Code,
    Html,
    Log,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactReadMetadataSource {
    Latest,
    HistorySnapshot,
    LatestFallback,
}

impl ArtifactReadMetadataSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::HistorySnapshot => "history_snapshot",
            Self::LatestFallback => "latest_fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactReadMetadataFallbackReason {
    HistoryMetadataMissing,
    HistoryMetadataInvalid,
    HistoryMetadataUnreadable,
}

impl ArtifactReadMetadataFallbackReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HistoryMetadataMissing => "history_metadata_missing",
            Self::HistoryMetadataInvalid => "history_metadata_invalid",
            Self::HistoryMetadataUnreadable => "history_metadata_unreadable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ArtifactPreview {
    pub kind: ArtifactPreviewKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ArtifactMetadata {
    pub artifact_id: ArtifactId,
    pub artifact_type: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<ArtifactPreview>,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    #[ts(type = "string")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    #[ts(type = "string")]
    pub updated_at: OffsetDateTime,
    pub version: u32,
    pub content_path: String,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ArtifactProvenance>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ThreadEvent {
    pub seq: EventSeq,
    #[serde(with = "time::serde::rfc3339")]
    #[schemars(with = "String")]
    #[ts(type = "string")]
    pub timestamp: OffsetDateTime,
    pub thread_id: ThreadId,
    #[serde(flatten)]
    pub kind: ThreadEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextRef {
    File(ContextRefFile),
    Diff(ContextRefDiff),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextRefFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextRefDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttachmentSource {
    Path { path: String },
    Url { url: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct TurnAttachmentImage {
    pub source: AttachmentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct TurnAttachmentFile {
    pub source: AttachmentSource,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnAttachment {
    Image(TurnAttachmentImage),
    File(TurnAttachmentFile),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnDirective {
    Plan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AgentStepToolCall {
    pub name: String,
    pub call_id: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AgentStepToolResult {
    pub call_id: String,
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadEventKind {
    ThreadCreated {
        cwd: String,
    },

    ThreadSystemPromptSnapshot {
        prompt_sha256: String,
        prompt_text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },

    ThreadArchived {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ThreadUnarchived {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ThreadPaused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ThreadUnpaused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    TurnStarted {
        turn_id: TurnId,
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_refs: Option<Vec<ContextRef>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<TurnAttachment>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directives: Option<Vec<TurnDirective>>,
        #[serde(default)]
        priority: TurnPriority,
    },

    ModelRouted {
        turn_id: TurnId,
        selected_model: String,
        rule_source: ModelRoutingRuleSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule_id: Option<String>,
    },

    TurnInterruptRequested {
        turn_id: TurnId,
        reason: Option<String>,
    },

    TurnCompleted {
        turn_id: TurnId,
        status: TurnStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ThreadConfigUpdated {
        approval_policy: ApprovalPolicy,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sandbox_policy: Option<policy_meta::WriteScope>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sandbox_writable_roots: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sandbox_network_access: Option<SandboxNetworkAccess>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        show_thinking: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        openai_base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allowed_tools: Option<Option<Vec<String>>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execpolicy_rules: Option<Vec<String>>,
    },

    ApprovalRequested {
        approval_id: ApprovalId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        action: String,
        #[ts(type = "any")]
        params: serde_json::Value,
    },

    ApprovalDecided {
        approval_id: ApprovalId,
        decision: ApprovalDecision,
        #[serde(default, skip_serializing_if = "is_false")]
        remember: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ToolStarted {
        tool_id: ToolId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "any")]
        params: Option<serde_json::Value>,
    },

    ToolCompleted {
        tool_id: ToolId,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        structured_error: Option<StructuredTextData>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "any")]
        result: Option<serde_json::Value>,
    },

    AgentStep {
        turn_id: TurnId,
        step: u32,
        model: String,
        response_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<AgentStepToolCall>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_results: Vec<AgentStepToolResult>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "any")]
        token_usage: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warnings_count: Option<u32>,
    },

    AssistantMessage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "any")]
        token_usage: Option<serde_json::Value>,
    },

    ProcessStarted {
        process_id: ProcessId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        argv: Vec<String>,
        cwd: String,
        stdout_path: String,
        stderr_path: String,
    },

    ProcessInterruptRequested {
        process_id: ProcessId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ProcessKillRequested {
        process_id: ProcessId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    ProcessExited {
        process_id: ProcessId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    AttentionMarkerSet {
        marker: AttentionMarkerKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_id: Option<ArtifactId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process_id: Option<ProcessId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },

    AttentionMarkerCleared {
        marker: AttentionMarkerKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    CheckpointCreated {
        checkpoint_id: CheckpointId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        snapshot_ref: String,
    },

    CheckpointRestored {
        checkpoint_id: CheckpointId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        status: CheckpointRestoreStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report_artifact_id: Option<ArtifactId>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ThreadEventKindTag {
    ThreadCreated,
    ThreadSystemPromptSnapshot,
    ThreadArchived,
    ThreadUnarchived,
    ThreadPaused,
    ThreadUnpaused,
    TurnStarted,
    ModelRouted,
    TurnInterruptRequested,
    TurnCompleted,
    ThreadConfigUpdated,
    ApprovalRequested,
    ApprovalDecided,
    ToolStarted,
    ToolCompleted,
    AgentStep,
    AssistantMessage,
    ProcessStarted,
    ProcessInterruptRequested,
    ProcessKillRequested,
    ProcessExited,
    AttentionMarkerSet,
    AttentionMarkerCleared,
    CheckpointCreated,
    CheckpointRestored,
}

impl ThreadEventKindTag {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadEventKindTag::ThreadCreated => "thread_created",
            ThreadEventKindTag::ThreadSystemPromptSnapshot => "thread_system_prompt_snapshot",
            ThreadEventKindTag::ThreadArchived => "thread_archived",
            ThreadEventKindTag::ThreadUnarchived => "thread_unarchived",
            ThreadEventKindTag::ThreadPaused => "thread_paused",
            ThreadEventKindTag::ThreadUnpaused => "thread_unpaused",
            ThreadEventKindTag::TurnStarted => "turn_started",
            ThreadEventKindTag::ModelRouted => "model_routed",
            ThreadEventKindTag::TurnInterruptRequested => "turn_interrupt_requested",
            ThreadEventKindTag::TurnCompleted => "turn_completed",
            ThreadEventKindTag::ThreadConfigUpdated => "thread_config_updated",
            ThreadEventKindTag::ApprovalRequested => "approval_requested",
            ThreadEventKindTag::ApprovalDecided => "approval_decided",
            ThreadEventKindTag::ToolStarted => "tool_started",
            ThreadEventKindTag::ToolCompleted => "tool_completed",
            ThreadEventKindTag::AgentStep => "agent_step",
            ThreadEventKindTag::AssistantMessage => "assistant_message",
            ThreadEventKindTag::ProcessStarted => "process_started",
            ThreadEventKindTag::ProcessInterruptRequested => "process_interrupt_requested",
            ThreadEventKindTag::ProcessKillRequested => "process_kill_requested",
            ThreadEventKindTag::ProcessExited => "process_exited",
            ThreadEventKindTag::AttentionMarkerSet => "attention_marker_set",
            ThreadEventKindTag::AttentionMarkerCleared => "attention_marker_cleared",
            ThreadEventKindTag::CheckpointCreated => "checkpoint_created",
            ThreadEventKindTag::CheckpointRestored => "checkpoint_restored",
        }
    }
}

impl fmt::Display for ThreadEventKindTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ThreadEventKindTag {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "thread_created" => Ok(ThreadEventKindTag::ThreadCreated),
            "thread_system_prompt_snapshot" => Ok(ThreadEventKindTag::ThreadSystemPromptSnapshot),
            "thread_archived" => Ok(ThreadEventKindTag::ThreadArchived),
            "thread_unarchived" => Ok(ThreadEventKindTag::ThreadUnarchived),
            "thread_paused" => Ok(ThreadEventKindTag::ThreadPaused),
            "thread_unpaused" => Ok(ThreadEventKindTag::ThreadUnpaused),
            "turn_started" => Ok(ThreadEventKindTag::TurnStarted),
            "model_routed" => Ok(ThreadEventKindTag::ModelRouted),
            "turn_interrupt_requested" => Ok(ThreadEventKindTag::TurnInterruptRequested),
            "turn_completed" => Ok(ThreadEventKindTag::TurnCompleted),
            "thread_config_updated" => Ok(ThreadEventKindTag::ThreadConfigUpdated),
            "approval_requested" => Ok(ThreadEventKindTag::ApprovalRequested),
            "approval_decided" => Ok(ThreadEventKindTag::ApprovalDecided),
            "tool_started" => Ok(ThreadEventKindTag::ToolStarted),
            "tool_completed" => Ok(ThreadEventKindTag::ToolCompleted),
            "agent_step" => Ok(ThreadEventKindTag::AgentStep),
            "assistant_message" => Ok(ThreadEventKindTag::AssistantMessage),
            "process_started" => Ok(ThreadEventKindTag::ProcessStarted),
            "process_interrupt_requested" => Ok(ThreadEventKindTag::ProcessInterruptRequested),
            "process_kill_requested" => Ok(ThreadEventKindTag::ProcessKillRequested),
            "process_exited" => Ok(ThreadEventKindTag::ProcessExited),
            "attention_marker_set" => Ok(ThreadEventKindTag::AttentionMarkerSet),
            "attention_marker_cleared" => Ok(ThreadEventKindTag::AttentionMarkerCleared),
            "checkpoint_created" => Ok(ThreadEventKindTag::CheckpointCreated),
            "checkpoint_restored" => Ok(ThreadEventKindTag::CheckpointRestored),
            _ => Err(format!("unknown thread event kind tag: {s}")),
        }
    }
}

pub const THREAD_EVENT_KIND_TAGS: &[&str] = &[
    "thread_created",
    "thread_system_prompt_snapshot",
    "thread_archived",
    "thread_unarchived",
    "thread_paused",
    "thread_unpaused",
    "turn_started",
    "model_routed",
    "turn_interrupt_requested",
    "turn_completed",
    "thread_config_updated",
    "approval_requested",
    "approval_decided",
    "tool_started",
    "tool_completed",
    "agent_step",
    "assistant_message",
    "process_started",
    "process_interrupt_requested",
    "process_kill_requested",
    "process_exited",
    "attention_marker_set",
    "attention_marker_cleared",
    "checkpoint_created",
    "checkpoint_restored",
];

impl ThreadEventKind {
    pub fn tag_enum(&self) -> ThreadEventKindTag {
        match self {
            ThreadEventKind::ThreadCreated { .. } => ThreadEventKindTag::ThreadCreated,
            ThreadEventKind::ThreadSystemPromptSnapshot { .. } => {
                ThreadEventKindTag::ThreadSystemPromptSnapshot
            }
            ThreadEventKind::ThreadArchived { .. } => ThreadEventKindTag::ThreadArchived,
            ThreadEventKind::ThreadUnarchived { .. } => ThreadEventKindTag::ThreadUnarchived,
            ThreadEventKind::ThreadPaused { .. } => ThreadEventKindTag::ThreadPaused,
            ThreadEventKind::ThreadUnpaused { .. } => ThreadEventKindTag::ThreadUnpaused,
            ThreadEventKind::TurnStarted { .. } => ThreadEventKindTag::TurnStarted,
            ThreadEventKind::ModelRouted { .. } => ThreadEventKindTag::ModelRouted,
            ThreadEventKind::TurnInterruptRequested { .. } => {
                ThreadEventKindTag::TurnInterruptRequested
            }
            ThreadEventKind::TurnCompleted { .. } => ThreadEventKindTag::TurnCompleted,
            ThreadEventKind::ThreadConfigUpdated { .. } => ThreadEventKindTag::ThreadConfigUpdated,
            ThreadEventKind::ApprovalRequested { .. } => ThreadEventKindTag::ApprovalRequested,
            ThreadEventKind::ApprovalDecided { .. } => ThreadEventKindTag::ApprovalDecided,
            ThreadEventKind::ToolStarted { .. } => ThreadEventKindTag::ToolStarted,
            ThreadEventKind::ToolCompleted { .. } => ThreadEventKindTag::ToolCompleted,
            ThreadEventKind::AgentStep { .. } => ThreadEventKindTag::AgentStep,
            ThreadEventKind::AssistantMessage { .. } => ThreadEventKindTag::AssistantMessage,
            ThreadEventKind::ProcessStarted { .. } => ThreadEventKindTag::ProcessStarted,
            ThreadEventKind::ProcessInterruptRequested { .. } => {
                ThreadEventKindTag::ProcessInterruptRequested
            }
            ThreadEventKind::ProcessKillRequested { .. } => {
                ThreadEventKindTag::ProcessKillRequested
            }
            ThreadEventKind::ProcessExited { .. } => ThreadEventKindTag::ProcessExited,
            ThreadEventKind::AttentionMarkerSet { .. } => ThreadEventKindTag::AttentionMarkerSet,
            ThreadEventKind::AttentionMarkerCleared { .. } => {
                ThreadEventKindTag::AttentionMarkerCleared
            }
            ThreadEventKind::CheckpointCreated { .. } => ThreadEventKindTag::CheckpointCreated,
            ThreadEventKind::CheckpointRestored { .. } => ThreadEventKindTag::CheckpointRestored,
        }
    }

    pub fn tag(&self) -> &'static str {
        self.tag_enum().as_str()
    }
}

pub fn is_known_thread_event_kind_tag(tag: &str) -> bool {
    ThreadEventKindTag::from_str(tag).is_ok()
}

pub fn normalize_thread_event_kind_filter(
    kinds: &[String],
) -> Result<std::collections::HashSet<ThreadEventKindTag>, Vec<String>> {
    let mut requested = std::collections::HashSet::new();
    let mut invalid = Vec::new();

    for kind in kinds {
        let normalized = kind.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        match ThreadEventKindTag::from_str(&normalized) {
            Ok(kind_tag) => {
                requested.insert(kind_tag);
            }
            Err(_) => invalid.push(normalized),
        }
    }

    invalid.sort_unstable();
    invalid.dedup();
    if invalid.is_empty() {
        Ok(requested)
    } else {
        Err(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_event_kind_tags_are_known_and_unique() {
        let mut unique = std::collections::HashSet::new();
        for tag in THREAD_EVENT_KIND_TAGS {
            assert!(is_known_thread_event_kind_tag(tag));
            assert!(
                unique.insert(*tag),
                "duplicate thread event kind tag: {tag}"
            );
        }
        assert_eq!(unique.len(), THREAD_EVENT_KIND_TAGS.len());
    }

    #[test]
    fn normalize_thread_event_kind_filter_normalizes_and_rejects_invalid() {
        let valid = vec![
            " attention_marker_set ".to_string(),
            "ATTENTION_MARKER_CLEARED".to_string(),
            "".to_string(),
            "   ".to_string(),
        ];
        let normalized = normalize_thread_event_kind_filter(&valid)
            .expect("expected known kinds to normalize successfully");
        assert!(normalized.contains(&ThreadEventKindTag::AttentionMarkerSet));
        assert!(normalized.contains(&ThreadEventKindTag::AttentionMarkerCleared));

        let invalid = vec![
            "turn_started".to_string(),
            "not_real".to_string(),
            "NOT_REAL".to_string(),
        ];
        let err = normalize_thread_event_kind_filter(&invalid)
            .expect_err("expected unknown kinds to be rejected");
        assert_eq!(err, vec!["not_real".to_string()]);
    }
}
