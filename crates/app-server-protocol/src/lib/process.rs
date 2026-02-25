#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessStartParams {
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub argv: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessListParams {
    #[serde(default)]
    #[ts(optional)]
    pub thread_id: Option<omne_protocol::ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessKillParams {
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInterruptParams {
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessTailParams {
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub stream: ProcessStream,
    #[serde(default)]
    #[ts(optional)]
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessFollowParams {
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    pub stream: ProcessStream,
    #[serde(default)]
    pub since_offset: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInspectParams {
    pub process_id: omne_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<omne_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Running,
    Exited,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInfo {
    pub process_id: omne_protocol::ProcessId,
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<omne_protocol::TurnId>,
    pub argv: Vec<String>,
    pub cwd: String,
    pub started_at: String,
    pub status: ProcessStatus,
    #[serde(default)]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    pub stdout_path: String,
    pub stderr_path: String,
    pub last_update_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessListResponse {
    pub processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessStartResponse {
    pub process_id: omne_protocol::ProcessId,
    pub stdout_path: String,
    pub stderr_path: String,
    pub effective_env_summary: serde_json::Value,
    #[serde(default)]
    #[ts(optional)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInspectResponse {
    pub tool_id: omne_protocol::ToolId,
    pub process: ProcessInfo,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessTailResponse {
    pub tool_id: omne_protocol::ToolId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessFollowResponse {
    pub tool_id: omne_protocol::ToolId,
    pub text: String,
    pub next_offset: u64,
    pub eof: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessSignalResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub thread_id: omne_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub remembered: Option<bool>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessNeedsApprovalResponse {
    pub needs_approval: bool,
    pub thread_id: omne_protocol::ThreadId,
    pub approval_id: omne_protocol::ApprovalId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessAllowedToolsDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub tool: String,
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProcessModeDecision {
    Allow,
    Prompt,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub thread_id: omne_protocol::ThreadId,
    pub mode: String,
    pub decision: ProcessModeDecision,
    pub decision_source: String,
    pub tool_override_hit: bool,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessUnknownModeDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub thread_id: omne_protocol::ThreadId,
    pub mode: String,
    pub decision: ProcessModeDecision,
    pub available: String,
    #[serde(default)]
    #[ts(optional)]
    pub load_error: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessSandboxPolicyDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub sandbox_policy: omne_protocol::SandboxPolicy,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessSandboxNetworkDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub sandbox_network_access: omne_protocol::SandboxNetworkAccess,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessExecPolicyDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub decision: ExecPolicyDecision,
    pub matched_rules: Vec<ExecPolicyRuleMatch>,
    #[serde(default)]
    #[ts(optional)]
    pub justification: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessExecPolicyLoadDeniedResponse {
    pub tool_id: omne_protocol::ToolId,
    pub denied: bool,
    pub mode: String,
    pub error: String,
    pub details: String,
    #[serde(default)]
    #[ts(optional)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ExecPolicyDecision {
    Allow,
    Prompt,
    PromptStrict,
    Forbidden,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum ExecPolicyRuleMatch {
    PrefixRuleMatch {
        #[serde(rename = "matchedPrefix")]
        matched_prefix: Vec<String>,
        decision: ExecPolicyDecision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        justification: Option<String>,
    },
    HeuristicsRuleMatch {
        command: Vec<String>,
        decision: ExecPolicyDecision,
    },
}
