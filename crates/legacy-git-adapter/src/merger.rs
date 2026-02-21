use std::path::Path;

use anyhow::Context;
use async_trait::async_trait;
use omne_core::{
    CheckSummary, MergeResult, PullRequest, PullRequestStatus, Repository, Session, SessionPaths,
    StepSummary,
};
use tracing::info;

use crate::checks::{os_arg, run_git_capture, run_git_step};
use crate::git::GitCli;
use crate::identity::ensure_git_identity;
use crate::lock::lock_exclusive;

#[derive(Clone, Debug)]
pub struct GitMerger {
    git: GitCli,
}

impl GitMerger {
    pub fn new() -> Self {
        Self { git: GitCli }
    }

    async fn ensure_merge_repo(session_paths: &SessionPaths) -> anyhow::Result<std::path::PathBuf> {
        let merge_root = session_paths.merge_dir();
        tokio::fs::create_dir_all(&merge_root).await?;
        Ok(merge_root.join("repo"))
    }

    async fn failed_merge_result(
        base_branch: &str,
        mut checks: CheckSummary,
        artifacts_dir: &Path,
        err: anyhow::Error,
    ) -> MergeResult {
        let _ = tokio::fs::create_dir_all(artifacts_dir).await;
        let error_log = artifacts_dir.join("merge-error.log");
        let error_log_path = match tokio::fs::write(&error_log, format!("{err:#}\n")).await {
            Ok(()) => Some(error_log),
            Err(_) => None,
        };

        checks.steps.push(StepSummary {
            name: "merge_error".to_string(),
            ok: false,
            exit_code: None,
            log_path: error_log_path.clone(),
        });

        MergeResult {
            merged: false,
            base_branch: base_branch.to_string(),
            merge_commit: None,
            merged_prs: Vec::new(),
            checks,
            error: Some(format!("{err:#}")),
            error_log_path,
        }
    }
}

impl Default for GitMerger {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl omne_core::Merger for GitMerger {
    async fn merge(
        &self,
        repo: &Repository,
        session: &Session,
        session_paths: &SessionPaths,
        prs: &[PullRequest],
    ) -> anyhow::Result<MergeResult> {
        let ready: Vec<&PullRequest> = prs
            .iter()
            .filter(|pr| matches!(pr.status, PullRequestStatus::Ready))
            .collect();
        if ready.is_empty() {
            return Ok(MergeResult {
                merged: false,
                base_branch: session.base_branch.clone(),
                merge_commit: None,
                merged_prs: Vec::new(),
                ..Default::default()
            });
        }

        let artifacts_dir = session_paths.merge_dir().join("artifacts");
        let _repo_lock = match lock_exclusive(&repo.lock_path).await {
            Ok(lock) => lock,
            Err(err) => {
                return Ok(Self::failed_merge_result(
                    &session.base_branch,
                    CheckSummary::default(),
                    &artifacts_dir,
                    err,
                )
                .await);
            }
        };

        let merge_repo_dir = Self::ensure_merge_repo(session_paths).await?;
        let merge_root = merge_repo_dir
            .parent()
            .context("merge repo dir has no parent")?;

        if tokio::fs::try_exists(&merge_repo_dir)
            .await
            .unwrap_or(false)
        {
            let _ = tokio::fs::remove_dir_all(&merge_repo_dir).await;
        }

        tokio::fs::create_dir_all(&artifacts_dir).await?;
        let mut checks = CheckSummary::default();

        let clone_log = artifacts_dir.join("git-clone.log");
        let clone_args = vec![
            os_arg("clone"),
            os_arg("--shared"),
            os_arg(repo.bare_path.as_path()),
            os_arg(merge_repo_dir.as_path()),
        ];
        let clone_step =
            match run_git_step(&self.git, merge_root, "git_clone", &clone_args, &clone_log).await {
                Ok(step) => step,
                Err(err) => {
                    return Ok(Self::failed_merge_result(
                        &session.base_branch,
                        checks,
                        &artifacts_dir,
                        err,
                    )
                    .await);
                }
            };
        let clone_ok = clone_step.ok;
        checks.steps.push(clone_step);
        if !clone_ok {
            return Ok(Self::failed_merge_result(
                &session.base_branch,
                checks,
                &artifacts_dir,
                anyhow::anyhow!("git clone failed; see {}", clone_log.display()),
            )
            .await);
        }

        let identity_steps =
            match ensure_git_identity(&self.git, &merge_repo_dir, &artifacts_dir).await {
                Ok(steps) => steps,
                Err(err) => {
                    return Ok(Self::failed_merge_result(
                        &session.base_branch,
                        checks,
                        &artifacts_dir,
                        err,
                    )
                    .await);
                }
            };
        let identity_ok = identity_steps.iter().all(|step| step.ok);
        checks.steps.extend(identity_steps);
        if !identity_ok {
            return Ok(Self::failed_merge_result(
                &session.base_branch,
                checks,
                &artifacts_dir,
                anyhow::anyhow!(
                    "git identity configuration failed; see logs under {}",
                    artifacts_dir.display()
                ),
            )
            .await);
        }

        let base_ref = format!("origin/{}", session.base_branch);
        let checkout_log = artifacts_dir.join("git-checkout-base.log");
        let checkout_args = vec![
            os_arg("checkout"),
            os_arg("-B"),
            os_arg(session.base_branch.as_str()),
            os_arg(base_ref.as_str()),
        ];
        let checkout_step = match run_git_step(
            &self.git,
            &merge_repo_dir,
            "git_checkout_base",
            &checkout_args,
            &checkout_log,
        )
        .await
        {
            Ok(step) => step,
            Err(err) => {
                return Ok(Self::failed_merge_result(
                    &session.base_branch,
                    checks,
                    &artifacts_dir,
                    err,
                )
                .await);
            }
        };
        let checkout_ok = checkout_step.ok;
        checks.steps.push(checkout_step);
        if !checkout_ok {
            return Ok(Self::failed_merge_result(
                &session.base_branch,
                checks,
                &artifacts_dir,
                anyhow::anyhow!("git checkout base failed; see {}", checkout_log.display()),
            )
            .await);
        }

        let mut merged_prs = Vec::new();
        for pr in ready {
            let remote_ref = format!("origin/{}", pr.head_branch);
            info!(pr = %pr.head_branch, "merging PR branch");
            let merge_log = artifacts_dir.join(format!("git-merge-{}.log", pr.id.as_str()));
            let step_name = format!("git_merge_{}", pr.id.as_str());
            let merge_args = vec![
                os_arg("merge"),
                os_arg("--no-ff"),
                os_arg("--no-edit"),
                os_arg(remote_ref.as_str()),
            ];
            let merge_step = match run_git_step(
                &self.git,
                &merge_repo_dir,
                &step_name,
                &merge_args,
                &merge_log,
            )
            .await
            {
                Ok(step) => step,
                Err(err) => {
                    return Ok(Self::failed_merge_result(
                        &session.base_branch,
                        checks,
                        &artifacts_dir,
                        err,
                    )
                    .await);
                }
            };
            let merge_ok = merge_step.ok;
            checks.steps.push(merge_step);
            if !merge_ok {
                return Ok(Self::failed_merge_result(
                    &session.base_branch,
                    checks,
                    &artifacts_dir,
                    anyhow::anyhow!(
                        "git merge failed for {}; see {}",
                        pr.id,
                        merge_log.display()
                    ),
                )
                .await);
            }
            merged_prs.push(pr.id.clone());
        }

        let push_ref = format!("HEAD:{}", session.base_branch);
        let push_log = artifacts_dir.join("git-push-base.log");
        let push_args = vec![os_arg("push"), os_arg("origin"), os_arg(push_ref.as_str())];
        let push_step = match run_git_step(
            &self.git,
            &merge_repo_dir,
            "git_push_base",
            &push_args,
            &push_log,
        )
        .await
        {
            Ok(step) => step,
            Err(err) => {
                return Ok(Self::failed_merge_result(
                    &session.base_branch,
                    checks,
                    &artifacts_dir,
                    err,
                )
                .await);
            }
        };
        let push_ok = push_step.ok;
        checks.steps.push(push_step);
        if !push_ok {
            return Ok(Self::failed_merge_result(
                &session.base_branch,
                checks,
                &artifacts_dir,
                anyhow::anyhow!("git push failed; see {}", push_log.display()),
            )
            .await);
        }

        let rev_log = artifacts_dir.join("git-rev-parse-head.log");
        let rev_args = vec![os_arg("rev-parse"), os_arg("HEAD")];
        let (rev_step, rev_output) = match run_git_capture(
            &self.git,
            &merge_repo_dir,
            "git_rev_parse_head",
            &rev_args,
            &rev_log,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                return Ok(Self::failed_merge_result(
                    &session.base_branch,
                    checks,
                    &artifacts_dir,
                    err,
                )
                .await);
            }
        };
        checks.steps.push(rev_step);
        if !rev_output.ok {
            return Ok(Self::failed_merge_result(
                &session.base_branch,
                checks,
                &artifacts_dir,
                anyhow::anyhow!(
                    "git rev-parse HEAD failed after merge; see {}",
                    rev_log.display()
                ),
            )
            .await);
        }
        let merge_commit = Some(rev_output.stdout.trim().to_string());

        Ok(MergeResult {
            merged: true,
            base_branch: session.base_branch.clone(),
            merge_commit,
            merged_prs,
            checks,
            error: None,
            error_log_path: None,
        })
    }
}
