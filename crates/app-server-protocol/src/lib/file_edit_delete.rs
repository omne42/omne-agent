#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileEditParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub path: String,
    pub edits: Vec<FileEditOp>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileEditOp {
    pub old: String,
    pub new: String,
    #[serde(default)]
    #[ts(optional)]
    pub expected_replacements: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileDeleteParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}
