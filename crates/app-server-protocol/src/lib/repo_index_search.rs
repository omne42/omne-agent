#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct RepoSearchParams {
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
    #[serde(default)]
    #[ts(optional)]
    pub include_glob: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub max_files: Option<usize>,
}
