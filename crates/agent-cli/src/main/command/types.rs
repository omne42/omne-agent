#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkflowFileFrontmatterV1 {
    version: u32,
    #[serde(default)]
    name: Option<String>,
    mode: String,
    #[serde(default, rename = "subagent-fork")]
    subagent_fork: bool,
    #[serde(default)]
    allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    context: Vec<WorkflowContextStep>,
    #[serde(default)]
    inputs: Vec<WorkflowInputDecl>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkflowContextStep {
    argv: Vec<String>,
    summary: String,
    #[serde(default)]
    ok_exit_codes: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorkflowInputDecl {
    name: String,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Clone)]
struct WorkflowFile {
    frontmatter: WorkflowFileFrontmatterV1,
    body: String,
    modes_load_error: Option<String>,
}

#[derive(Debug, Clone)]
struct WorkflowTask {
    id: String,
    title: String,
    body: String,
    depends_on: Vec<String>,
    priority: WorkflowTaskPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowTaskPriority {
    High,
    Normal,
    Low,
}

impl WorkflowTaskPriority {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Self::High),
            "normal" => Some(Self::Normal),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    fn rank(self) -> usize {
        match self {
            Self::High => 0,
            Self::Normal => 1,
            Self::Low => 2,
        }
    }
}

#[derive(Debug, Clone)]
struct WorkflowTaskResult {
    task_id: String,
    title: String,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    result_artifact_id: Option<ArtifactId>,
    result_artifact_error: Option<String>,
    result_artifact_error_id: Option<ArtifactId>,
    status: TurnStatus,
    reason: Option<String>,
    dependency_blocked: bool,
    assistant_text: Option<String>,
    pending_approval: Option<WorkflowPendingApproval>,
}

#[derive(Debug, Clone)]
struct WorkflowPendingApproval {
    approval_id: ApprovalId,
    action: String,
    summary: Option<String>,
    approve_cmd: Option<String>,
    deny_cmd: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct FanOutSchedulingParams {
    env_max_concurrent_subagents: usize,
    effective_concurrency_limit: usize,
    priority_aging_rounds: usize,
}

#[derive(Debug)]
struct FanOutScheduler {
    tasks: Vec<WorkflowTask>,
    fan_in_artifact_id: omne_protocol::ArtifactId,
    scheduling: FanOutSchedulingParams,
    subagent_fork: bool,
    parent_cwd: Option<String>,
    started_ids: BTreeSet<String>,
    ready_wait_rounds: BTreeMap<String, usize>,
    task_statuses: BTreeMap<String, TurnStatus>,
    active: Vec<FanOutActiveTask>,
    finished: Vec<WorkflowTaskResult>,
    final_summary_written: bool,
    started_at: Instant,
    last_progress_print: Instant,
    last_progress_artifact_write: Instant,
}

#[derive(Debug)]
struct FanOutActiveTask {
    task_id: String,
    title: String,
    thread_id: ThreadId,
    turn_id: TurnId,
    since_seq: u64,
    assistant_text: Option<String>,
}
