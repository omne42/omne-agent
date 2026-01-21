use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
pub enum ApprovalPolicy {
    AutoApprove,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPolicy {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ArtifactProvenance {
    pub thread_id: ThreadId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<ToolId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<ProcessId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ArtifactMetadata {
    pub artifact_id: ArtifactId,
    pub artifact_type: String,
    pub summary: String,
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

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadEventKind {
    ThreadCreated {
        cwd: String,
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
        sandbox_policy: Option<SandboxPolicy>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        openai_base_url: Option<String>,
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
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "any")]
        result: Option<serde_json::Value>,
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
}
