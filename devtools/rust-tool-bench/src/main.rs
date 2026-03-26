use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::Context;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(name = "rust-tool-bench")]
#[command(about = "Dev-only benchmark runner using real omne Rust CLI")]
struct Args {
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,

    #[arg(long)]
    omne_bin: Option<PathBuf>,

    #[arg(long, default_value = "scripts/tool_suite/cases.facade.full.v1.json")]
    cases: PathBuf,

    #[arg(long)]
    out_dir: Option<PathBuf>,

    #[arg(long)]
    cwd: Option<PathBuf>,

    #[arg(long)]
    mode: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    openai_base_url: Option<String>,

    #[arg(long, default_value = "auto-approve")]
    approval_policy: String,

    #[arg(long, default_value = "workspace-write")]
    sandbox_policy: String,

    #[arg(long)]
    max_cases: Option<usize>,

    #[arg(long, default_value_t = false)]
    fail_fast: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CaseDef {
    id: String,
    user_prompt: String,
    target_tool: String,
    #[serde(default)]
    success_args: Value,
    #[serde(default)]
    capability_hint: Option<String>,
}

#[derive(Debug, Serialize)]
struct CaseResult {
    id: String,
    target_tool: String,
    capability_hint: Option<String>,
    user_prompt: String,
    expected_success_args: Value,
    thread_id: Option<String>,
    turn_id: Option<String>,
    turn_status: Option<String>,
    turn_reason: Option<String>,
    used_target_tool: bool,
    target_tool_calls: usize,
    total_tool_calls: usize,
    pass: bool,
    error: Option<String>,
    assistant: Value,
    exec_stdout: Option<String>,
    exec_stderr: Option<String>,
    tool_started: Vec<Value>,
    tool_completed: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct Summary {
    generated_at: String,
    repo_root: String,
    cases_path: String,
    out_dir: String,
    omne_bin: String,
    total_cases: usize,
    passed_cases: usize,
    failed_cases: usize,
    pass_rate: f64,
    config: Value,
    results: Vec<CaseResult>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    run(args)
}

fn run(mut args: Args) -> anyhow::Result<()> {
    let repo_root = std::fs::canonicalize(&args.repo_root)
        .with_context(|| format!("canonicalize repo_root {}", args.repo_root.display()))?;

    let case_path = if args.cases.is_absolute() {
        args.cases.clone()
    } else {
        repo_root.join(&args.cases)
    };
    let case_text = std::fs::read_to_string(&case_path)
        .with_context(|| format!("read cases file {}", case_path.display()))?;
    let mut cases: Vec<CaseDef> =
        serde_json::from_str(&case_text).context("parse case json as array")?;
    if let Some(max_cases) = args.max_cases {
        cases.truncate(max_cases);
    }
    if cases.is_empty() {
        anyhow::bail!("no cases to run");
    }

    let omne_bin = resolve_omne_bin(&repo_root, args.omne_bin.take())?;

    let out_dir = if let Some(out_dir) = args.out_dir.clone() {
        if out_dir.is_absolute() {
            out_dir
        } else {
            repo_root.join(out_dir)
        }
    } else {
        let ts = time::OffsetDateTime::now_utc()
            .format(&time::format_description::parse(
                "[year][month][day]_[hour][minute][second]",
            )?)
            .unwrap_or_else(|_| "unknown_time".to_string());
        repo_root.join(format!("docs/reports/rust-tool-bench-{ts}"))
    };
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create out_dir {}", out_dir.display()))?;

    let default_cwd = args
        .cwd
        .clone()
        .map(|v| {
            if v.is_absolute() {
                v
            } else {
                repo_root.join(v)
            }
        })
        .unwrap_or_else(|| repo_root.clone());

    let mut results = Vec::<CaseResult>::new();
    let mut passed_cases = 0usize;

    for (idx, case) in cases.iter().enumerate() {
        eprintln!(
            "[rust-tool-bench] ({}/{}) {} target={}",
            idx + 1,
            cases.len(),
            case.id,
            case.target_tool
        );
        let case_result = run_case(&args, &repo_root, &omne_bin, &default_cwd, case);
        let case_result = match case_result {
            Ok(value) => value,
            Err(err) => CaseResult {
                id: case.id.clone(),
                target_tool: case.target_tool.clone(),
                capability_hint: case.capability_hint.clone(),
                user_prompt: case.user_prompt.clone(),
                expected_success_args: case.success_args.clone(),
                thread_id: None,
                turn_id: None,
                turn_status: None,
                turn_reason: None,
                used_target_tool: false,
                target_tool_calls: 0,
                total_tool_calls: 0,
                pass: false,
                error: Some(err.to_string()),
                assistant: serde_json::json!({}),
                exec_stdout: None,
                exec_stderr: None,
                tool_started: vec![],
                tool_completed: vec![],
            },
        };
        if case_result.pass {
            passed_cases = passed_cases.saturating_add(1);
        }
        if args.fail_fast && !case_result.pass {
            results.push(case_result);
            break;
        }
        results.push(case_result);
    }

    let total_cases = results.len();
    let failed_cases = total_cases.saturating_sub(passed_cases);
    let pass_rate = if total_cases == 0 {
        0.0
    } else {
        (passed_cases as f64) * 100.0 / (total_cases as f64)
    };

    let summary = Summary {
        generated_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "".to_string()),
        repo_root: repo_root.display().to_string(),
        cases_path: case_path.display().to_string(),
        out_dir: out_dir.display().to_string(),
        omne_bin: omne_bin.display().to_string(),
        total_cases,
        passed_cases,
        failed_cases,
        pass_rate,
        config: serde_json::json!({
            "cwd": default_cwd.display().to_string(),
            "mode": args.mode,
            "model": args.model,
            "openai_base_url": args.openai_base_url,
            "approval_policy": args.approval_policy,
            "sandbox_policy": args.sandbox_policy,
            "max_cases": args.max_cases,
            "fail_fast": args.fail_fast
        }),
        results,
    };

    let raw_path = out_dir.join("raw_results.json");
    std::fs::write(&raw_path, serde_json::to_string_pretty(&summary)?)
        .with_context(|| format!("write {}", raw_path.display()))?;

    let report_path = out_dir.join("report.md");
    std::fs::write(&report_path, render_markdown(&summary))
        .with_context(|| format!("write {}", report_path.display()))?;

    eprintln!(
        "[rust-tool-bench] done: passed {}/{} ({:.2}%)",
        summary.passed_cases, summary.total_cases, summary.pass_rate
    );
    eprintln!("[rust-tool-bench] raw: {}", raw_path.display());
    eprintln!("[rust-tool-bench] report: {}", report_path.display());
    Ok(())
}

fn run_case(
    args: &Args,
    repo_root: &Path,
    omne_bin: &Path,
    cwd: &Path,
    case: &CaseDef,
) -> anyhow::Result<CaseResult> {
    let mut exec_args = vec![
        "exec".to_string(),
        "--json".to_string(),
        "--on-approval".to_string(),
        "approve".to_string(),
        "--approval-policy".to_string(),
        args.approval_policy.clone(),
        "--sandbox-policy".to_string(),
        args.sandbox_policy.clone(),
        "--cwd".to_string(),
        cwd.display().to_string(),
    ];
    if let Some(mode) = &args.mode {
        exec_args.push("--mode".to_string());
        exec_args.push(mode.clone());
    }
    if let Some(model) = &args.model {
        exec_args.push("--model".to_string());
        exec_args.push(model.clone());
    }
    if let Some(openai_base_url) = &args.openai_base_url {
        exec_args.push("--openai-base-url".to_string());
        exec_args.push(openai_base_url.clone());
    }
    exec_args.push(case.user_prompt.clone());

    let exec_output = run_omne_command(omne_bin, repo_root, &exec_args)
        .with_context(|| format!("run omne exec for case {}", case.id))?;

    let exec_stdout = String::from_utf8_lossy(&exec_output.stdout).to_string();
    let exec_stderr = String::from_utf8_lossy(&exec_output.stderr).to_string();
    if !exec_output.status.success() {
        return Ok(CaseResult {
            id: case.id.clone(),
            target_tool: case.target_tool.clone(),
            capability_hint: case.capability_hint.clone(),
            user_prompt: case.user_prompt.clone(),
            expected_success_args: case.success_args.clone(),
            thread_id: None,
            turn_id: None,
            turn_status: None,
            turn_reason: None,
            used_target_tool: false,
            target_tool_calls: 0,
            total_tool_calls: 0,
            pass: false,
            error: Some(format!("exec failed with status {}", exec_output.status)),
            assistant: serde_json::json!({}),
            exec_stdout: Some(exec_stdout),
            exec_stderr: Some(exec_stderr),
            tool_started: vec![],
            tool_completed: vec![],
        });
    }

    let exec_json: Value = serde_json::from_str(&exec_stdout)
        .with_context(|| format!("parse exec --json output for case {}", case.id))?;
    let thread_id = exec_json
        .get("thread_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let turn_id = exec_json
        .get("turn_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let turn_status = exec_json
        .get("status")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let turn_reason = exec_json
        .get("reason")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let assistant = exec_json
        .get("assistant")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let mut tool_started = Vec::<Value>::new();
    let mut tool_completed = Vec::<Value>::new();
    let mut target_tool_calls = 0usize;
    let mut total_tool_calls = 0usize;
    let mut used_target_tool = false;

    if let Some(thread_id_text) = &thread_id {
        let events_args = vec![
            "thread".to_string(),
            "events".to_string(),
            thread_id_text.clone(),
            "--since-seq".to_string(),
            "0".to_string(),
            "--json".to_string(),
        ];
        let events_output = run_omne_command(omne_bin, repo_root, &events_args)
            .with_context(|| format!("run omne thread events for {}", case.id))?;
        if events_output.status.success() {
            let events_stdout = String::from_utf8_lossy(&events_output.stdout).to_string();
            let events_json: Value =
                serde_json::from_str(&events_stdout).context("parse thread events json")?;
            if let Some(events) = events_json.get("events").and_then(Value::as_array) {
                let turn_id_filter = turn_id.clone();
                for event in events {
                    let (kind_payload, kind_type) = if let Some(kind) = event.get("kind") {
                        let Some(kind_type) = kind.get("type").and_then(Value::as_str) else {
                            continue;
                        };
                        (kind, kind_type)
                    } else {
                        let Some(kind_type) = event.get("type").and_then(Value::as_str) else {
                            continue;
                        };
                        (event, kind_type)
                    };
                    match kind_type {
                        "tool_started" => {
                            if !event_belongs_to_turn(kind_payload, turn_id_filter.as_deref()) {
                                continue;
                            }
                            total_tool_calls = total_tool_calls.saturating_add(1);
                            let tool = kind_payload
                                .get("tool")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let params = kind_payload.get("params").cloned().unwrap_or(Value::Null);
                            let matched =
                                tool_matches_target(&case.target_tool, &tool, kind_payload.get("params"));
                            if matched {
                                used_target_tool = true;
                                target_tool_calls = target_tool_calls.saturating_add(1);
                            }
                            tool_started.push(serde_json::json!({
                                "tool": tool,
                                "params": params,
                                "matched_target": matched
                            }));
                        }
                        "tool_completed" => {
                            tool_completed.push(kind_payload.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let pass = turn_status.as_deref() == Some("completed") && used_target_tool;
    Ok(CaseResult {
        id: case.id.clone(),
        target_tool: case.target_tool.clone(),
        capability_hint: case.capability_hint.clone(),
        user_prompt: case.user_prompt.clone(),
        expected_success_args: case.success_args.clone(),
        thread_id,
        turn_id,
        turn_status,
        turn_reason,
        used_target_tool,
        target_tool_calls,
        total_tool_calls,
        pass,
        error: None,
        assistant,
        exec_stdout: Some(exec_stdout),
        exec_stderr: Some(exec_stderr),
        tool_started,
        tool_completed,
    })
}

fn event_belongs_to_turn(kind: &Value, turn_id: Option<&str>) -> bool {
    let Some(turn_id) = turn_id else {
        return true;
    };
    let event_turn_id = kind.get("turn_id").and_then(Value::as_str);
    event_turn_id.is_none_or(|value| value == turn_id)
}

fn tool_matches_target(target_tool: &str, tool_name: &str, params: Option<&Value>) -> bool {
    if tool_name == target_tool || tool_name == format!("facade/{target_tool}") {
        return true;
    }
    if let Some(value) = tool_name.strip_prefix("facade/")
        && value == target_tool
    {
        return true;
    }
    if let Some(value) = params
        .and_then(|v| v.get("facade_tool"))
        .and_then(Value::as_str)
        && value == target_tool
    {
        return true;
    }

    let aliases: HashSet<&str> = match target_tool {
        "workspace" => HashSet::from([
            "file_read",
            "file_glob",
            "file_grep",
            "repo_search",
            "repo_index",
            "repo_symbols",
            "repo_goto_definition",
            "repo_find_references",
            "file_write",
            "file_patch",
            "file_edit",
            "file_delete",
            "fs_mkdir",
        ]),
        "process" => HashSet::from([
            "process_start",
            "process_inspect",
            "process_tail",
            "process_follow",
            "process_kill",
            "process_interrupt",
        ]),
        "thread" => HashSet::from([
            "thread_diff",
            "thread_state",
            "thread_usage",
            "thread_events",
            "thread_hook_run",
            "subagent_spawn",
            "subagent_send_input",
            "subagent_wait",
            "subagent_close",
            "request_user_input",
            "ask_user",
        ]),
        "artifact" => HashSet::from([
            "artifact_write",
            "update_plan",
            "artifact_list",
            "artifact_read",
            "artifact_delete",
        ]),
        "integration" => HashSet::from([
            "mcp_list_servers",
            "mcp_list_tools",
            "mcp_list_resources",
            "mcp_call",
            "web_search",
            "web_fetch",
            "view_image",
            "request_user_input",
        ]),
        _ => HashSet::new(),
    };
    aliases.contains(tool_name)
}

fn resolve_omne_bin(repo_root: &Path, requested: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(path) = requested {
        let full = if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        };
        if !full.exists() {
            anyhow::bail!("omne_bin does not exist: {}", full.display());
        }
        return Ok(full);
    }

    let candidate = repo_root.join("target/debug/omne");
    if candidate.exists() {
        return Ok(candidate);
    }

    let output = Command::new("cargo")
        .current_dir(repo_root)
        .arg("build")
        .arg("-p")
        .arg("omne")
        .output()
        .context("run cargo build -p omne")?;
    if !output.status.success() {
        anyhow::bail!(
            "cargo build -p omne failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    if !candidate.exists() {
        anyhow::bail!(
            "built omne binary but not found at expected path {}",
            candidate.display()
        );
    }
    Ok(candidate)
}

fn run_omne_command(omne_bin: &Path, repo_root: &Path, args: &[String]) -> anyhow::Result<Output> {
    Command::new(omne_bin)
        .current_dir(repo_root)
        .args(args)
        .output()
        .with_context(|| format!("run {} {}", omne_bin.display(), args.join(" ")))
}

fn render_markdown(summary: &Summary) -> String {
    let mut out = String::new();
    let _ = writeln!(&mut out, "# Rust Tool Benchmark (Dev Runner)");
    let _ = writeln!(&mut out);
    let _ = writeln!(&mut out, "- generated_at: {}", summary.generated_at);
    let _ = writeln!(&mut out, "- repo_root: `{}`", summary.repo_root);
    let _ = writeln!(&mut out, "- omne_bin: `{}`", summary.omne_bin);
    let _ = writeln!(&mut out, "- cases_path: `{}`", summary.cases_path);
    let _ = writeln!(
        &mut out,
        "- pass: {}/{} ({:.2}%)",
        summary.passed_cases, summary.total_cases, summary.pass_rate
    );
    let _ = writeln!(&mut out);
    let _ = writeln!(
        &mut out,
        "| case_id | target | pass | target_calls | tool_calls | status | reason |"
    );
    let _ = writeln!(&mut out, "| --- | --- | --- | ---: | ---: | --- | --- |");
    for item in &summary.results {
        let reason = item
            .error
            .clone()
            .or_else(|| item.turn_reason.clone())
            .unwrap_or_default()
            .replace('|', "\\|");
        let _ = writeln!(
            &mut out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            item.id,
            item.target_tool,
            if item.pass { "PASS" } else { "FAIL" },
            item.target_tool_calls,
            item.total_tool_calls,
            item.turn_status.clone().unwrap_or_default(),
            reason
        );
    }
    out
}
