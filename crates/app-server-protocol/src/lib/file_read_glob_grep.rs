#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum FileRoot {
    Workspace,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileReadParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
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
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
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
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
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
