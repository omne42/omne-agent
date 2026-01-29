#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct TurnStartParams {
    pub thread_id: pm_protocol::ThreadId,
    pub input: String,
    #[serde(default)]
    #[ts(optional)]
    pub context_refs: Option<Vec<pm_protocol::ContextRef>>,
    #[serde(default)]
    #[ts(optional)]
    pub attachments: Option<Vec<pm_protocol::TurnAttachment>>,
    #[serde(default)]
    #[ts(optional)]
    pub priority: Option<pm_protocol::TurnPriority>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct TurnInterruptParams {
    pub thread_id: pm_protocol::ThreadId,
    pub turn_id: pm_protocol::TurnId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}
