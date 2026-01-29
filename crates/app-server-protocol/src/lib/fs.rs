#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FsMkdirParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}
