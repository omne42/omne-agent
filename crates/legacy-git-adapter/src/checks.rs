use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;

use omne_core::StepSummary;
use tokio::process::Command;

use crate::git::{CommandOutput, GitCli};

pub(crate) fn os_arg(value: impl AsRef<OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

pub(crate) async fn write_command_error_log(
    log_path: &Path,
    program: &str,
    args: &[OsString],
    err_text: String,
) -> anyhow::Result<()> {
    if let Some(parent) = log_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut log = String::new();
    log.push_str("$ ");
    log.push_str(program);
    for arg in args {
        log.push(' ');
        log.push_str(&arg.to_string_lossy());
    }
    log.push('\n');
    log.push_str(&err_text);
    if !err_text.ends_with('\n') {
        log.push('\n');
    }
    tokio::fs::write(log_path, log).await?;
    Ok(())
}

pub async fn run_git_capture(
    git: &GitCli,
    repo_dir: &Path,
    name: &str,
    args: &[OsString],
    log_path: &Path,
) -> anyhow::Result<(StepSummary, CommandOutput)> {
    let output = match git.run(repo_dir, args, Some(log_path)).await {
        Ok(output) => output,
        Err(err) => {
            let err_text = format!("{err:#}");
            write_command_error_log(log_path, "git", args, err_text.clone()).await?;
            CommandOutput {
                ok: false,
                exit_code: None,
                stdout: String::new(),
                stderr: err_text,
            }
        }
    };

    let step = StepSummary {
        name: name.to_string(),
        ok: output.ok,
        exit_code: output.exit_code,
        log_path: Some(log_path.to_path_buf()),
    };
    Ok((step, output))
}

pub async fn run_cargo_step(
    repo_dir: &Path,
    name: &str,
    args: &[&str],
    log_path: &Path,
    target_dir: Option<&Path>,
) -> anyhow::Result<StepSummary> {
    if let Some(parent) = log_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut command = Command::new("cargo");
    command
        .current_dir(repo_dir)
        .args(args)
        .stdin(Stdio::null())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never");
    if let Some(target_dir) = target_dir {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    let output = match command.output().await {
        Ok(output) => output,
        Err(err) => {
            let args_os: Vec<OsString> = args.iter().map(|arg| os_arg(*arg)).collect();
            write_command_error_log(log_path, "cargo", &args_os, format!("{err:#}")).await?;
            return Ok(StepSummary {
                name: name.to_string(),
                ok: false,
                exit_code: None,
                log_path: Some(log_path.to_path_buf()),
            });
        }
    };

    let mut log = String::new();
    log.push_str("$ cargo ");
    log.push_str(&args.join(" "));
    log.push('\n');
    log.push_str(&String::from_utf8_lossy(&output.stdout));
    if !log.ends_with('\n') {
        log.push('\n');
    }
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    if !log.ends_with('\n') {
        log.push('\n');
    }
    tokio::fs::write(log_path, log).await?;

    Ok(StepSummary {
        name: name.to_string(),
        ok: output.status.success(),
        exit_code: output.status.code(),
        log_path: Some(log_path.to_path_buf()),
    })
}

pub async fn run_git_step(
    git: &GitCli,
    repo_dir: &Path,
    name: &str,
    args: &[OsString],
    log_path: &Path,
) -> anyhow::Result<StepSummary> {
    let (step, _output) = run_git_capture(git, repo_dir, name, args, log_path).await?;
    Ok(step)
}
