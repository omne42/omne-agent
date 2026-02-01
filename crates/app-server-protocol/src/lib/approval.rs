#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalDecideParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    pub approval_id: omne_agent_protocol::ApprovalId,
    pub decision: omne_agent_protocol::ApprovalDecision,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalListParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    pub include_decided: bool,
}
