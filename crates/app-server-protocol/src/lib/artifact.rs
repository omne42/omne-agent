#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactWriteParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub artifact_id: Option<omne_agent_protocol::ArtifactId>,
    pub artifact_type: String,
    pub summary: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactListParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactReadParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
    pub artifact_id: omne_agent_protocol::ArtifactId,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactDeleteParams {
    pub thread_id: omne_agent_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_agent_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_agent_protocol::ApprovalId>,
    pub artifact_id: omne_agent_protocol::ArtifactId,
}
