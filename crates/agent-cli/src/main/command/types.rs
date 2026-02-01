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
}

#[derive(Debug, Clone)]
struct WorkflowTask {
    id: String,
    title: String,
    body: String,
}

#[derive(Debug, Clone)]
struct WorkflowTaskResult {
    task_id: String,
    title: String,
    thread_id: ThreadId,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<String>,
    assistant_text: Option<String>,
}

#[derive(Debug)]
struct FanOutScheduler {
    tasks: Vec<WorkflowTask>,
    fan_in_artifact_id: omne_agent_protocol::ArtifactId,
    concurrency_limit: usize,
    subagent_fork: bool,
    parent_cwd: Option<String>,
    pending_idx: usize,
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

