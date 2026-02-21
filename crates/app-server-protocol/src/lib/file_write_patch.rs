#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileWriteParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub path: String,
    pub text: String,
    #[serde(default)]
    #[ts(optional)]
    pub create_parent_dirs: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FilePatchParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub path: String,
    pub patch: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}
