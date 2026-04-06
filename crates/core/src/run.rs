use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{PrName, Session, TaskId, TaskSpec};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StepSummary {
    pub name: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub log_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CheckSummary {
    pub steps: Vec<StepSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullRequestStatus {
    Draft,
    Ready,
    NoChanges,
    Merged,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: TaskId,
    pub head_branch: String,
    pub base_branch: String,
    pub status: PullRequestStatus,
    pub checks: CheckSummary,
    pub head_commit: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MergeResult {
    pub merged: bool,
    pub base_branch: String,
    pub merge_commit: Option<String>,
    pub merged_prs: Vec<TaskId>,
    #[serde(default)]
    pub checks: CheckSummary,
    pub error: Option<String>,
    pub error_log_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HookSpec {
    Command { program: PathBuf, args: Vec<String> },
    Webhook { url: String },
}

#[derive(Clone, Debug)]
pub struct RunRequest {
    pub pr_name: PrName,
    pub prompt: String,
    pub base_branch: String,
    pub tasks: Option<Vec<TaskSpec>>,
    pub apply_patch: Option<PathBuf>,
    pub hook: Option<HookSpec>,
    pub max_concurrency: usize,
    pub cargo_test: bool,
    pub auto_merge: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub session: Session,
    pub tasks: Vec<TaskSpec>,
    pub prs: Vec<PullRequest>,
    pub merge: MergeResult,
}
