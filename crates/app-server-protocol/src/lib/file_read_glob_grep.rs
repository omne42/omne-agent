use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum FileRoot {
    Workspace,
    Reference,
}

impl FileRoot {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Reference => "reference",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileReadParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub root: Option<FileRoot>,
    pub path: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileGlobParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub root: Option<FileRoot>,
    pub pattern: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileGrepParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub root: Option<FileRoot>,
    pub query: String,
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    #[ts(optional)]
    pub include_glob: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub max_matches: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes_per_file: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub max_files: Option<usize>,
}

define_tool_denied_response!(FileDeniedResponse {});

define_tool_needs_approval_response!(FileNeedsApprovalResponse {});

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileAllowedToolsDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub tool: String,
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum FileModeDecision {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub mode: String,
    pub decision: FileModeDecision,
    pub decision_source: String,
    pub tool_override_hit: bool,
    #[serde(default)]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileUnknownModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub mode: String,
    pub decision: FileModeDecision,
    pub available: String,
    #[serde(default)]
    #[ts(optional)]
    pub load_error: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileSandboxPolicyDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub sandbox_policy: policy_meta::WriteScope,
    #[serde(default)]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}
