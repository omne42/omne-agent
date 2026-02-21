#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct TurnStartParams {
    pub thread_id: omne_protocol::ThreadId,
    pub input: String,
    #[serde(default)]
    #[ts(optional)]
    pub context_refs: Option<Vec<omne_protocol::ContextRef>>,
    #[serde(default)]
    #[ts(optional)]
    pub attachments: Option<Vec<omne_protocol::TurnAttachment>>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<omne_protocol::TurnPriority>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct TurnInterruptParams {
    pub thread_id: omne_protocol::ThreadId,
    pub turn_id: omne_protocol::TurnId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}
