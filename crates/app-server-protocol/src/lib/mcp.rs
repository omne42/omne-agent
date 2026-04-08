use super::*;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpListServersParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpListToolsParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub server: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpListResourcesParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub server: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpCallParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub server: String,
    pub tool: String,
    #[serde(default)]
    #[ts(optional)]
    pub arguments: Option<serde_json::Value>,
}

define_tool_denied_response_skip_none!(McpDeniedResponse {});

define_tool_needs_approval_response!(McpNeedsApprovalResponse {
    pub thread_id: omne_protocol::ThreadId,
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpModeDecision {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub mode: String,
    pub decision: McpModeDecision,
    pub decision_source: String,
    #[serde(default)]
    pub tool_override_hit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpUnknownModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub mode: String,
    pub decision: McpModeDecision,
    pub available: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub load_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpAllowedToolsDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub tool: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpDisabledDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub reason: String,
    pub env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpSandboxPolicyDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub sandbox_policy: policy_meta::WriteScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpSandboxNetworkDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub sandbox_network_access: omne_protocol::SandboxNetworkAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpExecPolicyDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub decision: ExecPolicyDecision,
    pub matched_rules: Vec<ExecPolicyRuleMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpExecPolicyLoadDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub denied: bool,
    pub mode: String,
    pub error: String,
    pub details: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpFailedResponse {
    pub tool_id: omne_protocol::ToolId,
    #[serde(default)]
    pub failed: bool,
    pub error: String,
    pub server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub structured_error: Option<StructuredTextData>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpServerDescriptor {
    pub name: String,
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub supported: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpListServersResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub config_path: Option<String>,
    pub servers: Vec<McpServerDescriptor>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpActionInlineResponse {
    pub process_id: omne_protocol::ProcessId,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
pub struct McpActionArtifactResponse {
    pub process_id: omne_protocol::ProcessId,
    pub artifact: ArtifactWriteResponse,
    pub truncated: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum McpActionResponse {
    Inline(McpActionInlineResponse),
    Artifact(Box<McpActionArtifactResponse>),
}
