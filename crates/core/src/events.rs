use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::domain::{PrName, PullRequestStatus, RepositoryName, SessionId, TaskId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub repo: RepositoryName,
    pub pr_name: PrName,
    pub base_branch: String,
    pub tmp_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSummary {
    pub id: TaskId,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub id: TaskId,
    pub status: PullRequestStatus,
    pub head_branch: String,
    pub head_commit: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeSummary {
    pub merged: bool,
    pub base_branch: String,
    pub merge_commit: Option<String>,
    pub merged_prs: Vec<TaskId>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    SessionCreated { session: SessionSummary },
    TasksPlanned { tasks: Vec<TaskSummary> },
    TaskStarted { task: TaskSummary },
    TaskFinished { pr: PullRequestSummary },
    MergeStarted { ready_prs: Vec<TaskId> },
    MergeFinished { merge: MergeSummary },
    HookStarted,
    HookFinished { ok: bool },
}

impl std::fmt::Display for RunEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunEvent::SessionCreated { session } => write!(
                f,
                "session created id={} repo={} pr={} base={} tmp={}",
                session.id,
                session.repo.as_str(),
                session.pr_name.as_str(),
                session.base_branch,
                session.tmp_dir.display()
            ),
            RunEvent::TasksPlanned { tasks } => {
                write!(f, "tasks planned n={}", tasks.len())
            }
            RunEvent::TaskStarted { task } => {
                write!(
                    f,
                    "task started id={} title={}",
                    task.id.as_str(),
                    task.title
                )
            }
            RunEvent::TaskFinished { pr } => write!(
                f,
                "task finished id={} status={:?} branch={}",
                pr.id.as_str(),
                pr.status,
                pr.head_branch
            ),
            RunEvent::MergeStarted { ready_prs } => {
                write!(f, "merge started ready={}", ready_prs.len())
            }
            RunEvent::MergeFinished { merge } => write!(
                f,
                "merge finished merged={} commit={} error={}",
                merge.merged,
                merge.merge_commit.as_deref().unwrap_or("-"),
                merge.error.as_deref().unwrap_or("-")
            ),
            RunEvent::HookStarted => write!(f, "hook started"),
            RunEvent::HookFinished { ok } => write!(f, "hook finished ok={ok}"),
        }
    }
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<RunEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub fn emit(&self, event: RunEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RunEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_event_serializes_with_type_tag() -> anyhow::Result<()> {
        let event = RunEvent::HookStarted;
        let value = serde_json::to_value(&event)?;
        assert_eq!(value["type"], "hook_started");
        Ok(())
    }

    #[test]
    fn run_event_deserializes_from_type_tag() -> anyhow::Result<()> {
        let event: RunEvent = serde_json::from_str(r#"{"type":"hook_finished","ok":true}"#)?;
        assert!(matches!(event, RunEvent::HookFinished { ok: true }));
        Ok(())
    }
}
