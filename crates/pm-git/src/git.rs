use std::path::Path;
use std::process::Stdio;
use std::{ffi::OsString, fmt::Write as _};

use anyhow::Context;
use tokio::process::Command;

#[derive(Clone, Debug, Default)]
pub struct GitCli;

impl GitCli {
    pub async fn run(
        &self,
        cwd: &Path,
        args: &[OsString],
        log_path: Option<&Path>,
    ) -> anyhow::Result<CommandOutput> {
        let mut command_line = String::new();
        write!(&mut command_line, "git {}", format_args_for_log(args)).ok();

        let mut command = Command::new("git");
        command
            .current_dir(cwd)
            .args(args)
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never");
        let output = command
            .output()
            .await
            .with_context(|| format!("spawn {command_line}"))?;

        let result = CommandOutput {
            ok: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        };

        if let Some(path) = log_path {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut log = String::new();
            log.push_str("$ git ");
            log.push_str(&format_args_for_log(args));
            log.push('\n');
            if !result.stdout.is_empty() {
                log.push_str(&result.stdout);
                if !result.stdout.ends_with('\n') {
                    log.push('\n');
                }
            }
            if !result.stderr.is_empty() {
                log.push_str(&result.stderr);
                if !result.stderr.ends_with('\n') {
                    log.push('\n');
                }
            }
            tokio::fs::write(path, log).await?;
        }

        Ok(result)
    }
}

fn format_args_for_log(args: &[OsString]) -> String {
    let mut out = String::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(&arg.to_string_lossy());
    }
    out
}

#[derive(Clone, Debug)]
pub struct CommandOutput {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
