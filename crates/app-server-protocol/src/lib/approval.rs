use super::*;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalDecideParams {
    pub thread_id: omne_protocol::ThreadId,
    pub approval_id: omne_protocol::ApprovalId,
    pub decision: omne_protocol::ApprovalDecision,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalDecideResponse {
    pub ok: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_false")]
    pub forwarded: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub child_thread_id: Option<omne_protocol::ThreadId>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub child_approval_id: Option<omne_protocol::ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalListParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    pub include_decided: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalListResponse {
    #[serde(default)]
    pub approvals: Vec<ApprovalListItem>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalListItem {
    pub request: ApprovalRequestInfo,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub decision: Option<ApprovalDecisionInfo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalRequestInfo {
    pub approval_id: omne_protocol::ApprovalId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    pub action: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub action_id: Option<ThreadApprovalActionId>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub summary: Option<ThreadAttentionPendingApprovalSummary>,
    pub requested_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalDecisionInfo {
    pub decision: omne_protocol::ApprovalDecision,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    pub decided_at: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}
