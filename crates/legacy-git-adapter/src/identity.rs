use std::path::Path;

use omne_core::StepSummary;

use crate::checks::{os_arg, write_command_error_log};
use crate::git::{CommandOutput, GitCli};

fn treat_git_config_missing_as_ok(output: &CommandOutput) -> bool {
    output.exit_code == Some(1)
        && output.stdout.trim().is_empty()
        && output.stderr.trim().is_empty()
}

async fn run_git_logged(
    git: &GitCli,
    repo_dir: &Path,
    args: &[&str],
    log_path: &Path,
) -> CommandOutput {
    let args_os: Vec<std::ffi::OsString> = args.iter().map(|arg| os_arg(*arg)).collect();
    match git.run(repo_dir, &args_os, Some(log_path)).await {
        Ok(output) => output,
        Err(err) => {
            let _ = write_command_error_log(log_path, "git", &args_os, format!("{err:#}")).await;
            CommandOutput {
                ok: false,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("{err:#}"),
            }
        }
    }
}

pub async fn ensure_git_identity(
    git: &GitCli,
    repo_dir: &Path,
    artifacts_dir: &Path,
) -> anyhow::Result<Vec<StepSummary>> {
    const DEFAULT_NAME: &str = "omne";
    const DEFAULT_EMAIL: &str = "omne@example.invalid";

    tokio::fs::create_dir_all(artifacts_dir).await?;

    let mut steps = Vec::new();

    let get_email_log = artifacts_dir.join("git-config-get-user.email.log");
    let get_name_log = artifacts_dir.join("git-config-get-user.name.log");
    let set_email_log = artifacts_dir.join("git-config-set-user.email.log");
    let set_name_log = artifacts_dir.join("git-config-set-user.name.log");
    let set_gpg_log = artifacts_dir.join("git-config-set-commit.gpgsign.log");

    let email = run_git_logged(
        git,
        repo_dir,
        &["config", "--get", "user.email"],
        &get_email_log,
    )
    .await;
    steps.push(StepSummary {
        name: "git_config_get_user.email".to_string(),
        ok: email.ok || treat_git_config_missing_as_ok(&email),
        exit_code: email.exit_code,
        log_path: Some(get_email_log),
    });

    if !email.ok || email.stdout.trim().is_empty() {
        let out = run_git_logged(
            git,
            repo_dir,
            &["config", "user.email", DEFAULT_EMAIL],
            &set_email_log,
        )
        .await;
        steps.push(StepSummary {
            name: "git_config_set_user.email".to_string(),
            ok: out.ok,
            exit_code: out.exit_code,
            log_path: Some(set_email_log),
        });
    }

    let name = run_git_logged(
        git,
        repo_dir,
        &["config", "--get", "user.name"],
        &get_name_log,
    )
    .await;
    steps.push(StepSummary {
        name: "git_config_get_user.name".to_string(),
        ok: name.ok || treat_git_config_missing_as_ok(&name),
        exit_code: name.exit_code,
        log_path: Some(get_name_log),
    });

    if !name.ok || name.stdout.trim().is_empty() {
        let out = run_git_logged(
            git,
            repo_dir,
            &["config", "user.name", DEFAULT_NAME],
            &set_name_log,
        )
        .await;
        steps.push(StepSummary {
            name: "git_config_set_user.name".to_string(),
            ok: out.ok,
            exit_code: out.exit_code,
            log_path: Some(set_name_log),
        });
    }

    let out = run_git_logged(
        git,
        repo_dir,
        &["config", "commit.gpgsign", "false"],
        &set_gpg_log,
    )
    .await;
    steps.push(StepSummary {
        name: "git_config_set_commit.gpgsign".to_string(),
        ok: out.ok,
        exit_code: out.exit_code,
        log_path: Some(set_gpg_log),
    });

    Ok(steps)
}
