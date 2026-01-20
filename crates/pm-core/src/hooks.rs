use async_trait::async_trait;

use crate::domain::{HookSpec, RunResult};
use crate::paths::{PmPaths, SessionPaths};

#[async_trait]
pub trait HookRunner: Send + Sync {
    async fn run(
        &self,
        hook: &HookSpec,
        pm_paths: &PmPaths,
        session_paths: &SessionPaths,
        result: &RunResult,
    ) -> anyhow::Result<()>;
}

#[derive(Clone, Debug, Default)]
pub struct NoopHookRunner;

#[async_trait]
impl HookRunner for NoopHookRunner {
    async fn run(
        &self,
        _hook: &HookSpec,
        _pm_paths: &PmPaths,
        _session_paths: &SessionPaths,
        _result: &RunResult,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct CommandHookRunner;

#[async_trait]
impl HookRunner for CommandHookRunner {
    async fn run(
        &self,
        hook: &HookSpec,
        pm_paths: &PmPaths,
        session_paths: &SessionPaths,
        result: &RunResult,
    ) -> anyhow::Result<()> {
        let HookSpec::Command { program, args } = hook;

        let session = &result.session;
        let pm_session_dir = pm_paths.session_dir(session.id);
        let tmp_session_dir = session_paths.root();
        let result_json = tmp_session_dir.join("result.json");

        let status = tokio::process::Command::new(program)
            .args(args)
            .env("CODE_PM_SESSION_ID", session.id.to_string())
            .env("CODE_PM_REPO", session.repo.as_str())
            .env("CODE_PM_PR_NAME", session.pr_name.as_str())
            .env("CODE_PM_PM_ROOT", pm_paths.root().as_os_str())
            .env("CODE_PM_SESSION_DIR", pm_session_dir.as_os_str())
            .env("CODE_PM_TMP_DIR", tmp_session_dir.as_os_str())
            .env("CODE_PM_RESULT_JSON", result_json.as_os_str())
            .env(
                "CODE_PM_MERGED",
                if result.merge.merged { "1" } else { "0" },
            )
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!("hook command failed with status {status}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use time::OffsetDateTime;

    use super::*;
    use crate::domain::{
        CheckSummary, MergeResult, PrName, PullRequest, RepositoryName, RunResult, Session,
        SessionId,
    };

    #[tokio::test]
    async fn command_hook_runner_exports_expected_env_vars() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(dir.path().join(".code_pm"));

        let repo = RepositoryName::sanitize("repo");
        let session_id = SessionId::new();
        let session_paths = SessionPaths::new(&repo, session_id);
        tokio::fs::create_dir_all(session_paths.root()).await?;

        let result_json = session_paths.root().join("result.json");
        tokio::fs::write(&result_json, b"{}").await?;

        let session = Session {
            id: session_id,
            repo: repo.clone(),
            pr_name: PrName::sanitize("pr"),
            prompt: "x".to_string(),
            base_branch: "main".to_string(),
            created_at: OffsetDateTime::now_utc(),
        };

        let result = RunResult {
            session,
            tasks: Vec::new(),
            prs: Vec::<PullRequest>::new(),
            merge: MergeResult {
                merged: true,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        };

        let out_path = session_paths.root().join("hook-env.txt");
        let script = format!(
            "printf '%s\\n' \\\n  \"$CODE_PM_SESSION_ID\" \\\n  \"$CODE_PM_REPO\" \\\n  \"$CODE_PM_PR_NAME\" \\\n  \"$CODE_PM_PM_ROOT\" \\\n  \"$CODE_PM_SESSION_DIR\" \\\n  \"$CODE_PM_TMP_DIR\" \\\n  \"$CODE_PM_RESULT_JSON\" \\\n  \"$CODE_PM_MERGED\" \\\n  > '{}'",
            out_path.display()
        );

        let hook = HookSpec::Command {
            program: PathBuf::from("sh"),
            args: vec!["-c".to_string(), script],
        };

        let runner = CommandHookRunner;
        runner
            .run(&hook, &pm_paths, &session_paths, &result)
            .await?;

        let text = tokio::fs::read_to_string(&out_path).await?;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8);

        assert_eq!(lines[0], result.session.id.to_string());
        assert_eq!(lines[1], result.session.repo.as_str());
        assert_eq!(lines[2], result.session.pr_name.as_str());
        assert_eq!(lines[3], pm_paths.root().display().to_string());
        assert_eq!(
            lines[4],
            pm_paths
                .session_dir(result.session.id)
                .display()
                .to_string()
        );
        assert_eq!(lines[5], session_paths.root().display().to_string());
        assert_eq!(lines[6], result_json.display().to_string());
        assert_eq!(lines[7], "1");

        let _ = tokio::fs::remove_dir_all(session_paths.root()).await;
        Ok(())
    }
}
