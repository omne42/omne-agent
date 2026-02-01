use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Context;
use omne_agent_protocol::{ThreadEvent, ThreadEventKind, TurnStatus};

pub(crate) fn init_notify_hub_from_env() -> anyhow::Result<Option<notify_kit::Hub>> {
    let mut sinks: Vec<Arc<dyn notify_kit::Sink>> = Vec::new();

    if parse_env_bool("OMNE_AGENT_NOTIFY_SOUND")? {
        let command_argv = parse_env_json_string_array("OMNE_AGENT_NOTIFY_SOUND_CMD_JSON")?;
        sinks.push(Arc::new(notify_kit::SoundSink::new(
            notify_kit::SoundConfig { command_argv },
        )));
    }

    if let Some(webhook_url) = std::env::var("OMNE_AGENT_NOTIFY_FEISHU_WEBHOOK_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        sinks.push(Arc::new(notify_kit::FeishuWebhookSink::new(
            notify_kit::FeishuWebhookConfig::new(webhook_url),
        )?));
    }

    if sinks.is_empty() {
        return Ok(None);
    }

    let enabled_kinds = parse_env_event_kinds("OMNE_AGENT_NOTIFY_EVENTS")?;

    let hub = notify_kit::Hub::new(
        notify_kit::HubConfig {
            enabled_kinds: Some(enabled_kinds),
            ..notify_kit::HubConfig::default()
        },
        sinks,
    );
    Ok(Some(hub))
}

pub(crate) fn map_thread_event_to_notify_event(event: &ThreadEvent) -> Option<notify_kit::Event> {
    fn truncate_preview(text: &str, max_chars: usize) -> String {
        let text = text.trim();
        if text.is_empty() {
            return String::new();
        }
        if text.chars().count() <= max_chars {
            return text.to_string();
        }
        let mut out: String = text.chars().take(max_chars).collect();
        out.push('…');
        out
    }

    let thread_id = event.thread_id.to_string();

    match &event.kind {
        ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } => {
            let severity = match status {
                TurnStatus::Completed => notify_kit::Severity::Success,
                TurnStatus::Interrupted | TurnStatus::Cancelled => notify_kit::Severity::Warning,
                TurnStatus::Failed | TurnStatus::Stuck => notify_kit::Severity::Error,
            };
            let title = match status {
                TurnStatus::Completed => "turn completed",
                TurnStatus::Interrupted => "turn interrupted",
                TurnStatus::Failed => "turn failed",
                TurnStatus::Cancelled => "turn cancelled",
                TurnStatus::Stuck => "turn stuck",
            };

            let mut out = notify_kit::Event::new("turn_completed", severity, title)
                .with_tag("thread_id", thread_id)
                .with_tag("turn_id", turn_id.to_string())
                .with_tag("status", format!("{status:?}"));

            if let Some(reason) = reason.as_deref() {
                let reason = truncate_preview(reason, 400);
                if !reason.is_empty() {
                    out = out.with_body(reason);
                }
            }
            Some(out)
        }
        ThreadEventKind::ApprovalRequested {
            approval_id,
            turn_id,
            action,
            ..
        } => {
            let title = format!("approval requested: {action}");
            let mut out =
                notify_kit::Event::new("approval_requested", notify_kit::Severity::Warning, title)
                    .with_tag("thread_id", thread_id)
                    .with_tag("approval_id", approval_id.to_string())
                    .with_tag("action", action.clone());

            if let Some(turn_id) = turn_id.as_ref() {
                out = out.with_tag("turn_id", turn_id.to_string());
            }
            Some(out)
        }
        ThreadEventKind::AssistantMessage {
            turn_id,
            text,
            model,
            response_id,
            ..
        } => {
            let mut out = notify_kit::Event::new(
                "message_received",
                notify_kit::Severity::Info,
                "assistant message",
            )
            .with_tag("thread_id", thread_id)
            .with_body(truncate_preview(text, 600));

            if let Some(turn_id) = turn_id.as_ref() {
                out = out.with_tag("turn_id", turn_id.to_string());
            }
            if let Some(model) = model.as_deref() {
                out = out.with_tag("model", model.to_string());
            }
            if let Some(response_id) = response_id.as_deref() {
                out = out.with_tag("response_id", response_id.to_string());
            }
            Some(out)
        }
        _ => None,
    }
}

fn parse_env_bool(key: &str) -> anyhow::Result<bool> {
    let Some(value) = std::env::var(key).ok() else {
        return Ok(false);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(false);
    }
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        other => anyhow::bail!("{key}: invalid boolean value: {other}"),
    }
}

fn parse_env_json_string_array(key: &str) -> anyhow::Result<Option<Vec<String>>> {
    let Some(raw) = std::env::var(key).ok() else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let values = serde_json::from_str::<Vec<String>>(raw)
        .with_context(|| format!("{key}: parse json string array"))?;
    let values = values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(None);
    }
    Ok(Some(values))
}

fn parse_env_event_kinds(key: &str) -> anyhow::Result<BTreeSet<String>> {
    let Some(raw) = std::env::var(key).ok() else {
        return Ok(BTreeSet::from([
            "turn_completed".to_string(),
            "approval_requested".to_string(),
        ]));
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(BTreeSet::from([
            "turn_completed".to_string(),
            "approval_requested".to_string(),
        ]));
    }

    let mut out = BTreeSet::<String>::new();
    for part in raw.split(',') {
        let value = part.trim().to_ascii_lowercase();
        if value.is_empty() {
            continue;
        }
        match value.as_str() {
            "turn_completed" | "approval_requested" | "message_received" => {
                out.insert(value);
            }
            other => anyhow::bail!(
                "{key}: unknown event kind: {other} (expected: turn_completed, approval_requested, message_received)"
            ),
        }
    }

    if out.is_empty() {
        anyhow::bail!("{key}: must include at least one event kind");
    }

    Ok(out)
}
