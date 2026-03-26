use std::sync::OnceLock;

use omne_protocol::ThreadEventKind;
use regex::Regex;
use serde_json::Value;
use structured_text_protocol::{CatalogArgValueData, StructuredTextData};

const REDACTED: &str = "<REDACTED>";

struct Pattern {
    regex: Regex,
    replacement: &'static str,
}

fn build_pattern(regex: &'static str, replacement: &'static str) -> Option<Pattern> {
    match Regex::new(regex) {
        Ok(compiled) => Some(Pattern {
            regex: compiled,
            replacement,
        }),
        Err(err) => {
            tracing::error!(
                pattern = regex,
                error = %err,
                "invalid built-in redaction regex; skipping pattern"
            );
            None
        }
    }
}

fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let specs = [
            (
                r"(?s)-----BEGIN [A-Z ]+ PRIVATE KEY-----.*?-----END [A-Z ]+ PRIVATE KEY-----",
                "<REDACTED PRIVATE KEY>",
            ),
            (r"sk-[A-Za-z0-9]{20,}", "sk-<REDACTED>"),
            (r"ghp_[A-Za-z0-9]{20,}", "ghp_<REDACTED>"),
            (r"github_pat_[A-Za-z0-9_]{20,}", "github_pat_<REDACTED>"),
            (r"AIza[0-9A-Za-z_-]{35}", "AIza<REDACTED>"),
            (r"AKIA[0-9A-Z]{16}", "AKIA<REDACTED>"),
            (r"(?i)\bbearer\s+[A-Za-z0-9._-]{20,}", "Bearer <REDACTED>"),
        ];
        specs
            .into_iter()
            .filter_map(|(regex, replacement)| build_pattern(regex, replacement))
            .collect()
    })
}

pub fn redact_text(input: &str) -> String {
    let mut out = input.to_string();
    for pat in patterns() {
        out = pat.regex.replace_all(&out, pat.replacement).to_string();
    }
    out
}

pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.trim().trim_start_matches('-').to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "access_key",
        "secret_access_key",
        "client_secret",
        "private_key",
        "password",
        "passwd",
        "token",
        "refresh_token",
        "authorization",
        "cookie",
        "session",
    ]
    .iter()
    .any(|needle| k.contains(needle))
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
        Value::String(s) => {
            *s = redact_text(s);
        }
        Value::Array(items) => {
            for item in items {
                redact_json_value(item);
            }
        }
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if is_sensitive_key(k) {
                    *v = Value::String(REDACTED.to_string());
                } else {
                    redact_json_value(v);
                }
            }
        }
    }
}

fn is_usage_counter_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "total_tokens"
            | "input_tokens"
            | "output_tokens"
            | "cache_input_tokens"
            | "cache_creation_input_tokens"
    )
}

fn redact_token_usage_value(value: &mut Value) {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
        Value::String(s) => {
            *s = redact_text(s);
        }
        Value::Array(items) => {
            for item in items {
                redact_token_usage_value(item);
            }
        }
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if is_usage_counter_key(k) {
                    match v {
                        Value::Null | Value::Number(_) => {}
                        _ => redact_json_value(v),
                    }
                } else if is_sensitive_key(k) {
                    *v = Value::String(REDACTED.to_string());
                } else {
                    redact_token_usage_value(v);
                }
            }
        }
    }
}

fn redact_structured_message(value: &mut StructuredTextData) {
    match value {
        StructuredTextData::Catalog { args, .. } => {
            for arg in args {
                if is_sensitive_key(&arg.name) {
                    arg.value = CatalogArgValueData::Text(REDACTED.to_string());
                } else {
                    redact_catalog_arg_value(&mut arg.value);
                }
            }
        }
        StructuredTextData::Freeform { text } => {
            *text = redact_text(text);
        }
    }
}

fn redact_catalog_arg_value(value: &mut CatalogArgValueData) {
    match value {
        CatalogArgValueData::Text(text)
        | CatalogArgValueData::Signed(text)
        | CatalogArgValueData::Unsigned(text) => {
            *text = redact_text(text);
        }
        CatalogArgValueData::Bool(_) => {}
        CatalogArgValueData::NestedText(message) => redact_structured_message(message),
    }
}

fn redact_argv(argv: &mut [String]) {
    const NEXT_TOKEN_FLAGS: &[&str] = &[
        "--api-key",
        "--api_key",
        "--access-token",
        "--access_token",
        "--token",
        "--password",
        "--passwd",
        "--secret",
        "--client-secret",
        "--client_secret",
        "--authorization",
        "--cookie",
    ];

    for token in argv.iter_mut() {
        if let Some((k, _v)) = token.split_once('=') {
            if is_sensitive_key(k) {
                *token = format!("{k}={REDACTED}");
                continue;
            }
        }
        *token = redact_text(token);
    }

    let mut i = 0usize;
    while i + 1 < argv.len() {
        if NEXT_TOKEN_FLAGS.iter().any(|f| argv[i] == *f) {
            argv[i + 1] = REDACTED.to_string();
            i += 2;
            continue;
        }
        i += 1;
    }
}

pub(crate) fn redact_thread_event_kind(kind: &mut ThreadEventKind) {
    match kind {
        ThreadEventKind::ThreadCreated { cwd } => {
            *cwd = redact_text(cwd);
        }
        ThreadEventKind::ThreadSystemPromptSnapshot {
            prompt_text,
            source,
            ..
        } => {
            *prompt_text = redact_text(prompt_text);
            if let Some(source) = source {
                *source = redact_text(source);
            }
        }
        ThreadEventKind::ThreadArchived { reason }
        | ThreadEventKind::ThreadUnarchived { reason }
        | ThreadEventKind::ThreadPaused { reason }
        | ThreadEventKind::ThreadUnpaused { reason } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::TurnStarted {
            input,
            context_refs,
            attachments,
            ..
        } => {
            *input = redact_text(input);
            if let Some(context_refs) = context_refs {
                for ctx in context_refs {
                    match ctx {
                        omne_protocol::ContextRef::File(file) => {
                            file.path = redact_text(&file.path);
                        }
                        omne_protocol::ContextRef::Diff(_diff) => {}
                    }
                }
            }
            if let Some(attachments) = attachments {
                for attachment in attachments {
                    match attachment {
                        omne_protocol::TurnAttachment::Image(image) => {
                            match &mut image.source {
                                omne_protocol::AttachmentSource::Path { path } => {
                                    *path = redact_text(path);
                                }
                                omne_protocol::AttachmentSource::Url { url } => {
                                    *url = redact_text(url);
                                }
                            }
                            if let Some(media_type) = image.media_type.as_mut() {
                                *media_type = redact_text(media_type);
                            }
                        }
                        omne_protocol::TurnAttachment::File(file) => {
                            match &mut file.source {
                                omne_protocol::AttachmentSource::Path { path } => {
                                    *path = redact_text(path);
                                }
                                omne_protocol::AttachmentSource::Url { url } => {
                                    *url = redact_text(url);
                                }
                            }
                            file.media_type = redact_text(&file.media_type);
                            if let Some(filename) = file.filename.as_mut() {
                                *filename = redact_text(filename);
                            }
                        }
                    }
                }
            }
        }
        ThreadEventKind::ModelRouted {
            selected_model,
            reason,
            rule_id,
            ..
        } => {
            *selected_model = redact_text(selected_model);
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
            if let Some(rule_id) = rule_id {
                *rule_id = redact_text(rule_id);
            }
        }
        ThreadEventKind::TurnInterruptRequested { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::TurnCompleted { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::ThreadConfigUpdated {
            model,
            openai_base_url,
            sandbox_writable_roots,
            allowed_tools,
            execpolicy_rules,
            ..
        } => {
            if let Some(model) = model {
                *model = redact_text(model);
            }
            if let Some(openai_base_url) = openai_base_url {
                *openai_base_url = redact_text(openai_base_url);
            }
            if let Some(roots) = sandbox_writable_roots {
                for root in roots {
                    *root = redact_text(root);
                }
            }
            if let Some(Some(tools)) = allowed_tools {
                for tool in tools {
                    *tool = redact_text(tool);
                }
            }
            if let Some(rules) = execpolicy_rules {
                for rule in rules {
                    *rule = redact_text(rule);
                }
            }
        }
        ThreadEventKind::ApprovalRequested { params, .. } => {
            redact_json_value(params);
        }
        ThreadEventKind::ApprovalDecided { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::ToolStarted { params, .. } => {
            if let Some(params) = params {
                redact_json_value(params);
            }
        }
        ThreadEventKind::ToolCompleted {
            structured_error,
            error,
            result,
            ..
        } => {
            if let Some(structured_error) = structured_error {
                redact_structured_message(structured_error);
            }
            if let Some(error) = error {
                *error = redact_text(error);
            }
            if let Some(result) = result {
                redact_json_value(result);
            }
        }
        ThreadEventKind::AgentStep {
            model,
            response_id,
            text,
            tool_calls,
            tool_results,
            token_usage,
            ..
        } => {
            *model = redact_text(model);
            *response_id = redact_text(response_id);
            if let Some(text) = text.as_mut() {
                *text = redact_text(text);
            }
            for call in tool_calls {
                call.name = redact_text(&call.name);
                call.call_id = redact_text(&call.call_id);
                call.arguments = redact_text(&call.arguments);
            }
            for result in tool_results {
                result.call_id = redact_text(&result.call_id);
                result.output = redact_text(&result.output);
            }
            if let Some(token_usage) = token_usage.as_mut() {
                redact_token_usage_value(token_usage);
            }
        }
        ThreadEventKind::AssistantMessage { text, .. } => {
            *text = redact_text(text);
        }
        ThreadEventKind::ProcessStarted { argv, cwd, .. } => {
            redact_argv(argv);
            *cwd = redact_text(cwd);
        }
        ThreadEventKind::ProcessInterruptRequested { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::ProcessKillRequested { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::ProcessExited { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::AttentionMarkerSet {
            artifact_type,
            command,
            ..
        } => {
            if let Some(artifact_type) = artifact_type {
                *artifact_type = redact_text(artifact_type);
            }
            if let Some(command) = command {
                *command = redact_text(command);
            }
        }
        ThreadEventKind::AttentionMarkerCleared { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
        ThreadEventKind::CheckpointCreated {
            label,
            snapshot_ref,
            ..
        } => {
            if let Some(label) = label {
                *label = redact_text(label);
            }
            *snapshot_ref = redact_text(snapshot_ref);
        }
        ThreadEventKind::CheckpointRestored { reason, .. } => {
            if let Some(reason) = reason {
                *reason = redact_text(reason);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omne_protocol::TurnId;

    #[test]
    fn redacts_known_token_shapes() {
        let input = "hello sk-1234567890abcdefghijklmnop bearer ABCDEFGHIJKLMNOPQRSTUV";
        let redacted = redact_text(input);
        assert!(!redacted.contains("sk-1234567890abcdefghijklmnop"));
        assert!(redacted.contains("sk-<REDACTED>"));
        assert!(redacted.contains("Bearer <REDACTED>"));
    }

    #[test]
    fn sensitive_key_detection_covers_common_aliases() {
        assert!(is_sensitive_key("authorization"));
        assert!(is_sensitive_key("--api_key"));
        assert!(is_sensitive_key("x-refresh-token"));
        assert!(is_sensitive_key("session_cookie"));
        assert!(!is_sensitive_key("summary"));
        assert!(!is_sensitive_key("artifact_type"));
    }

    #[test]
    fn agent_step_usage_keeps_token_counters() {
        let mut kind = ThreadEventKind::AgentStep {
            turn_id: TurnId::new(),
            step: 1,
            model: "gpt-5".to_string(),
            response_id: "resp_1".to_string(),
            text: Some("ok".to_string()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            token_usage: Some(serde_json::json!({
                "total_tokens": 120,
                "input_tokens": 90,
                "output_tokens": 30,
                "cache_input_tokens": 55,
                "cache_creation_input_tokens": 8,
                "api_token": "shhh",
                "nested": {
                    "token": "secret"
                }
            })),
            warnings_count: None,
        };

        redact_thread_event_kind(&mut kind);

        let ThreadEventKind::AgentStep { token_usage, .. } = kind else {
            unreachable!("expected AgentStep");
        };
        let usage = token_usage.expect("token_usage should exist");
        assert_eq!(usage.get("total_tokens").and_then(Value::as_u64), Some(120));
        assert_eq!(
            usage.get("cache_input_tokens").and_then(Value::as_u64),
            Some(55)
        );
        assert_eq!(
            usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64),
            Some(8)
        );
        assert_eq!(
            usage.get("api_token").and_then(Value::as_str),
            Some("<REDACTED>")
        );
        assert_eq!(
            usage
                .get("nested")
                .and_then(Value::as_object)
                .and_then(|nested| nested.get("token"))
                .and_then(Value::as_str),
            Some("<REDACTED>")
        );
    }
}
