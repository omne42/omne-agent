#[derive(Parser)]
#[command(name = "omne-app-server")]
#[command(about = "OmneAgent v0.2.0 app-server (JSON-RPC over stdio)", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Override project data root directory (default: `./.omne_data/`).
    #[arg(long)]
    omne_root: Option<PathBuf>,

    /// Listen on a Unix socket instead of stdio (daemon mode).
    #[arg(long, value_name = "PATH")]
    listen: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH")]
    execpolicy_rules: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Generate TypeScript protocol types to an output directory.
    GenerateTs(GenerateOutArgs),
    /// Generate JSON Schema files to an output directory.
    GenerateJsonSchema(GenerateOutArgs),
}

#[cfg(test)]
mod hardening_mode_tests {
    use super::HardeningMode;

    #[test]
    fn hardening_mode_defaults_to_best_effort() {
        assert_eq!(HardeningMode::parse(None).unwrap(), HardeningMode::BestEffort);
    }

    #[test]
    fn hardening_mode_parses_off() {
        assert_eq!(
            HardeningMode::parse(Some("off")).unwrap(),
            HardeningMode::Off
        );
    }

    #[test]
    fn hardening_mode_parses_best_effort() {
        assert_eq!(
            HardeningMode::parse(Some("best_effort")).unwrap(),
            HardeningMode::BestEffort
        );
    }

    #[test]
    fn hardening_mode_rejects_invalid_value() {
        assert!(HardeningMode::parse(Some("wat")).is_err());
        assert!(HardeningMode::parse(Some("best-effort")).is_err());
        assert!(HardeningMode::parse(Some("")).is_err());
        assert!(HardeningMode::parse(Some(" ")).is_err());
    }
}

#[derive(clap::Args)]
struct GenerateOutArgs {
    /// Output directory.
    #[arg(long = "out", value_name = "DIR")]
    out_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;
const JSONRPC_INTERNAL_ERROR: i64 = -32603;
const JSONRPC_PARSE_ERROR: i64 = -32700;
const OMNE_NOT_INITIALIZED: i64 = -32_000;
const OMNE_ALREADY_INITIALIZED: i64 = -32_001;

static APP_NOTIFY_HUB: OnceLock<Option<Arc<notify_kit::Hub>>> = OnceLock::new();
static APP_TOKEN_BUDGET_WARNING_STATE: OnceLock<
    std::sync::Mutex<HashMap<ThreadId, TokenBudgetWarningEmitState>>,
> = OnceLock::new();
static APP_TOKEN_BUDGET_MARKER_STATE: OnceLock<
    std::sync::Mutex<HashMap<ThreadId, TokenBudgetAttentionMarkerEmitState>>,
> = OnceLock::new();
static APP_TOKEN_BUDGET_WARNING_THRESHOLD_RATIO: OnceLock<f64> = OnceLock::new();
static APP_TOKEN_BUDGET_WARNING_DEBOUNCE: OnceLock<Duration> = OnceLock::new();

fn init_notify_hub_from_env() -> anyhow::Result<()> {
    let hub = build_notify_hub_from_env()?;
    if APP_NOTIFY_HUB.set(hub).is_err() {
        tracing::warn!("notify hub already initialized; keeping the first instance");
    }
    Ok(())
}

fn app_notify_hub() -> Option<&'static Arc<notify_kit::Hub>> {
    APP_NOTIFY_HUB.get().and_then(|hub| hub.as_ref())
}

fn configured_total_token_budget_limit_for_notify() -> Option<u64> {
    std::env::var("OMNE_AGENT_MAX_TOTAL_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn parse_token_budget_warning_threshold_ratio_env() -> f64 {
    omne_notify_env::parse_token_budget_warning_threshold_ratio_from_env()
}

fn token_budget_warning_threshold_ratio() -> f64 {
    *APP_TOKEN_BUDGET_WARNING_THRESHOLD_RATIO.get_or_init(parse_token_budget_warning_threshold_ratio_env)
}

fn parse_token_budget_warning_debounce_env() -> Duration {
    omne_notify_env::parse_token_budget_warning_debounce_from_env()
}

fn token_budget_warning_debounce() -> Duration {
    *APP_TOKEN_BUDGET_WARNING_DEBOUNCE.get_or_init(parse_token_budget_warning_debounce_env)
}

fn build_notify_hub_from_env() -> anyhow::Result<Option<Arc<notify_kit::Hub>>> {
    let hub = build_omne_notify_hub_from_env(OmneNotifyHubOptions {
        // Service-side notifications are opt-in to avoid changing default app-server behavior.
        default_sound_enabled: false,
        require_sink: false,
    })?;
    Ok(hub.map(Arc::new))
}

fn attention_state_event(
    thread_id: &ThreadId,
    state: &'static str,
    severity: notify_kit::Severity,
    body: Option<String>,
) -> notify_kit::Event {
    let mut event = notify_kit::Event::new(
        "attention_state",
        severity,
        format!("attention: {thread_id} -> {state}"),
    )
    .with_tag("thread_id", thread_id.to_string())
    .with_tag("state", state.to_string());
    if let Some(body) = body.map(|text| text.trim().to_string()).filter(|text| !text.is_empty()) {
        event = event.with_body(body);
    }
    event
}

fn token_budget_warning_active(
    total_tokens_used: u64,
    token_budget_limit: Option<u64>,
    warning_threshold_ratio: f64,
) -> bool {
    let Some(limit) = token_budget_limit else {
        return false;
    };
    if total_tokens_used > limit {
        return false;
    }
    (total_tokens_used as f64 / limit as f64) >= warning_threshold_ratio
}

fn token_budget_warning_rising_edge(
    state_by_thread: &mut HashMap<ThreadId, TokenBudgetWarningEmitState>,
    thread_id: ThreadId,
    warning_active: bool,
    debounce: Duration,
    now: std::time::Instant,
) -> bool {
    let state = state_by_thread.entry(thread_id).or_default();
    let rising_edge = warning_active && state.last_active == Some(false);
    let debounced = state
        .last_emitted_at
        .is_some_and(|last| now.duration_since(last) < debounce);
    let should_emit = rising_edge && !debounced;
    if should_emit {
        state.last_emitted_at = Some(now);
    }
    state.last_active = Some(warning_active);
    should_emit
}

fn token_budget_exceeded_active(total_tokens_used: u64, token_budget_limit: Option<u64>) -> bool {
    token_budget_limit.is_some_and(|limit| total_tokens_used > limit)
}

fn token_budget_attention_marker_transitions(
    state_by_thread: &mut HashMap<ThreadId, TokenBudgetAttentionMarkerEmitState>,
    thread_id: ThreadId,
    total_tokens_used: u64,
    token_budget_limit: Option<u64>,
    warning_threshold_ratio: f64,
) -> Vec<TokenBudgetAttentionMarkerTransition> {
    let warning_active = token_budget_warning_active(
        total_tokens_used,
        token_budget_limit,
        warning_threshold_ratio,
    );
    let exceeded_active = token_budget_exceeded_active(total_tokens_used, token_budget_limit);
    let state = state_by_thread.entry(thread_id).or_default();

    let mut transitions = Vec::new();
    if state.warning_active.is_some_and(|previous| previous != warning_active) {
        transitions.push(if warning_active {
            TokenBudgetAttentionMarkerTransition::SetWarning
        } else {
            TokenBudgetAttentionMarkerTransition::ClearWarning
        });
    }
    if state
        .exceeded_active
        .is_some_and(|previous| previous != exceeded_active)
    {
        transitions.push(if exceeded_active {
            TokenBudgetAttentionMarkerTransition::SetExceeded
        } else {
            TokenBudgetAttentionMarkerTransition::ClearExceeded
        });
    }

    state.warning_active = Some(warning_active);
    state.exceeded_active = Some(exceeded_active);
    transitions
}

fn token_budget_attention_marker_event_kind(
    transition: TokenBudgetAttentionMarkerTransition,
    turn_id: Option<TurnId>,
) -> omne_protocol::ThreadEventKind {
    match transition {
        TokenBudgetAttentionMarkerTransition::SetWarning => {
            omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id,
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            }
        }
        TokenBudgetAttentionMarkerTransition::ClearWarning => {
            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id,
                reason: Some("token budget warning cleared".to_string()),
            }
        }
        TokenBudgetAttentionMarkerTransition::SetExceeded => {
            omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
                turn_id,
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            }
        }
        TokenBudgetAttentionMarkerTransition::ClearExceeded => {
            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
                turn_id,
                reason: Some("token budget exceeded cleared".to_string()),
            }
        }
    }
}

fn maybe_emit_external_token_budget_warning(thread_id: ThreadId, total_tokens_used: u64) {
    let Some(hub) = app_notify_hub() else {
        return;
    };

    let token_budget_limit = configured_total_token_budget_limit_for_notify();
    let warning_threshold_ratio = token_budget_warning_threshold_ratio();
    let warning_debounce = token_budget_warning_debounce();
    let warning_active = token_budget_warning_active(
        total_tokens_used,
        token_budget_limit,
        warning_threshold_ratio,
    );
    let should_emit = {
        let state_by_thread = APP_TOKEN_BUDGET_WARNING_STATE
            .get_or_init(|| std::sync::Mutex::new(HashMap::new()));
        let mut state_by_thread = state_by_thread
            .lock()
            .expect("token budget warning state mutex should not be poisoned");
        token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            warning_active,
            warning_debounce,
            std::time::Instant::now(),
        )
    };
    if !should_emit {
        return;
    }

    let Some(limit) = token_budget_limit else {
        return;
    };
    let utilization_pct = (total_tokens_used as f64 / limit as f64) * 100.0;
    let warning_threshold_pct = warning_threshold_ratio * 100.0;
    let body = format!(
        "token budget utilization={utilization_pct:.2}% used={total_tokens_used} limit={limit} threshold={warning_threshold_pct:.0}%"
    );
    let event = attention_state_event(
        &thread_id,
        "token_budget_warning",
        notify_kit::Severity::Warning,
        Some(body),
    );
    hub.notify(event);
}

#[derive(Debug, Clone, Copy, Default)]
struct TokenBudgetWarningEmitState {
    last_active: Option<bool>,
    last_emitted_at: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, Default)]
struct TokenBudgetAttentionMarkerEmitState {
    warning_active: Option<bool>,
    exceeded_active: Option<bool>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum TokenBudgetAttentionMarkerTransition {
    SetWarning,
    ClearWarning,
    SetExceeded,
    ClearExceeded,
}

fn attention_marker_notification_state(
    marker: &omne_protocol::AttentionMarkerKind,
) -> Option<(&'static str, notify_kit::Severity)> {
    match marker {
        omne_protocol::AttentionMarkerKind::FanOutLinkageIssue => {
            Some(("fan_out_linkage_issue", notify_kit::Severity::Warning))
        }
        omne_protocol::AttentionMarkerKind::FanOutAutoApplyError => {
            Some(("fan_out_auto_apply_error", notify_kit::Severity::Error))
        }
        _ => None,
    }
}

fn is_token_budget_exceeded_reason(reason: Option<&str>) -> bool {
    reason.is_some_and(|text| text.contains("token budget exceeded:"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalAttentionNotification {
    state: &'static str,
    severity: notify_kit::Severity,
    body: Option<String>,
}

fn external_attention_notification(event: &ThreadEvent) -> Option<ExternalAttentionNotification> {
    match &event.kind {
        omne_protocol::ThreadEventKind::ApprovalRequested { .. } => {
            Some(ExternalAttentionNotification {
                state: "need_approval",
                severity: notify_kit::Severity::Warning,
                body: None,
            })
        }
        omne_protocol::ThreadEventKind::TurnCompleted { status, reason, .. } => {
            let (state, severity) = match status {
                TurnStatus::Failed => ("failed", notify_kit::Severity::Error),
                TurnStatus::Stuck => {
                    if is_token_budget_exceeded_reason(reason.as_deref()) {
                        ("token_budget_exceeded", notify_kit::Severity::Warning)
                    } else {
                        ("stuck", notify_kit::Severity::Warning)
                    }
                }
                _ => return None,
            };
            Some(ExternalAttentionNotification {
                state,
                severity,
                body: reason.clone(),
            })
        }
        omne_protocol::ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => {
            if !matches!(exit_code, Some(code) if *code != 0) {
                return None;
            }
            let mut body = reason.clone().unwrap_or_default();
            if body.trim().is_empty() {
                body = format!("process_id={process_id}, exit_code={exit_code:?}");
            } else {
                body = format!("{body}\nprocess_id={process_id}, exit_code={exit_code:?}");
            }
            Some(ExternalAttentionNotification {
                state: "failed",
                severity: notify_kit::Severity::Error,
                body: Some(body),
            })
        }
        omne_protocol::ThreadEventKind::AttentionMarkerSet { marker, .. } => {
            attention_marker_notification_state(marker).map(|(state, severity)| {
                ExternalAttentionNotification {
                    state,
                    severity,
                    body: None,
                }
            })
        }
        _ => None,
    }
}

fn external_attention_event(event: &ThreadEvent) -> Option<notify_kit::Event> {
    let spec = external_attention_notification(event)?;
    Some(attention_state_event(
        &event.thread_id,
        spec.state,
        spec.severity,
        spec.body,
    ))
}

fn emit_external_notification_with<F>(event: &ThreadEvent, mut emit: F)
where
    F: FnMut(notify_kit::Event),
{
    if let Some(notification_event) = external_attention_event(event) {
        emit(notification_event);
    }
}

fn emit_external_notification_for_event(event: &ThreadEvent) {
    let Some(hub) = app_notify_hub() else {
        return;
    };
    emit_external_notification_with(event, |notification_event| hub.notify(notification_event));
}

#[cfg(test)]
mod attention_marker_notification_tests {
    use super::ExternalAttentionNotification;
    use super::TokenBudgetAttentionMarkerEmitState;
    use super::TokenBudgetAttentionMarkerTransition;
    use super::TokenBudgetWarningEmitState;
    use super::attention_marker_notification_state;
    use super::external_attention_event;
    use super::external_attention_notification;
    use super::emit_external_notification_with;
    use super::token_budget_attention_marker_event_kind;
    use super::token_budget_attention_marker_transitions;
    use super::token_budget_warning_active;
    use super::token_budget_warning_rising_edge;
    use omne_protocol::ThreadEventKind;
    use serde_json::json;
    use time::OffsetDateTime;

    fn test_event(kind: ThreadEventKind) -> omne_protocol::ThreadEvent {
        omne_protocol::ThreadEvent {
            seq: omne_protocol::EventSeq::ZERO,
            timestamp: OffsetDateTime::now_utc(),
            thread_id: omne_protocol::ThreadId::new(),
            kind,
        }
    }

    fn collect_emitted_events(event: &omne_protocol::ThreadEvent) -> Vec<notify_kit::Event> {
        let mut out = Vec::new();
        emit_external_notification_with(event, |notification_event| out.push(notification_event));
        out
    }

    #[test]
    fn attention_marker_notification_state_maps_linkage_issue_to_warning() {
        assert_eq!(
            attention_marker_notification_state(&omne_protocol::AttentionMarkerKind::FanOutLinkageIssue),
            Some(("fan_out_linkage_issue", notify_kit::Severity::Warning))
        );
    }

    #[test]
    fn attention_marker_notification_state_maps_auto_apply_error_to_error() {
        assert_eq!(
            attention_marker_notification_state(
                &omne_protocol::AttentionMarkerKind::FanOutAutoApplyError
            ),
            Some(("fan_out_auto_apply_error", notify_kit::Severity::Error))
        );
    }

    #[test]
    fn attention_marker_notification_state_ignores_non_notification_markers() {
        assert_eq!(
            attention_marker_notification_state(&omne_protocol::AttentionMarkerKind::PlanReady),
            None
        );
        assert_eq!(
            attention_marker_notification_state(&omne_protocol::AttentionMarkerKind::DiffReady),
            None
        );
        assert_eq!(
            attention_marker_notification_state(&omne_protocol::AttentionMarkerKind::TestFailed),
            None
        );
        assert_eq!(
            attention_marker_notification_state(
                &omne_protocol::AttentionMarkerKind::TokenBudgetWarning
            ),
            None
        );
        assert_eq!(
            attention_marker_notification_state(
                &omne_protocol::AttentionMarkerKind::TokenBudgetExceeded
            ),
            None
        );
    }

    #[test]
    fn token_budget_warning_active_requires_limit_and_threshold_without_exceeded() {
        assert!(!token_budget_warning_active(95, None, 0.9));
        assert!(!token_budget_warning_active(181, Some(180), 0.9));
        assert!(!token_budget_warning_active(80, Some(100), 0.9));
        assert!(token_budget_warning_active(90, Some(100), 0.9));
        assert!(token_budget_warning_active(135, Some(150), 0.9));
    }

    #[test]
    fn token_budget_warning_rising_edge_fires_only_on_false_to_true() {
        let thread_id = omne_protocol::ThreadId::new();
        let mut state_by_thread: std::collections::HashMap<
            omne_protocol::ThreadId,
            TokenBudgetWarningEmitState,
        > =
            std::collections::HashMap::new();
        let debounce = std::time::Duration::from_millis(100);
        let now = std::time::Instant::now();

        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            false,
            debounce,
            now
        ));
        assert!(token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            true,
            debounce,
            now + std::time::Duration::from_millis(101)
        ));
        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            true,
            debounce,
            now + std::time::Duration::from_millis(102)
        ));
        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            false,
            debounce,
            now + std::time::Duration::from_millis(103)
        ));
    }

    #[test]
    fn token_budget_warning_rising_edge_respects_debounce_window() {
        let thread_id = omne_protocol::ThreadId::new();
        let mut state_by_thread: std::collections::HashMap<
            omne_protocol::ThreadId,
            TokenBudgetWarningEmitState,
        > =
            std::collections::HashMap::new();
        let debounce = std::time::Duration::from_millis(1000);
        let now = std::time::Instant::now();

        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            true,
            debounce,
            now
        ));
        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            false,
            debounce,
            now + std::time::Duration::from_millis(1)
        ));
        assert!(token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            true,
            debounce,
            now + std::time::Duration::from_millis(10)
        ));
        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            true,
            debounce,
            now + std::time::Duration::from_millis(200)
        ));
        assert!(!token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            false,
            debounce,
            now + std::time::Duration::from_millis(1200)
        ));
        assert!(token_budget_warning_rising_edge(
            &mut state_by_thread,
            thread_id,
            true,
            debounce,
            now + std::time::Duration::from_millis(2200)
        ));
    }

    #[test]
    fn token_budget_attention_marker_transitions_emit_on_state_changes_only() {
        let thread_id = omne_protocol::ThreadId::new();
        let mut state_by_thread: std::collections::HashMap<
            omne_protocol::ThreadId,
            TokenBudgetAttentionMarkerEmitState,
        > = std::collections::HashMap::new();

        assert_eq!(
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                90,
                Some(100),
                0.9
            ),
            Vec::<TokenBudgetAttentionMarkerTransition>::new()
        );
        assert_eq!(
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                101,
                Some(100),
                0.9
            ),
            vec![
                TokenBudgetAttentionMarkerTransition::ClearWarning,
                TokenBudgetAttentionMarkerTransition::SetExceeded
            ]
        );
        assert_eq!(
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                101,
                Some(100),
                0.9
            ),
            Vec::<TokenBudgetAttentionMarkerTransition>::new()
        );
        assert_eq!(
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                108,
                Some(120),
                0.9
            ),
            vec![
                TokenBudgetAttentionMarkerTransition::SetWarning,
                TokenBudgetAttentionMarkerTransition::ClearExceeded
            ]
        );
        assert_eq!(
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                120,
                Some(120),
                0.9
            ),
            Vec::<TokenBudgetAttentionMarkerTransition>::new()
        );
        assert_eq!(
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                80,
                Some(120),
                0.9
            ),
            vec![TokenBudgetAttentionMarkerTransition::ClearWarning]
        );
    }

    #[test]
    fn token_budget_attention_marker_event_kind_maps_set_transitions() {
        let turn_id = Some(omne_protocol::TurnId::new());
        let warning_event = token_budget_attention_marker_event_kind(
            TokenBudgetAttentionMarkerTransition::SetWarning,
            turn_id,
        );
        assert!(matches!(
            warning_event,
            ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id: marker_turn_id,
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            } if marker_turn_id == turn_id
        ));

        let exceeded_event = token_budget_attention_marker_event_kind(
            TokenBudgetAttentionMarkerTransition::SetExceeded,
            turn_id,
        );
        assert!(matches!(
            exceeded_event,
            ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
                turn_id: marker_turn_id,
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            } if marker_turn_id == turn_id
        ));
    }

    #[test]
    fn token_budget_attention_marker_event_kind_maps_clear_transitions() {
        let turn_id = Some(omne_protocol::TurnId::new());
        let warning_event = token_budget_attention_marker_event_kind(
            TokenBudgetAttentionMarkerTransition::ClearWarning,
            turn_id,
        );
        assert!(matches!(
            warning_event,
            ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id: marker_turn_id,
                reason,
            } if marker_turn_id == turn_id
                && reason.as_deref() == Some("token budget warning cleared")
        ));

        let exceeded_event = token_budget_attention_marker_event_kind(
            TokenBudgetAttentionMarkerTransition::ClearExceeded,
            turn_id,
        );
        assert!(matches!(
            exceeded_event,
            ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
                turn_id: marker_turn_id,
                reason,
            } if marker_turn_id == turn_id
                && reason.as_deref() == Some("token budget exceeded cleared")
        ));
    }

    #[test]
    fn external_attention_notification_maps_approval_requested() {
        let event = test_event(ThreadEventKind::ApprovalRequested {
            approval_id: omne_protocol::ApprovalId::new(),
            turn_id: None,
            action: "tool.execute".to_string(),
            params: json!({}),
        });
        assert_eq!(
            external_attention_notification(&event),
            Some(ExternalAttentionNotification {
                state: "need_approval",
                severity: notify_kit::Severity::Warning,
                body: None
            })
        );
    }

    #[test]
    fn external_attention_notification_maps_turn_failed_and_stuck() {
        let failed_event = test_event(ThreadEventKind::TurnCompleted {
            turn_id: omne_protocol::TurnId::new(),
            status: omne_protocol::TurnStatus::Failed,
            reason: Some("runtime error".to_string()),
        });
        assert_eq!(
            external_attention_notification(&failed_event),
            Some(ExternalAttentionNotification {
                state: "failed",
                severity: notify_kit::Severity::Error,
                body: Some("runtime error".to_string())
            })
        );

        let stuck_event = test_event(ThreadEventKind::TurnCompleted {
            turn_id: omne_protocol::TurnId::new(),
            status: omne_protocol::TurnStatus::Stuck,
            reason: None,
        });
        assert_eq!(
            external_attention_notification(&stuck_event),
            Some(ExternalAttentionNotification {
                state: "stuck",
                severity: notify_kit::Severity::Warning,
                body: None
            })
        );
    }

    #[test]
    fn external_attention_notification_maps_stuck_token_budget_exceeded() {
        let event = test_event(ThreadEventKind::TurnCompleted {
            turn_id: omne_protocol::TurnId::new(),
            status: omne_protocol::TurnStatus::Stuck,
            reason: Some("token budget exceeded: used 101 > limit 100".to_string()),
        });
        assert_eq!(
            external_attention_notification(&event),
            Some(ExternalAttentionNotification {
                state: "token_budget_exceeded",
                severity: notify_kit::Severity::Warning,
                body: Some("token budget exceeded: used 101 > limit 100".to_string())
            })
        );
    }

    #[test]
    fn external_attention_notification_keeps_other_budget_reasons_as_stuck() {
        let event = test_event(ThreadEventKind::TurnCompleted {
            turn_id: omne_protocol::TurnId::new(),
            status: omne_protocol::TurnStatus::Stuck,
            reason: Some("budget exceeded: steps".to_string()),
        });
        assert_eq!(
            external_attention_notification(&event),
            Some(ExternalAttentionNotification {
                state: "stuck",
                severity: notify_kit::Severity::Warning,
                body: Some("budget exceeded: steps".to_string())
            })
        );
    }

    #[test]
    fn external_attention_notification_ignores_non_failing_turn_completed() {
        let event = test_event(ThreadEventKind::TurnCompleted {
            turn_id: omne_protocol::TurnId::new(),
            status: omne_protocol::TurnStatus::Completed,
            reason: None,
        });
        assert_eq!(external_attention_notification(&event), None);
    }

    #[test]
    fn external_attention_notification_maps_nonzero_process_exit() {
        let event = test_event(ThreadEventKind::ProcessExited {
            process_id: omne_protocol::ProcessId::new(),
            exit_code: Some(42),
            reason: None,
        });
        let notification =
            external_attention_notification(&event).expect("non-zero process exit must notify");
        assert_eq!(notification.state, "failed");
        assert_eq!(notification.severity, notify_kit::Severity::Error);
        let body = notification.body.expect("process exit body must be set");
        assert!(body.contains("process_id="));
        assert!(body.contains("exit_code=Some(42)"));
    }

    #[test]
    fn external_attention_notification_ignores_zero_process_exit() {
        let event = test_event(ThreadEventKind::ProcessExited {
            process_id: omne_protocol::ProcessId::new(),
            exit_code: Some(0),
            reason: None,
        });
        assert_eq!(external_attention_notification(&event), None);
    }

    #[test]
    fn external_attention_notification_maps_fan_out_markers() {
        let linkage_event = test_event(ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
            turn_id: None,
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        });
        assert_eq!(
            external_attention_notification(&linkage_event),
            Some(ExternalAttentionNotification {
                state: "fan_out_linkage_issue",
                severity: notify_kit::Severity::Warning,
                body: None
            })
        );

        let auto_apply_event = test_event(ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
            turn_id: None,
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        });
        assert_eq!(
            external_attention_notification(&auto_apply_event),
            Some(ExternalAttentionNotification {
                state: "fan_out_auto_apply_error",
                severity: notify_kit::Severity::Error,
                body: None
            })
        );
    }

    #[test]
    fn external_attention_notification_ignores_non_notification_marker() {
        let event = test_event(ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: None,
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        });
        assert_eq!(external_attention_notification(&event), None);
    }

    #[test]
    fn emit_external_notification_with_emits_attention_state_event_with_tags() {
        let event = test_event(ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
            turn_id: None,
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        });
        let emitted = collect_emitted_events(&event);
        assert_eq!(emitted.len(), 1);

        let notification = &emitted[0];
        assert_eq!(notification.kind, "attention_state");
        assert_eq!(notification.severity, notify_kit::Severity::Warning);
        assert_eq!(
            notification.tags().get("state"),
            Some(&"fan_out_linkage_issue".to_string())
        );
        assert_eq!(
            notification.tags().get("thread_id"),
            Some(&event.thread_id.to_string())
        );
    }

    #[test]
    fn emit_external_notification_with_does_not_emit_for_non_notification_marker() {
        let event = test_event(ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: None,
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        });
        let emitted = collect_emitted_events(&event);
        assert!(emitted.is_empty());
    }

    #[test]
    fn external_attention_event_drops_blank_reason_body() {
        let event = test_event(ThreadEventKind::TurnCompleted {
            turn_id: omne_protocol::TurnId::new(),
            status: omne_protocol::TurnStatus::Failed,
            reason: Some("   ".to_string()),
        });
        let notification_event =
            external_attention_event(&event).expect("failed turn should emit attention event");
        assert!(notification_event.body().is_none());
    }
}

struct DiskWarningState {
    last_checked_at: Option<tokio::time::Instant>,
    last_reported_at: Option<tokio::time::Instant>,
}

#[derive(Clone)]
struct Server {
    cwd: PathBuf,
    notify_tx: broadcast::Sender<String>,
    thread_store: ThreadStore,
    threads: Arc<tokio::sync::Mutex<HashMap<ThreadId, Arc<ThreadRuntime>>>>,
    processes: Arc<tokio::sync::Mutex<HashMap<ProcessId, ProcessEntry>>>,
    mcp: Arc<tokio::sync::Mutex<McpManager>>,
    disk_warning: Arc<tokio::sync::Mutex<HashMap<ThreadId, DiskWarningState>>>,
    // Cache provider runtimes across turns to keep HTTP connection pools warm.
    // This materially improves prompt-cache hit rates when the upstream gateway
    // uses per-instance caches and connection-level stickiness.
    provider_runtimes: Arc<tokio::sync::Mutex<HashMap<String, crate::agent::ProviderRuntime>>>,
    exec_policy: omne_execpolicy::Policy,
}

impl Server {
    async fn get_or_load_thread(&self, thread_id: ThreadId) -> anyhow::Result<Arc<ThreadRuntime>> {
        {
            let threads = self.threads.lock().await;
            if let Some(rt) = threads.get(&thread_id) {
                return Ok(rt.clone());
            }
        }

        let handle = self
            .thread_store
            .resume_thread(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

        let rt = Arc::new(ThreadRuntime::new(handle, self.notify_tx.clone()));
        let mut threads = self.threads.lock().await;
        if let Some(existing) = threads.get(&thread_id) {
            return Ok(existing.clone());
        }
        threads.insert(thread_id, rt.clone());
        Ok(rt)
    }

    async fn evict_cached_thread(&self, thread_id: ThreadId) -> bool {
        self.threads.lock().await.remove(&thread_id).is_some()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProcessStatus {
    Running,
    Exited,
    Abandoned,
}

#[derive(Clone, Debug, Serialize)]
struct ProcessInfo {
    process_id: ProcessId,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    argv: Vec<String>,
    cwd: String,
    started_at: String,
    status: ProcessStatus,
    exit_code: Option<i32>,
    stdout_path: String,
    stderr_path: String,
    last_update_at: String,
}

#[derive(Clone)]
struct ProcessEntry {
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
    cmd_tx: mpsc::Sender<ProcessCommand>,
}

#[derive(Debug)]
enum ProcessCommand {
    Interrupt { reason: Option<String> },
    Kill { reason: Option<String> },
}

struct ThreadRuntime {
    handle: tokio::sync::Mutex<omne_core::ThreadHandle>,
    active_turn: tokio::sync::Mutex<Option<ActiveTurn>>,
    notify_tx: broadcast::Sender<String>,
    warned_item_delta_keys: tokio::sync::Mutex<std::collections::HashSet<String>>,
}

fn validate_context_refs(refs: &[omne_protocol::ContextRef]) -> anyhow::Result<()> {
    for ctx in refs {
        match ctx {
            omne_protocol::ContextRef::File(file) => {
                if file.path.trim().is_empty() {
                    anyhow::bail!("context_refs.file.path must be non-empty");
                }
                if let Some(start_line) = file.start_line {
                    if start_line == 0 {
                        anyhow::bail!("context_refs.file.start_line must be >= 1");
                    }
                }
                if let Some(end_line) = file.end_line {
                    if end_line == 0 {
                        anyhow::bail!("context_refs.file.end_line must be >= 1");
                    }
                    let Some(start_line) = file.start_line else {
                        anyhow::bail!("context_refs.file.end_line requires start_line");
                    };
                    if end_line < start_line {
                        anyhow::bail!("context_refs.file.end_line must be >= start_line");
                    }
                }
                if let Some(max_bytes) = file.max_bytes {
                    if max_bytes == 0 {
                        anyhow::bail!("context_refs.file.max_bytes must be >= 1");
                    }
                }
            }
            omne_protocol::ContextRef::Diff(diff) => {
                if let Some(max_bytes) = diff.max_bytes {
                    if max_bytes == 0 {
                        anyhow::bail!("context_refs.diff.max_bytes must be >= 1");
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_turn_attachments(attachments: &[omne_protocol::TurnAttachment]) -> anyhow::Result<()> {
    for attachment in attachments {
        match attachment {
            omne_protocol::TurnAttachment::Image(image) => {
                match &image.source {
                    omne_protocol::AttachmentSource::Path { path } => {
                        if path.trim().is_empty() {
                            anyhow::bail!("attachments.image.source.path must be non-empty");
                        }
                    }
                    omne_protocol::AttachmentSource::Url { url } => {
                        if url.trim().is_empty() {
                            anyhow::bail!("attachments.image.source.url must be non-empty");
                        }
                    }
                }
                if let Some(media_type) = image.media_type.as_deref() {
                    if media_type.trim().is_empty() {
                        anyhow::bail!("attachments.image.media_type must be non-empty when provided");
                    }
                }
            }
            omne_protocol::TurnAttachment::File(file) => {
                match &file.source {
                    omne_protocol::AttachmentSource::Path { path } => {
                        if path.trim().is_empty() {
                            anyhow::bail!("attachments.file.source.path must be non-empty");
                        }
                    }
                    omne_protocol::AttachmentSource::Url { url } => {
                        if url.trim().is_empty() {
                            anyhow::bail!("attachments.file.source.url must be non-empty");
                        }
                    }
                }
                if file.media_type.trim().is_empty() {
                    anyhow::bail!("attachments.file.media_type must be non-empty");
                }
                if let Some(filename) = file.filename.as_deref() {
                    if filename.trim().is_empty() {
                        anyhow::bail!("attachments.file.filename must be non-empty when provided");
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_turn_directives(directives: &[omne_protocol::TurnDirective]) -> anyhow::Result<()> {
    let mut seen_plan = false;
    for directive in directives {
        match directive {
            omne_protocol::TurnDirective::Plan => {
                if seen_plan {
                    anyhow::bail!("duplicate turn directive: plan");
                }
                seen_plan = true;
            }
        }
    }
    Ok(())
}

impl ThreadRuntime {
    fn new(handle: omne_core::ThreadHandle, notify_tx: broadcast::Sender<String>) -> Self {
        Self {
            handle: tokio::sync::Mutex::new(handle),
            active_turn: tokio::sync::Mutex::new(None),
            notify_tx,
            warned_item_delta_keys: tokio::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    fn emit_notification<T>(&self, method: &'static str, params: &T)
    where
        T: Serialize,
    {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        if let Ok(line) = serde_json::to_string(&payload) {
            if self.notify_tx.send(line).is_err() {
                tracing::debug!("dropped thread notification because there are no subscribers");
            }
        }
    }

    async fn emit_item_delta_warning_once(
        &self,
        key: String,
        thread_id: ThreadId,
        turn_id: TurnId,
        message: String,
    ) {
        {
            let mut warned = self.warned_item_delta_keys.lock().await;
            if !warned.insert(key) {
                return;
            }
        }

        self.emit_notification(
            "item/delta",
            &serde_json::json!({
                "thread_id": thread_id,
                "turn_id": turn_id,
                "response_id": "",
                "kind": "warning",
                "delta": message,
            }),
        );
    }

    async fn emit_event_notifications(&self, event: &ThreadEvent) {
        self.emit_notification("thread/event", event);

        match &event.kind {
            omne_protocol::ThreadEventKind::TurnStarted { .. } => {
                self.emit_notification("turn/started", event);
            }
            omne_protocol::ThreadEventKind::TurnCompleted { .. } => {
                self.emit_notification("turn/completed", event);
            }
            omne_protocol::ThreadEventKind::ToolStarted { .. }
            | omne_protocol::ThreadEventKind::ProcessStarted { .. }
            | omne_protocol::ThreadEventKind::ApprovalRequested { .. } => {
                self.emit_notification("item/started", event);
            }
            omne_protocol::ThreadEventKind::ToolCompleted { .. }
            | omne_protocol::ThreadEventKind::ProcessExited { .. }
            | omne_protocol::ThreadEventKind::ApprovalDecided { .. }
            | omne_protocol::ThreadEventKind::AssistantMessage { .. } => {
                self.emit_notification("item/completed", event);
            }
            omne_protocol::ThreadEventKind::AgentStep { .. } => {
                self.emit_notification("item/completed", event);
                self.emit_notification("agent/step", event);
            }
            _ => {}
        }

        emit_external_notification_for_event(event);
        let total_tokens_used = {
            let handle = self.handle.lock().await;
            handle.state().total_tokens_used
        };
        maybe_emit_external_token_budget_warning(event.thread_id, total_tokens_used);
    }

    async fn sync_token_budget_attention_markers(
        &self,
        thread_id: ThreadId,
        total_tokens_used: u64,
    ) {
        let token_budget_limit = configured_total_token_budget_limit_for_notify();
        let warning_threshold_ratio = token_budget_warning_threshold_ratio();
        let transitions = {
            let state_by_thread = APP_TOKEN_BUDGET_MARKER_STATE
                .get_or_init(|| std::sync::Mutex::new(HashMap::new()));
            let mut state_by_thread = state_by_thread
                .lock()
                .expect("token budget marker state mutex should not be poisoned");
            token_budget_attention_marker_transitions(
                &mut state_by_thread,
                thread_id,
                total_tokens_used,
                token_budget_limit,
                warning_threshold_ratio,
            )
        };
        for transition in transitions {
            let event_kind = token_budget_attention_marker_event_kind(transition, None);
            if let Err(err) = self.append_event_and_emit(event_kind).await {
                tracing::warn!(
                    thread_id = %thread_id,
                    total_tokens_used,
                    transition = ?transition,
                    error = %err,
                    "failed to append token budget attention marker event"
                );
            }
        }
    }

    async fn start_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        input: String,
        context_refs: Option<Vec<omne_protocol::ContextRef>>,
        attachments: Option<Vec<omne_protocol::TurnAttachment>>,
        directives: Option<Vec<omne_protocol::TurnDirective>>,
        priority: omne_protocol::TurnPriority,
    ) -> anyhow::Result<TurnId> {
        let mut handle = self.handle.lock().await;
        let state = handle.state();
        if state.archived {
            anyhow::bail!("thread is archived");
        }
        if state.paused {
            anyhow::bail!("thread is paused");
        }
        if state.active_turn_id.is_some() {
            anyhow::bail!("turn already active");
        }

        let context_refs = match context_refs {
            Some(refs) if refs.is_empty() => None,
            other => other,
        };
        if let Some(refs) = context_refs.as_deref() {
            validate_context_refs(refs)?;
        }

        let attachments = match attachments {
            Some(attachments) if attachments.is_empty() => None,
            other => other,
        };
        if let Some(attachments) = attachments.as_deref() {
            validate_turn_attachments(attachments)?;
        }

        let directives = match directives {
            Some(directives) if directives.is_empty() => None,
            other => other,
        };
        if let Some(directives) = directives.as_deref() {
            validate_turn_directives(directives)?;
        }

        let turn_id = TurnId::new();
        let input_for_event = input.clone();
        let clear_reason = Some("new turn started".to_string());
        let clear_plan_event = handle
            .append(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::PlanReady,
                turn_id: Some(turn_id),
                reason: clear_reason.clone(),
            })
            .await?;
        let clear_diff_event = handle
            .append(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::DiffReady,
                turn_id: Some(turn_id),
                reason: clear_reason.clone(),
            })
            .await?;
        let clear_fan_out_linkage_issue_event = handle
            .append(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                turn_id: Some(turn_id),
                reason: clear_reason,
            })
            .await?;
        let clear_fan_out_auto_apply_error_event = handle
            .append(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: Some(turn_id),
                reason: Some("new turn started".to_string()),
            })
            .await?;
        let event = handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: input_for_event,
                context_refs,
                attachments,
                directives,
                priority,
            })
            .await?;
        drop(handle);
        self.emit_event_notifications(&clear_plan_event).await;
        self.emit_event_notifications(&clear_diff_event).await;
        self.emit_event_notifications(&clear_fan_out_linkage_issue_event)
            .await;
        self.emit_event_notifications(&clear_fan_out_auto_apply_error_event)
            .await;
        self.emit_event_notifications(&event).await;

        let cancel = CancellationToken::new();
        {
            let mut active = self.active_turn.lock().await;
            *active = Some(ActiveTurn {
                turn_id,
                cancel: cancel.clone(),
                interrupt_reason: None,
            });
        }

        tokio::task::spawn_local(async move {
            self.run_turn(server, turn_id, cancel, input, priority).await;
        });

        Ok(turn_id)
    }

    async fn append_event(
        &self,
        kind: omne_protocol::ThreadEventKind,
    ) -> anyhow::Result<ThreadEvent> {
        let (event, total_tokens_used) = self.append_event_and_emit(kind).await?;
        self.sync_token_budget_attention_markers(event.thread_id, total_tokens_used)
            .await;
        Ok(event)
    }

    async fn append_event_and_emit(
        &self,
        kind: omne_protocol::ThreadEventKind,
    ) -> anyhow::Result<(ThreadEvent, u64)> {
        let mut handle = self.handle.lock().await;
        let event = handle.append(kind).await?;
        let total_tokens_used = handle.state().total_tokens_used;
        drop(handle);
        self.emit_event_notifications(&event).await;
        Ok((event, total_tokens_used))
    }

    async fn interrupt_turn(&self, turn_id: TurnId, reason: Option<String>) -> anyhow::Result<()> {
        let cancel = {
            let mut active = self.active_turn.lock().await;
            let Some(active) = active.as_mut() else {
                anyhow::bail!("no active turn");
            };
            if active.turn_id != turn_id {
                anyhow::bail!("turn is not active");
            }
            if active.interrupt_reason.is_none() {
                active.interrupt_reason = reason.clone();
            }
            active.cancel.clone()
        };

        let mut handle = self.handle.lock().await;
        if handle.state().active_turn_interrupt_requested {
            cancel.cancel();
            return Ok(());
        }
        let event = handle
            .append(omne_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason })
            .await?;
        drop(handle);
        self.emit_event_notifications(&event).await;

        cancel.cancel();
        Ok(())
    }

    async fn run_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        turn_id: TurnId,
        cancel: CancellationToken,
        input: String,
        priority: omne_protocol::TurnPriority,
    ) {
        let agent_fut =
            agent::run_agent_turn(server.clone(), self.clone(), turn_id, input, cancel.clone(), priority);

        let (status, reason) = tokio::select! {
            _ = cancel.cancelled() => {
                let reason = {
                    let active = self.active_turn.lock().await;
                    active.as_ref().and_then(|a| a.interrupt_reason.clone())
                };
                (TurnStatus::Interrupted, reason.or_else(|| Some("turn interrupted".to_string())))
            },
            result = agent_fut => {
                match result {
                    Ok(_completion) => (TurnStatus::Completed, None),
                    Err(err) => {
                        let status = classify_agent_turn_error(&err);
                        (status, Some(err.to_string()))
                    }
                }
            },
        };
        let reason_for_report = reason.clone();

        let mut handle = self.handle.lock().await;
        let thread_id = handle.thread_id();
        let turn_completed = handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status,
                reason,
            })
            .await;
        drop(handle);
        let turn_completed_event = match turn_completed {
            Ok(event) => Some(event),
            Err(err) => {
                let evicted = server.evict_cached_thread(thread_id).await;
                tracing::warn!(
                    thread_id = %thread_id,
                    turn_id = %turn_id,
                    error = %err,
                    evicted,
                    "failed to append TurnCompleted; evicted cached thread runtime"
                );
                None
            }
        };
        if let Some(event) = turn_completed_event {
            self.emit_event_notifications(&event).await;
        }

        if matches!(status, TurnStatus::Stuck) {
            if let Err(err) = maybe_write_stuck_report(
                server.as_ref(),
                thread_id,
                turn_id,
                reason_for_report.as_deref(),
            )
            .await
            {
                tracing::debug!(
                    thread_id = %thread_id,
                    turn_id = %turn_id,
                    error = %err,
                    "stuck report write failed"
                );
            }
        }

        let _ = run_stop_hooks(
            server.as_ref(),
            thread_id,
            turn_id,
            status,
            reason_for_report.as_deref(),
        )
        .await;

        let mut active = self.active_turn.lock().await;
        if active.as_ref().is_some_and(|a| a.turn_id == turn_id) {
            *active = None;
        }
    }
}

fn classify_agent_turn_error(err: &anyhow::Error) -> TurnStatus {
    for cause in err.chain() {
        if let Some(agent_err) = cause.downcast_ref::<agent::AgentTurnError>() {
            return match agent_err {
                agent::AgentTurnError::Cancelled => TurnStatus::Interrupted,
                agent::AgentTurnError::BudgetExceeded { .. }
                | agent::AgentTurnError::TokenBudgetExceeded { .. }
                | agent::AgentTurnError::OpenAiRequestTimedOut
                | agent::AgentTurnError::LoopDetected { .. } => TurnStatus::Stuck,
            };
        }
    }
    TurnStatus::Failed
}

struct ActiveTurn {
    turn_id: TurnId,
    cancel: CancellationToken,
    interrupt_reason: Option<String>,
}

#[cfg(test)]
mod server_cache_tests {
    use super::*;

    #[tokio::test]
    async fn evict_cached_thread_removes_loaded_runtime() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data")).await?;

        let server = crate::build_test_server_shared(repo_dir.join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        let thread_rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        server.threads.lock().await.insert(thread_id, thread_rt);

        assert!(server.evict_cached_thread(thread_id).await);
        assert!(server.threads.lock().await.get(&thread_id).is_none());
        assert!(!server.evict_cached_thread(thread_id).await);
        Ok(())
    }
}
