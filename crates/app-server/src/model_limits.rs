use ditto_core::config::ModelConfig;

const DEFAULT_CONTEXT_WINDOW_272K: u64 = 272_000;
const DEFAULT_CONTEXT_WINDOW_200K: u64 = 200_000;
const DEFAULT_CONTEXT_WINDOW_128K: u64 = 128_000;
const DEFAULT_CONTEXT_WINDOW_96K: u64 = 96_000;
const DEFAULT_CONTEXT_WINDOW_16K: u64 = 16_385;
const DEFAULT_CONTEXT_WINDOW_4_1: u64 = 1_047_576;

pub(crate) const DEFAULT_AUTO_COMPACT_THRESHOLD_PCT: u64 = 80;
pub(crate) const MAX_AUTO_COMPACT_THRESHOLD_PCT: u64 = 99;

#[derive(Clone, Copy, Debug)]
pub struct ModelLimits {
    pub context_window: Option<u64>,
    pub auto_compact_token_limit: Option<u64>,
}

pub fn resolve_model_limits(model: &str, config: Option<&ModelConfig>) -> ModelLimits {
    let context_window = config
        .and_then(|cfg| cfg.context_window)
        .or_else(|| default_context_window_for_model(model));

    let auto_compact_token_limit = config.and_then(|cfg| cfg.auto_compact_token_limit);

    ModelLimits {
        context_window,
        auto_compact_token_limit,
    }
}

pub fn effective_auto_compact_token_limit(
    context_window: Option<u64>,
    auto_compact_token_limit: Option<u64>,
    threshold_pct: u64,
) -> Option<u64> {
    if let Some(limit) = auto_compact_token_limit {
        return (limit > 0).then_some(limit);
    }

    let threshold_pct = threshold_pct.min(MAX_AUTO_COMPACT_THRESHOLD_PCT);
    if threshold_pct == 0 {
        return None;
    }

    context_window
        .map(|window| window.saturating_mul(threshold_pct) / 100)
        .filter(|limit| *limit > 0)
}

fn default_context_window_for_model(model: &str) -> Option<u64> {
    let slug = model.trim();
    if slug.is_empty() {
        return None;
    }
    let slug = slug.to_ascii_lowercase();

    if slug.starts_with("o3") || slug.starts_with("o4-mini") {
        return Some(DEFAULT_CONTEXT_WINDOW_200K);
    }
    if slug.starts_with("codex-mini-latest") {
        return Some(DEFAULT_CONTEXT_WINDOW_200K);
    }
    if slug.starts_with("gpt-4.1") {
        return Some(DEFAULT_CONTEXT_WINDOW_4_1);
    }
    if slug.starts_with("gpt-oss") || slug.starts_with("openai/gpt-oss") {
        return Some(DEFAULT_CONTEXT_WINDOW_96K);
    }
    if slug.starts_with("gpt-4o") {
        return Some(DEFAULT_CONTEXT_WINDOW_128K);
    }
    if slug.starts_with("gpt-3.5") {
        return Some(DEFAULT_CONTEXT_WINDOW_16K);
    }
    if slug.starts_with("gpt-5.2-codex")
        || slug.starts_with("bengalfox")
        || slug.starts_with("gpt-5.1-codex-max")
    {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }
    if (slug.starts_with("gpt-5-codex")
        || slug.starts_with("gpt-5.1-codex")
        || slug.starts_with("codex-"))
        && !slug.contains("-mini")
    {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }
    if slug.starts_with("gpt-5-codex")
        || slug.starts_with("gpt-5.1-codex")
        || slug.starts_with("codex-")
    {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }
    if (slug.starts_with("gpt-5.2") || slug.starts_with("boomslang")) && !slug.contains("codex") {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }
    if slug.starts_with("gpt-5.1") && !slug.contains("codex") {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }
    if slug.starts_with("gpt-5") && !slug.contains("codex") {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }
    if slug.starts_with("exp-") {
        return Some(DEFAULT_CONTEXT_WINDOW_272K);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_auto_compact_limit_prefers_explicit_limit() {
        assert_eq!(
            effective_auto_compact_token_limit(Some(200_000), Some(150_000), 80),
            Some(150_000)
        );
    }

    #[test]
    fn effective_auto_compact_limit_falls_back_to_context_window_threshold() {
        assert_eq!(
            effective_auto_compact_token_limit(Some(200_000), None, 80),
            Some(160_000)
        );
    }

    #[test]
    fn resolve_model_limits_keeps_context_window_default_without_auto_compact_default() {
        let limits = resolve_model_limits("gpt-4o", None);
        assert_eq!(limits.context_window, Some(128_000));
        assert_eq!(limits.auto_compact_token_limit, None);
    }
}
