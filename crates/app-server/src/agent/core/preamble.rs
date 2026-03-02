use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use crate::project_config::ProjectOpenAiOverrides;
use crate::model_limits::resolve_model_limits;
use base64::Engine;
use futures_util::stream::{self, StreamExt};
use ditto_llm::ThinkingIntensity;
use omne_protocol::{
    ApprovalDecision, ApprovalId, ArtifactId, ArtifactMetadata, EventSeq, ThreadEventKind, ThreadId,
    TurnId, TurnPriority,
};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

type OpenAiItem = Value;

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

const DEFAULT_MAX_CONCURRENT_LLM_REQUESTS: usize = 4;
const MAX_MAX_CONCURRENT_LLM_REQUESTS: usize = 256;
const DEFAULT_LLM_FOREGROUND_RESERVE: usize = 1;

const DEFAULT_AGENT_MAX_ATTACHMENTS: usize = 4;
const MAX_AGENT_MAX_ATTACHMENTS: usize = 32;
const DEFAULT_AGENT_MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_AGENT_MAX_ATTACHMENT_BYTES: u64 = 200 * 1024 * 1024;
const DEFAULT_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES: u64 = 0;
const MAX_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES: u64 = MAX_AGENT_MAX_ATTACHMENT_BYTES;

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

static LLM_WORKER_POOL: OnceLock<LlmWorkerPool> = OnceLock::new();

#[derive(Debug)]
struct LlmWorkerPool {
    total: Option<Arc<Semaphore>>,
    background: Option<Arc<Semaphore>>,
}

#[derive(Debug)]
struct LlmWorkerPermit {
    _background: Option<OwnedSemaphorePermit>,
    _total: Option<OwnedSemaphorePermit>,
}

impl LlmWorkerPermit {
    fn disabled() -> Self {
        Self {
            _background: None,
            _total: None,
        }
    }
}

impl LlmWorkerPool {
    fn global() -> &'static Self {
        LLM_WORKER_POOL.get_or_init(Self::from_env)
    }

    fn from_env() -> Self {
        let max_concurrent = parse_env_usize(
            "OMNE_MAX_CONCURRENT_LLM_REQUESTS",
            DEFAULT_MAX_CONCURRENT_LLM_REQUESTS,
            0,
            MAX_MAX_CONCURRENT_LLM_REQUESTS,
        );
        if max_concurrent == 0 {
            return Self {
                total: None,
                background: None,
            };
        }

        let reserve = parse_env_usize(
            "OMNE_LLM_FOREGROUND_RESERVE",
            DEFAULT_LLM_FOREGROUND_RESERVE,
            0,
            max_concurrent,
        )
        .min(max_concurrent);

        let background_limit = max_concurrent.saturating_sub(reserve);
        let total = Arc::new(Semaphore::new(max_concurrent));
        let background = if background_limit >= max_concurrent {
            None
        } else {
            Some(Arc::new(Semaphore::new(background_limit)))
        };

        Self {
            total: Some(total),
            background,
        }
    }

    async fn acquire(&self, priority: TurnPriority) -> anyhow::Result<LlmWorkerPermit> {
        let Some(total) = self.total.as_ref() else {
            return Ok(LlmWorkerPermit::disabled());
        };

        match priority {
            TurnPriority::Foreground => {
                let permit = total.clone().acquire_owned().await?;
                Ok(LlmWorkerPermit {
                    _background: None,
                    _total: Some(permit),
                })
            }
            TurnPriority::Background => {
                if let Some(background) = self.background.as_ref() {
                    let bg_permit = background.clone().acquire_owned().await?;
                    let total_permit = total.clone().acquire_owned().await?;
                    Ok(LlmWorkerPermit {
                        _background: Some(bg_permit),
                        _total: Some(total_permit),
                    })
                } else {
                    let permit = total.clone().acquire_owned().await?;
                    Ok(LlmWorkerPermit {
                        _background: None,
                        _total: Some(permit),
                    })
                }
            }
        }
    }
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
    let redacted = omne_core::redact_text(arguments);
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

fn should_auto_compact(
    total_tokens_used: u64,
    auto_compact_token_limit: Option<u64>,
    max_total_tokens: u64,
    threshold_pct: u64,
) -> bool {
    if let Some(limit) = auto_compact_token_limit {
        return limit > 0 && total_tokens_used >= limit;
    }
    if max_total_tokens == 0 || threshold_pct == 0 {
        return false;
    }
    let threshold_pct = threshold_pct.min(MAX_AUTO_SUMMARY_THRESHOLD_PCT);
    let threshold = max_total_tokens.saturating_mul(threshold_pct) / 100;
    threshold > 0 && total_tokens_used >= threshold
}

fn estimate_context_tokens(instructions: &str, input_items: &[OpenAiItem]) -> u64 {
    let mut chars = instructions.chars().count() as u64;
    for item in input_items {
        chars = chars.saturating_add(estimate_openai_item_chars(item));
    }
    (chars.saturating_add(3)) / 4
}

fn estimate_openai_item_chars(item: &OpenAiItem) -> u64 {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "message" => {
            let role = item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut chars = role.chars().count() as u64;
            let content = item.get("content").and_then(Value::as_array);
            if let Some(content) = content {
                for part in content {
                    let part_kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                    if part_kind != "input_text" && part_kind != "output_text" {
                        continue;
                    }
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        chars = chars.saturating_add(text.chars().count() as u64);
                    }
                }
            }
            chars
        }
        "function_call" => {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            (name.chars().count() as u64)
                .saturating_add(arguments.chars().count() as u64)
                .saturating_add(call_id.chars().count() as u64)
        }
        "function_call_output" => {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let output = item.get("output").and_then(Value::as_str).unwrap_or_default();
            (call_id.chars().count() as u64).saturating_add(output.chars().count() as u64)
        }
        _ => 0,
    }
}

fn render_items_for_summary(items: &[OpenAiItem], max_chars: usize) -> String {
    let mut out = String::new();

    for item in items {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "message" => {
                let role = item
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        let part_kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                        if part_kind != "input_text" && part_kind != "output_text" {
                            continue;
                        }
                        let Some(text) = part.get("text").and_then(Value::as_str) else {
                            continue;
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
            }
            "function_call" => {
                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                let arguments_raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let arguments = omne_core::redact_text(arguments_raw);
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
            "function_call_output" => {
                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                let output_raw = item.get("output").and_then(Value::as_str).unwrap_or("");
                let output = omne_core::redact_text(output_raw);
                let output_preview = truncate_chars(&output, 500);
                out.push_str("[tool_output] call_id=");
                out.push_str(call_id.trim());
                if !output_preview.trim().is_empty() {
                    out.push_str(" output=");
                    out.push_str(output_preview.trim());
                }
                out.push('\n');
            }
            _ => {}
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
    output: Vec<OpenAiItem>,
    usage: Option<Value>,
    warnings: Vec<ditto_llm::Warning>,
}

fn token_usage_json_from_ditto_usage(usage: &ditto_llm::Usage) -> Option<Value> {
    if usage.input_tokens.is_none()
        && usage.cache_input_tokens.is_none()
        && usage.cache_creation_input_tokens.is_none()
        && usage.output_tokens.is_none()
        && usage.total_tokens.is_none()
    {
        return None;
    }

    Some(serde_json::json!({
        "input_tokens": usage.input_tokens,
        "cache_input_tokens": usage.cache_input_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "output_tokens": usage.output_tokens,
        "total_tokens": usage.total_tokens,
    }))
}

fn response_items_to_ditto_messages(
    instructions: &str,
    items: &[OpenAiItem],
    attachments: &[ditto_llm::ContentPart],
) -> Vec<ditto_llm::Message> {
    let mut out = Vec::<ditto_llm::Message>::new();
    if !instructions.trim().is_empty() {
        out.push(ditto_llm::Message::system(instructions.to_string()));
    }

    for item in items {
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "message" => {
                let role = item
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let role = match role {
                    "system" => ditto_llm::Role::System,
                    "user" => ditto_llm::Role::User,
                    "assistant" => ditto_llm::Role::Assistant,
                    "tool" => ditto_llm::Role::Tool,
                    _ => ditto_llm::Role::User,
                };

                let mut parts = Vec::<ditto_llm::ContentPart>::new();
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        let part_kind = part.get("type").and_then(Value::as_str).unwrap_or("");
                        if part_kind != "input_text" && part_kind != "output_text" {
                            continue;
                        }
                        let Some(text) = part.get("text").and_then(Value::as_str) else {
                            continue;
                        };
                        if text.is_empty() {
                            continue;
                        }
                        parts.push(ditto_llm::ContentPart::Text {
                            text: text.to_string(),
                        });
                    }
                }
                if !parts.is_empty() {
                    out.push(ditto_llm::Message { role, content: parts });
                }
            }
            "function_call" => {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let Some(name) = item.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let arguments_raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                let raw = arguments_raw.trim();
                let raw_json = if raw.is_empty() { "{}" } else { raw };
                let args = serde_json::from_str::<Value>(raw_json)
                    .unwrap_or_else(|_| Value::String(arguments_raw.to_string()));
                out.push(ditto_llm::Message {
                    role: ditto_llm::Role::Assistant,
                    content: vec![ditto_llm::ContentPart::ToolCall {
                        id: call_id.to_string(),
                        name: name.to_string(),
                        arguments: args,
                    }],
                });
            }
            "function_call_output" => {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let output = item
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push(ditto_llm::Message {
                    role: ditto_llm::Role::Tool,
                    content: vec![ditto_llm::ContentPart::ToolResult {
                        tool_call_id: call_id.to_string(),
                        content: output.to_string(),
                        is_error: None,
                    }],
                });
            }
            _ => {}
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

fn apply_attachments_to_messages(
    mut messages: Vec<ditto_llm::Message>,
    attachments: &[ditto_llm::ContentPart],
) -> Vec<ditto_llm::Message> {
    if attachments.is_empty() {
        return messages;
    }

    if let Some(idx) = messages.iter().rposition(|msg| msg.role == ditto_llm::Role::User) {
        messages[idx].content.extend_from_slice(attachments);
    } else {
        messages.push(ditto_llm::Message {
            role: ditto_llm::Role::User,
            content: attachments.to_vec(),
        });
    }

    messages
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
            strict: Some(false),
        });
    }
    Ok(out)
}

fn tool_specs_total_json_bytes(specs: &[Value]) -> usize {
    specs
        .iter()
        .map(|spec| serde_json::to_vec(spec).map(|bytes| bytes.len()).unwrap_or(0))
        .sum()
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

#[async_trait]
trait FileUploader: Send + Sync {
    async fn upload_file(&self, filename: String, bytes: Vec<u8>) -> anyhow::Result<String>;
}

#[async_trait]
impl FileUploader for ditto_llm::OpenAI {
    async fn upload_file(&self, filename: String, bytes: Vec<u8>) -> anyhow::Result<String> {
        ditto_llm::OpenAI::upload_file(self, filename, bytes)
            .await
            .map_err(anyhow::Error::new)
    }
}

#[async_trait]
impl FileUploader for ditto_llm::OpenAICompatible {
    async fn upload_file(&self, filename: String, bytes: Vec<u8>) -> anyhow::Result<String> {
        ditto_llm::OpenAICompatible::upload_file(self, filename, bytes)
            .await
            .map_err(anyhow::Error::new)
    }
}

#[derive(Clone)]
pub(crate) struct ProviderRuntime {
    config: ditto_llm::ProviderConfig,
    capabilities: ditto_llm::ProviderCapabilities,
    client: Arc<dyn ditto_llm::LanguageModel>,
    openai_responses_client: Option<Arc<ditto_llm::OpenAI>>,
    file_uploader: Option<Arc<dyn FileUploader>>,
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
    let text = omne_core::redact_text(&err.to_string());
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
            "provider does not support tools: provider={provider} (omne requires tool calling; set [openai.providers.{provider}.capabilities.tools]=true)"
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
        http_headers: provider_config.http_headers.clone(),
        http_query_params: provider_config.http_query_params.clone(),
        auth: provider_config.auth.clone(),
        capabilities: Some(provider_capabilities),
    };

    let (client, openai_responses_client, file_uploader) = if provider_capabilities.reasoning {
        let openai = Arc::new(
            ditto_llm::OpenAI::from_config(&provider_for_llm, env)
                .await
                .context("build OpenAI Responses client")?,
        );
        let client: Arc<dyn ditto_llm::LanguageModel> = openai.clone();
        let file_uploader: Arc<dyn FileUploader> = openai.clone();
        (client, Some(openai), Some(file_uploader))
    } else {
        let chat = Arc::new(
            ditto_llm::OpenAICompatible::from_config(&provider_for_llm, env)
                .await
                .context("build OpenAI-compatible Chat Completions client")?,
        );
        let client: Arc<dyn ditto_llm::LanguageModel> = chat.clone();
        let file_uploader: Arc<dyn FileUploader> = chat;
        (client, None, Some(file_uploader))
    };

    Ok(ProviderRuntime {
        config: provider_for_llm,
        capabilities: provider_capabilities,
        client,
        openai_responses_client,
        file_uploader,
    })
}
