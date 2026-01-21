use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use pm_protocol::{ApprovalDecision, ApprovalId, EventSeq, ThreadEventKind, TurnId};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use tokio_util::sync::CancellationToken;

use super::ProcessCommand;

const DEFAULT_MODEL: &str = "gpt-4.1";
const DEFAULT_MAX_AGENT_STEPS: usize = 24;
const DEFAULT_MAX_TOOL_CALLS: usize = 128;
const DEFAULT_MAX_TURN_SECONDS: u64 = 10 * 60;
const DEFAULT_MAX_OPENAI_REQUEST_SECONDS: u64 = 120;

const MAX_MAX_AGENT_STEPS: usize = 10_000;
const MAX_MAX_TOOL_CALLS: usize = 10_000;
const MAX_MAX_TURN_SECONDS: u64 = 24 * 60 * 60;
const MAX_MAX_OPENAI_REQUEST_SECONDS: u64 = 60 * 60;

const DEFAULT_INSTRUCTIONS: &str = r#"
You are a coding agent.

- Use tools to read/write files and run commands.
- Processes are non-interactive: you can only start/inspect/tail/follow/kill them.
- Prefer small, reviewable changes; run checks/tests when relevant.
"#;

#[derive(Debug, Error)]
pub enum AgentTurnError {
    #[error("cancelled")]
    Cancelled,
    #[error("budget exceeded: {budget}")]
    BudgetExceeded { budget: &'static str },
    #[error("openai request timed out")]
    OpenAiRequestTimedOut,
}

pub async fn run_agent_turn(
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    turn_id: TurnId,
    _input: String,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let (thread_id, thread_model, thread_openai_base_url, thread_cwd) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            handle.thread_id(),
            state.model.clone(),
            state.openai_base_url.clone(),
            state.cwd.clone(),
        )
    };

    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("CODE_PM_OPENAI_API_KEY"))
        .context("OPENAI_API_KEY is required")?;
    let model = thread_model
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let base_url = thread_openai_base_url
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "https://api.openai.com".to_string());

    let openai = pm_openai::Client::new_with_base_url(api_key, base_url)?;
    let tools = build_tools();

    let max_agent_steps = parse_env_usize(
        "CODE_PM_AGENT_MAX_STEPS",
        DEFAULT_MAX_AGENT_STEPS,
        1,
        MAX_MAX_AGENT_STEPS,
    );
    let max_tool_calls = parse_env_usize(
        "CODE_PM_AGENT_MAX_TOOL_CALLS",
        DEFAULT_MAX_TOOL_CALLS,
        1,
        MAX_MAX_TOOL_CALLS,
    );
    let max_turn_duration = Duration::from_secs(parse_env_u64(
        "CODE_PM_AGENT_MAX_TURN_SECONDS",
        DEFAULT_MAX_TURN_SECONDS,
        1,
        MAX_MAX_TURN_SECONDS,
    ));
    let max_openai_request_duration = Duration::from_secs(parse_env_u64(
        "CODE_PM_AGENT_MAX_OPENAI_REQUEST_SECONDS",
        DEFAULT_MAX_OPENAI_REQUEST_SECONDS,
        1,
        MAX_MAX_OPENAI_REQUEST_SECONDS,
    ));

    let mut instructions = DEFAULT_INSTRUCTIONS.to_string();
    if let Some(cwd) = thread_cwd {
        let agents_path = PathBuf::from(cwd).join("AGENTS.md");
        if let Ok(contents) = tokio::fs::read_to_string(&agents_path).await {
            let contents = pm_core::redact_text(&contents);
            instructions.push_str("\n\n# Project instructions (AGENTS.md)\n\n");
            instructions.push_str(&contents);
        }
    }

    let mut input_items = build_conversation(&server, thread_id).await?;

    let mut last_response_id = String::new();
    let mut last_usage: Option<Value> = None;
    let mut last_text = String::new();
    let mut tool_calls_total = 0usize;
    let mut finished = false;
    let started_at = tokio::time::Instant::now();

    for _step in 0..max_agent_steps {
        if cancel.is_cancelled() {
            return Err(AgentTurnError::Cancelled.into());
        }
        if started_at.elapsed() > max_turn_duration {
            return Err(AgentTurnError::BudgetExceeded {
                budget: "turn_seconds",
            }
            .into());
        }

        let req = pm_openai::ResponsesApiRequest {
            model: &model,
            instructions: &instructions,
            input: &input_items,
            tools: &tools,
            tool_choice: "auto",
            parallel_tool_calls: false,
            store: false,
            stream: true,
        };

        let resp = match tokio::time::timeout(max_openai_request_duration, async {
            let mut stream = openai.create_response_stream(&req).await?;
            let mut response_id = String::new();
            let mut usage: Option<Value> = None;
            let mut output_items = Vec::new();
            let mut output_text = String::new();

            while let Some(event) = stream.recv().await {
                let event = event?;
                match event {
                    pm_openai::ResponseEvent::Created { response_id: id } => {
                        if response_id.is_empty()
                            && let Some(id) = id
                            && !id.trim().is_empty()
                        {
                            response_id = id;
                        }
                    }
                    pm_openai::ResponseEvent::OutputTextDelta(delta) => {
                        output_text.push_str(&delta);
                        let delta = pm_core::redact_text(&delta);
                        let response_id_snapshot = response_id.clone();
                        thread_rt.emit_notification(
                            "item/delta",
                            &serde_json::json!({
                                "thread_id": thread_id,
                                "turn_id": turn_id,
                                "response_id": response_id_snapshot,
                                "kind": "output_text",
                                "delta": delta,
                            }),
                        );
                    }
                    pm_openai::ResponseEvent::OutputItemDone(item) => output_items.push(item),
                    pm_openai::ResponseEvent::Completed {
                        response_id: id,
                        usage: u,
                    } => {
                        if response_id.is_empty() {
                            response_id = id;
                        }
                        usage = u;
                        break;
                    }
                    _ => {}
                }
            }

            if response_id.trim().is_empty() {
                response_id = "<unknown>".to_string();
            }

            Ok::<_, anyhow::Error>((
                pm_openai::ResponsesApiResponse {
                    id: response_id,
                    output: output_items,
                    usage,
                },
                output_text,
            ))
        })
        .await
        {
            Ok(result) => {
                let (mut resp, output_text) = result?;
                if extract_assistant_text(&resp.output).is_empty() && !output_text.is_empty() {
                    let output_text = pm_core::redact_text(&output_text);
                    resp.output.push(pm_openai::ResponseItem::Message {
                        role: "assistant".to_string(),
                        content: vec![pm_openai::ContentItem::OutputText { text: output_text }],
                    });
                }
                resp
            }
            Err(_) => return Err(AgentTurnError::OpenAiRequestTimedOut.into()),
        };
        last_response_id = resp.id.clone();
        last_usage = resp.usage.clone();

        let mut function_calls = Vec::new();
        last_text = extract_assistant_text(&resp.output);

        for item in resp.output {
            if let pm_openai::ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
            } = &item
            {
                function_calls.push((name.clone(), arguments.clone(), call_id.clone()));
            }
            input_items.push(item);
        }

        if function_calls.is_empty() {
            finished = true;
            break;
        }

        for (tool_name, arguments, call_id) in function_calls {
            tool_calls_total += 1;
            if tool_calls_total > max_tool_calls {
                return Err(AgentTurnError::BudgetExceeded {
                    budget: "tool_calls",
                }
                .into());
            }
            let args_json: Value = match serde_json::from_str(&arguments) {
                Ok(v) => v,
                Err(err) => {
                    let output = serde_json::json!({
                        "error": "invalid tool arguments",
                        "details": err.to_string(),
                        "arguments": arguments,
                    });
                    input_items.push(pm_openai::ResponseItem::FunctionCallOutput {
                        call_id,
                        output: serde_json::to_string(&output)?,
                    });
                    continue;
                }
            };

            let output_value = run_tool_call(
                &server,
                thread_id,
                Some(turn_id),
                &tool_name,
                args_json,
                cancel.clone(),
            )
            .await;
            let output_value = match output_value {
                Ok(v) => v,
                Err(err) => serde_json::json!({ "error": err.to_string() }),
            };

            input_items.push(pm_openai::ResponseItem::FunctionCallOutput {
                call_id,
                output: serde_json::to_string(&output_value)?,
            });
        }
    }

    if !finished {
        return Err(AgentTurnError::BudgetExceeded { budget: "steps" }.into());
    }

    if !last_text.is_empty() {
        let _ = thread_rt
            .append_event(ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                text: last_text.clone(),
                model: Some(model.clone()),
                response_id: Some(last_response_id.clone()),
                token_usage: last_usage.clone(),
            })
            .await;
    }

    Ok(())
}

fn parse_env_usize(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn parse_env_u64(key: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

async fn build_conversation(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
) -> anyhow::Result<Vec<pm_openai::ResponseItem>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    let mut input = Vec::new();
    for event in events {
        match event.kind {
            ThreadEventKind::TurnStarted { input: text, .. } => {
                input.push(pm_openai::ResponseItem::Message {
                    role: "user".to_string(),
                    content: vec![pm_openai::ContentItem::InputText { text }],
                });
            }
            ThreadEventKind::AssistantMessage { text, .. } => {
                input.push(pm_openai::ResponseItem::Message {
                    role: "assistant".to_string(),
                    content: vec![pm_openai::ContentItem::OutputText { text }],
                });
            }
            other => {
                if let Some(text) = format_event_for_context(&other) {
                    input.push(pm_openai::ResponseItem::Message {
                        role: "assistant".to_string(),
                        content: vec![pm_openai::ContentItem::OutputText { text }],
                    });
                }
            }
        }
    }
    Ok(input)
}

fn format_event_for_context(kind: &ThreadEventKind) -> Option<String> {
    match kind {
        ThreadEventKind::ThreadArchived { reason } => Some(format!(
            "[thread/archived] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ThreadUnarchived { reason } => Some(format!(
            "[thread/unarchived] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ThreadPaused { reason } => Some(format!(
            "[thread/paused] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ThreadUnpaused { reason } => Some(format!(
            "[thread/unpaused] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::TurnInterruptRequested { turn_id, reason } => Some(format!(
            "[turn/interrupt_requested] turn_id={turn_id} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } if !matches!(status, pm_protocol::TurnStatus::Completed) || reason.is_some() => {
            Some(format!(
                "[turn/completed] turn_id={turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            ))
        }
        ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            mode,
            model,
            openai_base_url,
        } => Some(format!(
            "[thread/config] approval_policy={approval_policy:?} sandbox_policy={} mode={} model={} openai_base_url={}",
            sandbox_policy
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|| "<unchanged>".to_string()),
            mode.as_deref().unwrap_or("<unchanged>"),
            model.as_deref().unwrap_or("<unchanged>"),
            openai_base_url.as_deref().unwrap_or("<unchanged>"),
        )),
        ThreadEventKind::ApprovalRequested {
            approval_id,
            turn_id,
            action,
            params,
        } => Some(format!(
            "[approval/request] approval_id={approval_id} turn_id={} action={action} params={}",
            turn_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            json_one_line(params, 4000),
        )),
        ThreadEventKind::ApprovalDecided {
            approval_id,
            decision,
            remember,
            reason,
        } => Some(format!(
            "[approval/decide] approval_id={approval_id} decision={decision:?} remember={remember} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool,
            params,
        } => Some(format!(
            "[tool/start] tool_id={tool_id} turn_id={} tool={tool} params={}",
            turn_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            params
                .as_ref()
                .map(|v| json_one_line(v, 4000))
                .unwrap_or_else(|| "{}".to_string()),
        )),
        ThreadEventKind::ToolCompleted {
            tool_id,
            status,
            error,
            result,
        } => Some(format!(
            "[tool/done] tool_id={tool_id} status={status:?} error={} result={}",
            error.as_deref().unwrap_or(""),
            result
                .as_ref()
                .map(|v| json_one_line(v, 4000))
                .unwrap_or_else(|| "{}".to_string()),
        )),
        ThreadEventKind::ProcessStarted {
            process_id,
            turn_id,
            argv,
            cwd,
            stdout_path,
            stderr_path,
        } => Some(format!(
            "[process/start] process_id={process_id} turn_id={} argv={} cwd={cwd} stdout={stdout_path} stderr={stderr_path}",
            turn_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            json_one_line(&serde_json::json!(argv), 2000),
        )),
        ThreadEventKind::ProcessInterruptRequested { process_id, reason } => Some(format!(
            "[process/interrupt_requested] process_id={process_id} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ProcessKillRequested { process_id, reason } => Some(format!(
            "[process/kill_requested] process_id={process_id} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => Some(format!(
            "[process/exited] process_id={process_id} exit_code={} reason={}",
            exit_code
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string()),
            reason.as_deref().unwrap_or("")
        )),
        _ => None,
    }
}

fn json_one_line(value: &Value, max_chars: usize) -> String {
    match serde_json::to_string(value) {
        Ok(s) => truncate_chars(&s, max_chars),
        Err(_) => "<invalid-json>".to_string(),
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }

    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn extract_assistant_text(items: &[pm_openai::ResponseItem]) -> String {
    let mut out = String::new();
    for item in items {
        let pm_openai::ResponseItem::Message { role, content } = item else {
            continue;
        };
        if role != "assistant" {
            continue;
        }
        for c in content {
            if let pm_openai::ContentItem::OutputText { text } = c {
                out.push_str(text);
            }
        }
    }
    out
}

