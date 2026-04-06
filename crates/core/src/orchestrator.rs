use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use time::OffsetDateTime;
use tokio::task::JoinSet;
use tracing::{info, warn};

use crate::domain::{Repository, Session, SessionId, TaskId, TaskSpec};
use crate::events::{
    EventBus, MergeSummary, PullRequestSummary, RunEvent, SessionSummary, TaskSummary,
};
use crate::hooks::HookRunner;
use crate::paths::{PmPaths, SessionPaths};
use crate::run::{
    CheckSummary, HookSpec, MergeResult, PullRequest, PullRequestStatus, RunRequest, RunResult,
    StepSummary,
};
use crate::storage::Storage;

#[async_trait]
pub trait Architect: Send + Sync {
    async fn split(&self, session: &Session) -> anyhow::Result<Vec<TaskSpec>>;
}

#[derive(Clone, Debug)]
pub struct RuleBasedArchitect {
    max_tasks: usize,
}

impl RuleBasedArchitect {
    pub fn new(max_tasks: usize) -> Self {
        Self {
            max_tasks: max_tasks.max(1),
        }
    }

    fn split_prompt(
        prompt: &str,
        pr_name: &crate::domain::PrName,
        max_tasks: usize,
    ) -> Vec<TaskSpec> {
        let max_tasks = max_tasks.max(1);

        let mut task_titles: Vec<String> = Vec::new();
        for line in prompt.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let Some(title) = extract_task_title(line) else {
                continue;
            };
            let title = title.trim();
            if title.is_empty() {
                continue;
            }

            task_titles.push(title.to_string());
            if task_titles.len() >= max_tasks {
                break;
            }
        }

        if task_titles.is_empty() {
            return vec![TaskSpec {
                id: TaskId::sanitize("main"),
                title: format!("Implement {}", pr_name.as_str()),
                description: Some("Phase 1: rule-based single-task fallback".to_string()),
            }];
        }

        task_titles
            .into_iter()
            .enumerate()
            .map(|(index, title)| TaskSpec {
                id: TaskId::sanitize(&format!("t{}", index + 1)),
                title,
                description: None,
            })
            .collect()
    }
}

impl Default for RuleBasedArchitect {
    fn default() -> Self {
        Self::new(8)
    }
}

#[async_trait]
impl Architect for RuleBasedArchitect {
    async fn split(&self, session: &Session) -> anyhow::Result<Vec<TaskSpec>> {
        Ok(Self::split_prompt(
            &session.prompt,
            &session.pr_name,
            self.max_tasks,
        ))
    }
}

fn extract_task_title(line: &str) -> Option<&str> {
    for prefix in [
        "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] ", "+ [ ] ", "+ [x] ", "+ [X] ",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest);
        }
    }

    if let Some(rest) = strip_numbered_prefix(line) {
        return Some(rest);
    }

    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest);
        }
    }

    None
}

fn strip_numbered_prefix(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    if i >= bytes.len() {
        return None;
    }
    let punct = bytes[i];
    if punct != b'.' && punct != b')' {
        return None;
    }
    i += 1;
    if i >= bytes.len() || bytes[i] != b' ' {
        return None;
    }
    i += 1;
    Some(&line[i..])
}

#[async_trait]
pub trait Coder: Send + Sync {
    async fn execute(
        &self,
        repo: &Repository,
        session: &Session,
        session_paths: &SessionPaths,
        request: &RunRequest,
        task: &TaskSpec,
    ) -> anyhow::Result<PullRequest>;
}

#[async_trait]
pub trait Merger: Send + Sync {
    async fn merge(
        &self,
        repo: &Repository,
        session: &Session,
        session_paths: &SessionPaths,
        prs: &[PullRequest],
    ) -> anyhow::Result<MergeResult>;
}

#[derive(Clone)]
pub struct Orchestrator {
    pub storage: Arc<dyn Storage>,
    pub hook_runner: Arc<dyn HookRunner>,
    pub events: EventBus,
    pub architect: Arc<dyn Architect>,
    pub coder: Arc<dyn Coder>,
    pub merger: Arc<dyn Merger>,
}

impl Orchestrator {
    pub async fn run(
        &self,
        omne_paths: &PmPaths,
        repo: Repository,
        mut request: RunRequest,
    ) -> anyhow::Result<RunResult> {
        let repo = Arc::new(repo);
        let tasks_override = request.tasks.take();
        let prompt = request.prompt.clone();
        let request = Arc::new(request);

        let session = Arc::new(Session {
            id: SessionId::new(),
            repo: repo.name.clone(),
            pr_name: request.pr_name.clone(),
            prompt,
            base_branch: request.base_branch.clone(),
            created_at: OffsetDateTime::now_utc(),
        });
        let session_paths = Arc::new(SessionPaths::new(&repo.name, session.id));

        tokio::fs::create_dir_all(session_paths.root()).await?;
        tokio::fs::create_dir_all(session_paths.logs_dir()).await?;
        tokio::fs::create_dir_all(session_paths.tasks_dir()).await?;

        self.write_session_artifacts(session_paths.as_ref(), session.as_ref())
            .await?;

        self.events.emit(RunEvent::SessionCreated {
            session: SessionSummary {
                id: session.id,
                repo: repo.name.clone(),
                pr_name: session.pr_name.clone(),
                base_branch: session.base_branch.clone(),
                tmp_dir: session_paths.root().to_path_buf(),
            },
        });

        info!(session_id = %session.id, repo = %repo.name, "session created");

        let tasks = match tasks_override {
            Some(tasks) if !tasks.is_empty() => tasks,
            Some(_) => anyhow::bail!("tasks override provided but empty"),
            None => self.architect.split(session.as_ref()).await?,
        };

        self.events.emit(RunEvent::TasksPlanned {
            tasks: tasks
                .iter()
                .map(|task| TaskSummary {
                    id: task.id.clone(),
                    title: task.title.clone(),
                })
                .collect(),
        });
        self.storage
            .put_json(
                &format!("sessions/{}/tasks", session.id),
                &serde_json::to_value(&tasks)?,
            )
            .await?;

        let mut prs = self
            .run_tasks(
                Arc::clone(&repo),
                Arc::clone(&session),
                Arc::clone(&session_paths),
                Arc::clone(&request),
                &tasks,
            )
            .await?;

        self.storage
            .put_json(
                &format!("sessions/{}/prs", session.id),
                &serde_json::to_value(&prs)?,
            )
            .await?;

        let ready_prs: Vec<TaskId> = prs
            .iter()
            .filter(|pr| matches!(pr.status, PullRequestStatus::Ready))
            .map(|pr| pr.id.clone())
            .collect();
        self.events.emit(RunEvent::MergeStarted { ready_prs });

        let merge = if request.auto_merge {
            match self
                .merger
                .merge(
                    repo.as_ref(),
                    session.as_ref(),
                    session_paths.as_ref(),
                    &prs,
                )
                .await
            {
                Ok(merge) => {
                    if let Some(error) = merge.error.as_deref() {
                        warn!(session_id = %session.id, error = %error, "merge failed");
                    }
                    merge
                }
                Err(err) => {
                    warn!(session_id = %session.id, error = %err, "merge failed");
                    self.merge_failure_result(session_paths.as_ref(), session.as_ref(), &err)
                        .await
                }
            }
        } else {
            MergeResult {
                merged: false,
                base_branch: session.base_branch.clone(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: CheckSummary::default(),
                error: None,
                error_log_path: None,
            }
        };

        self.events.emit(RunEvent::MergeFinished {
            merge: MergeSummary {
                merged: merge.merged,
                base_branch: merge.base_branch.clone(),
                merge_commit: merge.merge_commit.clone(),
                merged_prs: merge.merged_prs.clone(),
                error: merge.error.clone(),
            },
        });

        if merge.merged {
            let merged: std::collections::HashSet<TaskId> =
                merge.merged_prs.iter().cloned().collect();
            for pr in &mut prs {
                if merged.contains(&pr.id) {
                    pr.status = PullRequestStatus::Merged;
                }
            }
        }

        self.storage
            .put_json(
                &format!("sessions/{}/merge", session.id),
                &serde_json::to_value(&merge)?,
            )
            .await?;

        self.storage
            .put_json(
                &format!("sessions/{}/prs", session.id),
                &serde_json::to_value(&prs)?,
            )
            .await?;

        let result = RunResult {
            session: Arc::unwrap_or_clone(session),
            tasks,
            prs,
            merge,
        };

        self.write_result_artifacts(session_paths.as_ref(), &result)
            .await?;

        self.storage
            .put_json(
                &format!("sessions/{}/result", result.session.id),
                &serde_json::to_value(&result)?,
            )
            .await?;

        if let Some(hook) = &request.hook {
            self.events.emit(RunEvent::HookStarted);
            match self
                .run_hook(omne_paths, session_paths.as_ref(), &result, hook)
                .await
            {
                Ok(()) => {
                    self.events.emit(RunEvent::HookFinished { ok: true });
                }
                Err(err) => {
                    self.events.emit(RunEvent::HookFinished { ok: false });
                    return Err(err)
                        .with_context(|| format!("hook failed for session {}", result.session.id));
                }
            }
        }

        Ok(result)
    }

    async fn write_session_artifacts(
        &self,
        session_paths: &SessionPaths,
        session: &Session,
    ) -> anyhow::Result<()> {
        let session_json = serde_json::to_vec_pretty(session)?;
        tokio::fs::write(session_paths.root().join("session.json"), &session_json).await?;

        self.storage
            .put_json(
                &format!("sessions/{}/session", session.id),
                &serde_json::to_value(session)?,
            )
            .await?;

        self.storage
            .put_json(
                &format!("sessions/{}/meta", session.id),
                &serde_json::to_value(session.meta())?,
            )
            .await?;
        Ok(())
    }

    async fn run_tasks(
        &self,
        repo: Arc<Repository>,
        session: Arc<Session>,
        session_paths: Arc<SessionPaths>,
        request: Arc<RunRequest>,
        tasks: &[TaskSpec],
    ) -> anyhow::Result<Vec<PullRequest>> {
        let failed_pr = |task_id: &crate::domain::TaskId, checks: CheckSummary| PullRequest {
            id: task_id.clone(),
            head_branch: format!(
                "ai/{}/{}/{}/failed",
                session.pr_name.as_str(),
                session.id,
                task_id.as_str()
            ),
            base_branch: session.base_branch.clone(),
            status: PullRequestStatus::Failed,
            checks,
            head_commit: None,
        };

        let mut prs: Vec<Option<PullRequest>> = vec![None; tasks.len()];

        struct TaskExecContext {
            coder: Arc<dyn Coder>,
            events: EventBus,
            repo: Arc<Repository>,
            session: Arc<Session>,
            session_paths: Arc<SessionPaths>,
            request: Arc<RunRequest>,
        }

        fn spawn_task_job(
            join_set: &mut JoinSet<(usize, TaskSpec, anyhow::Result<PullRequest>)>,
            join_meta: &mut std::collections::HashMap<tokio::task::Id, (usize, TaskId)>,
            index: usize,
            task: TaskSpec,
            ctx: &TaskExecContext,
        ) {
            let coder = Arc::clone(&ctx.coder);
            let events = ctx.events.clone();
            let repo = Arc::clone(&ctx.repo);
            let session = Arc::clone(&ctx.session);
            let session_paths = Arc::clone(&ctx.session_paths);
            let request = Arc::clone(&ctx.request);

            let task_summary = TaskSummary {
                id: task.id.clone(),
                title: task.title.clone(),
            };
            let task_id = task.id.clone();

            let handle = join_set.spawn(async move {
                events.emit(RunEvent::TaskStarted { task: task_summary });
                let result = coder
                    .execute(
                        repo.as_ref(),
                        session.as_ref(),
                        session_paths.as_ref(),
                        request.as_ref(),
                        &task,
                    )
                    .await;
                (index, task, result)
            });
            join_meta.insert(handle.id(), (index, task_id));
        }

        let ctx = TaskExecContext {
            coder: Arc::clone(&self.coder),
            events: self.events.clone(),
            repo: Arc::clone(&repo),
            session: Arc::clone(&session),
            session_paths: Arc::clone(&session_paths),
            request: Arc::clone(&request),
        };

        let max = request.max_concurrency.max(1);
        let mut join_set: JoinSet<(usize, TaskSpec, anyhow::Result<PullRequest>)> = JoinSet::new();
        let mut join_meta: std::collections::HashMap<tokio::task::Id, (usize, TaskId)> =
            std::collections::HashMap::new();

        let mut next_index = 0usize;
        let mut in_flight = 0usize;

        while next_index < tasks.len() && in_flight < max {
            spawn_task_job(
                &mut join_set,
                &mut join_meta,
                next_index,
                tasks[next_index].clone(),
                &ctx,
            );
            next_index += 1;
            in_flight += 1;
        }

        while in_flight > 0 {
            let joined = join_set
                .join_next_with_id()
                .await
                .ok_or_else(|| anyhow::anyhow!("task join set ended unexpectedly"))?;
            in_flight -= 1;

            let (index, task, result) = match joined {
                Ok((id, value)) => {
                    join_meta.remove(&id);
                    value
                }
                Err(join_err) => {
                    let Some((index, task_id)) = join_meta.remove(&join_err.id()) else {
                        return Err(join_err.into());
                    };
                    let join_err_text = join_err.to_string();
                    warn!(
                        task_id = %task_id,
                        error = %join_err_text,
                        "task panicked or was cancelled"
                    );
                    let err = anyhow::anyhow!("task join error: {join_err_text}");
                    let checks = self
                        .task_failure_checks(session_paths.as_ref(), &task_id, &err)
                        .await;
                    let pr = failed_pr(&task_id, checks);
                    self.events.emit(RunEvent::TaskFinished {
                        pr: PullRequestSummary {
                            id: pr.id.clone(),
                            status: pr.status.clone(),
                            head_branch: pr.head_branch.clone(),
                            head_commit: pr.head_commit.clone(),
                        },
                    });
                    prs[index] = Some(pr);

                    while next_index < tasks.len() && in_flight < max {
                        spawn_task_job(
                            &mut join_set,
                            &mut join_meta,
                            next_index,
                            tasks[next_index].clone(),
                            &ctx,
                        );
                        next_index += 1;
                        in_flight += 1;
                    }

                    continue;
                }
            };

            let pr = match result {
                Ok(pr) => pr,
                Err(err) => {
                    warn!(task_id = %task.id, error = %err, "task failed");
                    let checks = self
                        .task_failure_checks(session_paths.as_ref(), &task.id, &err)
                        .await;
                    failed_pr(&task.id, checks)
                }
            };
            self.events.emit(RunEvent::TaskFinished {
                pr: PullRequestSummary {
                    id: pr.id.clone(),
                    status: pr.status.clone(),
                    head_branch: pr.head_branch.clone(),
                    head_commit: pr.head_commit.clone(),
                },
            });
            prs[index] = Some(pr);

            while next_index < tasks.len() && in_flight < max {
                spawn_task_job(
                    &mut join_set,
                    &mut join_meta,
                    next_index,
                    tasks[next_index].clone(),
                    &ctx,
                );
                next_index += 1;
                in_flight += 1;
            }
        }

        let prs = prs
            .into_iter()
            .enumerate()
            .map(|(index, pr)| {
                pr.ok_or_else(|| anyhow::anyhow!("missing PR result for task {}", tasks[index].id))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        Ok(prs)
    }

    async fn task_failure_checks(
        &self,
        session_paths: &SessionPaths,
        task_id: &crate::domain::TaskId,
        err: &anyhow::Error,
    ) -> CheckSummary {
        let artifacts_dir = session_paths.task_paths(task_id).artifacts_dir();
        let log_path = match tokio::fs::create_dir_all(&artifacts_dir).await {
            Ok(()) => {
                let error_log = artifacts_dir.join("error.log");
                let error_text = format!("{err:#}\n");
                match tokio::fs::write(&error_log, error_text).await {
                    Ok(()) => Some(error_log),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        };
        CheckSummary {
            steps: vec![StepSummary {
                name: "error".to_string(),
                ok: false,
                exit_code: None,
                log_path,
            }],
        }
    }

    async fn merge_failure_result(
        &self,
        session_paths: &SessionPaths,
        session: &Session,
        err: &anyhow::Error,
    ) -> MergeResult {
        let artifacts_dir = session_paths.merge_dir().join("artifacts");
        let log_path = match tokio::fs::create_dir_all(&artifacts_dir).await {
            Ok(()) => {
                let error_log = artifacts_dir.join("merge-error.log");
                let error_text = format!("{err:#}\n");
                match tokio::fs::write(&error_log, error_text).await {
                    Ok(()) => Some(error_log),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        };

        MergeResult {
            merged: false,
            base_branch: session.base_branch.clone(),
            merge_commit: None,
            merged_prs: Vec::new(),
            checks: CheckSummary {
                steps: vec![StepSummary {
                    name: "merge_error".to_string(),
                    ok: false,
                    exit_code: None,
                    log_path: log_path.clone(),
                }],
            },
            error: Some(format!("{err:#}")),
            error_log_path: log_path,
        }
    }

    async fn run_hook(
        &self,
        omne_paths: &PmPaths,
        session_paths: &SessionPaths,
        result: &RunResult,
        hook: &HookSpec,
    ) -> anyhow::Result<()> {
        self.hook_runner
            .run(hook, omne_paths, session_paths, result)
            .await?;
        info!(session_id = %result.session.id, "hook executed");
        Ok(())
    }

    async fn write_result_artifacts(
        &self,
        session_paths: &SessionPaths,
        result: &RunResult,
    ) -> anyhow::Result<()> {
        let result_json = serde_json::to_vec_pretty(result)?;
        tokio::fs::write(session_paths.root().join("result.json"), &result_json).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
