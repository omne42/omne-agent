use std::path::Path;

use async_trait::async_trait;
use omne_core::{
    CheckSummary, PullRequest, PullRequestStatus, Repository, RunRequest, Session, SessionPaths,
    StepSummary, TaskPaths, TaskSpec,
};
use tracing::info;

use crate::checks::{os_arg, run_cargo_step, run_git_capture, run_git_step};
use crate::git::GitCli;
use crate::identity::ensure_git_identity;
use crate::lock::{lock_exclusive, lock_shared};
use crate::repo::is_rust_repo;

struct FailedPrContext<'a> {
    task_paths: &'a TaskPaths,
    task: &'a TaskSpec,
    branch: &'a str,
    base_branch: &'a str,
    artifacts_dir: &'a Path,
}

#[derive(Clone, Debug)]
pub struct GitCoder {
    git: GitCli,
}

impl GitCoder {
    pub fn new() -> Self {
        Self { git: GitCli }
    }

    fn branch_name(session: &Session, task: &TaskSpec) -> String {
        format!(
            "ai/{}/{}/{}",
            session.pr_name.as_str(),
            session.id,
            task.id.as_str()
        )
    }

    async fn ensure_task_dirs(task_paths: &TaskPaths) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(task_paths.root()).await?;
        tokio::fs::create_dir_all(task_paths.artifacts_dir()).await?;
        Ok(())
    }

    async fn write_task_json(
        task_paths: &TaskPaths,
        task: &TaskSpec,
        pr: &PullRequest,
    ) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(&serde_json::json!({
            "task": task,
            "pr": pr,
        }))?;
        tokio::fs::write(task_paths.root().join("task.json"), json).await?;
        Ok(())
    }

    async fn failed_pr(
        ctx: &FailedPrContext<'_>,
        mut checks: CheckSummary,
        err: anyhow::Error,
        head_commit: Option<String>,
    ) -> anyhow::Result<PullRequest> {
        let error_log = ctx.artifacts_dir.join("error.log");
        let log_path = match tokio::fs::write(&error_log, format!("{err:#}\n")).await {
            Ok(()) => Some(error_log),
            Err(_) => None,
        };

        checks.steps.push(StepSummary {
            name: "error".to_string(),
            ok: false,
            exit_code: None,
            log_path,
        });

        let pr = PullRequest {
            id: ctx.task.id.clone(),
            head_branch: ctx.branch.to_string(),
            base_branch: ctx.base_branch.to_string(),
            status: PullRequestStatus::Failed,
            checks,
            head_commit,
        };
        Self::write_task_json(ctx.task_paths, ctx.task, &pr).await?;
        Ok(pr)
    }
}

#[async_trait]
impl omne_core::Coder for GitCoder {
    async fn execute(
        &self,
        repo: &Repository,
        session: &Session,
        session_paths: &SessionPaths,
        request: &RunRequest,
        task: &TaskSpec,
    ) -> anyhow::Result<PullRequest> {
        let task_paths = session_paths.task_paths(&task.id);
        Self::ensure_task_dirs(&task_paths).await?;

        let repo_dir = task_paths.repo_dir();
        let artifacts_dir = task_paths.artifacts_dir();

        let branch = Self::branch_name(session, task);
        let clone_log = artifacts_dir.join("git-clone.log");
        let branch_log = artifacts_dir.join("git-branch.log");
        let status_log = artifacts_dir.join("git-status.log");

        let fail_ctx = FailedPrContext {
            task_paths: &task_paths,
            task,
            branch: branch.as_str(),
            base_branch: session.base_branch.as_str(),
            artifacts_dir: artifacts_dir.as_path(),
        };

        let mut checks = CheckSummary::default();
        let head_commit = None;

        let clone_args = vec![
            os_arg("clone"),
            os_arg("--shared"),
            os_arg(repo.bare_path.as_path()),
            os_arg(repo_dir.as_path()),
        ];
        let _repo_read_lock = match lock_shared(&repo.lock_path).await {
            Ok(lock) => lock,
            Err(err) => {
                return Self::failed_pr(&fail_ctx, checks, err, head_commit).await;
            }
        };
        let clone_step = match run_git_step(
            &self.git,
            session_paths.root(),
            "git_clone",
            &clone_args,
            &clone_log,
        )
        .await
        {
            Ok(step) => step,
            Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
        };
        let clone_ok = clone_step.ok;
        checks.steps.push(clone_step);
        if !clone_ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git clone failed; see {}", clone_log.display()),
                head_commit,
            )
            .await;
        }
        drop(_repo_read_lock);

        let base_ref = format!("origin/{}", session.base_branch);
        let checkout_args = vec![
            os_arg("checkout"),
            os_arg("-B"),
            os_arg(branch.as_str()),
            os_arg(base_ref.as_str()),
        ];
        let branch_step = match run_git_step(
            &self.git,
            &repo_dir,
            "git_checkout_branch",
            &checkout_args,
            &branch_log,
        )
        .await
        {
            Ok(step) => step,
            Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
        };
        let branch_ok = branch_step.ok;
        checks.steps.push(branch_step);
        if !branch_ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git checkout failed; see {}", branch_log.display()),
                head_commit,
            )
            .await;
        }

        let identity_steps = match ensure_git_identity(&self.git, &repo_dir, &artifacts_dir).await {
            Ok(steps) => steps,
            Err(err) => {
                return Self::failed_pr(&fail_ctx, checks, err, head_commit).await;
            }
        };
        let identity_ok = identity_steps.iter().all(|step| step.ok);
        checks.steps.extend(identity_steps);
        if !identity_ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!(
                    "git identity configuration failed; see logs under {}",
                    artifacts_dir.display()
                ),
                head_commit,
            )
            .await;
        }

        if let Some(patch_path) = request.apply_patch.as_ref() {
            let patch_log = artifacts_dir.join("git-apply.log");
            let apply_args = vec![os_arg("apply"), os_arg(patch_path.as_path())];
            let step = match run_git_step(
                &self.git,
                &repo_dir,
                "git_apply",
                &apply_args,
                &patch_log,
            )
            .await
            {
                Ok(step) => step,
                Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
            };
            let ok = step.ok;
            checks.steps.push(step);
            if !ok {
                return Self::failed_pr(
                    &fail_ctx,
                    checks,
                    anyhow::anyhow!("git apply failed; see {}", patch_log.display()),
                    head_commit,
                )
                .await;
            }
        }

        if is_rust_repo(&repo_dir) {
            let fmt_log = artifacts_dir.join("cargo-fmt.log");
            let check_log = artifacts_dir.join("cargo-check.log");
            let test_log = artifacts_dir.join("cargo-test.log");
            let cargo_target_dir = task_paths.cargo_target_dir();
            let had_cargo_lock = repo_dir.join("Cargo.lock").is_file();

            if let Err(err) = tokio::fs::create_dir_all(&cargo_target_dir).await {
                return Self::failed_pr(&fail_ctx, checks, err.into(), head_commit).await;
            }

            let fmt_step = match run_cargo_step(
                &repo_dir,
                "cargo_fmt",
                &["fmt", "--all"],
                &fmt_log,
                Some(&cargo_target_dir),
            )
            .await
            {
                Ok(step) => step,
                Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
            };
            let fmt_ok = fmt_step.ok;
            checks.steps.push(fmt_step);
            if !fmt_ok {
                return Self::failed_pr(
                    &fail_ctx,
                    checks,
                    anyhow::anyhow!("cargo fmt failed; see {}", fmt_log.display()),
                    head_commit,
                )
                .await;
            }

            let check_step = match run_cargo_step(
                &repo_dir,
                "cargo_check",
                &["check", "--workspace", "--all-targets"],
                &check_log,
                Some(&cargo_target_dir),
            )
            .await
            {
                Ok(step) => step,
                Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
            };
            let check_ok = check_step.ok;
            checks.steps.push(check_step);
            if !check_ok {
                return Self::failed_pr(
                    &fail_ctx,
                    checks,
                    anyhow::anyhow!("cargo check failed; see {}", check_log.display()),
                    head_commit,
                )
                .await;
            }

            if request.cargo_test {
                let test_step = match run_cargo_step(
                    &repo_dir,
                    "cargo_test",
                    &["test", "--workspace", "--all-targets"],
                    &test_log,
                    Some(&cargo_target_dir),
                )
                .await
                {
                    Ok(step) => step,
                    Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
                };
                let test_ok = test_step.ok;
                checks.steps.push(test_step);
                if !test_ok {
                    return Self::failed_pr(
                        &fail_ctx,
                        checks,
                        anyhow::anyhow!("cargo test failed; see {}", test_log.display()),
                        head_commit,
                    )
                    .await;
                }
            }

            if !had_cargo_lock {
                let cargo_lock_path = repo_dir.join("Cargo.lock");
                if cargo_lock_path.is_file() {
                    let cleanup_log = artifacts_dir.join("cargo-cleanup-cargo-lock.log");
                    if let Err(err) = tokio::fs::remove_file(&cargo_lock_path).await {
                        let _ = tokio::fs::write(&cleanup_log, format!("{err:#}\n")).await;
                        checks.steps.push(StepSummary {
                            name: "cargo_cleanup_cargo_lock".to_string(),
                            ok: false,
                            exit_code: None,
                            log_path: Some(cleanup_log),
                        });
                        return Self::failed_pr(
                            &fail_ctx,
                            checks,
                            anyhow::anyhow!(
                                "failed to remove untracked Cargo.lock; see {}",
                                artifacts_dir.display()
                            ),
                            head_commit,
                        )
                        .await;
                    }
                }
            }
        }

        let status_args = vec![os_arg("status"), os_arg("--porcelain")];
        let (status_step, status_output) = match run_git_capture(
            &self.git,
            &repo_dir,
            "git_status",
            &status_args,
            &status_log,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
        };
        checks.steps.push(status_step);
        if !status_output.ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git status failed; see {}", status_log.display()),
                head_commit,
            )
            .await;
        }

        if status_output.stdout.trim().is_empty() {
            let pr = PullRequest {
                id: task.id.clone(),
                head_branch: branch.clone(),
                base_branch: session.base_branch.clone(),
                status: PullRequestStatus::NoChanges,
                checks,
                head_commit: None,
            };
            Self::write_task_json(&task_paths, task, &pr).await?;
            return Ok(pr);
        }

        let add_log = artifacts_dir.join("git-add.log");
        let add_args = vec![os_arg("add"), os_arg("-A")];
        let add_step =
            match run_git_step(&self.git, &repo_dir, "git_add", &add_args, &add_log).await {
                Ok(step) => step,
                Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
            };
        let add_ok = add_step.ok;
        checks.steps.push(add_step);
        if !add_ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git add failed; see {}", add_log.display()),
                head_commit,
            )
            .await;
        }

        let message = format!(
            "chore(ai): {} ({})",
            session.pr_name.as_str(),
            task.id.as_str()
        );

        let commit_log = artifacts_dir.join("git-commit.log");
        let commit_args = vec![os_arg("commit"), os_arg("-m"), os_arg(message.as_str())];
        let commit_step = match run_git_step(
            &self.git,
            &repo_dir,
            "git_commit",
            &commit_args,
            &commit_log,
        )
        .await
        {
            Ok(step) => step,
            Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
        };
        let commit_ok = commit_step.ok;
        checks.steps.push(commit_step);
        if !commit_ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git commit failed; see {}", commit_log.display()),
                head_commit,
            )
            .await;
        }

        let rev_log = artifacts_dir.join("git-rev-parse-head.log");
        let rev_args = vec![os_arg("rev-parse"), os_arg("HEAD")];
        let (rev_step, rev_output) = match run_git_capture(
            &self.git,
            &repo_dir,
            "git_rev_parse_head",
            &rev_args,
            &rev_log,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
        };
        checks.steps.push(rev_step);
        if !rev_output.ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git rev-parse HEAD failed; see {}", rev_log.display()),
                head_commit,
            )
            .await;
        }
        let head_commit = Some(rev_output.stdout.trim().to_string());

        let push_log = artifacts_dir.join("git-push.log");
        let push_args = vec![os_arg("push"), os_arg("origin"), os_arg(branch.as_str())];
        let _repo_lock = match lock_exclusive(&repo.lock_path).await {
            Ok(lock) => lock,
            Err(err) => {
                return Self::failed_pr(&fail_ctx, checks, err, head_commit).await;
            }
        };
        let push_step =
            match run_git_step(&self.git, &repo_dir, "git_push", &push_args, &push_log).await {
                Ok(step) => step,
                Err(err) => return Self::failed_pr(&fail_ctx, checks, err, head_commit).await,
            };
        let push_ok = push_step.ok;
        checks.steps.push(push_step);
        if !push_ok {
            return Self::failed_pr(
                &fail_ctx,
                checks,
                anyhow::anyhow!("git push failed; see {}", push_log.display()),
                head_commit,
            )
            .await;
        }

        info!(task_id = %task.id, branch = %branch, "task committed and pushed");

        let pr = PullRequest {
            id: task.id.clone(),
            head_branch: branch.clone(),
            base_branch: session.base_branch.clone(),
            status: PullRequestStatus::Ready,
            checks,
            head_commit,
        };
        Self::write_task_json(&task_paths, task, &pr).await?;
        Ok(pr)
    }
}

impl Default for GitCoder {
    fn default() -> Self {
        Self::new()
    }
}
