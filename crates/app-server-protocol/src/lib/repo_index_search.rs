use super::*;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoSearchParams {
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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoIndexParams {
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
    #[serde(default)]
    #[ts(optional)]
    pub include_glob: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoSymbolsParams {
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
    #[serde(default)]
    #[ts(optional)]
    pub include_glob: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub max_files: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes_per_file: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub max_symbols: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoSearchResponse {
    pub tool_id: omne_protocol::ToolId,
    pub artifact_id: omne_protocol::ArtifactId,
    pub created: bool,
    pub content_path: String,
    pub metadata_path: String,
    pub metadata: omne_protocol::ArtifactMetadata,
    pub root: String,
    pub matches: usize,
    pub truncated: bool,
    pub files_scanned: usize,
    pub files_skipped_too_large: usize,
    pub files_skipped_binary: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoIndexResponse {
    pub tool_id: omne_protocol::ToolId,
    pub artifact_id: omne_protocol::ArtifactId,
    pub created: bool,
    pub content_path: String,
    pub metadata_path: String,
    pub metadata: omne_protocol::ArtifactMetadata,
    pub root: String,
    pub paths_listed: usize,
    pub truncated: bool,
    pub files_scanned: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoSymbolsResponse {
    pub tool_id: omne_protocol::ToolId,
    pub artifact_id: omne_protocol::ArtifactId,
    pub created: bool,
    pub content_path: String,
    pub metadata_path: String,
    pub metadata: omne_protocol::ArtifactMetadata,
    pub root: String,
    pub symbols: usize,
    pub files_scanned: usize,
    pub files_parsed: usize,
    pub truncated_files: bool,
    pub truncated_symbols: bool,
    pub files_skipped_too_large: usize,
    pub files_skipped_binary: usize,
    pub files_failed_parse: usize,
}

define_tool_denied_response_skip_none!(RepoDeniedResponse {});

define_tool_needs_approval_response!(RepoNeedsApprovalResponse {
    pub thread_id: omne_protocol::ThreadId,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum RepoModeDecision {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub mode: String,
    pub decision: RepoModeDecision,
    pub decision_source: String,
    #[serde(default)]
    pub tool_override_hit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoUnknownModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub mode: String,
    pub decision: RepoModeDecision,
    pub available: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub load_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoAllowedToolsDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub tool: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}
