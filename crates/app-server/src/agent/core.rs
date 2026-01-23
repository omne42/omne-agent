use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use crate::project_config::ProjectOpenAiOverrides;
use futures_util::stream::{self, StreamExt};
use ditto_llm::ThinkingIntensity;
use pm_protocol::{
    ApprovalDecision, ApprovalId, ArtifactId, ArtifactMetadata, EventSeq, ThreadEventKind, ThreadId,
    TurnId,
};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use tokio_util::sync::CancellationToken;

const DEFAULT_MODEL: &str = "gpt-4.1";
const DEFAULT_OPENAI_PROVIDER: &str = "openai-codex-apikey";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MAX_AGENT_STEPS: usize = 24;
const DEFAULT_MAX_TOOL_CALLS: usize = 128;
const DEFAULT_MAX_PARALLEL_TOOL_CALLS: usize = 8;
const DEFAULT_MAX_TOTAL_TOKENS: u64 = 0;
const DEFAULT_MAX_TURN_SECONDS: u64 = 10 * 60;
const DEFAULT_MAX_OPENAI_REQUEST_SECONDS: u64 = 120;

const MAX_MAX_AGENT_STEPS: usize = 10_000;
const MAX_MAX_TOOL_CALLS: usize = 10_000;
const MAX_MAX_PARALLEL_TOOL_CALLS: usize = 128;
const MAX_MAX_TOTAL_TOKENS: u64 = 10_000_000;
const MAX_MAX_TURN_SECONDS: u64 = 24 * 60 * 60;
const MAX_MAX_OPENAI_REQUEST_SECONDS: u64 = 60 * 60;

const LOOP_DETECTOR_HISTORY_LIMIT: usize = 8;
const LOOP_DETECTOR_CONSECUTIVE_LIMIT: usize = 3;
const LOOP_DETECTOR_CYCLE_LENGTH: usize = 2;

const DEFAULT_AUTO_SUMMARY_THRESHOLD_PCT: u64 = 80;
const MAX_AUTO_SUMMARY_THRESHOLD_PCT: u64 = 99;
const DEFAULT_AUTO_SUMMARY_SOURCE_MAX_CHARS: usize = 50_000;
const MAX_AUTO_SUMMARY_SOURCE_MAX_CHARS: usize = 200_000;
const DEFAULT_AUTO_SUMMARY_TAIL_ITEMS: usize = 20;
const MAX_AUTO_SUMMARY_TAIL_ITEMS: usize = 200;
const DEFAULT_SUMMARY_CONTEXT_EVENT_LIMIT: usize = 200;
const MAX_SUMMARY_CONTEXT_EVENT_LIMIT: usize = 5_000;

const DEFAULT_INSTRUCTIONS: &str = r#"
You are a coding agent.

- Use tools to read/write files and run commands.
- Processes are non-interactive: you can only start/inspect/tail/follow/kill them.
- Prefer small, reviewable changes; run checks/tests when relevant.
"#;

const SUMMARY_INSTRUCTIONS: &str = r#"
You are writing a compact, redaction-safe summary of the current session state so that a coding agent can continue.

Requirements:
- Keep it concise and actionable.
- Do NOT include secrets (API keys, tokens, private keys) or large raw logs.
- Prefer references (thread_id/turn_id/tool_id/process_id/artifact_id) instead of inlining long content.

Output format (Markdown):
- What happened
- Current state
- Next actions
"#;

#[derive(Debug, Error)]
pub enum AgentTurnError {
    #[error("cancelled")]
    Cancelled,
    #[error("budget exceeded: {budget}")]
    BudgetExceeded { budget: &'static str },
    #[error("token budget exceeded: used {used} > limit {limit}")]
    TokenBudgetExceeded { used: u64, limit: u64 },
    #[error("openai request timed out")]
    OpenAiRequestTimedOut,
    #[error("loop_detected: {kind}")]
    LoopDetected { kind: &'static str },
}

struct LoopDetector {
    recent: Vec<u64>,
}

impl LoopDetector {
    fn new() -> Self {
        Self { recent: Vec::new() }
    }

    fn observe(&mut self, signature: u64) -> Option<&'static str> {
        self.recent.push(signature);
        if self.recent.len() > LOOP_DETECTOR_HISTORY_LIMIT {
            let drain = self.recent.len().saturating_sub(LOOP_DETECTOR_HISTORY_LIMIT);
            self.recent.drain(0..drain);
        }

        let mut consecutive = 0usize;
        for &value in self.recent.iter().rev() {
            if value == signature {
                consecutive += 1;
            } else {
                break;
            }
        }
        if consecutive >= LOOP_DETECTOR_CONSECUTIVE_LIMIT {
            return Some("consecutive");
        }

        let cycle_len = LOOP_DETECTOR_CYCLE_LENGTH;
        if cycle_len > 0 && self.recent.len() >= cycle_len.saturating_mul(2) {
            let total = self.recent.len();
            let mut matches = true;
            for idx in 0..cycle_len {
                if self.recent[total - 1 - idx] != self.recent[total - 1 - idx - cycle_len] {
                    matches = false;
                    break;
                }
            }
            if matches {
                return Some("cycle");
            }
        }

        None
    }
}

fn tool_call_signature(tool_name: &str, args: &Value) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_name.hash(&mut hasher);
    hash_json_value(args, &mut hasher);
    hasher.finish()
}

fn tool_call_signature_from_raw(tool_name: &str, arguments: &str) -> u64 {
    let redacted = pm_core::redact_text(arguments);
    match serde_json::from_str::<Value>(&redacted) {
        Ok(args) => tool_call_signature(tool_name, &args),
        Err(_) => {
            use std::hash::{Hash, Hasher};

            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            tool_name.hash(&mut hasher);
            redacted.hash(&mut hasher);
            hasher.finish()
        }
    }
}

fn step_signature(function_calls: &[(String, String, String)]) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    function_calls.len().hash(&mut hasher);
    for (tool_name, arguments, _) in function_calls {
        tool_call_signature_from_raw(tool_name, arguments).hash(&mut hasher);
    }
    hasher.finish()
}

fn hash_json_value(value: &Value, state: &mut impl std::hash::Hasher) {
    use std::hash::Hash;

    match value {
        Value::Null => 0u8.hash(state),
        Value::Bool(v) => {
            1u8.hash(state);
            v.hash(state);
        }
        Value::Number(v) => {
            2u8.hash(state);
            v.to_string().hash(state);
        }
        Value::String(v) => {
            3u8.hash(state);
            v.hash(state);
        }
        Value::Array(values) => {
            4u8.hash(state);
            values.len().hash(state);
            for value in values {
                hash_json_value(value, state);
            }
        }
        Value::Object(map) => {
            5u8.hash(state);
            map.len().hash(state);

            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                key.hash(state);
                if let Some(value) = map.get(key) {
                    hash_json_value(value, state);
                }
            }
        }
    }
}

fn should_auto_summarize(total_tokens_used: u64, max_total_tokens: u64, threshold_pct: u64) -> bool {
    if max_total_tokens == 0 {
        return false;
    }
    if threshold_pct == 0 {
        return false;
    }
    let threshold_pct = threshold_pct.min(MAX_AUTO_SUMMARY_THRESHOLD_PCT);
    let threshold = max_total_tokens.saturating_mul(threshold_pct) / 100;
    threshold > 0 && total_tokens_used >= threshold
}

fn estimate_context_tokens(instructions: &str, input_items: &[pm_openai::ResponseItem]) -> u64 {
    let mut chars = instructions.chars().count() as u64;
    for item in input_items {
        chars = chars.saturating_add(estimate_response_item_chars(item));
    }
    (chars.saturating_add(3)) / 4
}

fn estimate_response_item_chars(item: &pm_openai::ResponseItem) -> u64 {
    match item {
        pm_openai::ResponseItem::Message { role, content } => {
            let mut chars = role.chars().count() as u64;
            for part in content {
                match part {
                    pm_openai::ContentItem::InputText { text }
                    | pm_openai::ContentItem::OutputText { text } => {
                        chars = chars.saturating_add(text.chars().count() as u64);
                    }
                    pm_openai::ContentItem::Other => {}
                }
            }
            chars
        }
        pm_openai::ResponseItem::FunctionCall {
            name,
            arguments,
            call_id,
        } => {
            (name.chars().count() as u64)
                .saturating_add(arguments.chars().count() as u64)
                .saturating_add(call_id.chars().count() as u64)
        }
        pm_openai::ResponseItem::FunctionCallOutput { call_id, output } => {
            (call_id.chars().count() as u64).saturating_add(output.chars().count() as u64)
        }
        pm_openai::ResponseItem::Other => 0,
    }
}

fn render_items_for_summary(items: &[pm_openai::ResponseItem], max_chars: usize) -> String {
    let mut out = String::new();

    for item in items {
        match item {
            pm_openai::ResponseItem::Message { role, content } => {
                for part in content {
                    let text = match part {
                        pm_openai::ContentItem::InputText { text } => text,
                        pm_openai::ContentItem::OutputText { text } => text,
                        pm_openai::ContentItem::Other => continue,
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    out.push_str(role);
                    out.push_str(": ");
                    out.push_str(text.trim());
                    out.push('\n');
                }
            }
            pm_openai::ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
            } => {
                let arguments = pm_core::redact_text(arguments);
                let args_preview = truncate_chars(&arguments, 200);
                out.push_str("[tool_call] ");
                out.push_str(name.trim());
                out.push_str(" call_id=");
                out.push_str(call_id.trim());
                if !args_preview.trim().is_empty() {
                    out.push_str(" args=");
                    out.push_str(args_preview.trim());
                }
                out.push('\n');
            }
            pm_openai::ResponseItem::FunctionCallOutput { call_id, output } => {
                let output = pm_core::redact_text(output);
                let output_preview = truncate_chars(&output, 500);
                out.push_str("[tool_output] call_id=");
                out.push_str(call_id.trim());
                if !output_preview.trim().is_empty() {
                    out.push_str(" output=");
                    out.push_str(output_preview.trim());
                }
                out.push('\n');
            }
            pm_openai::ResponseItem::Other => {}
        }

        if max_chars > 0 && out.chars().count() > max_chars.saturating_mul(2) {
            break;
        }
    }

    truncate_chars(&out, max_chars)
}

pub async fn run_agent_turn(
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    turn_id: TurnId,
    input: String,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let (thread_id, thread_mode, thread_model, thread_openai_base_url, thread_cwd) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            handle.thread_id(),
            state.mode.clone(),
            state.model.clone(),
            state.openai_base_url.clone(),
            state.cwd.clone(),
        )
    };

    let thread_root = match thread_cwd.as_deref() {
        Some(thread_cwd) => Some(pm_core::resolve_dir(Path::new(thread_cwd), Path::new(".")).await?),
        None => None,
    };

    let mut project_overrides = if let Some(thread_root) = thread_root.as_deref() {
        crate::project_config::load_project_openai_overrides(thread_root).await
    } else {
        ProjectOpenAiOverrides::default()
    };

    let provider = project_overrides
        .provider
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_PROVIDER")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_OPENAI_PROVIDER.to_string());

    let builtin_provider_config = builtin_openai_provider_config(&provider);
    let provider_overrides = project_overrides.providers.get(&provider);
    if builtin_provider_config.is_none() && provider_overrides.is_none() {
        anyhow::bail!(
            "unknown openai provider: {provider} (expected: openai-codex-apikey, openai-auth-command; or define [openai.providers.{provider}] in project config)"
        );
    }

    let mut provider_config = builtin_provider_config.unwrap_or_default();
    if let Some(overrides) = provider_overrides {
        provider_config = merge_provider_config(provider_config, overrides);
    }

    let forced_model = thread_model.is_some();
    let global_default_model = thread_model
        .or(project_overrides.model.clone())
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.default_model.clone())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let base_url = thread_openai_base_url
        .or(project_overrides.base_url)
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.base_url.clone())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("openai provider {provider} is missing base_url"))?;

    let env = ditto_llm::Env {
        dotenv: std::mem::take(&mut project_overrides.dotenv),
    };
    let auth = provider_config
        .auth
        .clone()
        .unwrap_or(ditto_llm::ProviderAuth::ApiKeyEnv { keys: Vec::new() });
    let api_key = ditto_llm::resolve_auth_token(&auth, &env)
        .await
        .context("resolve openai auth token")?;

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
    let max_total_tokens = parse_env_u64(
        "CODE_PM_AGENT_MAX_TOTAL_TOKENS",
        DEFAULT_MAX_TOTAL_TOKENS,
        0,
        MAX_MAX_TOTAL_TOKENS,
    );
    let auto_summary_threshold_pct = parse_env_u64(
        "CODE_PM_AGENT_AUTO_SUMMARY_THRESHOLD_PCT",
        DEFAULT_AUTO_SUMMARY_THRESHOLD_PCT,
        1,
        MAX_AUTO_SUMMARY_THRESHOLD_PCT,
    );
    let auto_summary_source_max_chars = parse_env_usize(
        "CODE_PM_AGENT_AUTO_SUMMARY_SOURCE_MAX_CHARS",
        DEFAULT_AUTO_SUMMARY_SOURCE_MAX_CHARS,
        1,
        MAX_AUTO_SUMMARY_SOURCE_MAX_CHARS,
    );
    let auto_summary_tail_items = parse_env_usize(
        "CODE_PM_AGENT_AUTO_SUMMARY_TAIL_ITEMS",
        DEFAULT_AUTO_SUMMARY_TAIL_ITEMS,
        0,
        MAX_AUTO_SUMMARY_TAIL_ITEMS,
    );
    let parallel_tool_calls = parse_env_bool("CODE_PM_AGENT_PARALLEL_TOOL_CALLS", false);
    let max_parallel_tool_calls = parse_env_usize(
        "CODE_PM_AGENT_MAX_PARALLEL_TOOL_CALLS",
        DEFAULT_MAX_PARALLEL_TOOL_CALLS,
        1,
        MAX_MAX_PARALLEL_TOOL_CALLS,
    );
    let response_format = match std::env::var("CODE_PM_AGENT_RESPONSE_FORMAT_JSON") {
        Ok(raw) => {
            let raw = raw.trim();
            if raw.is_empty() {
                None
            } else {
                Some(
                    serde_json::from_str::<Value>(raw)
                        .context("parse CODE_PM_AGENT_RESPONSE_FORMAT_JSON")?,
                )
            }
        }
        Err(_) => None,
    };

    let mut instructions = DEFAULT_INSTRUCTIONS.to_string();

    if let Some(user_instructions_path) = resolve_user_instructions_path() {
        if let Ok(contents) = tokio::fs::read_to_string(&user_instructions_path).await {
            let contents = pm_core::redact_text(&contents);
            instructions.push_str("\n\n# User instructions\n\n");
            instructions.push_str(&format!(
                "_Source: {}_\n\n",
                user_instructions_path.display()
            ));
            instructions.push_str(&contents);
        }
    }

    if let Some(cwd) = thread_cwd.as_deref() {
        let agents_path = PathBuf::from(cwd).join("AGENTS.md");
        if let Ok(contents) = tokio::fs::read_to_string(&agents_path).await {
            let contents = pm_core::redact_text(&contents);
            instructions.push_str("\n\n# Project instructions (AGENTS.md)\n\n");
            instructions.push_str(&contents);
        }
    }

    if let Some(skills) = load_skills_from_input(&input, thread_cwd.as_deref()).await? {
        instructions.push_str(&skills);
    }

    let mut input_items = build_conversation(&server, thread_id).await?;
    let context_tokens_estimate = estimate_context_tokens(&instructions, &input_items);

    let router_config = match thread_root.as_deref() {
        Some(thread_root) => pm_core::router::load_router_config(thread_root).await?,
        None => None,
    };
    let routed = pm_core::router::route_model(
        router_config.as_ref().map(|loaded| &loaded.config),
        Some(thread_mode.as_str()),
        &input,
        &global_default_model,
        forced_model,
        context_tokens_estimate,
    );
    let pm_core::router::ModelRouteDecision {
        selected_model: model,
        rule_source,
        reason,
        rule_id,
    } = routed;

    if !provider_config.model_whitelist.is_empty()
        && !provider_config
            .model_whitelist
            .iter()
            .any(|allowed| allowed.trim() == model)
    {
        anyhow::bail!(
            "model not allowed by provider whitelist: provider={provider} model={model}"
        );
    }

    let _ = thread_rt
        .append_event(ThreadEventKind::ModelRouted {
            turn_id,
            selected_model: model.clone(),
            rule_source,
            reason,
            rule_id,
        })
        .await;

    let mut last_response_id = String::new();
    let mut last_usage: Option<Value> = None;
    let mut last_text = String::new();
    let mut tool_calls_total = 0usize;
    let mut loop_detector = LoopDetector::new();
    let mut total_tokens_used = 0u64;
    let mut did_auto_summary = false;
    let mut attempted_auto_summary = false;
    let mut finished = false;
    let started_at = tokio::time::Instant::now();

    let reasoning_effort = match ditto_llm::select_model_config(&project_overrides.models, &model)
        .map(|cfg| cfg.thinking)
        .unwrap_or_default()
    {
        ThinkingIntensity::Unsupported => None,
        ThinkingIntensity::Small => Some(pm_openai::ReasoningEffort::Low),
        ThinkingIntensity::Medium => Some(pm_openai::ReasoningEffort::Medium),
        ThinkingIntensity::High => Some(pm_openai::ReasoningEffort::High),
        ThinkingIntensity::XHigh => Some(pm_openai::ReasoningEffort::XHigh),
    };

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
            response_format: response_format.as_ref(),
            reasoning: reasoning_effort.map(|effort| pm_openai::ReasoningConfig { effort }),
            parallel_tool_calls,
            store: false,
            stream: true,
        };

        let resp = match tokio::time::timeout(max_openai_request_duration, async {
            let mut stream = openai.create_response_stream(&req).await?;
            let mut response_id = String::new();
            let mut usage: Option<pm_openai::TokenUsage> = None;
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
                    pm_openai::ResponseEvent::Failed { response_id: id, error } => {
                        let failed_response_id = id
                            .as_deref()
                            .filter(|id| !id.trim().is_empty())
                            .or(if response_id.is_empty() {
                                None
                            } else {
                                Some(response_id.as_str())
                            })
                            .unwrap_or("");
                        anyhow::bail!(
                            "openai response failed: response_id={} type={} code={} message={} param={}",
                            failed_response_id,
                            error.r#type.as_deref().unwrap_or(""),
                            error.code.as_deref().unwrap_or(""),
                            error.message.as_deref().unwrap_or(""),
                            error.param.as_deref().unwrap_or("")
                        );
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
                            if let Some(id) = id {
                                response_id = id;
                            }
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
        last_usage = resp
            .usage
            .as_ref()
            .and_then(|usage| serde_json::to_value(usage).ok());
        if max_total_tokens > 0 {
            if let Some(tokens) = resp.usage.as_ref().and_then(usage_total_tokens) {
                total_tokens_used = total_tokens_used.saturating_add(tokens);
                if total_tokens_used > max_total_tokens {
                    return Err(
                        AgentTurnError::TokenBudgetExceeded {
                            used: total_tokens_used,
                            limit: max_total_tokens,
                        }
                        .into(),
                    );
                }
            }
        }

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

        let signature = step_signature(&function_calls);
        if let Some(kind) = loop_detector.observe(signature) {
            return Err(AgentTurnError::LoopDetected { kind }.into());
        }

        let can_parallelize_read_only = parallel_tool_calls
            && function_calls.len() > 1
            && function_calls
                .iter()
                .all(|(tool_name, _, _)| tool_is_read_only(tool_name));

        if can_parallelize_read_only {
            let batch_size = function_calls.len();
            if tool_calls_total + batch_size > max_tool_calls {
                return Err(AgentTurnError::BudgetExceeded {
                    budget: "tool_calls",
                }
                .into());
            }
            tool_calls_total += batch_size;

            let mut outputs = vec![None::<pm_openai::ResponseItem>; batch_size];
            let mut calls = Vec::new();

            for (idx, (tool_name, arguments, call_id)) in function_calls.into_iter().enumerate() {
                let args_json: Value = match serde_json::from_str(&arguments) {
                    Ok(v) => v,
                    Err(err) => {
                        let output = serde_json::json!({
                            "error": "invalid tool arguments",
                            "details": err.to_string(),
                            "arguments": arguments,
                        });
                        outputs[idx] = Some(pm_openai::ResponseItem::FunctionCallOutput {
                            call_id,
                            output: serde_json::to_string(&output)?,
                        });
                        continue;
                    }
                };
                calls.push((idx, tool_name, args_json, call_id));
            }

            let results = stream::iter(calls)
                .map(|(idx, tool_name, args_json, call_id)| {
                    let server = server.clone();
                    let cancel = cancel.clone();
                    async move {
                        let output_value = run_tool_call(
                            &server,
                            thread_id,
                            Some(turn_id),
                            &tool_name,
                            args_json,
                            cancel,
                        )
                        .await;
                        (idx, call_id, output_value)
                    }
                })
                .buffer_unordered(max_parallel_tool_calls)
                .collect::<Vec<_>>()
                .await;

            for (idx, call_id, output_value) in results {
                let output_value = match output_value {
                    Ok(v) => v,
                    Err(err) => serde_json::json!({ "error": err.to_string() }),
                };
                outputs[idx] = Some(pm_openai::ResponseItem::FunctionCallOutput {
                    call_id,
                    output: serde_json::to_string(&output_value)?,
                });
            }

            for output in outputs.into_iter().flatten() {
                input_items.push(output);
            }
        } else {
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

        if !attempted_auto_summary
            && should_auto_summarize(total_tokens_used, max_total_tokens, auto_summary_threshold_pct)
        {
            attempted_auto_summary = true;
            let cfg = AutoCompactSummaryConfig {
                threshold_pct: auto_summary_threshold_pct.min(MAX_AUTO_SUMMARY_THRESHOLD_PCT),
                source_max_chars: auto_summary_source_max_chars,
                tail_items: auto_summary_tail_items,
            };
            let ctx = AutoCompactSummaryContext {
                server: &server,
                thread_id,
                turn_id,
                model: &model,
                openai: &openai,
                max_openai_request_duration,
                max_total_tokens,
                total_tokens_used: &mut total_tokens_used,
                input_items: &mut input_items,
            };
            if !did_auto_summary && auto_compact_summary(ctx, cfg).await? {
                did_auto_summary = true;
            }
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

struct AutoCompactSummaryContext<'a> {
    server: &'a super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    model: &'a str,
    openai: &'a pm_openai::Client,
    max_openai_request_duration: Duration,
    max_total_tokens: u64,
    total_tokens_used: &'a mut u64,
    input_items: &'a mut Vec<pm_openai::ResponseItem>,
}

#[derive(Clone, Copy)]
struct AutoCompactSummaryConfig {
    threshold_pct: u64,
    source_max_chars: usize,
    tail_items: usize,
}

async fn auto_compact_summary(
    ctx: AutoCompactSummaryContext<'_>,
    cfg: AutoCompactSummaryConfig,
) -> anyhow::Result<bool> {
    let AutoCompactSummaryContext {
        server,
        thread_id,
        turn_id,
        model,
        openai,
        max_openai_request_duration,
        max_total_tokens,
        total_tokens_used,
        input_items,
    } = ctx;

    let transcript = render_items_for_summary(input_items, cfg.source_max_chars);
    if transcript.trim().is_empty() {
        return Ok(false);
    }

    let prompt = format!(
        "# Summarize session\n\nthread_id: {thread_id}\nturn_id: {turn_id}\n\n## Transcript\n\n{transcript}"
    );

    let summary_input = vec![pm_openai::ResponseItem::Message {
        role: "user".to_string(),
        content: vec![pm_openai::ContentItem::InputText { text: prompt }],
    }];
    let tools = Vec::<Value>::new();
    let req = pm_openai::ResponsesApiRequest {
        model,
        instructions: SUMMARY_INSTRUCTIONS,
        input: &summary_input,
        tools: &tools,
        tool_choice: "none",
        response_format: None,
        reasoning: None,
        parallel_tool_calls: false,
        store: false,
        stream: false,
    };

    let resp = match tokio::time::timeout(max_openai_request_duration, openai.create_response(&req))
        .await
    {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) => return Ok(false),
        Err(_) => return Ok(false),
    };

    if max_total_tokens > 0 {
        if let Some(tokens) = resp.usage.as_ref().and_then(usage_total_tokens) {
            *total_tokens_used = total_tokens_used.saturating_add(tokens);
            if *total_tokens_used > max_total_tokens {
                return Err(
                    AgentTurnError::TokenBudgetExceeded {
                        used: *total_tokens_used,
                        limit: max_total_tokens,
                    }
                    .into(),
                );
            }
        }
    }

    let summary_text = extract_assistant_text(&resp.output);
    let summary_text = summary_text.trim();
    if summary_text.is_empty() {
        return Ok(false);
    }
    let summary_text = pm_core::redact_text(summary_text);
    let summary_text = truncate_chars(&summary_text, 20_000);

    let artifact_value = match crate::handle_artifact_write(
        server,
        crate::ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "summary".to_string(),
            summary: format!("Summary (auto compact at {0}% of token budget)", cfg.threshold_pct),
            text: summary_text.clone(),
        },
    )
    .await
    {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };

    if artifact_value
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || artifact_value
            .get("denied")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return Ok(false);
    }

    let artifact_id = artifact_value
        .get("artifact_id")
        .cloned()
        .and_then(|value| serde_json::from_value::<ArtifactId>(value).ok());

    let tail_count = cfg.tail_items.min(input_items.len());
    let mut tail = input_items
        .iter()
        .rev()
        .take(tail_count)
        .cloned()
        .collect::<Vec<_>>();
    tail.reverse();

    let mut system_text = String::new();
    system_text.push_str("# Context summary\n\n");
    system_text.push_str(summary_text.trim());
    if let Some(artifact_id) = artifact_id {
        system_text.push_str(&format!("\n\n(summary artifact_id: {artifact_id})"));
    }

    input_items.clear();
    input_items.push(pm_openai::ResponseItem::Message {
        role: "system".to_string(),
        content: vec![pm_openai::ContentItem::InputText { text: system_text }],
    });
    input_items.extend(tail);

    Ok(true)
}

fn resolve_user_instructions_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CODE_PM_USER_INSTRUCTIONS_FILE") {
        let path = path.trim();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    let home = home_dir()?;
    Some(home.join(".codepm_data").join("AGENTS.md"))
}

fn builtin_openai_provider_config(provider: &str) -> Option<ditto_llm::ProviderConfig> {
    match provider {
        "openai-codex-apikey" => Some(ditto_llm::ProviderConfig {
            base_url: Some(DEFAULT_OPENAI_BASE_URL.to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            auth: Some(ditto_llm::ProviderAuth::ApiKeyEnv { keys: Vec::new() }),
        }),
        "openai-auth-command" => Some(ditto_llm::ProviderConfig {
            base_url: Some(DEFAULT_OPENAI_BASE_URL.to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            auth: Some(ditto_llm::ProviderAuth::Command { command: Vec::new() }),
        }),
        _ => None,
    }
}

fn merge_provider_config(
    mut base: ditto_llm::ProviderConfig,
    overrides: &ditto_llm::ProviderConfig,
) -> ditto_llm::ProviderConfig {
    if let Some(base_url) = overrides
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        base.base_url = Some(base_url.to_string());
    }
    if let Some(default_model) = overrides
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        base.default_model = Some(default_model.to_string());
    }
    if !overrides.model_whitelist.is_empty() {
        base.model_whitelist =
            ditto_llm::normalize_string_list(overrides.model_whitelist.clone());
    }
    if let Some(auth) = overrides.auth.clone() {
        base.auth = Some(auth);
    }
    base
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
}

async fn load_skills_from_input(input: &str, thread_cwd: Option<&str>) -> anyhow::Result<Option<String>> {
    let skill_names = parse_skill_names(input);
    if skill_names.is_empty() {
        return Ok(None);
    }

    let Some(thread_cwd) = thread_cwd else {
        return Ok(None);
    };

    let mut out = String::new();
    for name in skill_names {
        if let Some((path, contents)) = load_skill(&name, PathBuf::from(thread_cwd)).await? {
            out.push_str("\n\n# Skill\n\n");
            out.push_str(&format!("_Name: `{}`_\n\n", name));
            out.push_str(&format!("_Source: {}_\n\n", path.display()));
            out.push_str(&contents);
        } else {
            out.push_str("\n\n# Skill (missing)\n\n");
            out.push_str(&format!("_Name: `{}`_\n\n", name));
            out.push_str("_Reason: not found in configured skill directories._\n");
        }
    }

    Ok(Some(out))
}

fn parse_skill_names(input: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = std::collections::HashSet::<String>::new();

    let chars = input.chars().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] != '$' {
            idx += 1;
            continue;
        }
        idx += 1;
        let start = idx;
        while idx < chars.len() {
            let c = chars[idx];
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                idx += 1;
                continue;
            }
            break;
        }
        if idx <= start {
            continue;
        }
        let name = chars[start..idx].iter().collect::<String>();
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }

    out
}

async fn load_skill(name: &str, thread_root: PathBuf) -> anyhow::Result<Option<(PathBuf, String)>> {
    let mut roots = Vec::<PathBuf>::new();

    if let Ok(dir) = std::env::var("CODE_PM_SKILLS_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            roots.push(PathBuf::from(dir));
        }
    }

    roots.push(thread_root.join(".codepm_data").join("spec").join("skills"));
    roots.push(thread_root.join(".codex").join("skills"));

    if let Some(home) = home_dir() {
        roots.push(home.join(".codepm_data").join("spec").join("skills"));
    }

    let candidates = [name.to_string(), name.to_ascii_lowercase()];
    for root in roots {
        for candidate in candidates.iter() {
            let path = root.join(candidate).join("SKILL.md");
            match tokio::fs::read_to_string(&path).await {
                Ok(contents) => {
                    let contents = pm_core::redact_text(&contents);
                    return Ok(Some((path, contents)));
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
            }
        }
    }

    Ok(None)
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

fn parse_bool_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|value| parse_bool_value(&value))
        .unwrap_or(default)
}

fn tool_is_read_only(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_read"
            | "file_glob"
            | "file_grep"
            | "process_inspect"
            | "process_tail"
            | "process_follow"
            | "artifact_list"
            | "artifact_read"
            | "thread_state"
            | "thread_events"
    )
}

fn usage_total_tokens(usage: &pm_openai::TokenUsage) -> Option<u64> {
    usage.total_tokens.or_else(|| match (usage.input_tokens, usage.output_tokens) {
        (Some(input), Some(output)) => input.checked_add(output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    })
}

async fn load_latest_summary_artifact(
    server: &super::Server,
    thread_id: ThreadId,
) -> anyhow::Result<Option<(ArtifactMetadata, String)>> {
    let dir = crate::user_artifacts_dir_for_thread(server, thread_id);
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };

    let mut latest: Option<ArtifactMetadata> = None;
    loop {
        let Some(entry) = read_dir
            .next_entry()
            .await
            .with_context(|| format!("read {}", dir.display()))?
        else {
            break;
        };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".metadata.json") {
            continue;
        }

        let meta = match crate::read_artifact_metadata(&path).await {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.artifact_type != "summary" {
            continue;
        }

        let should_replace = match &latest {
            None => true,
            Some(prev) => meta
                .updated_at
                .unix_timestamp_nanos()
                .cmp(&prev.updated_at.unix_timestamp_nanos())
                .then_with(|| meta.artifact_id.cmp(&prev.artifact_id))
                .is_gt(),
        };
        if should_replace {
            latest = Some(meta);
        }
    }

    let Some(meta) = latest else {
        return Ok(None);
    };

    let (content_path, _metadata_path) = crate::user_artifact_paths(server, thread_id, meta.artifact_id);
    let text = tokio::fs::read_to_string(&content_path)
        .await
        .with_context(|| format!("read {}", content_path.display()))?;
    Ok(Some((meta, text)))
}

async fn build_conversation(
    server: &super::Server,
    thread_id: ThreadId,
) -> anyhow::Result<Vec<pm_openai::ResponseItem>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    let mut input = Vec::new();

    if let Some((meta, summary_text)) = load_latest_summary_artifact(server, thread_id).await? {
        let event_limit = parse_env_usize(
            "CODE_PM_AGENT_SUMMARY_CONTEXT_EVENT_LIMIT",
            DEFAULT_SUMMARY_CONTEXT_EVENT_LIMIT,
            0,
            MAX_SUMMARY_CONTEXT_EVENT_LIMIT,
        );

        let summary_text = pm_core::redact_text(&summary_text);
        if !summary_text.trim().is_empty() {
            input.push(pm_openai::ResponseItem::Message {
                role: "system".to_string(),
                content: vec![pm_openai::ContentItem::InputText {
                    text: format!(
                        "# Context summary\n\n{}\n\n(summary artifact_id: {})",
                        summary_text.trim(),
                        meta.artifact_id
                    ),
                }],
            });
        }

        let mut start_idx = 0usize;
        if let Some(summary_turn_id) = meta.provenance.as_ref().and_then(|p| p.turn_id) {
            if let Some(idx) = events.iter().rposition(|event| {
                matches!(
                    &event.kind,
                    ThreadEventKind::TurnCompleted { turn_id, .. } if *turn_id == summary_turn_id
                )
            }) {
                start_idx = idx + 1;
            }
        }

        let mut slice = &events[start_idx..];
        if event_limit > 0 && slice.len() > event_limit {
            slice = &slice[slice.len().saturating_sub(event_limit)..];
        }

        for event in slice {
            match &event.kind {
                ThreadEventKind::TurnStarted { input: text, .. } => {
                    input.push(pm_openai::ResponseItem::Message {
                        role: "user".to_string(),
                        content: vec![pm_openai::ContentItem::InputText { text: text.clone() }],
                    });
                }
                ThreadEventKind::AssistantMessage { text, .. } => {
                    input.push(pm_openai::ResponseItem::Message {
                        role: "assistant".to_string(),
                        content: vec![pm_openai::ContentItem::OutputText { text: text.clone() }],
                    });
                }
                other => {
                    if let Some(text) = format_event_for_context(other) {
                        input.push(pm_openai::ResponseItem::Message {
                            role: "assistant".to_string(),
                            content: vec![pm_openai::ContentItem::OutputText { text }],
                        });
                    }
                }
            }
        }

        return Ok(input);
    }

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
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            model,
            openai_base_url,
        } => Some(format!(
            "[thread/config] approval_policy={approval_policy:?} sandbox_policy={} sandbox_writable_roots={} sandbox_network_access={} mode={} model={} openai_base_url={}",
            sandbox_policy
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|| "<unchanged>".to_string()),
            sandbox_writable_roots
                .as_ref()
                .map(|roots| json_one_line(&serde_json::json!(roots), 2000))
                .unwrap_or_else(|| "<unchanged>".to_string()),
            sandbox_network_access
                .as_ref()
                .map(|access| format!("{access:?}"))
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

#[cfg(test)]
mod tool_parallelism_tests {
    use super::*;

    #[test]
    fn parse_bool_value_accepts_common_values() {
        assert_eq!(parse_bool_value("1"), Some(true));
        assert_eq!(parse_bool_value("true"), Some(true));
        assert_eq!(parse_bool_value("YES"), Some(true));
        assert_eq!(parse_bool_value("on"), Some(true));

        assert_eq!(parse_bool_value("0"), Some(false));
        assert_eq!(parse_bool_value("false"), Some(false));
        assert_eq!(parse_bool_value("No"), Some(false));
        assert_eq!(parse_bool_value("off"), Some(false));

        assert_eq!(parse_bool_value("maybe"), None);
        assert_eq!(parse_bool_value(""), None);
    }

    #[test]
    fn usage_total_tokens_prefers_total_tokens() {
        let usage = pm_openai::TokenUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            total_tokens: Some(20),
            input_tokens_details: None,
            output_tokens_details: None,
            other: std::collections::BTreeMap::new(),
        };
        assert_eq!(usage_total_tokens(&usage), Some(20));
    }

    #[test]
    fn usage_total_tokens_falls_back_to_input_plus_output() {
        let usage = pm_openai::TokenUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            total_tokens: None,
            input_tokens_details: None,
            output_tokens_details: None,
            other: std::collections::BTreeMap::new(),
        };
        assert_eq!(usage_total_tokens(&usage), Some(15));
    }

    #[test]
    fn tool_is_read_only_is_conservative() {
        assert!(tool_is_read_only("file_read"));
        assert!(tool_is_read_only("file_glob"));
        assert!(tool_is_read_only("file_grep"));
        assert!(tool_is_read_only("process_inspect"));
        assert!(tool_is_read_only("process_tail"));
        assert!(tool_is_read_only("process_follow"));
        assert!(tool_is_read_only("artifact_list"));
        assert!(tool_is_read_only("artifact_read"));
        assert!(tool_is_read_only("thread_state"));
        assert!(tool_is_read_only("thread_events"));

        assert!(!tool_is_read_only("file_write"));
        assert!(!tool_is_read_only("file_patch"));
        assert!(!tool_is_read_only("file_edit"));
        assert!(!tool_is_read_only("file_delete"));
        assert!(!tool_is_read_only("fs_mkdir"));
        assert!(!tool_is_read_only("process_start"));
        assert!(!tool_is_read_only("process_kill"));
        assert!(!tool_is_read_only("artifact_write"));
        assert!(!tool_is_read_only("artifact_delete"));
        assert!(!tool_is_read_only("thread_hook_run"));
        assert!(!tool_is_read_only("agent_spawn"));
    }
}

#[cfg(test)]
mod loop_detection_tests {
    use super::*;

    #[test]
    fn tool_call_signature_is_stable_for_object_key_order() {
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"b": 2, "a": 1});
        assert_eq!(tool_call_signature("file_read", &a), tool_call_signature("file_read", &b));
    }

    #[test]
    fn loop_detector_trips_on_consecutive_calls() {
        let mut detector = LoopDetector::new();
        let sig = tool_call_signature("file_read", &serde_json::json!({"path": "a.txt"}));
        assert_eq!(detector.observe(sig), None);
        assert_eq!(detector.observe(sig), None);
        assert_eq!(detector.observe(sig), Some("consecutive"));
    }

    #[test]
    fn loop_detector_trips_on_short_cycle() {
        let mut detector = LoopDetector::new();
        let a = tool_call_signature("file_read", &serde_json::json!({"path": "a.txt"}));
        let b = tool_call_signature("file_read", &serde_json::json!({"path": "b.txt"}));

        assert_eq!(detector.observe(a), None);
        assert_eq!(detector.observe(b), None);
        assert_eq!(detector.observe(a), None);
        assert_eq!(detector.observe(b), Some("cycle"));
    }
}

#[cfg(test)]
mod auto_summary_tests {
    use super::*;

    use pm_core::{PmPaths, ThreadStore};
    use pm_protocol::TurnStatus;
    use tokio::sync::broadcast;

    fn build_test_server(pm_root: PathBuf) -> crate::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        crate::Server {
            cwd: pm_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    #[test]
    fn should_auto_summarize_triggers_at_threshold() {
        assert!(!should_auto_summarize(0, 0, 80));
        assert!(!should_auto_summarize(79, 100, 80));
        assert!(should_auto_summarize(80, 100, 80));
        assert!(should_auto_summarize(100, 100, 80));
    }

    #[tokio::test]
    async fn build_conversation_prefers_latest_summary_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id1 = TurnId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id: turn_id1,
                input: "first".to_string(),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id1),
                text: "hello".to_string(),
                model: None,
                response_id: None,
                token_usage: None,
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnCompleted {
                turn_id: turn_id1,
                status: TurnStatus::Completed,
                reason: None,
            })
            .await?;

        let _summary = crate::handle_artifact_write(
            &server,
            crate::ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id1),
                approval_id: None,
                artifact_id: None,
                artifact_type: "summary".to_string(),
                summary: "summary".to_string(),
                text: "This is the summary.".to_string(),
            },
        )
        .await?;

        let turn_id2 = TurnId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id: turn_id2,
                input: "second".to_string(),
            })
            .await?;

        let items = build_conversation(&server, thread_id).await?;
        assert!(
            items.iter().any(|item| match item {
                pm_openai::ResponseItem::Message { role, content } => {
                    role == "system"
                        && content.iter().any(|part| match part {
                            pm_openai::ContentItem::InputText { text } => {
                                text.contains("This is the summary.")
                            }
                            _ => false,
                        })
                }
                _ => false,
            }),
            "expected system summary message"
        );
        assert!(
            items.iter().any(|item| match item {
                pm_openai::ResponseItem::Message { role, content } => {
                    role == "user"
                        && content.iter().any(|part| match part {
                            pm_openai::ContentItem::InputText { text } => text == "second",
                            _ => false,
                        })
                }
                _ => false,
            }),
            "expected latest turn input"
        );
        assert!(
            !items.iter().any(|item| match item {
                pm_openai::ResponseItem::Message { role, content } => {
                    role == "user"
                        && content.iter().any(|part| match part {
                            pm_openai::ContentItem::InputText { text } => text == "first",
                            _ => false,
                        })
                }
                _ => false,
            }),
            "expected summary to replace older turn input"
        );

        Ok(())
    }
}
