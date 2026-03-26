use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub const OMNE_NOTIFY_SOUND_ENV: &str = "OMNE_NOTIFY_SOUND";
pub const OMNE_NOTIFY_WEBHOOK_URL_ENV: &str = "OMNE_NOTIFY_WEBHOOK_URL";
pub const OMNE_NOTIFY_WEBHOOK_FIELD_ENV: &str = "OMNE_NOTIFY_WEBHOOK_FIELD";
pub const OMNE_NOTIFY_FEISHU_WEBHOOK_URL_ENV: &str = "OMNE_NOTIFY_FEISHU_WEBHOOK_URL";
pub const OMNE_NOTIFY_SLACK_WEBHOOK_URL_ENV: &str = "OMNE_NOTIFY_SLACK_WEBHOOK_URL";
pub const OMNE_NOTIFY_TIMEOUT_MS_ENV: &str = "OMNE_NOTIFY_TIMEOUT_MS";
pub const OMNE_NOTIFY_EVENTS_ENV: &str = "OMNE_NOTIFY_EVENTS";
pub const OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT_ENV: &str =
    "OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT";
pub const OMNE_NOTIFY_TOKEN_BUDGET_WARNING_DEBOUNCE_MS_ENV: &str =
    "OMNE_NOTIFY_TOKEN_BUDGET_WARNING_DEBOUNCE_MS";
pub const DEFAULT_TOKEN_BUDGET_WARNING_THRESHOLD_RATIO: f64 = 0.9;
pub const DEFAULT_TOKEN_BUDGET_WARNING_DEBOUNCE: Duration = Duration::from_millis(30_000);

#[derive(Debug, Clone, Copy)]
pub struct NotifyEnvOptions {
    pub default_sound_enabled: bool,
    pub require_sink: bool,
}

pub fn build_notify_hub_from_env(
    options: NotifyEnvOptions,
) -> anyhow::Result<Option<notify_kit::Hub>> {
    build_notify_hub_from_reader(&|key| std::env::var(key).ok(), options)
}

pub fn build_notify_hub_from_reader<F>(
    get: &F,
    options: NotifyEnvOptions,
) -> anyhow::Result<Option<notify_kit::Hub>>
where
    F: Fn(&str) -> Option<String>,
{
    let sound_enabled =
        notify_env_bool(OMNE_NOTIFY_SOUND_ENV, get).unwrap_or(options.default_sound_enabled);
    let timeout = parse_notify_timeout_ms_env(OMNE_NOTIFY_TIMEOUT_MS_ENV, get)
        .map_err(|err| anyhow::anyhow!("invalid {OMNE_NOTIFY_TIMEOUT_MS_ENV}: {err}"))?;

    let mut sinks: Vec<Arc<dyn notify_kit::Sink>> = Vec::new();
    if sound_enabled {
        sinks.push(Arc::new(notify_kit::SoundSink::new(
            notify_kit::SoundConfig { command_argv: None },
        )));
    }

    if let Some(url) = notify_env_nonempty(OMNE_NOTIFY_WEBHOOK_URL_ENV, get) {
        let mut cfg = notify_kit::GenericWebhookConfig::new(url).with_timeout(timeout);
        if let Some(field) = notify_env_nonempty(OMNE_NOTIFY_WEBHOOK_FIELD_ENV, get) {
            cfg = cfg.with_payload_field(field);
        }
        let sink = notify_kit::GenericWebhookSink::new(cfg)
            .map_err(|err| anyhow::anyhow!("build generic webhook sink: {err:#}"))?;
        sinks.push(Arc::new(sink));
    }

    if let Some(url) = notify_env_nonempty(OMNE_NOTIFY_FEISHU_WEBHOOK_URL_ENV, get) {
        let cfg = notify_kit::FeishuWebhookConfig::new(url).with_timeout(timeout);
        let sink = notify_kit::FeishuWebhookSink::new(cfg)
            .map_err(|err| anyhow::anyhow!("build feishu sink: {err:#}"))?;
        sinks.push(Arc::new(sink));
    }

    if let Some(url) = notify_env_nonempty(OMNE_NOTIFY_SLACK_WEBHOOK_URL_ENV, get) {
        let cfg = notify_kit::SlackWebhookConfig::new(url).with_timeout(timeout);
        let sink = notify_kit::SlackWebhookSink::new(cfg)
            .map_err(|err| anyhow::anyhow!("build slack sink: {err:#}"))?;
        sinks.push(Arc::new(sink));
    }

    if sinks.is_empty() {
        if options.require_sink {
            anyhow::bail!(
                "no notification sinks configured (enable {OMNE_NOTIFY_SOUND_ENV}=1 or provide webhook envs)"
            );
        }
        return Ok(None);
    }

    let enabled_kinds = get(OMNE_NOTIFY_EVENTS_ENV).and_then(|raw| {
        let set = raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        if set.is_empty() { None } else { Some(set) }
    });

    Ok(Some(notify_kit::Hub::new_with_limits(
        notify_kit::HubConfig {
            enabled_kinds,
            per_sink_timeout: timeout,
        },
        sinks,
        notify_kit::HubLimits::default(),
    )))
}

pub fn parse_token_budget_warning_threshold_ratio_from_env() -> f64 {
    parse_token_budget_warning_threshold_ratio_from_reader(&|key| std::env::var(key).ok())
}

pub fn parse_token_budget_warning_threshold_ratio_from_reader<F>(get: &F) -> f64
where
    F: Fn(&str) -> Option<String>,
{
    notify_env_nonempty(OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT_ENV, get)
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|value| *value > 0.0 && *value <= 100.0)
        .map(|value| value / 100.0)
        .unwrap_or(DEFAULT_TOKEN_BUDGET_WARNING_THRESHOLD_RATIO)
}

pub fn parse_token_budget_warning_debounce_from_env() -> Duration {
    parse_token_budget_warning_debounce_from_reader(&|key| std::env::var(key).ok())
}

pub fn parse_token_budget_warning_debounce_from_reader<F>(get: &F) -> Duration
where
    F: Fn(&str) -> Option<String>,
{
    let timeout_ms = notify_env_nonempty(OMNE_NOTIFY_TOKEN_BUDGET_WARNING_DEBOUNCE_MS_ENV, get)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TOKEN_BUDGET_WARNING_DEBOUNCE.as_millis() as u64);
    Duration::from_millis(timeout_ms.max(1))
}

fn parse_notify_bool_env_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn notify_env_bool<F>(key: &str, get: &F) -> Option<bool>
where
    F: Fn(&str) -> Option<String>,
{
    get(key).and_then(|value| parse_notify_bool_env_value(&value))
}

fn notify_env_nonempty<F>(key: &str, get: &F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_notify_timeout_ms_env<F>(key: &str, get: &F) -> anyhow::Result<Duration>
where
    F: Fn(&str) -> Option<String>,
{
    let timeout_ms = notify_env_nonempty(key, get)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(5000);
    Ok(Duration::from_millis(timeout_ms.max(1)))
}

#[cfg(test)]
mod notify_env_tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn build_notify_hub_from_reader_supports_sound_only() {
        let env = HashMap::from([(String::from(OMNE_NOTIFY_SOUND_ENV), String::from("1"))]);
        let hub = build_notify_hub_from_reader(
            &|key| env.get(key).cloned(),
            NotifyEnvOptions {
                default_sound_enabled: false,
                require_sink: true,
            },
        )
        .expect("build hub")
        .expect("hub present");

        assert_eq!(
            hub.try_notify(notify_kit::Event::new(
                "attention_state",
                notify_kit::Severity::Info,
                "title",
            )),
            Err(notify_kit::TryNotifyError::NoTokioRuntime)
        );
    }

    #[test]
    fn build_notify_hub_from_reader_returns_none_when_sink_optional() {
        let env = HashMap::<String, String>::new();
        let hub = build_notify_hub_from_reader(
            &|key| env.get(key).cloned(),
            NotifyEnvOptions {
                default_sound_enabled: false,
                require_sink: false,
            },
        )
        .expect("build hub");

        assert!(hub.is_none());
    }

    #[test]
    fn build_notify_hub_from_reader_preserves_enabled_kinds_filter() {
        let env = HashMap::from([(
            String::from(OMNE_NOTIFY_EVENTS_ENV),
            String::from("attention_state"),
        )]);
        let hub = build_notify_hub_from_reader(
            &|key| env.get(key).cloned(),
            NotifyEnvOptions {
                default_sound_enabled: true,
                require_sink: true,
            },
        )
        .expect("build hub")
        .expect("hub present");

        assert_eq!(
            hub.try_notify(notify_kit::Event::new(
                "other_kind",
                notify_kit::Severity::Info,
                "title",
            )),
            Ok(())
        );
    }

    #[test]
    fn parse_token_budget_warning_threshold_ratio_from_reader_uses_valid_percent() {
        let env = HashMap::from([(
            String::from(OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT_ENV),
            String::from("95"),
        )]);

        let ratio =
            parse_token_budget_warning_threshold_ratio_from_reader(&|key| env.get(key).cloned());

        assert!((ratio - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_token_budget_warning_threshold_ratio_from_reader_falls_back_on_invalid_value() {
        let env = HashMap::from([(
            String::from(OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT_ENV),
            String::from("200"),
        )]);

        let ratio =
            parse_token_budget_warning_threshold_ratio_from_reader(&|key| env.get(key).cloned());

        assert!((ratio - DEFAULT_TOKEN_BUDGET_WARNING_THRESHOLD_RATIO).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_token_budget_warning_debounce_from_reader_clamps_minimum_duration() {
        let env = HashMap::from([(
            String::from(OMNE_NOTIFY_TOKEN_BUDGET_WARNING_DEBOUNCE_MS_ENV),
            String::from("0"),
        )]);

        let debounce =
            parse_token_budget_warning_debounce_from_reader(&|key| env.get(key).cloned());

        assert_eq!(debounce, Duration::from_millis(1));
    }
}
