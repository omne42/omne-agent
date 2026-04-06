use async_trait::async_trait;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

use crate::paths::{PmPaths, SessionPaths};
use crate::run::{HookSpec, RunResult};

const HOOK_SCRUBBED_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "OMNE_OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENROUTER_API_KEY",
    "GEMINI_API_KEY",
];

fn scrub_command_hook_env(cmd: &mut tokio::process::Command) {
    for key in HOOK_SCRUBBED_ENV_KEYS {
        cmd.env_remove(key);
    }
}

#[async_trait]
pub trait HookRunner: Send + Sync {
    async fn run(
        &self,
        hook: &HookSpec,
        omne_paths: &PmPaths,
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
        omne_paths: &PmPaths,
        session_paths: &SessionPaths,
        result: &RunResult,
    ) -> anyhow::Result<()> {
        let (program, args) = match hook {
            HookSpec::Command { program, args } => (program, args),
            HookSpec::Webhook { .. } => {
                anyhow::bail!("unsupported hook spec: webhook (expected command hook)")
            }
        };

        let session = &result.session;
        let omne_session_dir = omne_paths.session_dir(session.id);
        let tmp_session_dir = session_paths.root();
        let result_json = tmp_session_dir.join("result.json");

        let logs_dir = session_paths.logs_dir();
        tokio::fs::create_dir_all(&logs_dir).await?;
        let stdout_log = logs_dir.join("hook.stdout.log");
        let stderr_log = logs_dir.join("hook.stderr.log");

        let mut cmd = tokio::process::Command::new(program);
        scrub_command_hook_env(&mut cmd);
        let mut child = cmd
            .args(args)
            .env("OMNE_SESSION_ID", session.id.to_string())
            .env("OMNE_REPO", session.repo.as_str())
            .env("OMNE_PR_NAME", session.pr_name.as_str())
            .env("OMNE_PM_ROOT", omne_paths.root().as_os_str())
            .env("OMNE_SESSION_DIR", omne_session_dir.as_os_str())
            .env("OMNE_TMP_DIR", tmp_session_dir.as_os_str())
            .env("OMNE_RESULT_JSON", result_json.as_os_str())
            .env("OMNE_MERGED", if result.merge.merged { "1" } else { "0" })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing child stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing child stderr"))?;

        let stdout_log_task_path = stdout_log.clone();
        let stderr_log_task_path = stderr_log.clone();

        let stdout_task = tokio::spawn(async move {
            let mut stdout = stdout;
            let mut file = tokio::fs::File::create(&stdout_log_task_path).await?;
            tokio::io::copy(&mut stdout, &mut file).await?;
            file.flush().await?;
            Ok::<(), anyhow::Error>(())
        });
        let stderr_task = tokio::spawn(async move {
            let mut stderr = stderr;
            let mut file = tokio::fs::File::create(&stderr_log_task_path).await?;
            tokio::io::copy(&mut stderr, &mut file).await?;
            file.flush().await?;
            Ok::<(), anyhow::Error>(())
        });

        let status = child.wait().await?;
        stdout_task.await??;
        stderr_task.await??;

        if !status.success() {
            anyhow::bail!(
                "hook command failed with status {status}; logs: {} / {}",
                stdout_log.display(),
                stderr_log.display()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use time::OffsetDateTime;
    use tokio::sync::Mutex;

    use super::*;
    use crate::domain::{PrName, RepositoryName, Session, SessionId};
    use crate::run::{CheckSummary, MergeResult, PullRequest, RunResult};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard;

    impl EnvGuard {
        fn set(vars: &[(&'static str, &'static str)]) -> Self {
            for (key, value) in vars {
                // Tests serialize env mutation with `env_lock`.
                unsafe { std::env::set_var(key, value) };
            }
            Self
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in HOOK_SCRUBBED_ENV_KEYS {
                // Tests serialize env mutation with `env_lock`.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    struct TestContext {
        _tmp: tempfile::TempDir,
        omne_paths: PmPaths,
        session_paths: SessionPaths,
        result: RunResult,
    }

    async fn setup() -> anyhow::Result<TestContext> {
        let tmp = tempfile::tempdir()?;
        let omne_paths = PmPaths::new(tmp.path().join(".omne_data"));

        let repo = RepositoryName::sanitize("repo");
        let session_id = SessionId::new();
        let session_paths = SessionPaths::new_in(tmp.path().join("tmp"), &repo, session_id);
        tokio::fs::create_dir_all(session_paths.root()).await?;
        tokio::fs::write(session_paths.root().join("result.json"), b"{}").await?;

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

        Ok(TestContext {
            _tmp: tmp,
            omne_paths,
            session_paths,
            result,
        })
    }

    #[tokio::test]
    async fn command_hook_runner_exports_expected_env_vars() -> anyhow::Result<()> {
        let ctx = setup().await?;
        let result_json = ctx.session_paths.root().join("result.json");

        let out_path = ctx.session_paths.root().join("hook-env.txt");
        let script = format!(
            "printf '%s\\n' \\\n  \"$OMNE_SESSION_ID\" \\\n  \"$OMNE_REPO\" \\\n  \"$OMNE_PR_NAME\" \\\n  \"$OMNE_PM_ROOT\" \\\n  \"$OMNE_SESSION_DIR\" \\\n  \"$OMNE_TMP_DIR\" \\\n  \"$OMNE_RESULT_JSON\" \\\n  \"$OMNE_MERGED\" \\\n  > '{}'",
            out_path.display()
        );

        let hook = HookSpec::Command {
            program: PathBuf::from("sh"),
            args: vec!["-c".to_string(), script],
        };

        let runner = CommandHookRunner;
        runner
            .run(&hook, &ctx.omne_paths, &ctx.session_paths, &ctx.result)
            .await?;

        let hook_stdout_log = ctx.session_paths.logs_dir().join("hook.stdout.log");
        let hook_stderr_log = ctx.session_paths.logs_dir().join("hook.stderr.log");
        assert!(tokio::fs::try_exists(&hook_stdout_log).await?);
        assert!(tokio::fs::try_exists(&hook_stderr_log).await?);

        let text = tokio::fs::read_to_string(&out_path).await?;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 8);

        assert_eq!(lines[0], ctx.result.session.id.to_string());
        assert_eq!(lines[1], ctx.result.session.repo.as_str());
        assert_eq!(lines[2], ctx.result.session.pr_name.as_str());
        assert_eq!(lines[3], ctx.omne_paths.root().display().to_string());
        assert_eq!(
            lines[4],
            ctx.omne_paths
                .session_dir(ctx.result.session.id)
                .display()
                .to_string()
        );
        assert_eq!(lines[5], ctx.session_paths.root().display().to_string());
        assert_eq!(lines[6], result_json.display().to_string());
        assert_eq!(lines[7], "1");

        Ok(())
    }

    #[tokio::test]
    async fn command_hook_runner_captures_stdout_and_stderr() -> anyhow::Result<()> {
        let ctx = setup().await?;

        let hook = HookSpec::Command {
            program: PathBuf::from("sh"),
            args: vec![
                "-c".to_string(),
                "printf 'hello stdout\\n'; printf 'hello stderr\\n' 1>&2".to_string(),
            ],
        };

        let runner = CommandHookRunner;
        runner
            .run(&hook, &ctx.omne_paths, &ctx.session_paths, &ctx.result)
            .await?;

        let stdout_log = ctx.session_paths.logs_dir().join("hook.stdout.log");
        let stderr_log = ctx.session_paths.logs_dir().join("hook.stderr.log");
        assert_eq!(
            tokio::fs::read_to_string(&stdout_log).await?,
            "hello stdout\n"
        );
        assert_eq!(
            tokio::fs::read_to_string(&stderr_log).await?,
            "hello stderr\n"
        );

        Ok(())
    }

    #[tokio::test]
    async fn command_hook_runner_scrubs_sensitive_provider_env_vars() -> anyhow::Result<()> {
        let _env_guard = env_lock().lock().await;
        let _vars = EnvGuard::set(&[
            ("OPENAI_API_KEY", "openai-secret"),
            ("OMNE_OPENAI_API_KEY", "omne-openai-secret"),
            ("ANTHROPIC_API_KEY", "anthropic-secret"),
            ("OPENROUTER_API_KEY", "openrouter-secret"),
            ("GEMINI_API_KEY", "gemini-secret"),
        ]);
        let ctx = setup().await?;
        let out_path = ctx.session_paths.root().join("hook-sensitive-env.txt");
        let script = format!(
            "printf '%s\\n' \\\n  \"$OPENAI_API_KEY\" \\\n  \"$OMNE_OPENAI_API_KEY\" \\\n  \"$ANTHROPIC_API_KEY\" \\\n  \"$OPENROUTER_API_KEY\" \\\n  \"$GEMINI_API_KEY\" \\\n  > '{}'",
            out_path.display()
        );

        let hook = HookSpec::Command {
            program: PathBuf::from("sh"),
            args: vec!["-c".to_string(), script],
        };

        let runner = CommandHookRunner;
        runner
            .run(&hook, &ctx.omne_paths, &ctx.session_paths, &ctx.result)
            .await?;

        let text = tokio::fs::read_to_string(&out_path).await?;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines, vec!["", "", "", "", ""]);

        Ok(())
    }

    #[tokio::test]
    async fn command_hook_runner_writes_logs_on_failure() -> anyhow::Result<()> {
        let ctx = setup().await?;

        let hook = HookSpec::Command {
            program: PathBuf::from("sh"),
            args: vec![
                "-c".to_string(),
                "printf 'goodbye stderr\\n' 1>&2; exit 7".to_string(),
            ],
        };

        let runner = CommandHookRunner;
        let err = runner
            .run(&hook, &ctx.omne_paths, &ctx.session_paths, &ctx.result)
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("hook command failed"));
        assert!(msg.contains("hook.stdout.log"));
        assert!(msg.contains("hook.stderr.log"));

        let stderr_log = ctx.session_paths.logs_dir().join("hook.stderr.log");
        assert_eq!(
            tokio::fs::read_to_string(&stderr_log).await?,
            "goodbye stderr\n"
        );

        Ok(())
    }
}
