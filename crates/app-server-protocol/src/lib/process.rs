#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessStartParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub argv: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessListParams {
    #[serde(default)]
    #[ts(optional)]
    pub thread_id: Option<pm_protocol::ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessKillParams {
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInterruptParams {
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
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
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub stream: ProcessStream,
    #[serde(default)]
    #[ts(optional)]
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessFollowParams {
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub stream: ProcessStream,
    #[serde(default)]
    pub since_offset: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInspectParams {
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    #[serde(default)]
    #[ts(optional)]
    pub max_lines: Option<usize>,
}
