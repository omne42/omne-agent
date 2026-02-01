use std::sync::OnceLock;

use omne_agent_protocol::ThreadEventKind;
use regex::Regex;
use serde_json::Value;

const REDACTED: &str = "<REDACTED>";

struct Pattern {
    regex: Regex,
    replacement: &'static str,
}

fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Pattern {
                regex: Regex::new(
                    r"(?s)-----BEGIN [A-Z ]+ PRIVATE KEY-----.*?-----END [A-Z ]+ PRIVATE KEY-----",
                )
                .expect("private key regex"),
                replacement: "<REDACTED PRIVATE KEY>",
            },
            Pattern {
                regex: Regex::new(r"sk-[A-Za-z0-9]{20,}").expect("sk regex"),
                replacement: "sk-<REDACTED>",
            },
            Pattern {
                regex: Regex::new(r"ghp_[A-Za-z0-9]{20,}").expect("ghp regex"),
                replacement: "ghp_<REDACTED>",
            },
            Pattern {
                regex: Regex::new(r"github_pat_[A-Za-z0-9_]{20,}").expect("github_pat regex"),
                replacement: "github_pat_<REDACTED>",
            },
            Pattern {
                regex: Regex::new(r"AIza[0-9A-Za-z_-]{35}").expect("google api key regex"),
                replacement: "AIza<REDACTED>",
            },
            Pattern {
                regex: Regex::new(r"AKIA[0-9A-Z]{16}").expect("aws access key id regex"),
                replacement: "AKIA<REDACTED>",
            },
            Pattern {
                regex: Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._-]{20,}").expect("bearer regex"),
                replacement: "Bearer <REDACTED>",
            },
        ]
    })
}

pub fn redact_text(input: &str) -> String {
    let mut out = input.to_string();
    for pat in patterns() {
        out = pat.regex.replace_all(&out, pat.replacement).to_string();
    }
    out
}

fn is_sensitive_key(key: &str) -> bool {
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
                        omne_agent_protocol::ContextRef::File(file) => {
                            file.path = redact_text(&file.path);
                        }
                        omne_agent_protocol::ContextRef::Diff(_diff) => {}
                    }
                }
            }
            if let Some(attachments) = attachments {
                for attachment in attachments {
                    match attachment {
                        omne_agent_protocol::TurnAttachment::Image(image) => {
                            match &mut image.source {
                                omne_agent_protocol::AttachmentSource::Path { path } => {
                                    *path = redact_text(path);
                                }
                                omne_agent_protocol::AttachmentSource::Url { url } => {
                                    *url = redact_text(url);
                                }
                            }
                            if let Some(media_type) = image.media_type.as_mut() {
                                *media_type = redact_text(media_type);
                            }
                        }
                        omne_agent_protocol::TurnAttachment::File(file) => {
                            match &mut file.source {
                                omne_agent_protocol::AttachmentSource::Path { path } => {
                                    *path = redact_text(path);
                                }
                                omne_agent_protocol::AttachmentSource::Url { url } => {
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
        ThreadEventKind::ToolCompleted { error, result, .. } => {
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
                redact_json_value(token_usage);
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

    #[test]
    fn redacts_known_token_shapes() {
        let input = "hello sk-1234567890abcdefghijklmnop bearer ABCDEFGHIJKLMNOPQRSTUV";
        let redacted = redact_text(input);
        assert!(!redacted.contains("sk-1234567890abcdefghijklmnop"));
        assert!(redacted.contains("sk-<REDACTED>"));
        assert!(redacted.contains("Bearer <REDACTED>"));
    }
}
