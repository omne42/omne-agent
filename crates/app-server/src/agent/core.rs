use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use crate::project_config::ProjectOpenAiOverrides;
use base64::Engine;
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
const DEFAULT_LLM_MAX_ATTEMPTS: usize = 3;
const MAX_LLM_MAX_ATTEMPTS: usize = 20;
const DEFAULT_LLM_RETRY_BASE_DELAY_MS: u64 = 200;
const DEFAULT_LLM_RETRY_MAX_DELAY_MS: u64 = 2_000;
const MAX_LLM_RETRY_DELAY_MS: u64 = 60_000;

const DEFAULT_AGENT_MAX_ATTACHMENTS: usize = 4;
const MAX_AGENT_MAX_ATTACHMENTS: usize = 32;
const DEFAULT_AGENT_MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_AGENT_MAX_ATTACHMENT_BYTES: u64 = 200 * 1024 * 1024;

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

#[derive(Debug, Error)]
enum LlmAttemptError {
    #[error("llm request timed out")]
    TimedOut,
    #[error(transparent)]
    Ditto(#[from] ditto_llm::DittoError),
}

#[derive(Debug)]
struct LlmAttemptFailure {
    error: LlmAttemptError,
    emitted_output: bool,
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

#[derive(Debug, Clone)]
struct AgentLlmResponse {
    id: String,
    output: Vec<pm_openai::ResponseItem>,
    usage: Option<pm_openai::TokenUsage>,
    warnings: Vec<ditto_llm::Warning>,
}

fn token_usage_from_ditto_usage(usage: &ditto_llm::Usage) -> Option<pm_openai::TokenUsage> {
    if usage.input_tokens.is_none() && usage.output_tokens.is_none() && usage.total_tokens.is_none()
    {
        return None;
    }

    Some(pm_openai::TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        input_tokens_details: None,
        output_tokens_details: None,
        other: std::collections::BTreeMap::new(),
    })
}

fn response_items_to_ditto_messages(
    instructions: &str,
    items: &[pm_openai::ResponseItem],
    attachments: &[ditto_llm::ContentPart],
) -> Vec<ditto_llm::Message> {
    let mut out = Vec::<ditto_llm::Message>::new();
    if !instructions.trim().is_empty() {
        out.push(ditto_llm::Message::system(instructions.to_string()));
    }

    for item in items {
        match item {
            pm_openai::ResponseItem::Message { role, content } => {
                let role = match role.as_str() {
                    "system" => ditto_llm::Role::System,
                    "user" => ditto_llm::Role::User,
                    "assistant" => ditto_llm::Role::Assistant,
                    "tool" => ditto_llm::Role::Tool,
                    _ => ditto_llm::Role::User,
                };

                let mut parts = Vec::<ditto_llm::ContentPart>::new();
                for part in content {
                    match part {
                        pm_openai::ContentItem::InputText { text }
                        | pm_openai::ContentItem::OutputText { text } => {
                            if text.is_empty() {
                                continue;
                            }
                            parts.push(ditto_llm::ContentPart::Text { text: text.clone() });
                        }
                        pm_openai::ContentItem::Other => {}
                    }
                }
                if !parts.is_empty() {
                    out.push(ditto_llm::Message { role, content: parts });
                }
            }
            pm_openai::ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
            } => {
                let args = serde_json::from_str::<Value>(arguments)
                    .unwrap_or_else(|_| Value::String(arguments.clone()));
                out.push(ditto_llm::Message {
                    role: ditto_llm::Role::Assistant,
                    content: vec![ditto_llm::ContentPart::ToolCall {
                        id: call_id.clone(),
                        name: name.clone(),
                        arguments: args,
                    }],
                });
            }
            pm_openai::ResponseItem::FunctionCallOutput { call_id, output } => {
                out.push(ditto_llm::Message {
                    role: ditto_llm::Role::Tool,
                    content: vec![ditto_llm::ContentPart::ToolResult {
                        tool_call_id: call_id.clone(),
                        content: output.clone(),
                        is_error: None,
                    }],
                });
            }
            pm_openai::ResponseItem::Other => {}
        }
    }

    if !attachments.is_empty() {
        if let Some(idx) = out.iter().rposition(|msg| msg.role == ditto_llm::Role::User) {
            out[idx].content.extend_from_slice(attachments);
        } else {
            out.push(ditto_llm::Message {
                role: ditto_llm::Role::User,
                content: attachments.to_vec(),
            });
        }
    }

    out
}

fn tool_specs_to_ditto_tools(specs: &[Value]) -> anyhow::Result<Vec<ditto_llm::Tool>> {
    let mut out = Vec::<ditto_llm::Tool>::new();
    for spec in specs {
        let obj = spec
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("tool spec must be an object"))?;
        let kind = obj.get("type").and_then(Value::as_str).unwrap_or("");
        if kind != "function" {
            anyhow::bail!("unsupported tool spec type: {kind}");
        }
        let function = obj
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("tool spec.function must be an object"))?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("tool spec.function.name must be a string"))?;
        let description = function
            .get("description")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let parameters = function.get("parameters").cloned().unwrap_or(Value::Null);
        out.push(ditto_llm::Tool {
            name: name.to_string(),
            description,
            parameters,
            strict: None,
        });
    }
    Ok(out)
}

fn log_llm_warnings(thread_id: ThreadId, turn_id: TurnId, warnings: &[ditto_llm::Warning]) {
    if warnings.is_empty() {
        return;
    }
    let warnings_json = serde_json::to_string(warnings).unwrap_or_else(|_| "<unknown>".to_string());
    tracing::info!(
        thread_id = %thread_id,
        turn_id = %turn_id,
        warnings = warnings_json,
        "llm warnings"
    );
}

#[derive(Clone)]
struct ProviderRuntime {
    config: ditto_llm::ProviderConfig,
    capabilities: ditto_llm::ProviderCapabilities,
    client: Arc<dyn ditto_llm::LanguageModel>,
}

fn parse_csv_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let value = part.to_string();
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

fn build_provider_candidates(primary: &str, fallbacks: Vec<String>) -> Vec<String> {
    let mut out = vec![primary.to_string()];
    for provider in fallbacks {
        if provider == primary {
            continue;
        }
        if out.iter().any(|existing| existing == &provider) {
            continue;
        }
        out.push(provider);
    }
    out
}

fn build_model_candidates(primary: &str, fallbacks: Vec<String>) -> Vec<String> {
    let mut out = vec![primary.to_string()];
    for model in fallbacks {
        if model == primary {
            continue;
        }
        if out.iter().any(|existing| existing == &model) {
            continue;
        }
        out.push(model);
    }
    out
}

fn model_allowed_by_whitelist(model: &str, whitelist: &[String]) -> bool {
    whitelist.is_empty() || whitelist.iter().any(|allowed| allowed.trim() == model)
}

fn llm_error_is_retryable(err: &LlmAttemptError) -> bool {
    match err {
        LlmAttemptError::TimedOut => true,
        LlmAttemptError::Ditto(ditto_llm::DittoError::Api { status, .. }) => {
            status.as_u16() == 429 || status.is_server_error()
        }
        LlmAttemptError::Ditto(ditto_llm::DittoError::Http(err)) => err.is_timeout() || err.is_connect(),
        LlmAttemptError::Ditto(ditto_llm::DittoError::Io(_)) => true,
        LlmAttemptError::Ditto(ditto_llm::DittoError::InvalidResponse(_))
        | LlmAttemptError::Ditto(ditto_llm::DittoError::AuthCommand(_))
        | LlmAttemptError::Ditto(ditto_llm::DittoError::Json(_)) => false,
    }
}

fn llm_error_prefers_provider_fallback(err: &LlmAttemptError) -> bool {
    match err {
        LlmAttemptError::TimedOut => true,
        LlmAttemptError::Ditto(ditto_llm::DittoError::Api { status, .. }) => {
            status.as_u16() == 429 || status.is_server_error()
        }
        _ => false,
    }
}

fn llm_error_prefers_model_fallback(err: &LlmAttemptError) -> bool {
    match err {
        LlmAttemptError::Ditto(ditto_llm::DittoError::Api { status, .. }) => {
            matches!(status.as_u16(), 400 | 404 | 413 | 422)
        }
        _ => false,
    }
}

fn llm_error_summary(err: &LlmAttemptError) -> String {
    let text = pm_core::redact_text(&err.to_string());
    truncate_chars(&text, 300)
}

fn retry_backoff_delay(failure_count: usize, base: Duration, max: Duration) -> Duration {
    if base.is_zero() || max.is_zero() {
        return Duration::ZERO;
    }
    let exponent = failure_count.saturating_sub(1).min(10) as u32;
    let multiplier = 1u32 << exponent;
    let delay = base * multiplier;
    if delay > max { max } else { delay }
}

async fn build_provider_runtime(
    provider: &str,
    project_overrides: &ProjectOpenAiOverrides,
    base_url_override: Option<&str>,
    env: &ditto_llm::Env,
) -> anyhow::Result<ProviderRuntime> {
    let builtin_provider_config = builtin_openai_provider_config(provider);
    let provider_overrides = project_overrides.providers.get(provider);
    if builtin_provider_config.is_none() && provider_overrides.is_none() {
        anyhow::bail!(
            "unknown openai provider: {provider} (expected: openai-codex-apikey, openai-auth-command; or define [openai.providers.{provider}] in project config)"
        );
    }

    let mut provider_config = builtin_provider_config.unwrap_or_default();
    if let Some(overrides) = provider_overrides {
        provider_config = merge_provider_config(provider_config, overrides);
    }

    let provider_capabilities = provider_config
        .capabilities
        .unwrap_or_else(ditto_llm::ProviderCapabilities::openai_responses);
    if !provider_capabilities.tools {
        anyhow::bail!(
            "provider does not support tools: provider={provider} (CodePM requires tool calling; set [openai.providers.{provider}.capabilities.tools]=true)"
        );
    }
    if !provider_capabilities.streaming {
        anyhow::bail!(
            "provider does not support streaming: provider={provider} (set [openai.providers.{provider}.capabilities.streaming]=true or choose a streaming-capable provider)"
        );
    }

    let base_url = base_url_override
        .map(|value| value.to_string())
        .or(provider_config.base_url.clone())
        .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
    let base_url = base_url.trim().to_string();
    if base_url.is_empty() {
        anyhow::bail!("openai provider {provider} is missing base_url");
    }

    let provider_for_llm = ditto_llm::ProviderConfig {
        base_url: Some(base_url),
        default_model: provider_config.default_model.clone(),
        model_whitelist: provider_config.model_whitelist.clone(),
        auth: provider_config.auth.clone(),
        capabilities: Some(provider_capabilities),
    };

    let client: Arc<dyn ditto_llm::LanguageModel> = if provider_capabilities.reasoning {
        Arc::new(
            ditto_llm::OpenAI::from_config(&provider_for_llm, env)
                .await
                .context("build OpenAI Responses client")?,
        )
    } else {
        Arc::new(
            ditto_llm::OpenAICompatible::from_config(&provider_for_llm, env)
                .await
                .context("build OpenAI-compatible Chat Completions client")?,
        )
    };

    Ok(ProviderRuntime {
        config: provider_for_llm,
        capabilities: provider_capabilities,
        client,
    })
}

async fn run_llm_stream_once(
    llm: Arc<dyn ditto_llm::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    req: ditto_llm::GenerateRequest,
    max_openai_request_duration: Duration,
) -> Result<AgentLlmResponse, LlmAttemptFailure> {
    #[derive(Default)]
    struct ToolCallBuffer {
        name: Option<String>,
        arguments: String,
    }

    let mut emitted_output = false;

    let inner = async {
        let mut stream = llm.stream(req).await?;
        let mut response_id = String::new();
        let mut usage: Option<ditto_llm::Usage> = None;
        let mut output_items = Vec::<pm_openai::ResponseItem>::new();
        let mut output_text = String::new();
        let mut tool_call_order = Vec::<String>::new();
        let mut tool_calls = std::collections::BTreeMap::<String, ToolCallBuffer>::new();
        let mut seen_tool_call_ids = std::collections::HashSet::<String>::new();
        let mut warnings = Vec::<ditto_llm::Warning>::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            match chunk {
                ditto_llm::StreamChunk::Warnings { warnings: w } => warnings.extend(w),
                ditto_llm::StreamChunk::ResponseId { id } => {
                    if response_id.is_empty() && !id.trim().is_empty() {
                        response_id = id;
                    }
                }
                ditto_llm::StreamChunk::TextDelta { text } => {
                    if text.is_empty() {
                        continue;
                    }
                    emitted_output = true;
                    output_text.push_str(&text);
                    let delta = pm_core::redact_text(&text);
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
                ditto_llm::StreamChunk::ToolCallStart { id, name } => {
                    emitted_output = true;
                    let slot = tool_calls.entry(id.clone()).or_default();
                    if slot.name.is_none() && !name.trim().is_empty() {
                        slot.name = Some(name);
                    }
                    if seen_tool_call_ids.insert(id.clone()) {
                        tool_call_order.push(id);
                    }
                }
                ditto_llm::StreamChunk::ToolCallDelta {
                    id,
                    arguments_delta,
                } => {
                    emitted_output = true;
                    let slot = tool_calls.entry(id.clone()).or_default();
                    slot.arguments.push_str(&arguments_delta);
                    if seen_tool_call_ids.insert(id.clone()) {
                        tool_call_order.push(id);
                    }
                }
                ditto_llm::StreamChunk::ReasoningDelta { .. } => {
                    emitted_output = true;
                }
                ditto_llm::StreamChunk::Usage(u) => usage = Some(u),
                ditto_llm::StreamChunk::FinishReason(_) => {}
            }
        }

        if response_id.trim().is_empty() {
            response_id = "<unknown>".to_string();
        }

        if !output_text.is_empty() {
            let output_text = pm_core::redact_text(&output_text);
            output_items.push(pm_openai::ResponseItem::Message {
                role: "assistant".to_string(),
                content: vec![pm_openai::ContentItem::OutputText { text: output_text }],
            });
        }

        for id in tool_call_order {
            let Some(call) = tool_calls.get(&id) else {
                continue;
            };
            let Some(name) = call.name.as_deref().filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            let args = if call.arguments.trim().is_empty() {
                "{}".to_string()
            } else {
                call.arguments.clone()
            };
            output_items.push(pm_openai::ResponseItem::FunctionCall {
                name: name.to_string(),
                arguments: args,
                call_id: id,
            });
        }

        Ok::<_, ditto_llm::DittoError>(AgentLlmResponse {
            id: response_id,
            output: output_items,
            usage: usage.as_ref().and_then(token_usage_from_ditto_usage),
            warnings,
        })
    };

    match tokio::time::timeout(max_openai_request_duration, inner).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(err)) => Err(LlmAttemptFailure {
            error: err.into(),
            emitted_output,
        }),
        Err(_) => Err(LlmAttemptFailure {
            error: LlmAttemptError::TimedOut,
            emitted_output,
        }),
    }
}

pub async fn run_agent_turn(
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    turn_id: TurnId,
    input: String,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let (thread_id, thread_mode, thread_model, thread_openai_base_url, thread_cwd, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            handle.thread_id(),
            state.mode.clone(),
            state.model.clone(),
            state.openai_base_url.clone(),
            state.cwd.clone(),
            state.allowed_tools.clone(),
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
        .clone()
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

    let base_url_override = thread_openai_base_url
        .clone()
        .or(project_overrides.base_url.clone())
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });
    let base_url = base_url_override
        .clone()
        .or(provider_config.base_url.clone())
        .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
    let base_url = base_url.trim().to_string();
    if base_url.is_empty() {
        anyhow::bail!("openai provider {provider} is missing base_url");
    }

    let provider_capabilities = provider_config
        .capabilities
        .unwrap_or_else(ditto_llm::ProviderCapabilities::openai_responses);
    if !provider_capabilities.tools {
        anyhow::bail!(
            "provider does not support tools: provider={provider} (CodePM requires tool calling; set [openai.providers.{provider}.capabilities.tools]=true)"
        );
    }
    if !provider_capabilities.streaming {
        anyhow::bail!(
            "provider does not support streaming: provider={provider} (set [openai.providers.{provider}.capabilities.streaming]=true or choose a streaming-capable provider)"
        );
    }

    let env = ditto_llm::Env {
        dotenv: std::mem::take(&mut project_overrides.dotenv),
    };
    let provider_for_llm = ditto_llm::ProviderConfig {
        base_url: Some(base_url),
        default_model: provider_config.default_model.clone(),
        model_whitelist: provider_config.model_whitelist.clone(),
        auth: provider_config.auth.clone(),
        capabilities: Some(provider_capabilities),
    };
    let model_client: Arc<dyn ditto_llm::LanguageModel> = if provider_capabilities.reasoning {
        Arc::new(
            ditto_llm::OpenAI::from_config(&provider_for_llm, &env)
                .await
                .context("build OpenAI Responses client")?,
        )
    } else {
        Arc::new(
            ditto_llm::OpenAICompatible::from_config(&provider_for_llm, &env)
                .await
                .context("build OpenAI-compatible Chat Completions client")?,
        )
    };

    let fallbacks = std::env::var("CODE_PM_OPENAI_FALLBACK_PROVIDERS")
        .ok()
        .map(|value| parse_csv_list(&value))
        .unwrap_or_else(|| project_overrides.fallback_providers.clone());
    let provider_candidates = build_provider_candidates(&provider, fallbacks);
    let mut provider_cache = std::collections::BTreeMap::<String, ProviderRuntime>::new();
    provider_cache.insert(
        provider.clone(),
        ProviderRuntime {
            config: provider_for_llm,
            capabilities: provider_capabilities,
            client: model_client.clone(),
        },
    );

    let tool_specs = build_tools();
    let tools = tool_specs_to_ditto_tools(&tool_specs).context("parse tool schemas")?;

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
    let llm_max_attempts = parse_env_usize(
        "CODE_PM_AGENT_LLM_MAX_ATTEMPTS",
        DEFAULT_LLM_MAX_ATTEMPTS,
        1,
        MAX_LLM_MAX_ATTEMPTS,
    );
    let llm_retry_base_delay = Duration::from_millis(parse_env_u64(
        "CODE_PM_AGENT_LLM_RETRY_BASE_DELAY_MS",
        DEFAULT_LLM_RETRY_BASE_DELAY_MS,
        0,
        MAX_LLM_RETRY_DELAY_MS,
    ));
    let llm_retry_max_delay = Duration::from_millis(parse_env_u64(
        "CODE_PM_AGENT_LLM_RETRY_MAX_DELAY_MS",
        DEFAULT_LLM_RETRY_MAX_DELAY_MS,
        0,
        MAX_LLM_RETRY_DELAY_MS,
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
                    serde_json::from_str::<ditto_llm::ResponseFormat>(raw)
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

    let session_start_hook_contexts =
        super::run_session_start_hooks(server.as_ref(), thread_id, turn_id).await;
    if !session_start_hook_contexts.is_empty() {
        instructions.push_str("\n\n# Additional context (hooks/session_start)\n\n");
        for ctx in &session_start_hook_contexts {
            if let Some(summary) = ctx.summary.as_deref() {
                instructions.push_str(&format!("## {}\n\n", summary.trim()));
            }
            instructions.push_str(ctx.text.trim());
            instructions.push_str("\n\n");
        }
    }

    let mut input_items = build_conversation(&server, thread_id).await?;
    if let Ok(context_refs) = load_turn_context_refs(&server, thread_id, turn_id).await {
        if !context_refs.is_empty() {
            let ctx_items =
                context_refs_to_messages(&server, thread_id, turn_id, &context_refs, cancel.clone())
                    .await;
            match ctx_items {
                Ok(ctx_items) => insert_context_before_last_user_message(&mut input_items, ctx_items),
                Err(err) => {
                    input_items.push(pm_openai::ResponseItem::Message {
                        role: "system".to_string(),
                        content: vec![pm_openai::ContentItem::InputText {
                            text: format!("[context_refs] failed to resolve: {}", err),
                        }],
                    });
                }
            }
        }
    }

    let attachments = load_turn_attachments(&server, thread_id, turn_id).await?;
    let max_attachments = parse_env_usize(
        "CODE_PM_AGENT_MAX_ATTACHMENTS",
        DEFAULT_AGENT_MAX_ATTACHMENTS,
        0,
        MAX_AGENT_MAX_ATTACHMENTS,
    );
    if max_attachments > 0 && attachments.len() > max_attachments {
        anyhow::bail!(
            "too many attachments: count={} max={}",
            attachments.len(),
            max_attachments
        );
    }
    let max_attachment_bytes = parse_env_u64(
        "CODE_PM_AGENT_MAX_ATTACHMENT_BYTES",
        DEFAULT_AGENT_MAX_ATTACHMENT_BYTES,
        0,
        MAX_AGENT_MAX_ATTACHMENT_BYTES,
    );
    let attachment_parts = if attachments.is_empty() {
        Vec::new()
    } else {
        attachments_to_ditto_parts(
            thread_root.as_deref(),
            thread_mode.as_str(),
            allowed_tools.as_deref(),
            &attachments,
            max_attachment_bytes,
        )
        .await?
    };
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
        selected_model,
        rule_source,
        reason,
        rule_id,
    } = routed;

    let mut model = selected_model;
    if !model_allowed_by_whitelist(&model, &provider_config.model_whitelist) {
        anyhow::bail!(
            "model not allowed by provider whitelist: provider={provider} model={model}"
        );
    }

    let model_fallbacks = std::env::var("CODE_PM_AGENT_FALLBACK_MODELS")
        .ok()
        .map(|value| parse_csv_list(&value))
        .unwrap_or_default();
    let mut model_candidates = build_model_candidates(&model, model_fallbacks);
    if !provider_config.model_whitelist.is_empty() {
        model_candidates.retain(|candidate| {
            model_allowed_by_whitelist(candidate, &provider_config.model_whitelist)
        });
    }
    let mut model_idx = 0usize;

    let reason = reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{value}; provider={provider}"))
        .or_else(|| Some(format!("provider={provider}")));

    let _ = thread_rt
        .append_event(ThreadEventKind::ModelRouted {
            turn_id,
            selected_model: model.clone(),
            rule_source,
            reason,
            rule_id: rule_id.clone(),
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
    let mut active_provider_idx = 0usize;

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

        let messages =
            response_items_to_ditto_messages(&instructions, &input_items, &attachment_parts);
        let mut req_base = ditto_llm::GenerateRequest::from(messages);
        req_base.model = Some(model.clone());
        req_base.tools = Some(tools.clone());
        req_base.tool_choice = Some(ditto_llm::ToolChoice::Auto);

        let mut provider_index = active_provider_idx.min(provider_candidates.len().saturating_sub(1));
        let mut attempts = 0usize;
        let mut failure_count = 0usize;
        let mut last_failure: Option<LlmAttemptFailure> = None;

        let resp = loop {
            if cancel.is_cancelled() {
                return Err(AgentTurnError::Cancelled.into());
            }
            if started_at.elapsed() > max_turn_duration {
                return Err(AgentTurnError::BudgetExceeded {
                    budget: "turn_seconds",
                }
                .into());
            }
            if provider_index >= provider_candidates.len() {
                if let Some(failure) = last_failure.as_ref()
                    && llm_error_prefers_model_fallback(&failure.error)
                    && model_idx + 1 < model_candidates.len()
                {
                    let cause = llm_error_summary(&failure.error);
                    let prev = model.clone();
                    model_idx += 1;
                    model = model_candidates[model_idx].clone();
                    req_base.model = Some(model.clone());
                    provider_index = 0;
                    attempts = 0;
                    failure_count = 0;
                    last_failure = None;

                    let reason = format!("model_fallback: from={prev} to={model}; cause={cause}");
                    let _ = thread_rt
                        .append_event(ThreadEventKind::ModelRouted {
                            turn_id,
                            selected_model: model.clone(),
                            rule_source,
                            reason: Some(reason),
                            rule_id: rule_id.clone(),
                        })
                        .await;
                    continue;
                }

                match last_failure {
                    Some(LlmAttemptFailure {
                        error: LlmAttemptError::TimedOut,
                        ..
                    }) => return Err(AgentTurnError::OpenAiRequestTimedOut.into()),
                    Some(LlmAttemptFailure { error, .. }) => {
                        return Err(anyhow::Error::new(error).context("llm stream failed"))
                    }
                    None => anyhow::bail!("no usable openai provider available for model={model}"),
                }
            }

            let provider_name = provider_candidates
                .get(provider_index)
                .cloned()
                .unwrap_or_else(|| provider.clone());
            let runtime = match provider_cache.get(&provider_name).cloned() {
                Some(runtime) => runtime,
                None => match build_provider_runtime(
                    &provider_name,
                    &project_overrides,
                    base_url_override.as_deref(),
                    &env,
                )
                .await
                {
                    Ok(runtime) => {
                        provider_cache.insert(provider_name.clone(), runtime.clone());
                        runtime
                    }
                    Err(err) => {
                        tracing::warn!(
                            thread_id = %thread_id,
                            turn_id = %turn_id,
                            provider = provider_name,
                            error = %err,
                            "failed to build provider client; skipping"
                        );
                        provider_index = provider_index.saturating_add(1);
                        continue;
                    }
                },
            };

            if !model_allowed_by_whitelist(&model, &runtime.config.model_whitelist) {
                provider_index = provider_index.saturating_add(1);
                continue;
            }

            let reasoning_effort = if runtime.capabilities.reasoning {
                match ditto_llm::select_model_config(&project_overrides.models, &model)
                    .map(|cfg| cfg.thinking)
                    .unwrap_or_default()
                {
                    ThinkingIntensity::Unsupported => None,
                    ThinkingIntensity::Small => Some(ditto_llm::ReasoningEffort::Low),
                    ThinkingIntensity::Medium => Some(ditto_llm::ReasoningEffort::Medium),
                    ThinkingIntensity::High => Some(ditto_llm::ReasoningEffort::High),
                    ThinkingIntensity::XHigh => Some(ditto_llm::ReasoningEffort::XHigh),
                }
            } else {
                None
            };

            let provider_options = ditto_llm::ProviderOptions {
                reasoning_effort,
                response_format: response_format.clone(),
                parallel_tool_calls: Some(parallel_tool_calls),
            };
            let req = req_base
                .clone()
                .with_provider_options(provider_options)
                .context("encode provider_options")?;

            attempts += 1;
            match run_llm_stream_once(
                runtime.client.clone(),
                thread_rt.clone(),
                thread_id,
                turn_id,
                req,
                max_openai_request_duration,
            )
            .await
            {
                Ok(resp) => {
                    active_provider_idx = provider_index;
                    break resp;
                }
                Err(failure) => {
                    let should_fallback = llm_error_prefers_provider_fallback(&failure.error)
                        && provider_index + 1 < provider_candidates.len();
                    let is_retryable = llm_error_is_retryable(&failure.error);
                    last_failure = Some(failure);

                    let Some(failure) = last_failure.as_ref() else {
                        anyhow::bail!("llm stream failed");
                    };
                    if failure.emitted_output {
                        let summary = llm_error_summary(&failure.error);
                        anyhow::bail!("llm stream failed after emitting output: {summary}");
                    }

                    if attempts >= llm_max_attempts {
                        if llm_error_prefers_model_fallback(&failure.error)
                            && model_idx + 1 < model_candidates.len()
                        {
                            let cause = llm_error_summary(&failure.error);
                            let prev = model.clone();
                            model_idx += 1;
                            model = model_candidates[model_idx].clone();
                            req_base.model = Some(model.clone());
                            provider_index = 0;
                            attempts = 0;
                            failure_count = 0;
                            last_failure = None;

                            let reason =
                                format!("model_fallback: from={prev} to={model}; cause={cause}");
                            let _ = thread_rt
                                .append_event(ThreadEventKind::ModelRouted {
                                    turn_id,
                                    selected_model: model.clone(),
                                    rule_source,
                                    reason: Some(reason),
                                    rule_id: rule_id.clone(),
                                })
                                .await;
                            continue;
                        }

                        match &failure.error {
                            LlmAttemptError::TimedOut => {
                                return Err(AgentTurnError::OpenAiRequestTimedOut.into())
                            }
                            _ => {
                                let summary = llm_error_summary(&failure.error);
                                anyhow::bail!("llm stream failed after {attempts} attempts: {summary}");
                            }
                        }
                    }

                    if should_fallback {
                        let prev = provider_name.clone();
                        provider_index += 1;
                        let next = provider_candidates
                            .get(provider_index)
                            .cloned()
                            .unwrap_or_else(|| "<unknown>".to_string());
                        let cause = llm_error_summary(&failure.error);
                        let reason = format!("provider_fallback: from={prev} to={next}; cause={cause}");
                        let _ = thread_rt
                            .append_event(ThreadEventKind::ModelRouted {
                                turn_id,
                                selected_model: model.clone(),
                                rule_source,
                                reason: Some(reason),
                                rule_id: rule_id.clone(),
                            })
                            .await;
                        continue;
                    }

                    if !is_retryable {
                        if llm_error_prefers_model_fallback(&failure.error)
                            && model_idx + 1 < model_candidates.len()
                        {
                            let cause = llm_error_summary(&failure.error);
                            let prev = model.clone();
                            model_idx += 1;
                            model = model_candidates[model_idx].clone();
                            req_base.model = Some(model.clone());
                            provider_index = 0;
                            attempts = 0;
                            failure_count = 0;
                            last_failure = None;

                            let reason =
                                format!("model_fallback: from={prev} to={model}; cause={cause}");
                            let _ = thread_rt
                                .append_event(ThreadEventKind::ModelRouted {
                                    turn_id,
                                    selected_model: model.clone(),
                                    rule_source,
                                    reason: Some(reason),
                                    rule_id: rule_id.clone(),
                                })
                                .await;
                            continue;
                        }

                        let summary = llm_error_summary(&failure.error);
                        anyhow::bail!("llm stream failed: {summary}");
                    }

                    failure_count += 1;
                    let delay = retry_backoff_delay(
                        failure_count,
                        llm_retry_base_delay,
                        llm_retry_max_delay,
                    );
                    if !delay.is_zero() {
                        tokio::select! {
                            _ = cancel.cancelled() => return Err(AgentTurnError::Cancelled.into()),
                            _ = tokio::time::sleep(delay) => {}
                        }
                    }
                }
            }
        };

        if !resp.warnings.is_empty() {
            log_llm_warnings(thread_id, turn_id, &resp.warnings);
        }
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

            let mut outputs =
                vec![None::<(String, Value, Vec<pm_openai::ResponseItem>)>; batch_size];
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
                        outputs[idx] = Some((call_id, output, Vec::new()));
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
                        let outcome = run_tool_call(
                            &server,
                            thread_id,
                            Some(turn_id),
                            &tool_name,
                            args_json,
                            cancel,
                        )
                        .await;
                        (idx, call_id, outcome)
                    }
                })
                .buffer_unordered(max_parallel_tool_calls)
                .collect::<Vec<_>>()
                .await;

            for (idx, call_id, outcome) in results {
                let (output_value, hook_messages) = match outcome {
                    Ok(outcome) => (outcome.output, outcome.hook_messages),
                    Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                };
                outputs[idx] = Some((call_id, output_value, hook_messages));
            }

            for (call_id, output_value, hook_messages) in outputs.into_iter().flatten() {
                input_items.push(pm_openai::ResponseItem::FunctionCallOutput {
                    call_id,
                    output: serde_json::to_string(&output_value)?,
                });
                for message in hook_messages {
                    input_items.push(message);
                }
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

                let outcome = run_tool_call(
                    &server,
                    thread_id,
                    Some(turn_id),
                    &tool_name,
                    args_json,
                    cancel.clone(),
                )
                .await;
                let (output_value, hook_messages) = match outcome {
                    Ok(outcome) => (outcome.output, outcome.hook_messages),
                    Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                };

                input_items.push(pm_openai::ResponseItem::FunctionCallOutput {
                    call_id,
                    output: serde_json::to_string(&output_value)?,
                });
                for message in hook_messages {
                    input_items.push(message);
                }
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
                llm: model_client.clone(),
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
    llm: Arc<dyn ditto_llm::LanguageModel>,
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
        llm,
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

    let messages = vec![
        ditto_llm::Message::system(SUMMARY_INSTRUCTIONS),
        ditto_llm::Message::user(prompt),
    ];
    let mut req = ditto_llm::GenerateRequest::from(messages);
    req.model = Some(model.to_string());
    req.tool_choice = Some(ditto_llm::ToolChoice::None);

    let resp = match tokio::time::timeout(max_openai_request_duration, llm.generate(req)).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) => return Ok(false),
        Err(_) => return Ok(false),
    };

    if max_total_tokens > 0
        && let Some(usage) = token_usage_from_ditto_usage(&resp.usage)
        && let Some(tokens) = usage_total_tokens(&usage)
    {
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

    let summary_text = resp.text();
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
            capabilities: None,
        }),
        "openai-auth-command" => Some(ditto_llm::ProviderConfig {
            base_url: Some(DEFAULT_OPENAI_BASE_URL.to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            auth: Some(ditto_llm::ProviderAuth::Command { command: Vec::new() }),
            capabilities: None,
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
    if let Some(capabilities) = overrides.capabilities {
        base.capabilities = Some(capabilities);
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

async fn load_turn_context_refs(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> anyhow::Result<Vec<pm_protocol::ContextRef>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    for event in events.iter().rev() {
        let ThreadEventKind::TurnStarted {
            turn_id: ev_turn_id,
            context_refs,
            ..
        } = &event.kind
        else {
            continue;
        };
        if *ev_turn_id != turn_id {
            continue;
        }
        return Ok(context_refs.clone().unwrap_or_default());
    }

    Ok(Vec::new())
}

async fn load_turn_attachments(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> anyhow::Result<Vec<pm_protocol::TurnAttachment>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    for event in events.iter().rev() {
        let ThreadEventKind::TurnStarted {
            turn_id: ev_turn_id,
            attachments,
            ..
        } = &event.kind
        else {
            continue;
        };
        if *ev_turn_id != turn_id {
            continue;
        }
        return Ok(attachments.clone().unwrap_or_default());
    }

    Ok(Vec::new())
}

fn infer_image_media_type(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn filename_from_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn filename_from_url(url: &str) -> Option<String> {
    let url = url.split('?').next().unwrap_or(url);
    url.rsplit('/')
        .next()
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.to_string())
}

async fn read_attachment_bytes(
    thread_root: &Path,
    path: &str,
    max_bytes: u64,
) -> anyhow::Result<Vec<u8>> {
    let rel = Path::new(path);
    if rel.file_name() == Some(std::ffi::OsStr::new(".env")) {
        anyhow::bail!("refusing to attach secrets file (.env)");
    }

    let resolved = pm_core::resolve_file(thread_root, rel, pm_core::PathAccess::Read, false)
        .await
        .with_context(|| format!("resolve attachment path: {}", rel.display()))?;

    if resolved.file_name() == Some(std::ffi::OsStr::new(".env")) {
        anyhow::bail!("refusing to attach secrets file (.env)");
    }

    let metadata = tokio::fs::metadata(&resolved)
        .await
        .with_context(|| format!("stat {}", resolved.display()))?;
    if metadata.len() > max_bytes {
        anyhow::bail!(
            "attachment too large: path={} bytes={} max_bytes={}",
            rel.display(),
            metadata.len(),
            max_bytes
        );
    }

    tokio::fs::read(&resolved)
        .await
        .with_context(|| format!("read {}", resolved.display()))
}

async fn attachments_to_ditto_parts(
    thread_root: Option<&Path>,
    mode_name: &str,
    allowed_tools: Option<&[String]>,
    attachments: &[pm_protocol::TurnAttachment],
    max_bytes: u64,
) -> anyhow::Result<Vec<ditto_llm::ContentPart>> {
    if max_bytes == 0 {
        anyhow::bail!("attachments are disabled (max_bytes=0)");
    }

    let has_local_paths = attachments.iter().any(|attachment| match attachment {
        pm_protocol::TurnAttachment::Image(image) => {
            matches!(image.source, pm_protocol::AttachmentSource::Path { .. })
        }
        pm_protocol::TurnAttachment::File(file) => {
            matches!(file.source, pm_protocol::AttachmentSource::Path { .. })
        }
    });

    if has_local_paths {
        if let Some(allowed_tools) = allowed_tools
            && !allowed_tools.iter().any(|allowed| allowed == "file/read")
        {
            let allowed_json = serde_json::to_string(allowed_tools)
                .unwrap_or_else(|_| format!("{allowed_tools:?}"));
            anyhow::bail!(
                "attachments with local paths require file/read to be allowed (thread allowed_tools={allowed_json})"
            );
        }

        let Some(thread_root) = thread_root else {
            anyhow::bail!("cannot attach local files without thread cwd/root");
        };

        let catalog = pm_core::modes::ModeCatalog::load(thread_root).await;
        let mode = match catalog.mode(mode_name) {
            Some(mode) => mode,
            None => {
                let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
                anyhow::bail!(
                    "unknown mode: {mode_name} (available: {available}; load_error={})",
                    catalog.load_error.as_deref().unwrap_or("")
                );
            }
        };

        for attachment in attachments {
            let path = match attachment {
                pm_protocol::TurnAttachment::Image(image) => match &image.source {
                    pm_protocol::AttachmentSource::Path { path } => Some(path.as_str()),
                    _ => None,
                },
                pm_protocol::TurnAttachment::File(file) => match &file.source {
                    pm_protocol::AttachmentSource::Path { path } => Some(path.as_str()),
                    _ => None,
                },
            };
            let Some(path) = path else {
                continue;
            };

            let rel = pm_core::modes::relative_path_under_root(thread_root, Path::new(path));
            let base_decision = match rel.as_ref() {
                Ok(rel) if mode.permissions.edit.is_denied(rel) => pm_core::modes::Decision::Deny,
                Ok(_) => mode.permissions.read,
                Err(_) => pm_core::modes::Decision::Deny,
            };
            let effective_decision = match mode.tool_overrides.get("file/read").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision != pm_core::modes::Decision::Allow {
                anyhow::bail!(
                    "mode denies file attachment read: mode={mode_name} decision={effective_decision:?} path={path}"
                );
            }
        }
    }

    let mut out = Vec::new();
    for attachment in attachments {
        match attachment {
            pm_protocol::TurnAttachment::Image(image) => match &image.source {
                pm_protocol::AttachmentSource::Url { url } => {
                    out.push(ditto_llm::ContentPart::Image {
                        source: ditto_llm::ImageSource::Url { url: url.clone() },
                    });
                }
                pm_protocol::AttachmentSource::Path { path } => {
                    let Some(thread_root) = thread_root else {
                        anyhow::bail!("cannot attach local files without thread cwd/root");
                    };
                    let media_type = image
                        .media_type
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| infer_image_media_type(path))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "unsupported image type: path={path} (expected: png/jpg/jpeg/webp/gif)"
                            )
                        })?;
                    let bytes = read_attachment_bytes(thread_root, path, max_bytes).await?;
                    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                    out.push(ditto_llm::ContentPart::Image {
                        source: ditto_llm::ImageSource::Base64 {
                            media_type: media_type.to_string(),
                            data,
                        },
                    });
                }
            },
            pm_protocol::TurnAttachment::File(file) => match &file.source {
                pm_protocol::AttachmentSource::Url { url } => {
                    let filename = file
                        .filename
                        .clone()
                        .or_else(|| filename_from_url(url));
                    out.push(ditto_llm::ContentPart::File {
                        filename,
                        media_type: file.media_type.clone(),
                        source: ditto_llm::FileSource::Url { url: url.clone() },
                    });
                }
                pm_protocol::AttachmentSource::Path { path } => {
                    let Some(thread_root) = thread_root else {
                        anyhow::bail!("cannot attach local files without thread cwd/root");
                    };
                    let bytes = read_attachment_bytes(thread_root, path, max_bytes).await?;
                    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                    let filename = file
                        .filename
                        .clone()
                        .or_else(|| filename_from_path(path));
                    out.push(ditto_llm::ContentPart::File {
                        filename,
                        media_type: file.media_type.clone(),
                        source: ditto_llm::FileSource::Base64 { data },
                    });
                }
            },
        }
    }

    Ok(out)
}

async fn context_refs_to_messages(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    refs: &[pm_protocol::ContextRef],
    cancel: CancellationToken,
) -> anyhow::Result<Vec<pm_openai::ResponseItem>> {
    const DEFAULT_CONTEXT_FILE_MAX_BYTES: u64 = 64 * 1024;
    const MAX_CONTEXT_FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;
    const DEFAULT_CONTEXT_DIFF_MAX_BYTES: u64 = 1024 * 1024;
    const MAX_CONTEXT_DIFF_MAX_BYTES: u64 = 16 * 1024 * 1024;

    let mut out = Vec::new();

    for ctx in refs {
        match ctx {
            pm_protocol::ContextRef::File(file) => {
                let max_bytes = file
                    .max_bytes
                    .unwrap_or(DEFAULT_CONTEXT_FILE_MAX_BYTES)
                    .min(MAX_CONTEXT_FILE_MAX_BYTES);

                let args = serde_json::json!({
                    "path": file.path,
                    "max_bytes": max_bytes,
                });

                let (output, hook_messages) =
                    match run_tool_call(server, thread_id, Some(turn_id), "file_read", args, cancel.clone())
                        .await
                    {
                        Ok(outcome) => (outcome.output, outcome.hook_messages),
                        Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                    };

                out.extend(hook_messages);

                let denied = output
                    .get("denied")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let resolved_path = output
                    .get("resolved_path")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let truncated = output
                    .get("truncated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut text = output.get("text").and_then(Value::as_str).unwrap_or("").to_string();

                if !denied && (file.start_line.is_some() || file.end_line.is_some()) {
                    let start_line = file.start_line.unwrap_or(1);
                    let end_line = file.end_line;
                    let lines = text.lines().collect::<Vec<_>>();
                    let start_idx = usize::try_from(start_line.saturating_sub(1)).unwrap_or(usize::MAX);
                    let end_idx = end_line
                        .and_then(|v| usize::try_from(v).ok())
                        .unwrap_or(lines.len());
                    if start_idx < lines.len() {
                        let end_idx = end_idx.clamp(start_idx, lines.len());
                        text = lines[start_idx..end_idx].join("\n");
                    } else if !truncated {
                        text.clear();
                    }
                }

                let mut msg = String::new();
                msg.push_str("# Context (@file)\n\n");
                msg.push_str(&format!("path: {}\n", file.path.trim()));
                if let Some(start) = file.start_line {
                    msg.push_str(&format!(
                        "range: L{}{}\n",
                        start,
                        file.end_line.map(|e| format!("-L{}", e)).unwrap_or_default()
                    ));
                }
                if !resolved_path.trim().is_empty() {
                    msg.push_str(&format!("resolved_path: {}\n", resolved_path.trim()));
                }
                if truncated {
                    msg.push_str("truncated: true\n");
                }
                if denied {
                    msg.push_str("\nstatus: denied\n");
                    msg.push_str(&format!("details: {}\n", json_one_line(&output, 2000)));
                    out.push(pm_openai::ResponseItem::Message {
                        role: "system".to_string(),
                        content: vec![pm_openai::ContentItem::InputText { text: msg }],
                    });
                    continue;
                }

                msg.push_str("\n```text\n");
                msg.push_str(text.trim_end());
                msg.push_str("\n```\n");

                out.push(pm_openai::ResponseItem::Message {
                    role: "system".to_string(),
                    content: vec![pm_openai::ContentItem::InputText { text: msg }],
                });
            }
            pm_protocol::ContextRef::Diff(diff) => {
                let max_bytes = diff
                    .max_bytes
                    .unwrap_or(DEFAULT_CONTEXT_DIFF_MAX_BYTES)
                    .min(MAX_CONTEXT_DIFF_MAX_BYTES);

                let args = serde_json::json!({
                    "max_bytes": max_bytes,
                });
                let (output, hook_messages) =
                    match run_tool_call(server, thread_id, Some(turn_id), "thread_diff", args, cancel.clone())
                        .await
                    {
                        Ok(outcome) => (outcome.output, outcome.hook_messages),
                        Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                    };

                out.extend(hook_messages);

                let denied = output
                    .get("denied")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let artifact = output.get("artifact").cloned().unwrap_or(Value::Null);
                let artifact_id = artifact.get("artifact_id").and_then(Value::as_str).unwrap_or("");
                let summary = artifact.get("summary").and_then(Value::as_str).unwrap_or("");

                let mut msg = String::new();
                msg.push_str("# Context (@diff)\n\n");
                if !artifact_id.trim().is_empty() {
                    msg.push_str(&format!("artifact_id: {artifact_id}\n"));
                }
                if !summary.trim().is_empty() {
                    msg.push_str(&format!("summary: {summary}\n"));
                }
                if denied {
                    msg.push_str("\nstatus: denied\n");
                    msg.push_str(&format!("details: {}\n", json_one_line(&output, 2000)));
                } else {
                    msg.push_str("\nNote: diff content is stored as an artifact. Use `artifact_read` if you need the full text.\n");
                }

                out.push(pm_openai::ResponseItem::Message {
                    role: "system".to_string(),
                    content: vec![pm_openai::ContentItem::InputText { text: msg }],
                });
            }
        }
    }

    Ok(out)
}

fn insert_context_before_last_user_message(
    input_items: &mut Vec<pm_openai::ResponseItem>,
    ctx_items: Vec<pm_openai::ResponseItem>,
) {
    if ctx_items.is_empty() {
        return;
    }

    let insert_at = input_items
        .iter()
        .rposition(|item| matches!(item, pm_openai::ResponseItem::Message { role, .. } if role == "user"))
        .unwrap_or(input_items.len());

    input_items.splice(insert_at..insert_at, ctx_items);
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
            allowed_tools,
        } => Some(format!(
            "[thread/config] approval_policy={approval_policy:?} sandbox_policy={} sandbox_writable_roots={} sandbox_network_access={} mode={} model={} openai_base_url={} allowed_tools={}",
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
            match allowed_tools {
                None => "<unchanged>".to_string(),
                Some(None) => "null".to_string(),
                Some(Some(tools)) => json_one_line(&serde_json::json!(tools), 2000),
            },
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
mod llm_retry_tests {
    use super::*;

    #[test]
    fn parse_csv_list_trims_and_dedupes() {
        assert_eq!(
            parse_csv_list(" openai-a, openai-b,openai-a,, ,openai-c "),
            vec![
                "openai-a".to_string(),
                "openai-b".to_string(),
                "openai-c".to_string()
            ]
        );
    }

    #[test]
    fn build_provider_candidates_keeps_primary_and_uniques() {
        assert_eq!(
            build_provider_candidates(
                "primary",
                vec![
                    "fallback-1".to_string(),
                    "primary".to_string(),
                    "fallback-1".to_string(),
                    "fallback-2".to_string(),
                ]
            ),
            vec![
                "primary".to_string(),
                "fallback-1".to_string(),
                "fallback-2".to_string()
            ]
        );
    }

    #[test]
    fn build_model_candidates_keeps_primary_and_uniques() {
        assert_eq!(
            build_model_candidates(
                "gpt-4.1-mini",
                vec![
                    "gpt-4.1".to_string(),
                    "gpt-4.1-mini".to_string(),
                    "gpt-4.1".to_string(),
                    "gpt-4.1".to_string(),
                ]
            ),
            vec!["gpt-4.1-mini".to_string(), "gpt-4.1".to_string()]
        );
    }

    #[test]
    fn llm_error_classification_is_conservative() {
        use reqwest::StatusCode;

        let rate_limited = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "rate limit".to_string(),
        });
        assert!(llm_error_is_retryable(&rate_limited));
        assert!(llm_error_prefers_provider_fallback(&rate_limited));

        let server_error = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::BAD_GATEWAY,
            body: "upstream".to_string(),
        });
        assert!(llm_error_is_retryable(&server_error));
        assert!(llm_error_prefers_provider_fallback(&server_error));

        let bad_request = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::BAD_REQUEST,
            body: "invalid".to_string(),
        });
        assert!(!llm_error_is_retryable(&bad_request));
        assert!(!llm_error_prefers_provider_fallback(&bad_request));
        assert!(llm_error_prefers_model_fallback(&bad_request));

        let unauthorized = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::UNAUTHORIZED,
            body: "auth".to_string(),
        });
        assert!(!llm_error_prefers_model_fallback(&unauthorized));

        let timed_out = LlmAttemptError::TimedOut;
        assert!(llm_error_is_retryable(&timed_out));
        assert!(llm_error_prefers_provider_fallback(&timed_out));
        assert!(!llm_error_prefers_model_fallback(&timed_out));
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
            mcp: Arc::new(tokio::sync::Mutex::new(crate::McpManager::default())),
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
                context_refs: None,
                attachments: None,
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
                context_refs: None,
                attachments: None,
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
