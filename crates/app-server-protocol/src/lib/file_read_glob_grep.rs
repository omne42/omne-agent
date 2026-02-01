#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum FileRoot {
    Workspace,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileReadParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
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
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub root: Option<FileRoot>,
    /// Optional scope limiter for `glob`/`grep` style operations.
    ///
    /// This is mainly useful when the workspace is backed by DB-VFS, where broad patterns may
    /// require an explicit prefix to avoid whole-workspace scans.
    #[serde(default)]
    #[ts(optional)]
    pub path_prefix: Option<String>,
    pub pattern: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileGrepParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub root: Option<FileRoot>,
    /// Optional scope limiter for `glob`/`grep` style operations.
    ///
    /// This is mainly useful when the workspace is backed by DB-VFS, where broad patterns may
    /// require an explicit prefix to avoid whole-workspace scans.
    #[serde(default)]
    #[ts(optional)]
    pub path_prefix: Option<String>,
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
