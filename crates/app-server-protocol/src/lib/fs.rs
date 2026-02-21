#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FsMkdirParams {
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
