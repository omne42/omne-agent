use std::sync::OnceLock;

use pm_protocol::ThreadEventKind;
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
        ThreadEventKind::TurnStarted { input, .. } => {
            *input = redact_text(input);
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
            ..
        } => {
            if let Some(model) = model {
                *model = redact_text(model);
            }
            if let Some(openai_base_url) = openai_base_url {
                *openai_base_url = redact_text(openai_base_url);
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
        ThreadEventKind::AssistantMessage { text, .. } => {
            *text = redact_text(text);
        }
        ThreadEventKind::ProcessStarted { argv, cwd, .. } => {
            redact_argv(argv);
            *cwd = redact_text(cwd);
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
