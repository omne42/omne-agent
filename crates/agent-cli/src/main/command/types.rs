use super::*;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkflowFileFrontmatterV1 {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) name: Option<String>,
    pub(super) mode: String,
    #[serde(default)]
    pub(super) show_thinking: Option<bool>,
    #[serde(default, rename = "subagent-fork")]
    pub(super) subagent_fork: bool,
    #[serde(default)]
    pub(super) allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub(super) context: Vec<WorkflowContextStep>,
    #[serde(default)]
    pub(super) inputs: Vec<WorkflowInputDecl>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkflowContextStep {
    pub(super) argv: Vec<String>,
    pub(super) summary: String,
    #[serde(default)]
    pub(super) ok_exit_codes: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkflowInputDecl {
    pub(super) name: String,
    #[serde(default)]
    pub(super) required: bool,
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowFile {
    pub(super) frontmatter: WorkflowFileFrontmatterV1,
    pub(super) body: String,
    pub(super) modes_load_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowTask {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) body: String,
    pub(super) depends_on: Vec<String>,
    pub(super) priority: WorkflowTaskPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkflowTaskPriority {
    High,
    Normal,
    Low,
}

impl WorkflowTaskPriority {
    pub(super) fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Self::High),
            "normal" => Some(Self::Normal),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    pub(super) fn rank(self) -> usize {
        match self {
            Self::High => 0,
            Self::Normal => 1,
            Self::Low => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowTaskResult {
    pub(super) task_id: String,
    pub(super) title: String,
    pub(super) thread_id: Option<ThreadId>,
    pub(super) turn_id: Option<TurnId>,
    pub(super) result_artifact_id: Option<ArtifactId>,
    pub(super) result_artifact_error: Option<String>,
    pub(super) result_artifact_error_id: Option<ArtifactId>,
    pub(super) status: TurnStatus,
    pub(super) reason: Option<String>,
    pub(super) dependency_blocked: bool,
    pub(super) assistant_text: Option<String>,
    pub(super) pending_approval: Option<WorkflowPendingApproval>,
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowPendingApproval {
    pub(super) approval_id: ApprovalId,
    pub(super) action: String,
    pub(super) summary: Option<String>,
    pub(super) approve_cmd: Option<String>,
    pub(super) deny_cmd: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FanOutSchedulingParams {
    pub(super) env_max_concurrent_subagents: usize,
    pub(super) effective_concurrency_limit: usize,
    pub(super) priority_aging_rounds: usize,
}

#[derive(Debug)]
pub(super) struct FanOutScheduler {
    pub(super) tasks: Vec<WorkflowTask>,
    pub(super) fan_in_artifact_id: omne_protocol::ArtifactId,
    pub(super) scheduling: FanOutSchedulingParams,
    pub(super) subagent_fork: bool,
    pub(super) parent_cwd: Option<String>,
    pub(super) started_ids: BTreeSet<String>,
    pub(super) ready_wait_rounds: BTreeMap<String, usize>,
    pub(super) task_statuses: BTreeMap<String, TurnStatus>,
    pub(super) active: Vec<FanOutActiveTask>,
    pub(super) finished: Vec<WorkflowTaskResult>,
    pub(super) final_summary_written: bool,
    pub(super) started_at: Instant,
    pub(super) last_progress_print: Instant,
    pub(super) last_progress_artifact_write: Instant,
}

#[derive(Debug)]
pub(super) struct FanOutActiveTask {
    pub(super) task_id: String,
    pub(super) title: String,
    pub(super) thread_id: ThreadId,
    pub(super) turn_id: TurnId,
    pub(super) since_seq: u64,
    pub(super) assistant_text: Option<String>,
}
