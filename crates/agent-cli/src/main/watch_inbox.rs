struct BellNotifier {
    hub: notify_kit::Hub,
}

impl BellNotifier {
    fn from_env() -> anyhow::Result<Self> {
        const OMNE_NOTIFY_SOUND_ENV: &str = "OMNE_NOTIFY_SOUND";
        const OMNE_NOTIFY_WEBHOOK_URL_ENV: &str = "OMNE_NOTIFY_WEBHOOK_URL";
        const OMNE_NOTIFY_WEBHOOK_FIELD_ENV: &str = "OMNE_NOTIFY_WEBHOOK_FIELD";
        const OMNE_NOTIFY_FEISHU_WEBHOOK_URL_ENV: &str = "OMNE_NOTIFY_FEISHU_WEBHOOK_URL";
        const OMNE_NOTIFY_SLACK_WEBHOOK_URL_ENV: &str = "OMNE_NOTIFY_SLACK_WEBHOOK_URL";
        const OMNE_NOTIFY_TIMEOUT_MS_ENV: &str = "OMNE_NOTIFY_TIMEOUT_MS";
        const OMNE_NOTIFY_EVENTS_ENV: &str = "OMNE_NOTIFY_EVENTS";

        let timeout = parse_notify_timeout_ms_env(OMNE_NOTIFY_TIMEOUT_MS_ENV)
            .with_context(|| format!("invalid {OMNE_NOTIFY_TIMEOUT_MS_ENV}"))?;
        let mut sinks: Vec<Arc<dyn notify_kit::Sink>> = Vec::new();

        let sound_enabled = env_bool(OMNE_NOTIFY_SOUND_ENV).unwrap_or(true);
        if sound_enabled {
            sinks.push(Arc::new(notify_kit::SoundSink::new(
                notify_kit::SoundConfig { command_argv: None },
            )));
        }

        if let Some(url) = env_nonempty(OMNE_NOTIFY_WEBHOOK_URL_ENV) {
            let mut cfg = notify_kit::GenericWebhookConfig::new(url).with_timeout(timeout);
            if let Some(field) = env_nonempty(OMNE_NOTIFY_WEBHOOK_FIELD_ENV) {
                cfg = cfg.with_payload_field(field);
            }
            sinks.push(Arc::new(
                notify_kit::GenericWebhookSink::new(cfg)
                    .context("build generic webhook sink")?,
            ));
        }

        if let Some(url) = env_nonempty(OMNE_NOTIFY_FEISHU_WEBHOOK_URL_ENV) {
            let cfg = notify_kit::FeishuWebhookConfig::new(url).with_timeout(timeout);
            sinks.push(Arc::new(
                notify_kit::FeishuWebhookSink::new(cfg).context("build feishu sink")?,
            ));
        }

        if let Some(url) = env_nonempty(OMNE_NOTIFY_SLACK_WEBHOOK_URL_ENV) {
            let cfg = notify_kit::SlackWebhookConfig::new(url).with_timeout(timeout);
            sinks.push(Arc::new(
                notify_kit::SlackWebhookSink::new(cfg).context("build slack sink")?,
            ));
        }

        if sinks.is_empty() {
            anyhow::bail!(
                "no notification sinks configured (enable {OMNE_NOTIFY_SOUND_ENV}=1 or provide webhook envs)"
            );
        }

        let enabled_kinds = std::env::var(OMNE_NOTIFY_EVENTS_ENV).ok().and_then(|raw| {
            let set = raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect::<std::collections::BTreeSet<_>>();
            if set.is_empty() { None } else { Some(set) }
        });

        Ok(Self {
            hub: notify_kit::Hub::new(
                notify_kit::HubConfig {
                    enabled_kinds,
                    per_sink_timeout: timeout,
                },
                sinks,
            ),
        })
    }

    fn notify_attention_state(&self, title: String, state: &str) {
        let severity = attention_state_severity(state);
        self.hub.notify(
            notify_kit::Event::new("attention_state", severity, title).with_tag("state", state),
        );
    }

    fn notify_stale_process(&self, title: String) {
        self.hub.notify(
            notify_kit::Event::new("stale_process", notify_kit::Severity::Warning, title)
                .with_tag("state", "stale_process"),
        );
    }

}

fn attention_state_severity(state: &str) -> notify_kit::Severity {
    match state {
        "failed" => notify_kit::Severity::Error,
        "fan_out_auto_apply_error" => notify_kit::Severity::Error,
        "need_approval"
        | "stuck"
        | "fan_out_linkage_issue"
        | "fan_in_dependency_blocked"
        | "fan_in_result_diagnostics"
        | "token_budget_exceeded"
        | "token_budget_warning" => notify_kit::Severity::Warning,
        _ => notify_kit::Severity::Info,
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_notify_timeout_ms_env(key: &str) -> anyhow::Result<Duration> {
    let timeout = std::env::var(key)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .map(|raw| raw.parse::<u64>())
        .transpose()?
        .unwrap_or(5000);
    Ok(Duration::from_millis(timeout.max(1)))
}

fn parse_token_budget_warning_threshold_ratio_env() -> f64 {
    const ENV_KEY: &str = "OMNE_NOTIFY_TOKEN_BUDGET_UTILIZATION_THRESHOLD_PCT";
    std::env::var(ENV_KEY)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| raw.parse::<f64>().ok())
        .filter(|value| *value > 0.0 && *value <= 100.0)
        .map(|value| value / 100.0)
        .unwrap_or(0.9)
}

fn token_budget_warning_present(
    token_budget_limit: Option<u64>,
    token_budget_utilization: Option<f64>,
    token_budget_exceeded: Option<bool>,
    warning_threshold_ratio: f64,
) -> bool {
    if token_budget_limit.is_none() || token_budget_exceeded.unwrap_or(false) {
        return false;
    }
    token_budget_utilization.is_some_and(|value| value >= warning_threshold_ratio)
}

async fn run_watch(app: &mut App, args: WatchArgs) -> anyhow::Result<()> {
    let debug_summary_cache =
        args.debug_summary_cache || env_bool("OMNE_WATCH_SUMMARY_CACHE_DEBUG").unwrap_or(false);
    let mut watch_iteration: u64 = 0;
    let mut since_seq = args.since_seq;
    let mut last_state: Option<&'static str> = None;
    let mut last_bell_at: Option<Instant> = None;
    let mut last_stale_present: Option<bool> = None;
    let mut last_stale_bell_at: Option<Instant> = None;
    let mut last_linkage_present: Option<bool> = None;
    let mut last_linkage_bell_at: Option<Instant> = None;
    let mut last_auto_apply_error_present: Option<bool> = None;
    let mut last_auto_apply_error_bell_at: Option<Instant> = None;
    let mut last_fan_in_dependency_blocked_present: Option<bool> = None;
    let mut last_fan_in_dependency_blocked_bell_at: Option<Instant> = None;
    let mut last_fan_in_result_diagnostics_present: Option<bool> = None;
    let mut last_fan_in_result_diagnostics_bell_at: Option<Instant> = None;
    let warning_threshold_ratio = parse_token_budget_warning_threshold_ratio_env();
    let warning_threshold_pct = warning_threshold_ratio * 100.0;
    let warning_threshold_label = format!("{warning_threshold_pct:.0}%");
    let mut last_token_budget_exceeded_present: Option<bool> = None;
    let mut last_token_budget_exceeded_bell_at: Option<Instant> = None;
    let mut last_token_budget_warning_present: Option<bool> = None;
    let mut last_token_budget_warning_bell_at: Option<Instant> = None;
    let mut last_detail_summary: Option<WatchDetailSummarySnapshot> = None;
    let mut suppress_initial_bell = true;
    let bell_notifier = BellNotifier::from_env()?;

    loop {
        watch_iteration = watch_iteration.saturating_add(1);
        let resp = app
            .thread_subscribe(
                args.thread_id,
                since_seq,
                args.max_events,
                Some(args.wait_ms),
            )
            .await?;
        since_seq = resp.last_seq;

        let mut state_update: Option<&'static str> = None;
        for event in &resp.events {
            if let Some(state) = attention_state_update(event) {
                state_update = Some(state);
            }
            if args.json {
                println!("{}", serde_json::to_string(event)?);
            } else {
                render_event(event);
            }
        }

        let refresh_detail_summary = args.details
            && !resp.events.is_empty()
            && (last_detail_summary.is_none() || should_refresh_watch_detail_summary(&resp.events));

        let attention = if args.bell {
            Some(app.thread_attention(args.thread_id).await?)
        } else if refresh_detail_summary {
            app.thread_attention(args.thread_id).await.ok()
        } else {
            None
        };

        if refresh_detail_summary {
            let previous_snapshot = last_detail_summary.as_ref();
            let refresh_auto_apply =
                previous_snapshot.is_none() || should_refresh_watch_auto_apply_summary(&resp.events);
            let refresh_fan_in_blocker = previous_snapshot.is_none()
                || should_refresh_watch_fan_in_dependency_blocker_summary(&resp.events);
            let refresh_fan_in_diagnostics = previous_snapshot.is_none()
                || should_refresh_watch_fan_in_result_diagnostics_summary(&resp.events);
            let refresh_subagent_pending = previous_snapshot.is_none()
                || should_refresh_watch_subagent_pending_summary(&resp.events);

            let (auto_apply_summary, auto_apply_source) = if refresh_auto_apply {
                latest_fan_out_auto_apply_summary_with_source(app, args.thread_id, attention.as_ref())
                    .await
            } else {
                (
                    previous_snapshot.and_then(|value| value.auto_apply.clone()),
                    SummarySource::Previous,
                )
            };
            let (fan_in_blocker, fan_in_source) = if refresh_fan_in_blocker {
                latest_fan_in_dependency_blocked_summary_with_source(
                    app,
                    args.thread_id,
                    attention.as_ref(),
                )
                .await
            } else {
                (
                    previous_snapshot.and_then(|value| value.fan_in_blocker.clone()),
                    SummarySource::Previous,
                )
            };
            let (fan_in_diagnostics, fan_in_diagnostics_source) = if refresh_fan_in_diagnostics {
                latest_fan_in_result_diagnostics_summary_with_source(
                    app,
                    args.thread_id,
                    attention.as_ref(),
                )
                .await
            } else {
                (
                    previous_snapshot.and_then(|value| value.fan_in_diagnostics.clone()),
                    SummarySource::Previous,
                )
            };
            let (subagent_pending, subagent_source) = if refresh_subagent_pending {
                if let Some(summary) = attention
                    .as_ref()
                    .and_then(|value| summarize_subagent_pending_approvals(&value.pending_approvals))
                {
                    (Some(summary), SummarySource::Attention)
                } else if let Some(summary) =
                    previous_snapshot.and_then(|value| value.subagent_pending.clone())
                {
                    (Some(summary), SummarySource::Previous)
                } else {
                    (None, SummarySource::None)
                }
            } else {
                if let Some(summary) = previous_snapshot.and_then(|value| value.subagent_pending.clone()) {
                    (Some(summary), SummarySource::Previous)
                } else {
                    (None, SummarySource::None)
                }
            };
            let snapshot = WatchDetailSummarySnapshot {
                auto_apply: auto_apply_summary.clone(),
                fan_in_blocker: fan_in_blocker.clone(),
                fan_in_diagnostics,
                subagent_pending,
            };
            if should_emit_watch_detail_summary(last_detail_summary.as_ref(), &snapshot) {
                if args.json {
                    for row in watch_detail_summary_json_rows_with_delta(
                        args.thread_id,
                        last_detail_summary.as_ref(),
                        &snapshot,
                    ) {
                        println!("{}", serde_json::to_string(&row)?);
                    }
                } else {
                    for line in watch_detail_summary_lines_with_delta(
                        last_detail_summary.as_ref(),
                        &snapshot,
                    ) {
                        println!("{line}");
                    }
                }
            }
            if debug_summary_cache {
                if args.json {
                    let row = build_watch_summary_refresh_debug_json_row(
                        watch_iteration,
                        resp.events.len(),
                        refresh_auto_apply,
                        refresh_fan_in_blocker,
                        refresh_fan_in_diagnostics,
                        refresh_subagent_pending,
                        auto_apply_source,
                        fan_in_source,
                        fan_in_diagnostics_source,
                        subagent_source,
                    );
                    println!("{}", serde_json::to_string(&row)?);
                } else {
                    eprintln!(
                        "{}",
                        format_watch_summary_refresh_debug(
                            watch_iteration,
                            resp.events.len(),
                            refresh_auto_apply,
                            refresh_fan_in_blocker,
                            refresh_fan_in_diagnostics,
                            refresh_subagent_pending,
                            auto_apply_source,
                            fan_in_source,
                            fan_in_diagnostics_source,
                            subagent_source,
                        )
                    );
                }
            }
            last_detail_summary = Some(snapshot);
        }

        if args.bell && !suppress_initial_bell {
            if let Some(state) = state_update {
                maybe_bell(
                    &bell_notifier,
                    state,
                    args.debounce_ms,
                    &mut last_state,
                    &mut last_bell_at,
                )?;
            }
        }

        if args.bell {
            let att = attention
                .as_ref()
                .expect("attention must be loaded when bell notifications are enabled");
            let stale_present = !att.stale_processes.is_empty();
            let linkage_issue_present = att.has_fan_out_linkage_issue;
            let auto_apply_error_present = att.has_fan_out_auto_apply_error;
            let fan_in_dependency_blocked_present = att.has_fan_in_dependency_blocked;
            let fan_in_result_diagnostics_present = att.has_fan_in_result_diagnostics;
            let token_budget_exceeded_present = att.token_budget_exceeded.unwrap_or(false);
            let token_budget_warning_active = att.token_budget_warning_active.unwrap_or_else(|| {
                token_budget_warning_present(
                    att.token_budget_limit,
                    att.token_budget_utilization,
                    att.token_budget_exceeded,
                    warning_threshold_ratio,
                )
            });
            if suppress_initial_bell {
                last_stale_present = Some(stale_present);
                last_linkage_present = Some(linkage_issue_present);
                last_auto_apply_error_present = Some(auto_apply_error_present);
                last_fan_in_dependency_blocked_present = Some(fan_in_dependency_blocked_present);
                last_fan_in_result_diagnostics_present = Some(fan_in_result_diagnostics_present);
                last_token_budget_exceeded_present = Some(token_budget_exceeded_present);
                last_token_budget_warning_present = Some(token_budget_warning_active);
            } else {
                maybe_bell_stale(
                    &bell_notifier,
                    stale_present,
                    args.debounce_ms,
                    &mut last_stale_present,
                    &mut last_stale_bell_at,
                )?;
                maybe_bell_linkage_issue_per_thread(
                    &bell_notifier,
                    &args.thread_id,
                    linkage_issue_present,
                    args.debounce_ms,
                    &mut last_linkage_present,
                    &mut last_linkage_bell_at,
                )?;
                maybe_bell_auto_apply_error_per_thread(
                    &bell_notifier,
                    &args.thread_id,
                    auto_apply_error_present,
                    args.debounce_ms,
                    &mut last_auto_apply_error_present,
                    &mut last_auto_apply_error_bell_at,
                )?;
                maybe_bell_fan_in_dependency_blocked_per_thread(
                    &bell_notifier,
                    &args.thread_id,
                    fan_in_dependency_blocked_present,
                    args.debounce_ms,
                    &mut last_fan_in_dependency_blocked_present,
                    &mut last_fan_in_dependency_blocked_bell_at,
                )?;
                maybe_bell_fan_in_result_diagnostics_per_thread(
                    &bell_notifier,
                    &args.thread_id,
                    fan_in_result_diagnostics_present,
                    args.debounce_ms,
                    &mut last_fan_in_result_diagnostics_present,
                    &mut last_fan_in_result_diagnostics_bell_at,
                )?;
                maybe_bell_token_budget_exceeded_per_thread(
                    &bell_notifier,
                    &args.thread_id,
                    token_budget_exceeded_present,
                    args.debounce_ms,
                    &mut last_token_budget_exceeded_present,
                    &mut last_token_budget_exceeded_bell_at,
                )?;
                maybe_bell_token_budget_warning_per_thread(
                    &bell_notifier,
                    &args.thread_id,
                    token_budget_warning_active,
                    warning_threshold_label.as_str(),
                    args.debounce_ms,
                    &mut last_token_budget_warning_present,
                    &mut last_token_budget_warning_bell_at,
                )?;
            }
        }
        suppress_initial_bell = false;

        if resp.timed_out {
            continue;
        }
        if resp.has_more {
            continue;
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ThreadMeta {
    thread_id: ThreadId,
    cwd: String,
    archived: bool,
    #[serde(default)]
    archived_at: Option<String>,
    #[serde(default)]
    archived_reason: Option<String>,
    approval_policy: ApprovalPolicy,
    sandbox_policy: SandboxPolicy,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
    last_seq: u64,
    #[serde(default)]
    active_turn_id: Option<TurnId>,
    #[serde(default)]
    active_turn_interrupt_requested: bool,
    #[serde(default)]
    last_turn_id: Option<TurnId>,
    #[serde(default)]
    last_turn_status: Option<TurnStatus>,
    #[serde(default)]
    last_turn_reason: Option<String>,
    #[serde(default)]
    token_budget_limit: Option<u64>,
    #[serde(default)]
    token_budget_remaining: Option<u64>,
    #[serde(default)]
    token_budget_utilization: Option<f64>,
    #[serde(default)]
    token_budget_exceeded: Option<bool>,
    #[serde(default)]
    token_budget_warning_active: Option<bool>,
    attention_state: String,
    #[serde(default)]
    has_fan_out_linkage_issue: bool,
    #[serde(default)]
    has_fan_out_auto_apply_error: bool,
    #[serde(default, skip_serializing)]
    fan_out_auto_apply: Option<FanOutAutoApplyInboxSummary>,
    #[serde(default)]
    has_fan_in_dependency_blocked: bool,
    #[serde(default, skip_serializing)]
    fan_in_dependency_blocker: Option<FanInDependencyBlockedInboxSummary>,
    #[serde(default)]
    has_fan_in_result_diagnostics: bool,
    #[serde(default, skip_serializing)]
    fan_in_result_diagnostics: Option<FanInResultDiagnosticsInboxSummary>,
    #[serde(default)]
    pending_subagent_proxy_approvals: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct ThreadListMetaResponse {
    threads: Vec<ThreadMeta>,
}

type ThreadAttention = omne_app_server_protocol::ThreadAttentionResponse;
type FanOutAutoApplyInboxSummary = omne_app_server_protocol::ThreadFanOutAutoApplySummary;
type FanInDependencyBlockedInboxSummary =
    omne_app_server_protocol::ThreadFanInDependencyBlockedSummary;
type FanInResultDiagnosticsInboxSummary =
    omne_app_server_protocol::ThreadFanInResultDiagnosticsSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubagentPendingApprovalsSummary {
    total: usize,
    states: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct WatchDetailSummarySnapshot {
    auto_apply: Option<FanOutAutoApplyInboxSummary>,
    fan_in_blocker: Option<FanInDependencyBlockedInboxSummary>,
    fan_in_diagnostics: Option<FanInResultDiagnosticsInboxSummary>,
    subagent_pending: Option<SubagentPendingApprovalsSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummarySource {
    Previous,
    Attention,
    Artifact,
    None,
}

#[derive(Debug, Default, Clone, Serialize)]
struct InboxSummaryCacheStats {
    fan_out_meta: usize,
    fan_out_cache_some: usize,
    fan_out_cache_none: usize,
    fan_out_attention: usize,
    fan_out_fetch_some: usize,
    fan_out_fetch_none: usize,
    fan_in_meta: usize,
    fan_in_cache_some: usize,
    fan_in_cache_none: usize,
    fan_in_attention: usize,
    fan_in_fetch_some: usize,
    fan_in_fetch_none: usize,
    fan_in_skip_unblocked: usize,
    fan_in_diag_meta: usize,
    fan_in_diag_cache_some: usize,
    fan_in_diag_cache_none: usize,
    fan_in_diag_attention: usize,
    fan_in_diag_fetch_some: usize,
    fan_in_diag_fetch_none: usize,
    fan_in_diag_skip_absent: usize,
    subagent_cache_some: usize,
    subagent_cache_none: usize,
    subagent_meta: usize,
    subagent_attention_some: usize,
    subagent_attention_none: usize,
    subagent_fetch_some: usize,
    subagent_fetch_none: usize,
    subagent_skip_no_pending: usize,
}

async fn run_inbox(app: &mut App, args: InboxArgs) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(args.poll_ms.max(200));
    let debug_summary_cache =
        args.debug_summary_cache || env_bool("OMNE_INBOX_SUMMARY_CACHE_DEBUG").unwrap_or(false);
    let warning_threshold_ratio = parse_token_budget_warning_threshold_ratio_env();
    let warning_threshold_pct = warning_threshold_ratio * 100.0;
    let warning_threshold_label = format!("{warning_threshold_pct:.0}%");
    let mut inbox_iteration: u64 = 0;

    let mut last_snapshot: std::collections::BTreeMap<ThreadId, ThreadMeta> =
        std::collections::BTreeMap::new();
    let mut bell_state: std::collections::HashMap<ThreadId, (Option<String>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut stale_bell_state: std::collections::HashMap<ThreadId, (Option<bool>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut linkage_bell_state: std::collections::HashMap<ThreadId, (Option<bool>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut auto_apply_bell_state: std::collections::HashMap<ThreadId, (Option<bool>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut fan_in_dependency_blocked_bell_state: std::collections::HashMap<
        ThreadId,
        (Option<bool>, Option<Instant>),
    > = std::collections::HashMap::new();
    let mut fan_in_result_diagnostics_bell_state: std::collections::HashMap<
        ThreadId,
        (Option<bool>, Option<Instant>),
    > = std::collections::HashMap::new();
    let mut token_budget_bell_state: std::collections::HashMap<ThreadId, (Option<bool>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut token_budget_warning_bell_state: std::collections::HashMap<ThreadId, (Option<bool>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut auto_apply_summary_cache =
        std::collections::HashMap::<ThreadId, Option<FanOutAutoApplyInboxSummary>>::new();
    let mut fan_in_summary_cache =
        std::collections::HashMap::<ThreadId, Option<FanInDependencyBlockedInboxSummary>>::new();
    let mut fan_in_diagnostics_summary_cache =
        std::collections::HashMap::<ThreadId, Option<FanInResultDiagnosticsInboxSummary>>::new();
    let mut subagent_pending_summary_cache =
        std::collections::HashMap::<ThreadId, Option<SubagentPendingApprovalsSummary>>::new();
    let bell_notifier = BellNotifier::from_env()?;

    loop {
        inbox_iteration = inbox_iteration.saturating_add(1);
        let raw = app.thread_list_meta(args.include_archived, false).await?;
        let resp: ThreadListMetaResponse = serde_json::from_value(serde_json::to_value(raw)?)?;
        let mut attention_cache = std::collections::HashMap::<ThreadId, ThreadAttention>::new();

        let mut current = std::collections::BTreeMap::<ThreadId, ThreadMeta>::new();
        for thread in resp.threads {
            current.insert(thread.thread_id, thread);
        }
        current = apply_inbox_filters(
            current,
            args.only_fan_out_linkage_issue,
            args.only_fan_out_auto_apply_error,
            args.only_fan_in_dependency_blocked,
            args.only_fan_in_result_diagnostics,
            args.only_token_budget_exceeded,
            args.only_token_budget_warning,
            warning_threshold_ratio,
            args.only_subagent_proxy_approval,
        );
        auto_apply_summary_cache.retain(|thread_id, _| current.contains_key(thread_id));
        fan_in_summary_cache.retain(|thread_id, _| current.contains_key(thread_id));
        fan_in_diagnostics_summary_cache.retain(|thread_id, _| current.contains_key(thread_id));
        subagent_pending_summary_cache.retain(|thread_id, _| current.contains_key(thread_id));

        if !args.watch {
            render_inbox_once(
                app,
                &current,
                args.details,
                args.json,
                debug_summary_cache,
                &mut attention_cache,
            )
            .await?;
            return Ok(());
        }

        render_inbox_changes(
            app,
            &last_snapshot,
            &current,
            args.details,
            args.json,
            &mut attention_cache,
            &mut auto_apply_summary_cache,
            &mut fan_in_summary_cache,
            &mut fan_in_diagnostics_summary_cache,
            &mut subagent_pending_summary_cache,
            debug_summary_cache,
            inbox_iteration,
        )
        .await?;
        if args.bell {
            for (thread_id, thread) in &current {
                let state = thread.attention_state.as_str();
                if !matches!(state, "need_approval" | "failed" | "stuck") {
                    bell_state.entry(*thread_id).or_insert((None, None)).0 =
                        Some(thread.attention_state.clone());
                } else {
                    let entry = bell_state.entry(*thread_id).or_insert((None, None));
                    maybe_bell_per_thread(
                        &bell_notifier,
                        thread_id,
                        &thread.attention_state,
                        args.debounce_ms,
                        &mut entry.0,
                        &mut entry.1,
                    )?;
                }

                if state == "running" {
                    let att = thread_attention_cached(app, &mut attention_cache, *thread_id).await?;
                    let stale_present = !att.stale_processes.is_empty();
                    let entry = stale_bell_state.entry(*thread_id).or_insert((None, None));
                    maybe_bell_stale_per_thread(
                        &bell_notifier,
                        thread_id,
                        stale_present,
                        args.debounce_ms,
                        &mut entry.0,
                        &mut entry.1,
                    )?;
                } else {
                    stale_bell_state.entry(*thread_id).or_insert((Some(false), None)).0 =
                        Some(false);
                }

                let entry = linkage_bell_state.entry(*thread_id).or_insert((None, None));
                maybe_bell_linkage_issue_per_thread(
                    &bell_notifier,
                    thread_id,
                    thread.has_fan_out_linkage_issue,
                    args.debounce_ms,
                    &mut entry.0,
                    &mut entry.1,
                )?;

                let auto_apply_error_present = thread.has_fan_out_auto_apply_error;
                let entry = auto_apply_bell_state.entry(*thread_id).or_insert((None, None));
                maybe_bell_auto_apply_error_per_thread(
                    &bell_notifier,
                    thread_id,
                    auto_apply_error_present,
                    args.debounce_ms,
                    &mut entry.0,
                    &mut entry.1,
                )?;
                let fan_in_dependency_blocked_present = thread.has_fan_in_dependency_blocked;
                let entry = fan_in_dependency_blocked_bell_state
                    .entry(*thread_id)
                    .or_insert((None, None));
                maybe_bell_fan_in_dependency_blocked_per_thread(
                    &bell_notifier,
                    thread_id,
                    fan_in_dependency_blocked_present,
                    args.debounce_ms,
                    &mut entry.0,
                    &mut entry.1,
                )?;

                let fan_in_result_diagnostics_present = thread.has_fan_in_result_diagnostics;
                let entry = fan_in_result_diagnostics_bell_state
                    .entry(*thread_id)
                    .or_insert((None, None));
                maybe_bell_fan_in_result_diagnostics_per_thread(
                    &bell_notifier,
                    thread_id,
                    fan_in_result_diagnostics_present,
                    args.debounce_ms,
                    &mut entry.0,
                    &mut entry.1,
                )?;

                let token_budget_exceeded_present = thread.token_budget_exceeded.unwrap_or(false);
                let entry = token_budget_bell_state.entry(*thread_id).or_insert((None, None));
                maybe_bell_token_budget_exceeded_per_thread(
                    &bell_notifier,
                    thread_id,
                    token_budget_exceeded_present,
                    args.debounce_ms,
                    &mut entry.0,
                    &mut entry.1,
                )?;
                let token_budget_warning_active =
                    thread.token_budget_warning_active.unwrap_or_else(|| {
                        token_budget_warning_present(
                            thread.token_budget_limit,
                            thread.token_budget_utilization,
                            thread.token_budget_exceeded,
                            warning_threshold_ratio,
                        )
                    });
                let entry = token_budget_warning_bell_state
                    .entry(*thread_id)
                    .or_insert((None, None));
                maybe_bell_token_budget_warning_per_thread(
                    &bell_notifier,
                    thread_id,
                    token_budget_warning_active,
                    warning_threshold_label.as_str(),
                    args.debounce_ms,
                    &mut entry.0,
                    &mut entry.1,
                )?;
            }
        }

        last_snapshot = current;
        tokio::time::sleep(poll_interval).await;
    }
}

fn apply_inbox_filters(
    mut threads: std::collections::BTreeMap<ThreadId, ThreadMeta>,
    only_fan_out_linkage_issue: bool,
    only_fan_out_auto_apply_error: bool,
    only_fan_in_dependency_blocked: bool,
    only_fan_in_result_diagnostics: bool,
    only_token_budget_exceeded: bool,
    only_token_budget_warning: bool,
    token_budget_warning_threshold_ratio: f64,
    only_subagent_proxy_approval: bool,
) -> std::collections::BTreeMap<ThreadId, ThreadMeta> {
    if only_fan_out_linkage_issue {
        threads.retain(|_, thread| thread.has_fan_out_linkage_issue);
    }
    if only_fan_out_auto_apply_error {
        threads.retain(|_, thread| thread.has_fan_out_auto_apply_error);
    }
    if only_fan_in_dependency_blocked {
        threads.retain(|_, thread| thread.has_fan_in_dependency_blocked);
    }
    if only_fan_in_result_diagnostics {
        threads.retain(|_, thread| thread.has_fan_in_result_diagnostics);
    }
    if only_token_budget_exceeded {
        threads.retain(|_, thread| thread.token_budget_exceeded.unwrap_or(false));
    }
    if only_token_budget_warning {
        threads.retain(|_, thread| {
            thread.token_budget_warning_active.unwrap_or_else(|| {
                token_budget_warning_present(
                    thread.token_budget_limit,
                    thread.token_budget_utilization,
                    thread.token_budget_exceeded,
                    token_budget_warning_threshold_ratio,
                )
            })
        });
    }
    if only_subagent_proxy_approval {
        threads.retain(|_, thread| thread.pending_subagent_proxy_approvals > 0);
    }
    threads
}

async fn render_inbox_once(
    app: &mut App,
    threads: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    details: bool,
    json: bool,
    debug_summary_cache: bool,
    attention_cache: &mut std::collections::HashMap<ThreadId, ThreadAttention>,
) -> anyhow::Result<()> {
    if json {
        let mut cache_stats = debug_summary_cache.then(InboxSummaryCacheStats::default);
        let empty_prev = std::collections::BTreeMap::new();
        let mut auto_apply_summary_cache =
            std::collections::HashMap::<ThreadId, Option<FanOutAutoApplyInboxSummary>>::new();
        let mut fan_in_summary_cache =
            std::collections::HashMap::<ThreadId, Option<FanInDependencyBlockedInboxSummary>>::new();
        let mut fan_in_diagnostics_summary_cache =
            std::collections::HashMap::<ThreadId, Option<FanInResultDiagnosticsInboxSummary>>::new();
        let mut subagent_pending_summary_cache =
            std::collections::HashMap::<ThreadId, Option<SubagentPendingApprovalsSummary>>::new();
        let auto_apply_summaries = if details {
            collect_fan_out_auto_apply_summaries_watch_json(
                app,
                &empty_prev,
                threads,
                attention_cache,
                &mut auto_apply_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let fan_in_blockers = if details {
            collect_fan_in_dependency_blocked_summaries_watch_json(
                app,
                &empty_prev,
                threads,
                attention_cache,
                &mut fan_in_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let subagent_pending = if details {
            collect_subagent_pending_approvals_summaries_watch_json(
                app,
                &empty_prev,
                threads,
                attention_cache,
                &mut subagent_pending_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let fan_in_diagnostics = if details {
            collect_fan_in_result_diagnostics_summaries_watch_json(
                app,
                &empty_prev,
                threads,
                attention_cache,
                &mut fan_in_diagnostics_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let rows = render_inbox_json_threads(
            threads.values(),
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            details,
        )?;
        let output = build_inbox_json_output(0, threads.len(), rows, cache_stats.as_ref())?;
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let warning_threshold_ratio = parse_token_budget_warning_threshold_ratio_env();
    println!("threads: {}", threads.len());
    for thread in threads.values() {
        render_thread_row(thread);
        if details {
            let att = thread_attention_cached(app, attention_cache, thread.thread_id).await?;
            render_thread_details(&att, warning_threshold_ratio);
            if let Some(summary) = thread.fan_out_auto_apply.as_ref() {
                println!("  {}", format_fan_out_auto_apply_summary(summary));
            } else if let Some(summary) =
                latest_fan_out_auto_apply_summary_with_attention(
                    app,
                    thread.thread_id,
                    Some(att),
                )
                .await
            {
                println!("  {}", format_fan_out_auto_apply_summary(&summary));
            }
            if let Some(summary) = thread.fan_in_dependency_blocker.as_ref() {
                println!("  {}", format_fan_in_dependency_blocked_summary(&summary));
            } else if !thread.has_fan_in_dependency_blocked {
                // list_meta already says there is no blocked fan-in summary; skip fallback reads.
            } else if let Some(summary) = latest_fan_in_dependency_blocked_summary_with_attention(
                app,
                thread.thread_id,
                Some(att),
            )
            .await
            {
                println!("  {}", format_fan_in_dependency_blocked_summary(&summary));
            }
            if let Some(summary) = thread.fan_in_result_diagnostics.as_ref() {
                println!("  {}", format_fan_in_result_diagnostics_summary(summary));
            } else if thread.has_fan_in_result_diagnostics
                && let Some(summary) = latest_fan_in_result_diagnostics_summary_with_attention(
                    app,
                    thread.thread_id,
                    Some(att),
                )
                .await
            {
                println!("  {}", format_fan_in_result_diagnostics_summary(&summary));
            }
        }
    }
    Ok(())
}

async fn render_inbox_changes(
    app: &mut App,
    prev: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    cur: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    details: bool,
    json: bool,
    attention_cache: &mut std::collections::HashMap<ThreadId, ThreadAttention>,
    auto_apply_summary_cache: &mut std::collections::HashMap<
        ThreadId,
        Option<FanOutAutoApplyInboxSummary>,
    >,
    fan_in_summary_cache: &mut std::collections::HashMap<
        ThreadId,
        Option<FanInDependencyBlockedInboxSummary>,
    >,
    fan_in_diagnostics_summary_cache: &mut std::collections::HashMap<
        ThreadId,
        Option<FanInResultDiagnosticsInboxSummary>,
    >,
    subagent_pending_summary_cache: &mut std::collections::HashMap<
        ThreadId,
        Option<SubagentPendingApprovalsSummary>,
    >,
    debug_summary_cache: bool,
    inbox_iteration: u64,
) -> anyhow::Result<()> {
    if json {
        let mut cache_stats = debug_summary_cache.then(InboxSummaryCacheStats::default);
        let auto_apply_summaries = if details {
            collect_fan_out_auto_apply_summaries_watch_json(
                app,
                prev,
                cur,
                attention_cache,
                auto_apply_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let fan_in_blockers = if details {
            collect_fan_in_dependency_blocked_summaries_watch_json(
                app,
                prev,
                cur,
                attention_cache,
                fan_in_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let subagent_pending = if details {
            collect_subagent_pending_approvals_summaries_watch_json(
                app,
                prev,
                cur,
                attention_cache,
                subagent_pending_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let fan_in_diagnostics = if details {
            collect_fan_in_result_diagnostics_summaries_watch_json(
                app,
                prev,
                cur,
                attention_cache,
                fan_in_diagnostics_summary_cache,
                cache_stats.as_mut(),
            )
            .await
        } else {
            std::collections::BTreeMap::new()
        };
        let rows = render_inbox_json_threads(
            cur.values(),
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            details,
        )?;
        let output = build_inbox_json_output(prev.len(), cur.len(), rows, cache_stats.as_ref())?;
        println!("{}", serde_json::to_string(&output)?);
        if let Some(stats) = cache_stats.as_ref() {
            eprintln!(
                "{}",
                format_inbox_summary_cache_stats(inbox_iteration, prev.len(), cur.len(), stats)
            );
        }
        return Ok(());
    }

    let warning_threshold_ratio = parse_token_budget_warning_threshold_ratio_env();
    let changed_thread_ids = cur
        .iter()
        .filter_map(|(thread_id, meta)| {
            inbox_thread_changed(prev.get(thread_id), meta).then_some(*thread_id)
        })
        .collect::<Vec<_>>();

    if details && !changed_thread_ids.is_empty() {
        for thread_id in &changed_thread_ids {
            let _ = thread_attention_cached(app, attention_cache, *thread_id).await?;
        }
    }

    let resolve_watch_detail_summaries =
        details && (!changed_thread_ids.is_empty() || debug_summary_cache);
    let mut cache_stats = if resolve_watch_detail_summaries && debug_summary_cache {
        Some(InboxSummaryCacheStats::default())
    } else {
        None
    };
    let auto_apply_summaries = if resolve_watch_detail_summaries {
        collect_fan_out_auto_apply_summaries_watch_json(
            app,
            prev,
            cur,
            attention_cache,
            auto_apply_summary_cache,
            cache_stats.as_mut(),
        )
        .await
    } else {
        std::collections::BTreeMap::new()
    };
    let fan_in_blockers = if resolve_watch_detail_summaries {
        collect_fan_in_dependency_blocked_summaries_watch_json(
            app,
            prev,
            cur,
            attention_cache,
            fan_in_summary_cache,
            cache_stats.as_mut(),
        )
        .await
    } else {
        std::collections::BTreeMap::new()
    };
    let fan_in_diagnostics = if resolve_watch_detail_summaries {
        collect_fan_in_result_diagnostics_summaries_watch_json(
            app,
            prev,
            cur,
            attention_cache,
            fan_in_diagnostics_summary_cache,
            cache_stats.as_mut(),
        )
        .await
    } else {
        std::collections::BTreeMap::new()
    };
    if resolve_watch_detail_summaries && debug_summary_cache {
        let _ = collect_subagent_pending_approvals_summaries_watch_json(
            app,
            prev,
            cur,
            attention_cache,
            subagent_pending_summary_cache,
            cache_stats.as_mut(),
        )
        .await;
    }

    for thread_id in &changed_thread_ids {
        let Some(meta) = cur.get(thread_id) else {
            continue;
        };

        render_thread_row(meta);
        if details {
            let att = thread_attention_cached(app, attention_cache, *thread_id).await?;
            render_thread_details(&att, warning_threshold_ratio);
            if let Some(summary) = auto_apply_summaries.get(thread_id) {
                println!("  {}", format_fan_out_auto_apply_summary(summary));
            }
            if let Some(summary) = fan_in_blockers.get(thread_id) {
                println!("  {}", format_fan_in_dependency_blocked_summary(summary));
            }
            if let Some(summary) = fan_in_diagnostics.get(thread_id) {
                println!("  {}", format_fan_in_result_diagnostics_summary(summary));
            }
        }
    }

    for thread_id in prev.keys() {
        if !cur.contains_key(thread_id) {
            println!("thread removed: {thread_id}");
        }
    }

    if let Some(stats) = cache_stats.as_ref() {
        eprintln!(
            "{}",
            format_inbox_summary_cache_stats(inbox_iteration, prev.len(), cur.len(), stats)
        );
    }

    Ok(())
}

fn build_inbox_json_output(
    prev_count: usize,
    cur_count: usize,
    rows: Vec<serde_json::Value>,
    summary_cache_stats: Option<&InboxSummaryCacheStats>,
) -> anyhow::Result<serde_json::Value> {
    let mut obj = serde_json::Map::new();
    obj.insert("prev_count".to_string(), serde_json::json!(prev_count));
    obj.insert("cur_count".to_string(), serde_json::json!(cur_count));
    obj.insert("threads".to_string(), serde_json::Value::Array(rows));
    if let Some(stats) = summary_cache_stats {
        obj.insert(
            "summary_cache_stats".to_string(),
            serde_json::to_value(stats).context("serialize inbox summary_cache_stats json output")?,
        );
    }
    Ok(serde_json::Value::Object(obj))
}

async fn collect_fan_out_auto_apply_summaries_watch_json(
    app: &mut App,
    prev: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    cur: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    attention_cache: &std::collections::HashMap<ThreadId, ThreadAttention>,
    summary_cache: &mut std::collections::HashMap<ThreadId, Option<FanOutAutoApplyInboxSummary>>,
    stats: Option<&mut InboxSummaryCacheStats>,
) -> std::collections::BTreeMap<ThreadId, FanOutAutoApplyInboxSummary> {
    let mut summaries = std::collections::BTreeMap::new();
    let mut stats = stats;
    for (thread_id, thread) in cur {
        if let Some(summary) = thread.fan_out_auto_apply.as_ref() {
            summaries.insert(*thread_id, summary.clone());
            summary_cache.insert(*thread_id, Some(summary.clone()));
            if let Some(value) = stats.as_deref_mut() {
                value.fan_out_meta += 1;
            }
            continue;
        }
        if !inbox_thread_changed(prev.get(thread_id), thread)
            && let Some(cached) = summary_cache.get(thread_id)
        {
            if let Some(summary) = cached {
                summaries.insert(*thread_id, summary.clone());
                if let Some(value) = stats.as_deref_mut() {
                    value.fan_out_cache_some += 1;
                }
            } else if let Some(value) = stats.as_deref_mut() {
                value.fan_out_cache_none += 1;
            }
            continue;
        }
        if let Some(attention) = attention_cache.get(thread_id)
            && let Some(summary) = attention.fan_out_auto_apply.as_ref()
        {
            summaries.insert(*thread_id, summary.clone());
            summary_cache.insert(*thread_id, Some(summary.clone()));
            if let Some(value) = stats.as_deref_mut() {
                value.fan_out_attention += 1;
            }
            continue;
        }
        let fetched = latest_fan_out_auto_apply_summary(app, *thread_id).await;
        summary_cache.insert(*thread_id, fetched.clone());
        if let Some(summary) = fetched {
            summaries.insert(*thread_id, summary);
            if let Some(value) = stats.as_deref_mut() {
                value.fan_out_fetch_some += 1;
            }
        } else if let Some(value) = stats.as_deref_mut() {
            value.fan_out_fetch_none += 1;
        }
    }
    summaries
}

fn inbox_thread_changed(previous: Option<&ThreadMeta>, current: &ThreadMeta) -> bool {
    match previous {
        Some(old) => old.last_seq != current.last_seq || old.attention_state != current.attention_state,
        None => true,
    }
}

async fn thread_attention_cached<'a>(
    app: &mut App,
    cache: &'a mut std::collections::HashMap<ThreadId, ThreadAttention>,
    thread_id: ThreadId,
) -> anyhow::Result<&'a ThreadAttention> {
    if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(thread_id) {
        let attention = app.thread_attention(thread_id).await?;
        entry.insert(attention);
    }
    Ok(cache
        .get(&thread_id)
        .expect("thread attention cache must contain requested thread id"))
}

async fn collect_subagent_pending_approvals_summaries_watch_json(
    app: &mut App,
    prev: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    cur: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    attention_cache: &std::collections::HashMap<ThreadId, ThreadAttention>,
    summary_cache: &mut std::collections::HashMap<ThreadId, Option<SubagentPendingApprovalsSummary>>,
    stats: Option<&mut InboxSummaryCacheStats>,
) -> std::collections::BTreeMap<ThreadId, SubagentPendingApprovalsSummary> {
    let mut summaries = std::collections::BTreeMap::new();
    let mut stats = stats;
    for (thread_id, thread) in cur {
        if thread.pending_subagent_proxy_approvals == 0 {
            summary_cache.insert(*thread_id, None);
            if let Some(value) = stats.as_deref_mut() {
                value.subagent_skip_no_pending += 1;
            }
            continue;
        }
        if let Some(value) = stats.as_deref_mut() {
            value.subagent_meta += 1;
        }
        if !inbox_thread_changed(prev.get(thread_id), thread)
            && let Some(cached) = summary_cache.get(thread_id)
        {
            if let Some(summary) = cached {
                summaries.insert(*thread_id, summary.clone());
                if let Some(value) = stats.as_deref_mut() {
                    value.subagent_cache_some += 1;
                }
            } else if let Some(value) = stats.as_deref_mut() {
                value.subagent_cache_none += 1;
            }
            continue;
        }

        let summarized = if let Some(attention) = attention_cache.get(thread_id) {
            let summary = summarize_subagent_pending_approvals(&attention.pending_approvals);
            if summary.is_some() {
                if let Some(value) = stats.as_deref_mut() {
                    value.subagent_attention_some += 1;
                }
            } else if let Some(value) = stats.as_deref_mut() {
                value.subagent_attention_none += 1;
            }
            summary
        } else {
            let summary = app
                .thread_attention(*thread_id)
                .await
                .ok()
                .and_then(|attention| summarize_subagent_pending_approvals(&attention.pending_approvals));
            if summary.is_some() {
                if let Some(value) = stats.as_deref_mut() {
                    value.subagent_fetch_some += 1;
                }
            } else if let Some(value) = stats.as_deref_mut() {
                value.subagent_fetch_none += 1;
            }
            summary
        };
        summary_cache.insert(*thread_id, summarized.clone());
        if let Some(summary) = summarized {
            summaries.insert(*thread_id, summary);
        }
    }
    summaries
}

async fn collect_fan_in_dependency_blocked_summaries_watch_json(
    app: &mut App,
    prev: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    cur: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    attention_cache: &std::collections::HashMap<ThreadId, ThreadAttention>,
    summary_cache: &mut std::collections::HashMap<
        ThreadId,
        Option<FanInDependencyBlockedInboxSummary>,
    >,
    stats: Option<&mut InboxSummaryCacheStats>,
) -> std::collections::BTreeMap<ThreadId, FanInDependencyBlockedInboxSummary> {
    let mut summaries = std::collections::BTreeMap::new();
    let mut stats = stats;
    for (thread_id, thread) in cur {
        if let Some(summary) = thread.fan_in_dependency_blocker.as_ref() {
            summaries.insert(*thread_id, summary.clone());
            summary_cache.insert(*thread_id, Some(summary.clone()));
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_meta += 1;
            }
            continue;
        }
        if !thread.has_fan_in_dependency_blocked {
            summary_cache.insert(*thread_id, None);
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_skip_unblocked += 1;
            }
            continue;
        }
        if !inbox_thread_changed(prev.get(thread_id), thread)
            && let Some(cached) = summary_cache.get(thread_id)
        {
            if let Some(summary) = cached {
                summaries.insert(*thread_id, summary.clone());
                if let Some(value) = stats.as_deref_mut() {
                    value.fan_in_cache_some += 1;
                }
            } else if let Some(value) = stats.as_deref_mut() {
                value.fan_in_cache_none += 1;
            }
            continue;
        }
        if let Some(attention) = attention_cache.get(thread_id)
            && let Some(summary) = attention.fan_in_dependency_blocker.as_ref()
        {
            summaries.insert(*thread_id, summary.clone());
            summary_cache.insert(*thread_id, Some(summary.clone()));
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_attention += 1;
            }
            continue;
        }
        let fetched =
            latest_fan_in_dependency_blocked_summary_with_attention(app, *thread_id, None).await;
        summary_cache.insert(*thread_id, fetched.clone());
        if let Some(summary) = fetched {
            summaries.insert(*thread_id, summary);
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_fetch_some += 1;
            }
        } else if let Some(value) = stats.as_deref_mut() {
            value.fan_in_fetch_none += 1;
        }
    }
    summaries
}

async fn collect_fan_in_result_diagnostics_summaries_watch_json(
    app: &mut App,
    prev: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    cur: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    attention_cache: &std::collections::HashMap<ThreadId, ThreadAttention>,
    summary_cache: &mut std::collections::HashMap<
        ThreadId,
        Option<FanInResultDiagnosticsInboxSummary>,
    >,
    stats: Option<&mut InboxSummaryCacheStats>,
) -> std::collections::BTreeMap<ThreadId, FanInResultDiagnosticsInboxSummary> {
    let mut summaries = std::collections::BTreeMap::new();
    let mut stats = stats;
    for (thread_id, thread) in cur {
        if let Some(summary) = thread.fan_in_result_diagnostics.as_ref() {
            summaries.insert(*thread_id, summary.clone());
            summary_cache.insert(*thread_id, Some(summary.clone()));
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_diag_meta += 1;
            }
            continue;
        }
        if !thread.has_fan_in_result_diagnostics {
            summary_cache.insert(*thread_id, None);
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_diag_skip_absent += 1;
            }
            continue;
        }
        if !inbox_thread_changed(prev.get(thread_id), thread)
            && let Some(cached) = summary_cache.get(thread_id)
        {
            if let Some(summary) = cached {
                summaries.insert(*thread_id, summary.clone());
                if let Some(value) = stats.as_deref_mut() {
                    value.fan_in_diag_cache_some += 1;
                }
            } else if let Some(value) = stats.as_deref_mut() {
                value.fan_in_diag_cache_none += 1;
            }
            continue;
        }
        if let Some(attention) = attention_cache.get(thread_id)
            && let Some(summary) = attention.fan_in_result_diagnostics.as_ref()
        {
            summaries.insert(*thread_id, summary.clone());
            summary_cache.insert(*thread_id, Some(summary.clone()));
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_diag_attention += 1;
            }
            continue;
        }
        let fetched = latest_fan_in_result_diagnostics_summary_with_attention(app, *thread_id, None).await;
        summary_cache.insert(*thread_id, fetched.clone());
        if let Some(summary) = fetched {
            summaries.insert(*thread_id, summary);
            if let Some(value) = stats.as_deref_mut() {
                value.fan_in_diag_fetch_some += 1;
            }
        } else if let Some(value) = stats.as_deref_mut() {
            value.fan_in_diag_fetch_none += 1;
        }
    }
    summaries
}

fn format_inbox_summary_cache_stats(
    iteration: u64,
    prev_count: usize,
    cur_count: usize,
    stats: &InboxSummaryCacheStats,
) -> String {
    format!(
        "inbox_summary_cache iter={iteration} prev={prev_count} cur={cur_count} \
fan_out(meta={},cache_some={},cache_none={},attention={},fetch_some={},fetch_none={}) \
fan_in(meta={},cache_some={},cache_none={},attention={},fetch_some={},fetch_none={},skip_unblocked={}) \
fan_in_diag(meta={},cache_some={},cache_none={},attention={},fetch_some={},fetch_none={},skip_absent={}) \
subagent(meta={},cache_some={},cache_none={},attention_some={},attention_none={},fetch_some={},fetch_none={},skip_no_pending={})",
        stats.fan_out_meta,
        stats.fan_out_cache_some,
        stats.fan_out_cache_none,
        stats.fan_out_attention,
        stats.fan_out_fetch_some,
        stats.fan_out_fetch_none,
        stats.fan_in_meta,
        stats.fan_in_cache_some,
        stats.fan_in_cache_none,
        stats.fan_in_attention,
        stats.fan_in_fetch_some,
        stats.fan_in_fetch_none,
        stats.fan_in_skip_unblocked,
        stats.fan_in_diag_meta,
        stats.fan_in_diag_cache_some,
        stats.fan_in_diag_cache_none,
        stats.fan_in_diag_attention,
        stats.fan_in_diag_fetch_some,
        stats.fan_in_diag_fetch_none,
        stats.fan_in_diag_skip_absent,
        stats.subagent_meta,
        stats.subagent_cache_some,
        stats.subagent_cache_none,
        stats.subagent_attention_some,
        stats.subagent_attention_none,
        stats.subagent_fetch_some,
        stats.subagent_fetch_none,
        stats.subagent_skip_no_pending
    )
}

fn format_watch_summary_refresh_debug(
    iteration: u64,
    event_count: usize,
    refresh_auto_apply: bool,
    refresh_fan_in_blocker: bool,
    refresh_fan_in_diagnostics: bool,
    refresh_subagent_pending: bool,
    auto_apply_source: SummarySource,
    fan_in_source: SummarySource,
    fan_in_diagnostics_source: SummarySource,
    subagent_source: SummarySource,
) -> String {
    format!(
        "watch_summary_refresh iter={iteration} events={event_count} \
auto_apply(refresh={},source={}) fan_in(refresh={},source={}) fan_in_diag(refresh={},source={}) subagent(refresh={},source={})",
        refresh_auto_apply,
        summary_source_label(auto_apply_source),
        refresh_fan_in_blocker,
        summary_source_label(fan_in_source),
        refresh_fan_in_diagnostics,
        summary_source_label(fan_in_diagnostics_source),
        refresh_subagent_pending,
        summary_source_label(subagent_source)
    )
}

fn build_watch_summary_refresh_debug_json_row(
    iteration: u64,
    event_count: usize,
    refresh_auto_apply: bool,
    refresh_fan_in_blocker: bool,
    refresh_fan_in_diagnostics: bool,
    refresh_subagent_pending: bool,
    auto_apply_source: SummarySource,
    fan_in_source: SummarySource,
    fan_in_diagnostics_source: SummarySource,
    subagent_source: SummarySource,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "watch_summary_refresh_debug",
        "iteration": iteration,
        "event_count": event_count,
        "auto_apply": {
            "refresh": refresh_auto_apply,
            "source": summary_source_label(auto_apply_source),
        },
        "fan_in": {
            "refresh": refresh_fan_in_blocker,
            "source": summary_source_label(fan_in_source),
        },
        "fan_in_diagnostics": {
            "refresh": refresh_fan_in_diagnostics,
            "source": summary_source_label(fan_in_diagnostics_source),
        },
        "subagent": {
            "refresh": refresh_subagent_pending,
            "source": summary_source_label(subagent_source),
        },
    })
}

fn summary_source_label(source: SummarySource) -> &'static str {
    match source {
        SummarySource::Previous => "previous",
        SummarySource::Attention => "attention",
        SummarySource::Artifact => "artifact",
        SummarySource::None => "none",
    }
}

fn render_inbox_json_threads<'a, I>(
    threads: I,
    auto_apply_summaries: &std::collections::BTreeMap<ThreadId, FanOutAutoApplyInboxSummary>,
    fan_in_blockers: &std::collections::BTreeMap<ThreadId, FanInDependencyBlockedInboxSummary>,
    fan_in_diagnostics: &std::collections::BTreeMap<ThreadId, FanInResultDiagnosticsInboxSummary>,
    subagent_pending: &std::collections::BTreeMap<ThreadId, SubagentPendingApprovalsSummary>,
    include_detail_summaries: bool,
) -> anyhow::Result<Vec<serde_json::Value>>
where
    I: IntoIterator<Item = &'a ThreadMeta>,
{
    let mut out = Vec::new();
    let token_budget_warning_threshold_ratio = parse_token_budget_warning_threshold_ratio_env();
    for thread in threads {
        let mut row =
            serde_json::to_value(thread).context("serialize inbox thread row for json output")?;
        if let serde_json::Value::Object(obj) = &mut row {
            obj.insert(
                "token_budget_warning_active".to_string(),
                serde_json::Value::Bool(thread.token_budget_warning_active.unwrap_or_else(|| {
                    token_budget_warning_present(
                        thread.token_budget_limit,
                        thread.token_budget_utilization,
                        thread.token_budget_exceeded,
                        token_budget_warning_threshold_ratio,
                    )
                })),
            );
        }
        if let Some(summary) = auto_apply_summaries.get(&thread.thread_id) {
            if let serde_json::Value::Object(obj) = &mut row {
                obj.insert(
                    "fan_out_auto_apply".to_string(),
                    serde_json::to_value(summary)
                        .context("serialize fan_out_auto_apply json summary")?,
                );
            }
        }
        if let Some(summary) = fan_in_blockers.get(&thread.thread_id) {
            if let serde_json::Value::Object(obj) = &mut row {
                obj.insert(
                    "fan_in_dependency_blocker".to_string(),
                    serde_json::to_value(summary)
                        .context("serialize fan_in_dependency_blocker json summary")?,
                );
            }
        }
        if let Some(summary) = subagent_pending.get(&thread.thread_id) {
            if let serde_json::Value::Object(obj) = &mut row {
                obj.insert(
                    "subagent_pending".to_string(),
                    serde_json::to_value(summary)
                        .context("serialize subagent_pending json summary")?,
                );
            }
        }
        if include_detail_summaries {
            if let Some(summary) = fan_in_diagnostics.get(&thread.thread_id) {
                if let serde_json::Value::Object(obj) = &mut row {
                    obj.insert(
                        "fan_in_result_diagnostics".to_string(),
                        serde_json::to_value(summary)
                            .context("serialize fan_in_result_diagnostics json summary")?,
                    );
                }
            } else if let Some(summary) = thread.fan_in_result_diagnostics.as_ref()
                && let serde_json::Value::Object(obj) = &mut row
            {
                obj.insert(
                    "fan_in_result_diagnostics".to_string(),
                    serde_json::to_value(summary)
                        .context("serialize fan_in_result_diagnostics json summary")?,
                );
            }
        }
        out.push(row);
    }
    Ok(out)
}

fn render_thread_row(thread: &ThreadMeta) {
    println!("{}", format_thread_row(thread));
}

fn format_thread_row(thread: &ThreadMeta) -> String {
    let cwd = shorten_path(&thread.cwd, 60);
    let model = thread.model.as_deref().unwrap_or("-");
    let turn = thread
        .active_turn_id
        .or(thread.last_turn_id)
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{}  state={}  seq={}  turn={}  model={}  cwd={}",
        thread.thread_id, thread.attention_state, thread.last_seq, turn, model, cwd
    )
}

fn render_thread_details(att: &ThreadAttention, warning_threshold_ratio: f64) {
    for line in format_thread_detail_lines(att, warning_threshold_ratio) {
        println!("  {line}");
    }
}

fn format_thread_detail_lines(att: &ThreadAttention, warning_threshold_ratio: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let token_budget_exceeded = att.token_budget_exceeded.unwrap_or(false);
    let token_budget_warning = att.token_budget_warning_active.unwrap_or_else(|| {
        token_budget_warning_present(
            att.token_budget_limit,
            att.token_budget_utilization,
            att.token_budget_exceeded,
            warning_threshold_ratio,
        )
    });
    let markers = attention_detail_markers(
        att.has_plan_ready,
        att.has_diff_ready,
        att.has_fan_out_linkage_issue,
        att.has_fan_out_auto_apply_error,
        att.has_fan_in_dependency_blocked,
        att.has_fan_in_result_diagnostics,
        token_budget_exceeded,
        token_budget_warning,
        att.has_test_failed,
    );
    if !markers.is_empty() {
        lines.push(format!("markers: {}", markers.join(", ")));
    }
    if let Some(snapshot) = format_token_budget_snapshot(
        att.token_budget_limit,
        att.token_budget_remaining,
        att.token_budget_utilization,
        att.token_budget_exceeded,
    ) {
        lines.push(snapshot);
    }

    if !att.pending_approvals.is_empty() {
        let ids = att
            .pending_approvals
            .iter()
            .take(3)
            .map(|a| a.approval_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "approvals: {} ({ids}{})",
            att.pending_approvals.len(),
            if att.pending_approvals.len() > 3 {
                ", ..."
            } else {
                ""
            }
        ));
        let previews = att
            .pending_approvals
            .iter()
            .take(3)
            .map(format_pending_approval_preview)
            .collect::<Vec<_>>()
            .join("; ");
        lines.push(format!(
            "approval_details: {previews}{}",
            if att.pending_approvals.len() > 3 {
                "; ..."
            } else {
                ""
            }
        ));
        let commands = att
            .pending_approvals
            .iter()
            .take(3)
            .filter_map(format_pending_approval_commands)
            .collect::<Vec<_>>()
            .join("; ");
        if !commands.is_empty() {
            lines.push(format!(
                "approval_commands: {commands}{}",
                if att.pending_approvals.len() > 3 {
                    "; ..."
                } else {
                    ""
                }
            ));
        }
        if let Some(subagent_summary) =
            format_subagent_pending_approvals_summary(&att.pending_approvals)
        {
            lines.push(subagent_summary);
        }
    }
    if !att.running_processes.is_empty() {
        let ids = att
            .running_processes
            .iter()
            .take(3)
            .map(|p| p.process_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "processes: {} ({ids}{})",
            att.running_processes.len(),
            if att.running_processes.len() > 3 {
                ", ..."
            } else {
                ""
            }
        ));
    }
    if !att.failed_processes.is_empty() {
        let ids = att
            .failed_processes
            .iter()
            .take(3)
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "failed_processes: {} ({ids}{})",
            att.failed_processes.len(),
            if att.failed_processes.len() > 3 {
                ", ..."
            } else {
                ""
            }
        ));
    }
    if !att.stale_processes.is_empty() {
        let ids = att
            .stale_processes
            .iter()
            .take(3)
            .map(|p| p.process_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "stale_processes: {} ({ids}{})",
            att.stale_processes.len(),
            if att.stale_processes.len() > 3 {
                ", ..."
            } else {
                ""
            }
        ));
    }
    lines
}

fn attention_detail_markers(
    has_plan_ready: bool,
    has_diff_ready: bool,
    has_fan_out_linkage_issue: bool,
    has_fan_out_auto_apply_error: bool,
    has_fan_in_dependency_blocked: bool,
    has_fan_in_result_diagnostics: bool,
    has_token_budget_exceeded: bool,
    has_token_budget_warning: bool,
    has_test_failed: bool,
) -> Vec<&'static str> {
    attention_marker_parts(AttentionMarkerSummaryFlags {
        has_plan_ready,
        has_diff_ready,
        has_fan_out_linkage_issue,
        has_fan_out_auto_apply_error,
        has_fan_in_dependency_blocked,
        has_fan_in_result_diagnostics,
        has_subagent_proxy_approval: false,
        has_token_budget_exceeded,
        has_token_budget_warning,
        has_test_failed,
    })
}

fn format_token_budget_snapshot(
    token_budget_limit: Option<u64>,
    token_budget_remaining: Option<u64>,
    token_budget_utilization: Option<f64>,
    token_budget_exceeded: Option<bool>,
) -> Option<String> {
    let limit = token_budget_limit?;
    let remaining = token_budget_remaining
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let utilization = token_budget_utilization
        .map(|value| format!("{:.1}%", value * 100.0))
        .unwrap_or_else(|| "-".to_string());
    let exceeded = token_budget_exceeded.unwrap_or(false);
    Some(format!(
        "token_budget: remaining={remaining} limit={limit} utilization={utilization} exceeded={exceeded}"
    ))
}

fn format_subagent_pending_approvals_summary(
    approvals: &[omne_app_server_protocol::ThreadAttentionPendingApproval],
) -> Option<String> {
    let summary = summarize_subagent_pending_approvals(approvals)?;
    Some(format_subagent_pending_summary(&summary))
}

fn format_subagent_pending_summary(summary: &SubagentPendingApprovalsSummary) -> String {
    let state_counts = summary
        .states
        .iter()
        .map(|(state, count)| format!("{state}:{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("subagent_pending: total={} states={state_counts}", summary.total)
}

fn summarize_subagent_pending_approvals(
    approvals: &[omne_app_server_protocol::ThreadAttentionPendingApproval],
) -> Option<SubagentPendingApprovalsSummary> {
    let mut total = 0usize;
    let mut states = std::collections::BTreeMap::<String, usize>::new();

    for pending in approvals {
        if !is_subagent_proxy_pending_approval(pending) {
            continue;
        }
        total = total.saturating_add(1);
        let state = pending
            .summary
            .as_ref()
            .and_then(|summary| summary.child_attention_state.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_string());
        let entry = states.entry(state).or_default();
        *entry = entry.saturating_add(1);
    }

    if total == 0 {
        None
    } else {
        Some(SubagentPendingApprovalsSummary { total, states })
    }
}

fn is_subagent_proxy_pending_approval(
    pending: &omne_app_server_protocol::ThreadAttentionPendingApproval,
) -> bool {
    pending.action_id
        == Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval)
        || pending.action.as_deref() == Some("subagent/proxy_approval")
}

async fn latest_fan_out_auto_apply_summary(
    app: &mut App,
    thread_id: ThreadId,
) -> Option<FanOutAutoApplyInboxSummary> {
    let attention = app.thread_attention(thread_id).await.ok();
    latest_fan_out_auto_apply_summary_with_attention(app, thread_id, attention.as_ref()).await
}

async fn latest_fan_out_auto_apply_summary_with_attention(
    app: &mut App,
    thread_id: ThreadId,
    attention: Option<&ThreadAttention>,
) -> Option<FanOutAutoApplyInboxSummary> {
    latest_fan_out_auto_apply_summary_with_source(app, thread_id, attention)
        .await
        .0
}

async fn latest_fan_out_auto_apply_summary_from_artifacts(
    app: &mut App,
    thread_id: ThreadId,
) -> Option<FanOutAutoApplyInboxSummary> {
    let list = app.artifact_list(thread_id, None).await.ok()?;
    let latest = list
        .artifacts
        .iter()
        .filter(|artifact| artifact.artifact_type == "fan_out_result")
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.version.cmp(&right.version))
        })?;

    let read = app
        .artifact_read(thread_id, latest.artifact_id, None, Some(64 * 1024), None)
        .await
        .ok()?;
    let payload = read.fan_out_result.as_ref()?;
    fan_out_auto_apply_summary_from_payload(payload)
}

async fn latest_fan_out_auto_apply_summary_with_source(
    app: &mut App,
    thread_id: ThreadId,
    attention: Option<&ThreadAttention>,
) -> (Option<FanOutAutoApplyInboxSummary>, SummarySource) {
    if let Some(summary) = attention.and_then(|value| value.fan_out_auto_apply.clone()) {
        return (Some(summary), SummarySource::Attention);
    }
    let summary = latest_fan_out_auto_apply_summary_from_artifacts(app, thread_id).await;
    if summary.is_some() {
        (summary, SummarySource::Artifact)
    } else {
        (None, SummarySource::None)
    }
}

async fn latest_fan_in_dependency_blocked_summary_with_attention(
    app: &mut App,
    thread_id: ThreadId,
    attention: Option<&ThreadAttention>,
) -> Option<FanInDependencyBlockedInboxSummary> {
    latest_fan_in_dependency_blocked_summary_with_source(app, thread_id, attention)
        .await
        .0
}

async fn latest_fan_in_dependency_blocked_summary_from_artifacts(
    app: &mut App,
    thread_id: ThreadId,
) -> Option<FanInDependencyBlockedInboxSummary> {
    let list = app.artifact_list(thread_id, None).await.ok()?;
    let latest = list
        .artifacts
        .iter()
        .filter(|artifact| artifact.artifact_type == "fan_in_summary")
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.version.cmp(&right.version))
        })?;

    let read = app
        .artifact_read(thread_id, latest.artifact_id, None, Some(64 * 1024), None)
        .await
        .ok()?;
    let payload = read.fan_in_summary.as_ref()?;
    fan_in_dependency_blocked_summary_from_payload(payload)
}

async fn latest_fan_in_dependency_blocked_summary_with_source(
    app: &mut App,
    thread_id: ThreadId,
    attention: Option<&ThreadAttention>,
) -> (Option<FanInDependencyBlockedInboxSummary>, SummarySource) {
    if let Some(summary) = attention.and_then(|value| value.fan_in_dependency_blocker.clone()) {
        return (Some(summary), SummarySource::Attention);
    }
    let summary = latest_fan_in_dependency_blocked_summary_from_artifacts(app, thread_id).await;
    if summary.is_some() {
        (summary, SummarySource::Artifact)
    } else {
        (None, SummarySource::None)
    }
}

async fn latest_fan_in_result_diagnostics_summary_with_attention(
    app: &mut App,
    thread_id: ThreadId,
    attention: Option<&ThreadAttention>,
) -> Option<FanInResultDiagnosticsInboxSummary> {
    latest_fan_in_result_diagnostics_summary_with_source(app, thread_id, attention)
        .await
        .0
}

async fn latest_fan_in_result_diagnostics_summary_with_source(
    app: &mut App,
    thread_id: ThreadId,
    attention: Option<&ThreadAttention>,
) -> (Option<FanInResultDiagnosticsInboxSummary>, SummarySource) {
    if let Some(summary) = attention.and_then(|value| value.fan_in_result_diagnostics.clone()) {
        return (Some(summary), SummarySource::Attention);
    }
    let summary = latest_fan_in_result_diagnostics_summary_from_artifacts(app, thread_id).await;
    if summary.is_some() {
        (summary, SummarySource::Artifact)
    } else {
        (None, SummarySource::None)
    }
}

async fn latest_fan_in_result_diagnostics_summary_from_artifacts(
    app: &mut App,
    thread_id: ThreadId,
) -> Option<FanInResultDiagnosticsInboxSummary> {
    let list = app.artifact_list(thread_id, None).await.ok()?;
    let latest = list
        .artifacts
        .iter()
        .filter(|artifact| artifact.artifact_type == "fan_in_summary")
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.version.cmp(&right.version))
        })?;

    let read = app
        .artifact_read(thread_id, latest.artifact_id, None, Some(64 * 1024), None)
        .await
        .ok()?;
    let payload = read.fan_in_summary.as_ref()?;
    fan_in_result_diagnostics_summary_from_payload(payload)
}

fn fan_out_auto_apply_summary_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutResultStructuredData,
) -> Option<FanOutAutoApplyInboxSummary> {
    let auto_apply = payload.isolated_write_auto_apply.as_ref()?;
    if auto_apply.applied {
        return None;
    }

    let status = if auto_apply
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        "error".to_string()
    } else if auto_apply.attempted {
        "attempted_not_applied".to_string()
    } else if auto_apply.enabled {
        "enabled_not_attempted".to_string()
    } else {
        "disabled".to_string()
    };

    let stage = auto_apply.failure_stage.as_ref().map(|value| value.as_str().to_string());
    let patch_artifact_id = auto_apply
        .patch_artifact_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let recovery_commands = (!auto_apply.recovery_commands.is_empty())
        .then_some(auto_apply.recovery_commands.len());
    let recovery_1 = auto_apply
        .recovery_commands
        .first()
        .map(|first_command| {
            shorten_path(
                format_recovery_command_preview(first_command).as_str(),
                120,
            )
        });
    let error = auto_apply
        .error
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| shorten_path(value, 120));

    Some(FanOutAutoApplyInboxSummary {
        task_id: payload.task_id.clone(),
        status,
        stage,
        patch_artifact_id,
        recovery_commands,
        recovery_1,
        error,
    })
}

fn fan_in_dependency_blocked_summary_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanInSummaryStructuredData,
) -> Option<FanInDependencyBlockedInboxSummary> {
    let dependency_blocked_count = payload
        .tasks
        .iter()
        .filter(|task| task.dependency_blocked)
        .count();
    let task = payload.tasks.iter().find(|task| {
        task.dependency_blocked
            || task
                .dependency_blocker_task_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || task
                .dependency_blocker_status
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    })?;
    let mut diagnostics_tasks = 0usize;
    let mut diagnostics_matched_completion_total = 0u64;
    let mut diagnostics_pending_matching_tool_ids_total = 0usize;
    let mut diagnostics_scan_last_seq_max = 0u64;
    for item in &payload.tasks {
        if let Some(diagnostics) = item.result_artifact_diagnostics.as_ref() {
            diagnostics_tasks = diagnostics_tasks.saturating_add(1);
            diagnostics_matched_completion_total = diagnostics_matched_completion_total
                .saturating_add(diagnostics.matched_completion_count);
            diagnostics_pending_matching_tool_ids_total = diagnostics_pending_matching_tool_ids_total
                .saturating_add(diagnostics.pending_matching_tool_ids);
            diagnostics_scan_last_seq_max =
                diagnostics_scan_last_seq_max.max(diagnostics.scan_last_seq);
        }
    }

    let blocker_task_id = task
        .dependency_blocker_task_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let blocker_status = task
        .dependency_blocker_status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let reason = task
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| shorten_path(value, 120));

    Some(FanInDependencyBlockedInboxSummary {
        task_id: task.task_id.clone(),
        status: task.status.clone(),
        dependency_blocked_count,
        task_count: payload.task_count,
        dependency_blocked_ratio: if payload.task_count == 0 {
            0.0
        } else {
            dependency_blocked_count as f64 / payload.task_count as f64
        },
        diagnostics_tasks: (diagnostics_tasks > 0).then_some(diagnostics_tasks),
        diagnostics_matched_completion_total: (diagnostics_tasks > 0)
            .then_some(diagnostics_matched_completion_total),
        diagnostics_pending_matching_tool_ids_total: (diagnostics_tasks > 0)
            .then_some(diagnostics_pending_matching_tool_ids_total),
        diagnostics_scan_last_seq_max: (diagnostics_tasks > 0)
            .then_some(diagnostics_scan_last_seq_max),
        blocker_task_id,
        blocker_status,
        reason,
    })
}

fn fan_in_result_diagnostics_summary_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanInSummaryStructuredData,
) -> Option<FanInResultDiagnosticsInboxSummary> {
    let mut diagnostics_tasks = 0usize;
    let mut diagnostics_matched_completion_total = 0u64;
    let mut diagnostics_pending_matching_tool_ids_total = 0usize;
    let mut diagnostics_scan_last_seq_max = 0u64;
    for item in &payload.tasks {
        if let Some(diagnostics) = item.result_artifact_diagnostics.as_ref() {
            diagnostics_tasks = diagnostics_tasks.saturating_add(1);
            diagnostics_matched_completion_total = diagnostics_matched_completion_total
                .saturating_add(diagnostics.matched_completion_count);
            diagnostics_pending_matching_tool_ids_total = diagnostics_pending_matching_tool_ids_total
                .saturating_add(diagnostics.pending_matching_tool_ids);
            diagnostics_scan_last_seq_max =
                diagnostics_scan_last_seq_max.max(diagnostics.scan_last_seq);
        }
    }

    (diagnostics_tasks > 0).then_some(FanInResultDiagnosticsInboxSummary {
        task_count: payload.task_count,
        diagnostics_tasks,
        diagnostics_matched_completion_total,
        diagnostics_pending_matching_tool_ids_total,
        diagnostics_scan_last_seq_max,
    })
}

fn format_recovery_command_preview(
    command: &omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData,
) -> String {
    if command.argv.is_empty() {
        return command.label.clone();
    }
    format!("{}: {}", command.label, command.argv.join(" "))
}

fn format_fan_out_auto_apply_summary(summary: &FanOutAutoApplyInboxSummary) -> String {
    let mut out = format!(
        "fan_out_auto_apply: task_id={} status={}",
        summary.task_id, summary.status
    );
    if let Some(stage) = summary.stage.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(" stage=");
        out.push_str(stage);
    }
    if let Some(patch_artifact_id) = summary
        .patch_artifact_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" patch_artifact_id=");
        out.push_str(patch_artifact_id);
    }
    if let Some(recovery_commands) = summary.recovery_commands {
        out.push_str(" recovery_commands=");
        out.push_str(recovery_commands.to_string().as_str());
    }
    if let Some(recovery_1) = summary.recovery_1.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(" recovery_1=");
        out.push_str(recovery_1);
    }
    if let Some(error) = summary.error.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(" error=");
        out.push_str(error);
    }
    out
}

fn format_fan_in_dependency_blocked_summary(summary: &FanInDependencyBlockedInboxSummary) -> String {
    let mut out = format!(
        "fan_in_dependency_blocker: task_id={} status={} blocked={}/{}",
        summary.task_id, summary.status, summary.dependency_blocked_count, summary.task_count
    );
    if let Some(blocker_task_id) = summary
        .blocker_task_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" blocker_task_id=");
        out.push_str(blocker_task_id);
    }
    if let Some(blocker_status) = summary
        .blocker_status
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        out.push_str(" blocker_status=");
        out.push_str(blocker_status);
    }
    if let Some(reason) = summary.reason.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(" reason=");
        out.push_str(reason);
    }
    if let Some(diagnostics_tasks) = summary.diagnostics_tasks {
        out.push_str(" diagnostics_tasks=");
        out.push_str(diagnostics_tasks.to_string().as_str());
    }
    if let Some(diagnostics_matched_completion_total) = summary.diagnostics_matched_completion_total
    {
        out.push_str(" diagnostics_matched_completion_total=");
        out.push_str(diagnostics_matched_completion_total.to_string().as_str());
    }
    if let Some(diagnostics_pending_matching_tool_ids_total) =
        summary.diagnostics_pending_matching_tool_ids_total
    {
        out.push_str(" diagnostics_pending_matching_tool_ids_total=");
        out.push_str(diagnostics_pending_matching_tool_ids_total.to_string().as_str());
    }
    if let Some(diagnostics_scan_last_seq_max) = summary.diagnostics_scan_last_seq_max {
        out.push_str(" diagnostics_scan_last_seq_max=");
        out.push_str(diagnostics_scan_last_seq_max.to_string().as_str());
    }
    out
}

fn format_fan_in_result_diagnostics_summary(summary: &FanInResultDiagnosticsInboxSummary) -> String {
    format!(
        "fan_in_result_diagnostics: tasks={} diagnostics_tasks={} matched_completion_total={} pending_matching_tool_ids_total={} scan_last_seq_max={}",
        summary.task_count,
        summary.diagnostics_tasks,
        summary.diagnostics_matched_completion_total,
        summary.diagnostics_pending_matching_tool_ids_total,
        summary.diagnostics_scan_last_seq_max
    )
}

#[cfg(test)]
fn watch_detail_summary_lines(
    auto_apply: Option<&FanOutAutoApplyInboxSummary>,
    fan_in_blocker: Option<&FanInDependencyBlockedInboxSummary>,
    fan_in_diagnostics: Option<&FanInResultDiagnosticsInboxSummary>,
    subagent_pending: Option<&SubagentPendingApprovalsSummary>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(summary) = auto_apply {
        lines.push(format!("summary: {}", format_fan_out_auto_apply_summary(summary)));
    }
    if let Some(summary) = fan_in_blocker {
        lines.push(format!(
            "summary: {}",
            format_fan_in_dependency_blocked_summary(summary)
        ));
    }
    if let Some(summary) = fan_in_diagnostics {
        lines.push(format!(
            "summary: {}",
            format_fan_in_result_diagnostics_summary(summary)
        ));
    }
    if let Some(summary) = subagent_pending {
        lines.push(format!(
            "summary: {}",
            format_subagent_pending_summary(summary)
        ));
    }
    lines
}

fn watch_detail_summary_lines_with_delta(
    last: Option<&WatchDetailSummarySnapshot>,
    current: &WatchDetailSummarySnapshot,
) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(current_auto_apply) = current.auto_apply.as_ref() {
        let changed_fields = fan_out_auto_apply_changed_fields(
            last.and_then(|value| value.auto_apply.as_ref()),
            current_auto_apply,
        );
        if !changed_fields.is_empty() {
            lines.push(format!(
                "summary: {}",
                format_fan_out_auto_apply_summary(current_auto_apply)
            ));
        }
    } else if last.and_then(|value| value.auto_apply.as_ref()).is_some() {
        lines.push("summary: fan_out_auto_apply: cleared".to_string());
    }

    if let Some(current_fan_in_blocker) = current.fan_in_blocker.as_ref() {
        let changed_fields = fan_in_dependency_blocker_changed_fields(
            last.and_then(|value| value.fan_in_blocker.as_ref()),
            current_fan_in_blocker,
        );
        if !changed_fields.is_empty() {
            lines.push(format!(
                "summary: {}",
                format_fan_in_dependency_blocked_summary(current_fan_in_blocker)
            ));
        }
    } else if last.and_then(|value| value.fan_in_blocker.as_ref()).is_some() {
        lines.push("summary: fan_in_dependency_blocker: cleared".to_string());
    }

    if let Some(current_fan_in_diagnostics) = current.fan_in_diagnostics.as_ref() {
        let changed_fields = fan_in_result_diagnostics_changed_fields(
            last.and_then(|value| value.fan_in_diagnostics.as_ref()),
            current_fan_in_diagnostics,
        );
        if !changed_fields.is_empty() {
            lines.push(format!(
                "summary: {}",
                format_fan_in_result_diagnostics_summary(current_fan_in_diagnostics)
            ));
        }
    } else if last.and_then(|value| value.fan_in_diagnostics.as_ref()).is_some() {
        lines.push("summary: fan_in_result_diagnostics: cleared".to_string());
    }

    if let Some(current_subagent_pending) = current.subagent_pending.as_ref() {
        let changed_fields = subagent_pending_changed_fields(
            last.and_then(|value| value.subagent_pending.as_ref()),
            current_subagent_pending,
        );
        if !changed_fields.is_empty() {
            lines.push(format!(
                "summary: {}",
                format_subagent_pending_summary(current_subagent_pending)
            ));
        }
    } else if last.and_then(|value| value.subagent_pending.as_ref()).is_some() {
        lines.push("summary: subagent_pending: cleared".to_string());
    }

    lines
}

fn should_emit_watch_detail_summary(
    last: Option<&WatchDetailSummarySnapshot>,
    current: &WatchDetailSummarySnapshot,
) -> bool {
    let current_has_any = current.auto_apply.is_some()
        || current.fan_in_blocker.is_some()
        || current.fan_in_diagnostics.is_some()
        || current.subagent_pending.is_some();
    if !current_has_any {
        return last.is_some_and(|value| {
            value.auto_apply.is_some()
                || value.fan_in_blocker.is_some()
                || value.fan_in_diagnostics.is_some()
                || value.subagent_pending.is_some()
        });
    }
    last != Some(current)
}

#[cfg(test)]
fn watch_detail_summary_json_rows(
    thread_id: ThreadId,
    auto_apply: Option<&FanOutAutoApplyInboxSummary>,
    fan_in_blocker: Option<&FanInDependencyBlockedInboxSummary>,
    fan_in_diagnostics: Option<&FanInResultDiagnosticsInboxSummary>,
    subagent_pending: Option<&SubagentPendingApprovalsSummary>,
) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    if let Some(summary) = auto_apply {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_out_auto_apply",
            "payload": summary,
        }));
    }
    if let Some(summary) = fan_in_blocker {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_in_dependency_blocker",
            "payload": summary,
        }));
    }
    if let Some(summary) = fan_in_diagnostics {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_in_result_diagnostics",
            "payload": summary,
        }));
    }
    if let Some(summary) = subagent_pending {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "subagent_pending",
            "payload": summary,
        }));
    }
    rows
}

fn watch_detail_summary_json_rows_with_delta(
    thread_id: ThreadId,
    last: Option<&WatchDetailSummarySnapshot>,
    current: &WatchDetailSummarySnapshot,
) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();

    if let Some(current_auto_apply) = current.auto_apply.as_ref() {
        let changed_fields = fan_out_auto_apply_changed_fields(
            last.and_then(|value| value.auto_apply.as_ref()),
            current_auto_apply,
        );
        if !changed_fields.is_empty() {
            rows.push(serde_json::json!({
                "kind": "watch_detail_summary",
                "thread_id": thread_id,
                "summary_type": "fan_out_auto_apply",
                "payload": current_auto_apply,
                "changed_fields": changed_fields,
            }));
        }
    } else if last.and_then(|value| value.auto_apply.as_ref()).is_some() {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_out_auto_apply",
            "cleared": true,
            "changed_fields": ["cleared"],
        }));
    }

    if let Some(current_fan_in_blocker) = current.fan_in_blocker.as_ref() {
        let changed_fields = fan_in_dependency_blocker_changed_fields(
            last.and_then(|value| value.fan_in_blocker.as_ref()),
            current_fan_in_blocker,
        );
        if !changed_fields.is_empty() {
            rows.push(serde_json::json!({
                "kind": "watch_detail_summary",
                "thread_id": thread_id,
                "summary_type": "fan_in_dependency_blocker",
                "payload": current_fan_in_blocker,
                "changed_fields": changed_fields,
            }));
        }
    } else if last.and_then(|value| value.fan_in_blocker.as_ref()).is_some() {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_in_dependency_blocker",
            "cleared": true,
            "changed_fields": ["cleared"],
        }));
    }

    if let Some(current_fan_in_diagnostics) = current.fan_in_diagnostics.as_ref() {
        let changed_fields = fan_in_result_diagnostics_changed_fields(
            last.and_then(|value| value.fan_in_diagnostics.as_ref()),
            current_fan_in_diagnostics,
        );
        if !changed_fields.is_empty() {
            rows.push(serde_json::json!({
                "kind": "watch_detail_summary",
                "thread_id": thread_id,
                "summary_type": "fan_in_result_diagnostics",
                "payload": current_fan_in_diagnostics,
                "changed_fields": changed_fields,
            }));
        }
    } else if last.and_then(|value| value.fan_in_diagnostics.as_ref()).is_some() {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_in_result_diagnostics",
            "cleared": true,
            "changed_fields": ["cleared"],
        }));
    }

    if let Some(current_subagent_pending) = current.subagent_pending.as_ref() {
        let changed_fields = subagent_pending_changed_fields(
            last.and_then(|value| value.subagent_pending.as_ref()),
            current_subagent_pending,
        );
        if !changed_fields.is_empty() {
            rows.push(serde_json::json!({
                "kind": "watch_detail_summary",
                "thread_id": thread_id,
                "summary_type": "subagent_pending",
                "payload": current_subagent_pending,
                "changed_fields": changed_fields,
            }));
        }
    } else if last.and_then(|value| value.subagent_pending.as_ref()).is_some() {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "subagent_pending",
            "cleared": true,
            "changed_fields": ["cleared"],
        }));
    }

    rows
}

fn fan_out_auto_apply_changed_fields(
    last: Option<&FanOutAutoApplyInboxSummary>,
    current: &FanOutAutoApplyInboxSummary,
) -> Vec<&'static str> {
    let mut changed_fields = Vec::new();
    if last.map(|value| value.task_id.as_str()) != Some(current.task_id.as_str()) {
        changed_fields.push("task_id");
    }
    if last.map(|value| value.status.as_str()) != Some(current.status.as_str()) {
        changed_fields.push("status");
    }
    if last.and_then(|value| value.stage.as_deref()) != current.stage.as_deref() {
        changed_fields.push("stage");
    }
    if last.and_then(|value| value.patch_artifact_id.as_deref()) != current.patch_artifact_id.as_deref()
    {
        changed_fields.push("patch_artifact_id");
    }
    if last.and_then(|value| value.recovery_commands) != current.recovery_commands {
        changed_fields.push("recovery_commands");
    }
    if last.and_then(|value| value.recovery_1.as_deref()) != current.recovery_1.as_deref() {
        changed_fields.push("recovery_1");
    }
    if last.and_then(|value| value.error.as_deref()) != current.error.as_deref() {
        changed_fields.push("error");
    }
    changed_fields
}

fn fan_in_dependency_blocker_changed_fields(
    last: Option<&FanInDependencyBlockedInboxSummary>,
    current: &FanInDependencyBlockedInboxSummary,
) -> Vec<&'static str> {
    let mut changed_fields = Vec::new();
    if last.map(|value| value.task_id.as_str()) != Some(current.task_id.as_str()) {
        changed_fields.push("task_id");
    }
    if last.map(|value| value.status.as_str()) != Some(current.status.as_str()) {
        changed_fields.push("status");
    }
    if last.map(|value| value.dependency_blocked_count) != Some(current.dependency_blocked_count) {
        changed_fields.push("dependency_blocked_count");
    }
    if last.map(|value| value.task_count) != Some(current.task_count) {
        changed_fields.push("task_count");
    }
    if last.map(|value| value.dependency_blocked_ratio.to_bits())
        != Some(current.dependency_blocked_ratio.to_bits())
    {
        changed_fields.push("dependency_blocked_ratio");
    }
    if last.and_then(|value| value.blocker_task_id.as_deref()) != current.blocker_task_id.as_deref() {
        changed_fields.push("blocker_task_id");
    }
    if last.and_then(|value| value.blocker_status.as_deref()) != current.blocker_status.as_deref() {
        changed_fields.push("blocker_status");
    }
    if last.and_then(|value| value.reason.as_deref()) != current.reason.as_deref() {
        changed_fields.push("reason");
    }
    if last.and_then(|value| value.diagnostics_tasks) != current.diagnostics_tasks {
        changed_fields.push("diagnostics_tasks");
    }
    if last.and_then(|value| value.diagnostics_matched_completion_total)
        != current.diagnostics_matched_completion_total
    {
        changed_fields.push("diagnostics_matched_completion_total");
    }
    if last.and_then(|value| value.diagnostics_pending_matching_tool_ids_total)
        != current.diagnostics_pending_matching_tool_ids_total
    {
        changed_fields.push("diagnostics_pending_matching_tool_ids_total");
    }
    if last.and_then(|value| value.diagnostics_scan_last_seq_max)
        != current.diagnostics_scan_last_seq_max
    {
        changed_fields.push("diagnostics_scan_last_seq_max");
    }
    changed_fields
}

fn fan_in_result_diagnostics_changed_fields(
    last: Option<&FanInResultDiagnosticsInboxSummary>,
    current: &FanInResultDiagnosticsInboxSummary,
) -> Vec<&'static str> {
    let mut changed_fields = Vec::new();
    if last.map(|value| value.task_count) != Some(current.task_count) {
        changed_fields.push("task_count");
    }
    if last.map(|value| value.diagnostics_tasks) != Some(current.diagnostics_tasks) {
        changed_fields.push("diagnostics_tasks");
    }
    if last.map(|value| value.diagnostics_matched_completion_total)
        != Some(current.diagnostics_matched_completion_total)
    {
        changed_fields.push("diagnostics_matched_completion_total");
    }
    if last.map(|value| value.diagnostics_pending_matching_tool_ids_total)
        != Some(current.diagnostics_pending_matching_tool_ids_total)
    {
        changed_fields.push("diagnostics_pending_matching_tool_ids_total");
    }
    if last.map(|value| value.diagnostics_scan_last_seq_max)
        != Some(current.diagnostics_scan_last_seq_max)
    {
        changed_fields.push("diagnostics_scan_last_seq_max");
    }
    changed_fields
}

fn subagent_pending_changed_fields(
    last: Option<&SubagentPendingApprovalsSummary>,
    current: &SubagentPendingApprovalsSummary,
) -> Vec<&'static str> {
    let mut changed_fields = Vec::new();
    if last.map(|value| value.total) != Some(current.total) {
        changed_fields.push("total");
    }
    if last.map(|value| &value.states) != Some(&current.states) {
        changed_fields.push("states");
    }
    changed_fields
}

#[cfg(test)]
fn is_fan_out_auto_apply_error(summary: &FanOutAutoApplyInboxSummary) -> bool {
    summary.status == "error"
}

fn format_pending_approval_preview(
    pending: &omne_app_server_protocol::ThreadAttentionPendingApproval,
) -> String {
    let mut preview = format!(
        "{}:{}",
        pending.approval_id,
        approval_action_label_from_parts(pending.action_id, pending.action.as_deref())
    );
    if let Some(summary) = pending.summary.as_ref() {
        if let Some(subagent_link) = approval_subagent_link_from_summary(summary) {
            preview.push_str(&format!(" ({subagent_link})"));
        }
        if let Some(context_hint) = approval_summary_context_hint_from_summary(summary) {
            let context_hint = shorten_watch_approval_hint(context_hint, 48);
            preview.push_str(&format!(" ({context_hint})"));
        }
        if let Some(approve_cmd) = summary.approve_cmd.as_deref().filter(|v| !v.is_empty()) {
            let hint = shorten_watch_approval_hint(format!("approve_cmd={approve_cmd}"), 64);
            preview.push_str(&format!(" ({hint})"));
            if let Some(deny_cmd) = approval_deny_cmd_from_summary(summary) {
                let hint = shorten_watch_approval_hint(format!("deny_cmd={deny_cmd}"), 64);
                preview.push_str(&format!(" ({hint})"));
            }
        }
    }
    preview
}

fn format_pending_approval_commands(
    pending: &omne_app_server_protocol::ThreadAttentionPendingApproval,
) -> Option<String> {
    let summary = pending.summary.as_ref()?;
    let approve_cmd = approval_approve_cmd_from_summary(summary)?;
    let deny_cmd = approval_deny_cmd_from_summary(summary);
    let approve_cmd = shorten_path(&approve_cmd, 96);
    if let Some(deny_cmd) = deny_cmd {
        let deny_cmd = shorten_path(&deny_cmd, 96);
        Some(format!(
            "{}: approve_cmd={} deny_cmd={}",
            pending.approval_id, approve_cmd, deny_cmd
        ))
    } else {
        Some(format!(
            "{}: approve_cmd={}",
            pending.approval_id, approve_cmd
        ))
    }
}

fn shorten_watch_approval_hint(hint: String, max_len: usize) -> String {
    let Some((key, value)) = hint.split_once('=') else {
        return hint;
    };
    match key {
        "path" | "requirement" | "argv" | "cwd" | "approve_cmd" | "deny_cmd" => {
            format!("{key}={}", shorten_path(value, max_len))
        }
        _ => hint,
    }
}

fn shorten_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }
    let keep = max_len.saturating_sub(3);
    let tail = path.chars().rev().take(keep).collect::<String>();
    format!("...{}", tail.chars().rev().collect::<String>())
}

fn maybe_bell_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    state: &str,
    debounce_ms: u64,
    last_state: &mut Option<String>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let now = Instant::now();
    let debounced = last_state.as_deref().is_some_and(|s| s == state)
        && last_bell_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));

    if !debounced {
        eprintln!("attention: {thread_id} -> {state}");
        bell_notifier.notify_attention_state(format!("attention: {thread_id} -> {state}"), state);
        *last_bell_at = Some(now);
    }

    *last_state = Some(state.to_string());
    Ok(())
}

fn attention_state_update(event: &ThreadEvent) -> Option<&'static str> {
    match &event.kind {
        omne_protocol::ThreadEventKind::ApprovalRequested { .. } => Some("need_approval"),
        omne_protocol::ThreadEventKind::TurnStarted { .. } => Some("running"),
        omne_protocol::ThreadEventKind::AttentionMarkerSet { marker, .. } => match marker {
            omne_protocol::AttentionMarkerKind::FanOutLinkageIssue
            | omne_protocol::AttentionMarkerKind::FanOutAutoApplyError => Some("failed"),
            omne_protocol::AttentionMarkerKind::TokenBudgetWarning => Some("token_budget_warning"),
            omne_protocol::AttentionMarkerKind::TokenBudgetExceeded => Some("token_budget_exceeded"),
            _ => None,
        },
        omne_protocol::ThreadEventKind::TurnCompleted { status, .. } => match status {
            TurnStatus::Completed => Some("done"),
            TurnStatus::Interrupted => Some("interrupted"),
            TurnStatus::Failed => Some("failed"),
            TurnStatus::Cancelled => Some("cancelled"),
            TurnStatus::Stuck => Some("stuck"),
        },
        omne_protocol::ThreadEventKind::ProcessStarted { .. } => Some("running"),
        omne_protocol::ThreadEventKind::ProcessExited { exit_code, .. } => match exit_code {
            Some(code) if *code != 0 => Some("failed"),
            _ => None,
        },
        _ => None,
    }
}

fn should_refresh_watch_detail_summary(events: &[ThreadEvent]) -> bool {
    should_refresh_watch_auto_apply_summary(events)
        || should_refresh_watch_fan_in_dependency_blocker_summary(events)
        || should_refresh_watch_fan_in_result_diagnostics_summary(events)
        || should_refresh_watch_subagent_pending_summary(events)
}

fn should_refresh_watch_auto_apply_summary(events: &[ThreadEvent]) -> bool {
    events.iter().any(watch_auto_apply_summary_maybe_changed_by_event)
}

fn watch_auto_apply_summary_maybe_changed_by_event(event: &ThreadEvent) -> bool {
    match &event.kind {
        omne_protocol::ThreadEventKind::TurnStarted { .. }
        | omne_protocol::ThreadEventKind::TurnCompleted { .. }
        | omne_protocol::ThreadEventKind::ToolCompleted { .. } => true,
        omne_protocol::ThreadEventKind::AttentionMarkerSet { marker, .. }
        | omne_protocol::ThreadEventKind::AttentionMarkerCleared { marker, .. } => {
            matches!(
                marker,
                omne_protocol::AttentionMarkerKind::FanOutLinkageIssue
                    | omne_protocol::AttentionMarkerKind::FanOutAutoApplyError
            )
        }
        _ => false,
    }
}

fn should_refresh_watch_fan_in_dependency_blocker_summary(events: &[ThreadEvent]) -> bool {
    events.iter().any(watch_fan_in_dependency_blocker_maybe_changed_by_event)
}

fn watch_fan_in_dependency_blocker_maybe_changed_by_event(event: &ThreadEvent) -> bool {
    matches!(
        &event.kind,
        omne_protocol::ThreadEventKind::TurnStarted { .. }
            | omne_protocol::ThreadEventKind::TurnCompleted { .. }
            | omne_protocol::ThreadEventKind::ToolCompleted { .. }
    )
}

fn should_refresh_watch_fan_in_result_diagnostics_summary(events: &[ThreadEvent]) -> bool {
    events
        .iter()
        .any(watch_fan_in_result_diagnostics_maybe_changed_by_event)
}

fn watch_fan_in_result_diagnostics_maybe_changed_by_event(event: &ThreadEvent) -> bool {
    watch_fan_in_dependency_blocker_maybe_changed_by_event(event)
}

fn should_refresh_watch_subagent_pending_summary(events: &[ThreadEvent]) -> bool {
    events
        .iter()
        .any(watch_subagent_pending_summary_maybe_changed_by_event)
}

fn watch_subagent_pending_summary_maybe_changed_by_event(event: &ThreadEvent) -> bool {
    matches!(
        &event.kind,
        omne_protocol::ThreadEventKind::ApprovalRequested { .. }
            | omne_protocol::ThreadEventKind::ApprovalDecided { .. }
            | omne_protocol::ThreadEventKind::TurnStarted { .. }
            | omne_protocol::ThreadEventKind::TurnCompleted { .. }
    )
}

fn maybe_bell(
    bell_notifier: &BellNotifier,
    state: &'static str,
    debounce_ms: u64,
    last_state: &mut Option<&'static str>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let should_notify = matches!(state, "need_approval" | "failed" | "stuck");
    if !should_notify {
        *last_state = Some(state);
        return Ok(());
    }

    let now = Instant::now();
    let debounced = last_state.is_some_and(|s| s == state)
        && last_bell_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));

    if !debounced {
        bell_notifier.notify_attention_state(format!("attention -> {state}"), state);
        *last_bell_at = Some(now);
    }

    *last_state = Some(state);
    Ok(())
}

fn maybe_bell_stale(
    bell_notifier: &BellNotifier,
    stale_present: bool,
    debounce_ms: u64,
    last_stale_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(stale_present, debounce_ms, last_stale_present, last_bell_at) {
        bell_notifier.notify_stale_process("attention -> stale_process".to_string());
    }
    Ok(())
}

fn should_emit_presence_bell(
    present: bool,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> bool {
    if last_present.is_none() {
        *last_present = Some(present);
        return false;
    }

    let mut should_emit = false;
    if should_notify_presence_rising_edge(*last_present, present) {
        let now = Instant::now();
        let debounced = last_bell_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));
        if !debounced {
            *last_bell_at = Some(now);
            should_emit = true;
        }
    }

    *last_present = Some(present);
    should_emit
}

fn maybe_bell_stale_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    stale_present: bool,
    debounce_ms: u64,
    last_stale_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(stale_present, debounce_ms, last_stale_present, last_bell_at) {
        eprintln!("attention: {thread_id} -> stale_process");
        bell_notifier.notify_stale_process(format!("attention: {thread_id} -> stale_process"));
    }
    Ok(())
}

fn maybe_bell_linkage_issue_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    linkage_issue_present: bool,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(linkage_issue_present, debounce_ms, last_present, last_bell_at) {
        eprintln!("attention: {thread_id} -> fan_out_linkage_issue");
        bell_notifier.notify_attention_state(
            format!("attention: {thread_id} -> fan_out_linkage_issue"),
            "fan_out_linkage_issue",
        );
    }
    Ok(())
}

fn maybe_bell_auto_apply_error_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    auto_apply_error_present: bool,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(
        auto_apply_error_present,
        debounce_ms,
        last_present,
        last_bell_at,
    ) {
        eprintln!("attention: {thread_id} -> fan_out_auto_apply_error");
        bell_notifier.notify_attention_state(
            format!("attention: {thread_id} -> fan_out_auto_apply_error"),
            "fan_out_auto_apply_error",
        );
    }
    Ok(())
}

fn maybe_bell_fan_in_dependency_blocked_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    fan_in_dependency_blocked_present: bool,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(
        fan_in_dependency_blocked_present,
        debounce_ms,
        last_present,
        last_bell_at,
    ) {
        eprintln!("attention: {thread_id} -> fan_in_dependency_blocked");
        bell_notifier.notify_attention_state(
            format!("attention: {thread_id} -> fan_in_dependency_blocked"),
            "fan_in_dependency_blocked",
        );
    }
    Ok(())
}

fn maybe_bell_fan_in_result_diagnostics_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    fan_in_result_diagnostics_present: bool,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(
        fan_in_result_diagnostics_present,
        debounce_ms,
        last_present,
        last_bell_at,
    ) {
        eprintln!("attention: {thread_id} -> fan_in_result_diagnostics");
        bell_notifier.notify_attention_state(
            format!("attention: {thread_id} -> fan_in_result_diagnostics"),
            "fan_in_result_diagnostics",
        );
    }
    Ok(())
}

fn maybe_bell_token_budget_exceeded_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    token_budget_exceeded_present: bool,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(
        token_budget_exceeded_present,
        debounce_ms,
        last_present,
        last_bell_at,
    ) {
        eprintln!("attention: {thread_id} -> token_budget_exceeded");
        bell_notifier.notify_attention_state(
            format!("attention: {thread_id} -> token_budget_exceeded"),
            "token_budget_exceeded",
        );
    }
    Ok(())
}

fn maybe_bell_token_budget_warning_per_thread(
    bell_notifier: &BellNotifier,
    thread_id: &ThreadId,
    token_budget_warning_present: bool,
    warning_threshold_label: &str,
    debounce_ms: u64,
    last_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if should_emit_presence_bell(
        token_budget_warning_present,
        debounce_ms,
        last_present,
        last_bell_at,
    ) {
        eprintln!(
            "attention: {thread_id} -> token_budget_warning(threshold={warning_threshold_label})"
        );
        bell_notifier.notify_attention_state(
            format!(
                "attention: {thread_id} -> token_budget_warning(threshold={warning_threshold_label})"
            ),
            "token_budget_warning",
        );
    }
    Ok(())
}

fn should_notify_presence_rising_edge(last_present: Option<bool>, present: bool) -> bool {
    present && last_present == Some(false)
}

#[cfg(test)]
mod watch_inbox_tests {
    use super::*;

    fn event_with_kind(kind: omne_protocol::ThreadEventKind) -> ThreadEvent {
        ThreadEvent {
            seq: omne_protocol::EventSeq::ZERO,
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id: ThreadId::new(),
            kind,
        }
    }

    fn test_thread_meta(
        has_fan_out_linkage_issue: bool,
        has_fan_out_auto_apply_error: bool,
        has_fan_in_dependency_blocked: bool,
    ) -> ThreadMeta {
        ThreadMeta {
            thread_id: ThreadId::new(),
            cwd: "/tmp".to_string(),
            archived: false,
            archived_at: None,
            archived_reason: None,
            approval_policy: ApprovalPolicy::OnRequest,
            sandbox_policy: SandboxPolicy::WorkspaceWrite,
            model: None,
            openai_base_url: None,
            last_seq: 0,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
            last_turn_id: None,
            last_turn_status: None,
            last_turn_reason: None,
            token_budget_limit: None,
            token_budget_remaining: None,
            token_budget_utilization: None,
            token_budget_exceeded: None,
            token_budget_warning_active: None,
            attention_state: "running".to_string(),
            has_fan_out_linkage_issue,
            has_fan_out_auto_apply_error,
            fan_out_auto_apply: None,
            has_fan_in_dependency_blocked,
            fan_in_dependency_blocker: None,
            has_fan_in_result_diagnostics: false,
            fan_in_result_diagnostics: None,
            pending_subagent_proxy_approvals: 0,
        }
    }

    fn test_thread_attention() -> ThreadAttention {
        ThreadAttention {
            thread_id: ThreadId::new(),
            cwd: Some("/tmp/ws".to_string()),
            archived: false,
            archived_at: None,
            archived_reason: None,
            paused: false,
            paused_at: None,
            paused_reason: None,
            failed_processes: Vec::new(),
            approval_policy: ApprovalPolicy::OnRequest,
            sandbox_policy: SandboxPolicy::WorkspaceWrite,
            model: Some("gpt-5".to_string()),
            openai_base_url: None,
            last_seq: 0,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
            last_turn_id: None,
            last_turn_status: None,
            last_turn_reason: None,
            token_budget_limit: None,
            token_budget_remaining: None,
            token_budget_utilization: None,
            token_budget_exceeded: None,
            token_budget_warning_active: None,
            attention_state: "running".to_string(),
            pending_approvals: Vec::new(),
            running_processes: Vec::new(),
            stale_processes: Vec::new(),
            attention_markers: omne_app_server_protocol::ThreadAttentionMarkers {
                plan_ready: None,
                diff_ready: None,
                fan_out_linkage_issue: None,
                fan_out_auto_apply_error: None,
                test_failed: None,
                token_budget_warning: None,
                token_budget_exceeded: None,
            },
            has_plan_ready: false,
            has_diff_ready: false,
            has_fan_out_linkage_issue: false,
            has_fan_out_auto_apply_error: false,
            fan_out_auto_apply: None,
            has_fan_in_dependency_blocked: false,
            fan_in_dependency_blocker: None,
            has_fan_in_result_diagnostics: false,
            fan_in_result_diagnostics: None,
            has_test_failed: false,
        }
    }

    fn test_subagent_pending_approval(
        approve_cmd: &str,
    ) -> omne_app_server_protocol::ThreadAttentionPendingApproval {
        omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some(approve_cmd.to_string()),
                deny_cmd: None,
            }),
            requested_at: None,
        }
    }

    fn marker_set_event(marker: omne_protocol::AttentionMarkerKind) -> ThreadEvent {
        ThreadEvent {
            seq: omne_protocol::EventSeq::ZERO,
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id: ThreadId::new(),
            kind: omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker,
                turn_id: None,
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            },
        }
    }

    #[test]
    fn attention_state_update_marks_linkage_issue_marker_as_failed() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::FanOutLinkageIssue);
        assert_eq!(attention_state_update(&event), Some("failed"));
    }

    #[test]
    fn attention_state_update_marks_auto_apply_error_marker_as_failed() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::FanOutAutoApplyError);
        assert_eq!(attention_state_update(&event), Some("failed"));
    }

    #[test]
    fn attention_state_update_marks_token_budget_warning_marker() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::TokenBudgetWarning);
        assert_eq!(attention_state_update(&event), Some("token_budget_warning"));
    }

    #[test]
    fn attention_state_update_marks_token_budget_exceeded_marker() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::TokenBudgetExceeded);
        assert_eq!(attention_state_update(&event), Some("token_budget_exceeded"));
    }

    #[test]
    fn format_thread_row_renders_expected_shape() {
        let turn_id = TurnId::new();
        let mut thread = test_thread_meta(false, false, false);
        thread.thread_id = ThreadId::new();
        thread.attention_state = "failed".to_string();
        thread.last_seq = 42;
        thread.last_turn_id = Some(turn_id);
        thread.model = Some("gpt-5".to_string());
        thread.cwd = "/tmp/repo".to_string();

        let row = format_thread_row(&thread);
        assert_eq!(
            row,
            format!(
                "{}  state=failed  seq=42  turn={}  model=gpt-5  cwd=/tmp/repo",
                thread.thread_id, turn_id
            )
        );
    }

    #[test]
    fn format_thread_row_shortens_long_cwd() {
        let mut thread = test_thread_meta(false, false, false);
        thread.thread_id = ThreadId::new();
        thread.cwd = format!("/tmp/{}", "a".repeat(120));

        let row = format_thread_row(&thread);
        let cwd = row
            .split("  cwd=")
            .nth(1)
            .expect("thread row should contain cwd");
        assert!(cwd.starts_with("..."));
        assert!(cwd.len() <= 60);
    }

    #[test]
    fn format_thread_detail_lines_renders_expected_shape() {
        let mut att = test_thread_attention();
        let running_process_id = ProcessId::new();
        let failed_process_id = ProcessId::new();
        let stale_process_id = ProcessId::new();
        att.has_plan_ready = true;
        att.has_test_failed = true;
        att.token_budget_limit = Some(200);
        att.token_budget_remaining = Some(10);
        att.token_budget_utilization = Some(0.95);
        att.token_budget_exceeded = Some(false);
        att.token_budget_warning_active = Some(true);
        att.running_processes = vec![omne_app_server_protocol::ThreadAttentionRunningProcess {
            process_id: running_process_id,
            argv: vec!["cargo".to_string(), "test".to_string()],
            status: Some("running".to_string()),
        }];
        att.failed_processes = vec![failed_process_id];
        att.stale_processes = vec![omne_app_server_protocol::ThreadAttentionStaleProcess {
            process_id: stale_process_id,
            idle_seconds: 120,
            last_update_at: "2026-01-01T00:00:00Z".to_string(),
            stdout_path: "/tmp/stdout.log".to_string(),
            stderr_path: "/tmp/stderr.log".to_string(),
        }];

        let lines = format_thread_detail_lines(&att, 0.9);
        assert_eq!(
            lines,
            vec![
                "markers: plan_ready, token_budget_warning, test_failed".to_string(),
                "token_budget: remaining=10 limit=200 utilization=95.0% exceeded=false".to_string(),
                format!("processes: 1 ({running_process_id})"),
                format!("failed_processes: 1 ({failed_process_id})"),
                format!("stale_processes: 1 ({stale_process_id})"),
            ]
        );
    }

    #[test]
    fn format_thread_detail_lines_truncates_approval_sections_after_three_items() {
        let mut att = test_thread_attention();
        let pending_approvals = vec![
            test_subagent_pending_approval("x --approve"),
            test_subagent_pending_approval("x --approve"),
            test_subagent_pending_approval("x --approve"),
            test_subagent_pending_approval("x --approve"),
        ];
        let ids = pending_approvals
            .iter()
            .take(3)
            .map(|pending| pending.approval_id.to_string())
            .collect::<Vec<_>>();
        att.pending_approvals = pending_approvals;

        let lines = format_thread_detail_lines(&att, 0.9);
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            format!("approvals: 4 ({}, {}, {}, ...)", ids[0], ids[1], ids[2])
        );
        assert_eq!(
            lines[1],
            format!(
                "approval_details: {}:subagent/proxy_approval (approve_cmd=x --approve) (deny_cmd=x --deny); {}:subagent/proxy_approval (approve_cmd=x --approve) (deny_cmd=x --deny); {}:subagent/proxy_approval (approve_cmd=x --approve) (deny_cmd=x --deny); ...",
                ids[0], ids[1], ids[2]
            )
        );
        assert_eq!(
            lines[2],
            format!(
                "approval_commands: {}: approve_cmd=x --approve deny_cmd=x --deny; {}: approve_cmd=x --approve deny_cmd=x --deny; {}: approve_cmd=x --approve deny_cmd=x --deny; ...",
                ids[0], ids[1], ids[2]
            )
        );
        assert_eq!(lines[3], "subagent_pending: total=4 states=unknown:4");
    }

    #[test]
    fn attention_state_update_ignores_marker_clear_events() {
        let event = event_with_kind(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: None,
                reason: Some("new turn started".to_string()),
            });
        assert_eq!(attention_state_update(&event), None);
    }

    #[test]
    fn should_refresh_watch_detail_summary_true_for_tool_completed() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id: omne_protocol::ToolId::new(),
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: None,
        })];
        assert!(should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_detail_summary_true_for_attention_marker_set() {
        let events = vec![marker_set_event(
            omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
        )];
        assert!(should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_detail_summary_false_for_assistant_only_batch() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "ok".to_string(),
            model: None,
            response_id: None,
            token_usage: None,
        })];
        assert!(!should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_auto_apply_summary_true_for_auto_apply_marker_clear() {
        let events = vec![event_with_kind(
            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: None,
                reason: None,
            },
        )];
        assert!(should_refresh_watch_auto_apply_summary(&events));
    }

    #[test]
    fn should_refresh_watch_fan_in_summary_false_for_marker_only_batch() {
        let events = vec![marker_set_event(
            omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
        )];
        assert!(!should_refresh_watch_fan_in_dependency_blocker_summary(&events));
    }

    #[test]
    fn should_refresh_watch_fan_in_summary_true_for_turn_completed() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: TurnId::new(),
            status: TurnStatus::Completed,
            reason: None,
        })];
        assert!(should_refresh_watch_fan_in_dependency_blocker_summary(&events));
    }

    #[test]
    fn should_refresh_watch_subagent_pending_summary_true_for_approval_requested() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: "subagent/proxy_approval".to_string(),
            params: serde_json::json!({}),
        })];
        assert!(should_refresh_watch_subagent_pending_summary(&events));
        assert!(should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_subagent_pending_summary_true_for_turn_completed() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: TurnId::new(),
            status: TurnStatus::Completed,
            reason: None,
        })];
        assert!(should_refresh_watch_subagent_pending_summary(&events));
    }

    #[test]
    fn apply_inbox_filters_only_fan_out_linkage_issue_keeps_marked_threads() {
        let t1 = test_thread_meta(true, false, false);
        let t2 = test_thread_meta(false, false, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, true, false, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_without_marker_filter_keeps_all_threads() {
        let t1 = test_thread_meta(true, false, false);
        let t2 = test_thread_meta(false, true, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1);
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn inbox_thread_changed_true_when_prev_missing() {
        let current = test_thread_meta(false, false, false);
        assert!(inbox_thread_changed(None, &current));
    }

    #[test]
    fn inbox_thread_changed_false_when_seq_and_state_unchanged() {
        let current = test_thread_meta(false, false, false);
        let previous = current.clone();
        assert!(!inbox_thread_changed(Some(&previous), &current));
    }

    #[test]
    fn inbox_thread_changed_true_when_seq_or_state_changes() {
        let current = test_thread_meta(false, false, false);

        let mut previous_seq = current.clone();
        previous_seq.last_seq = current.last_seq.saturating_add(1);
        assert!(inbox_thread_changed(Some(&previous_seq), &current));

        let mut previous_state = current.clone();
        previous_state.attention_state = "failed".to_string();
        assert!(inbox_thread_changed(Some(&previous_state), &current));
    }

    #[test]
    fn apply_inbox_filters_only_fan_out_auto_apply_error_keeps_marked_threads() {
        let t1 = test_thread_meta(false, true, false);
        let t2 = test_thread_meta(false, false, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, true, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_with_both_marker_filters_requires_both_markers() {
        let t1 = test_thread_meta(true, true, false);
        let t2 = test_thread_meta(true, false, false);
        let t3 = test_thread_meta(false, true, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3);

        let filtered = apply_inbox_filters(threads, true, true, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_fan_in_dependency_blocked_keeps_marked_threads() {
        let t1 = test_thread_meta(false, false, true);
        let t2 = test_thread_meta(false, false, false);
        let t3 = test_thread_meta(false, false, true);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, true, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_subagent_proxy_approval_keeps_marked_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.pending_subagent_proxy_approvals = 1;
        let t2 = test_thread_meta(false, false, false);
        let mut t3 = test_thread_meta(false, false, false);
        t3.pending_subagent_proxy_approvals = 2;
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, false, 0.9, true);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_fan_in_result_diagnostics_keeps_marked_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.has_fan_in_result_diagnostics = true;
        let t2 = test_thread_meta(false, false, false);
        let mut t3 = test_thread_meta(false, false, false);
        t3.has_fan_in_result_diagnostics = true;
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, false, true, false, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_token_budget_exceeded_keeps_marked_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_exceeded = Some(true);
        let t2 = test_thread_meta(false, false, false);
        let mut t3 = test_thread_meta(false, false, false);
        t3.token_budget_exceeded = Some(true);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, false, false, true, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_token_budget_warning_keeps_warning_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_utilization = Some(0.95);
        t1.token_budget_exceeded = Some(false);

        let mut t2 = test_thread_meta(false, false, false);
        t2.token_budget_limit = Some(200);
        t2.token_budget_utilization = Some(0.89);
        t2.token_budget_exceeded = Some(false);

        let mut t3 = test_thread_meta(false, false, false);
        t3.token_budget_limit = Some(200);
        t3.token_budget_utilization = Some(0.97);
        t3.token_budget_exceeded = Some(true);

        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3);

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, true, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_token_budget_warning_prefers_server_flag() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_utilization = Some(0.10);
        t1.token_budget_exceeded = Some(false);
        t1.token_budget_warning_active = Some(true);

        let mut t2 = test_thread_meta(false, false, false);
        t2.token_budget_limit = Some(200);
        t2.token_budget_utilization = Some(0.95);
        t2.token_budget_exceeded = Some(false);
        t2.token_budget_warning_active = Some(false);

        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, true, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_token_budget_exceeded_and_warning_intersection_is_empty() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_utilization = Some(0.95);
        t1.token_budget_exceeded = Some(false);

        let mut t2 = test_thread_meta(false, false, false);
        t2.token_budget_limit = Some(200);
        t2.token_budget_utilization = Some(0.95);
        t2.token_budget_exceeded = Some(true);

        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1);
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, false, false, false, true, true, 0.9, false);
        assert!(filtered.is_empty());
    }

    #[test]
    fn should_notify_presence_rising_edge_only_on_false_to_true() {
        assert!(!should_notify_presence_rising_edge(None, true));
        assert!(!should_notify_presence_rising_edge(None, false));
        assert!(!should_notify_presence_rising_edge(Some(true), true));
        assert!(!should_notify_presence_rising_edge(Some(true), false));
        assert!(!should_notify_presence_rising_edge(Some(false), false));
        assert!(should_notify_presence_rising_edge(Some(false), true));
    }

    #[test]
    fn attention_state_severity_maps_expected_levels() {
        assert_eq!(attention_state_severity("failed"), notify_kit::Severity::Error);
        assert_eq!(
            attention_state_severity("fan_out_auto_apply_error"),
            notify_kit::Severity::Error
        );
        assert_eq!(
            attention_state_severity("fan_out_linkage_issue"),
            notify_kit::Severity::Warning
        );
        assert_eq!(
            attention_state_severity("fan_in_dependency_blocked"),
            notify_kit::Severity::Warning
        );
        assert_eq!(
            attention_state_severity("fan_in_result_diagnostics"),
            notify_kit::Severity::Warning
        );
        assert_eq!(
            attention_state_severity("token_budget_warning"),
            notify_kit::Severity::Warning
        );
        assert_eq!(attention_state_severity("running"), notify_kit::Severity::Info);
    }

    #[test]
    fn attention_detail_markers_include_fan_in_statuses() {
        let markers =
            attention_detail_markers(false, false, false, false, true, true, false, false, false);
        assert_eq!(
            markers,
            vec!["fan_in_dependency_blocked", "fan_in_result_diagnostics"]
        );
    }

    #[test]
    fn attention_detail_markers_include_token_budget_statuses() {
        let markers =
            attention_detail_markers(false, false, false, false, false, false, true, true, false);
        assert_eq!(
            markers,
            vec!["token_budget_exceeded", "token_budget_warning"]
        );
    }

    #[test]
    fn attention_detail_markers_preserve_display_order() {
        let markers = attention_detail_markers(true, true, true, true, true, true, true, true, true);
        assert_eq!(
            markers,
            vec![
                "plan_ready",
                "diff_ready",
                "fan_out_linkage_issue",
                "fan_out_auto_apply_error",
                "fan_in_dependency_blocked",
                "fan_in_result_diagnostics",
                "token_budget_exceeded",
                "token_budget_warning",
                "test_failed",
            ]
        );
    }

    #[test]
    fn should_emit_presence_bell_tracks_initial_value_without_notifying() {
        let mut last_present = None;
        let mut last_bell_at = None;
        assert!(!should_emit_presence_bell(
            true,
            1000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
        assert!(last_bell_at.is_none());
    }

    #[test]
    fn should_emit_presence_bell_notifies_on_rising_edge() {
        let mut last_present = Some(false);
        let mut last_bell_at = None;
        assert!(should_emit_presence_bell(
            true,
            1000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
        assert!(last_bell_at.is_some());
    }

    #[test]
    fn should_emit_presence_bell_respects_debounce_window() {
        let mut last_present = Some(false);
        let mut last_bell_at = Some(Instant::now());
        assert!(!should_emit_presence_bell(
            true,
            60_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
    }

    #[test]
    fn should_emit_presence_bell_does_not_notify_on_falling_edge() {
        let mut last_present = Some(true);
        let mut last_bell_at = Some(Instant::now());
        assert!(!should_emit_presence_bell(
            false,
            1_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(false));
    }

    #[test]
    fn should_emit_presence_bell_notifies_on_next_rising_edge_after_fall() {
        let mut last_present = Some(true);
        let mut last_bell_at = None;
        assert!(!should_emit_presence_bell(
            false,
            1_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert!(should_emit_presence_bell(
            true,
            1_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
        assert!(last_bell_at.is_some());
    }

    #[test]
    fn should_emit_presence_bell_debounces_rising_edge_after_fall_if_recently_notified() {
        let mut last_present = Some(true);
        let mut last_bell_at = Some(Instant::now());
        assert!(!should_emit_presence_bell(
            false,
            60_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert!(!should_emit_presence_bell(
            true,
            60_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
    }

    #[test]
    fn token_budget_warning_present_only_triggers_near_limit_without_exceeded() {
        assert!(!token_budget_warning_present(None, Some(0.95), Some(false), 0.9));
        assert!(!token_budget_warning_present(Some(200), Some(0.95), Some(true), 0.9));
        assert!(!token_budget_warning_present(Some(200), None, Some(false), 0.9));
        assert!(!token_budget_warning_present(Some(200), Some(0.89), Some(false), 0.9));
        assert!(token_budget_warning_present(
            Some(200),
            Some(0.90),
            Some(false),
            0.9
        ));
        assert!(token_budget_warning_present(
            Some(200),
            Some(0.95),
            Some(false),
            0.9
        ));
    }

    #[test]
    fn format_token_budget_snapshot_omits_when_limit_absent() {
        let line = format_token_budget_snapshot(None, Some(0), Some(1.0), Some(true));
        assert!(line.is_none());
    }

    #[test]
    fn format_token_budget_snapshot_formats_all_fields() {
        let line = format_token_budget_snapshot(Some(200), Some(0), Some(1.25), Some(true))
            .expect("token budget line");
        assert_eq!(
            line,
            "token_budget: remaining=0 limit=200 utilization=125.0% exceeded=true"
        );
    }

    #[test]
    fn format_token_budget_snapshot_uses_defaults_for_missing_fields() {
        let line = format_token_budget_snapshot(Some(200), None, None, None).expect("token budget line");
        assert_eq!(
            line,
            "token_budget: remaining=- limit=200 utilization=- exceeded=false"
        );
    }

    #[test]
    fn pending_approval_preview_prefers_action_id_and_summary_hint() {
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("legacy/action".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ArtifactWrite),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: Some("need_approval".to_string()),
                argv: Some(vec!["cargo".to_string(), "test".to_string()]),
                cwd: Some("/tmp/workspace".to_string()),
                process_id: None,
                artifact_type: None,
                path: Some("/tmp/workspace/README.md".to_string()),
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains(":artifact/write"));
        assert!(preview.contains("path=/tmp/workspace/README.md"));
        assert!(!preview.contains("legacy/action"));
    }

    #[test]
    fn pending_approval_preview_falls_back_to_legacy_action() {
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("process/start".to_string()),
            action_id: None,
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: Some(vec!["python".to_string(), "script.py".to_string()]),
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains(":process/start"));
        assert!(preview.contains("argv=python script.py"));
    }

    #[test]
    fn pending_approval_preview_surfaces_subagent_child_ids() {
        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: Some("on_request".to_string()),
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: Some("process/start".to_string()),
                hook: None,
                child_thread_id: Some(child_thread_id),
                child_turn_id: Some(child_turn_id),
                child_approval_id: Some(child_approval_id),
                child_attention_state: Some("running".to_string()),
                child_last_turn_status: Some(TurnStatus::Stuck),
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains(":subagent/proxy_approval"));
        assert!(preview.contains("child_thread_id="));
        assert!(preview.contains(&child_thread_id.to_string()));
        assert!(preview.contains("child_turn_id="));
        assert!(preview.contains(&child_turn_id.to_string()));
        assert!(preview.contains("child_approval_id="));
        assert!(preview.contains(&child_approval_id.to_string()));
        assert!(preview.contains("child_attention_state=running"));
        assert!(preview.contains("child_last_turn_status=stuck"));
        assert!(!preview.contains("subagent="));
    }

    #[test]
    fn pending_approval_preview_includes_path_with_subagent_child_ids() {
        let child_thread_id = ThreadId::new();
        let child_approval_id = ApprovalId::new();
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: Some("/tmp/ws/src/main.rs".to_string()),
                server: None,
                tool: Some("file/write".to_string()),
                hook: None,
                child_thread_id: Some(child_thread_id),
                child_turn_id: None,
                child_approval_id: Some(child_approval_id),
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains("child_thread_id="));
        assert!(preview.contains("child_approval_id="));
        assert!(preview.contains("path=/tmp/ws/src/main.rs"));
    }

    #[test]
    fn approval_summary_display_combines_subagent_ids_and_context_hint() {
        let child_thread_id = ThreadId::new();
        let child_approval_id = ApprovalId::new();
        let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
            requirement: None,
            argv: None,
            cwd: None,
            process_id: None,
            artifact_type: None,
            path: Some("/tmp/ws/src/main.rs".to_string()),
            server: None,
            tool: None,
            hook: None,
            child_thread_id: Some(child_thread_id),
            child_turn_id: None,
            child_approval_id: Some(child_approval_id),
            child_attention_state: None,
            child_last_turn_status: None,
            approve_cmd: None,
            deny_cmd: None,
        };
        let display = approval_summary_display_from_summary(&summary).expect("summary");
        assert!(display.contains("child_thread_id="));
        assert!(display.contains(&child_thread_id.to_string()));
        assert!(display.contains("child_approval_id="));
        assert!(display.contains(&child_approval_id.to_string()));
        assert!(display.contains("path=/tmp/ws/src/main.rs"));
        assert!(!display.contains("subagent="));
    }

    #[test]
    fn pending_approval_preview_includes_approve_cmd_when_present() {
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some(
                    "omne approval decide thread-1 approval-1 --approve".to_string(),
                ),
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains("approve_cmd="));
        assert!(preview.contains("--approve"));
        assert!(preview.contains("deny_cmd="));
        assert!(preview.contains("--deny"));
    }

    #[test]
    fn pending_approval_commands_include_approve_and_deny_cmd() {
        let approval_id = ApprovalId::new();
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id,
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some(
                    "omne approval decide thread-1 approval-1 --approve".to_string(),
                ),
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let commands = format_pending_approval_commands(&pending).expect("commands");
        assert!(commands.contains(&approval_id.to_string()));
        assert!(commands.contains("approve_cmd="));
        assert!(commands.contains("--approve"));
        assert!(commands.contains("deny_cmd="));
        assert!(commands.contains("--deny"));
    }

    #[test]
    fn format_subagent_pending_approvals_summary_reports_state_breakdown() {
        let approvals = vec![
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("subagent/proxy_approval".to_string()),
                action_id: Some(
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                ),
                params: None,
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    requirement: None,
                    argv: None,
                    cwd: None,
                    process_id: None,
                    artifact_type: None,
                    path: None,
                    server: None,
                    tool: None,
                    hook: None,
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: None,
                    child_attention_state: Some("running".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                }),
                requested_at: None,
            },
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("subagent/proxy_approval".to_string()),
                action_id: Some(
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                ),
                params: None,
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    requirement: None,
                    argv: None,
                    cwd: None,
                    process_id: None,
                    artifact_type: None,
                    path: None,
                    server: None,
                    tool: None,
                    hook: None,
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: None,
                    child_attention_state: Some("FAILED".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                }),
                requested_at: None,
            },
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("subagent/proxy_approval".to_string()),
                action_id: None,
                params: None,
                summary: None,
                requested_at: None,
            },
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("process/start".to_string()),
                action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                params: None,
                summary: None,
                requested_at: None,
            },
        ];

        let line = format_subagent_pending_approvals_summary(&approvals).expect("summary");
        assert!(line.starts_with("subagent_pending: total=3 states="));
        assert!(line.contains("running:1"));
        assert!(line.contains("failed:1"));
        assert!(line.contains("unknown:1"));
    }

    #[test]
    fn format_subagent_pending_approvals_summary_skips_non_subagent_items() {
        let approvals = vec![omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("process/start".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
            params: None,
            summary: None,
            requested_at: None,
        }];
        assert!(format_subagent_pending_approvals_summary(&approvals).is_none());
    }

    #[test]
    fn approval_action_label_unknown_id_falls_back_to_raw_action() {
        let label = approval_action_label_from_parts(
            Some(omne_app_server_protocol::ThreadApprovalActionId::Unknown),
            Some("custom/tool"),
        );
        assert_eq!(label, "custom/tool");
    }

    #[test]
    fn approval_summary_from_params_extracts_path_and_requirement() {
        let params = serde_json::json!({
            "approval": { "requirement": "on_request" },
            "target_path": "/tmp/ws/main.rs"
        });
        let summary = approval_summary_from_params(&params).expect("summary");
        assert_eq!(summary.requirement.as_deref(), Some("on_request"));
        assert_eq!(summary.path.as_deref(), Some("/tmp/ws/main.rs"));
    }

    #[test]
    fn approval_summary_from_params_extracts_subagent_proxy_child_request_fields() {
        let params = serde_json::json!({
            "subagent_proxy": {
                "kind": "approval",
                "task_id": "t1"
            },
            "child_request": {
                "action": "process/start",
                "params": {
                    "approval": { "requirement": "prompt_strict" },
                    "argv": ["cargo", "test"],
                    "cwd": "/tmp/ws"
                }
            }
        });
        let summary = approval_summary_from_params(&params).expect("summary");
        assert_eq!(summary.requirement.as_deref(), Some("prompt_strict"));
        assert_eq!(
            summary.argv,
            Some(vec!["cargo".to_string(), "test".to_string()])
        );
        assert_eq!(summary.cwd.as_deref(), Some("/tmp/ws"));
        assert_eq!(summary.tool.as_deref(), Some("process/start"));
    }

    #[test]
    fn approval_summary_from_params_with_context_includes_approve_cmd_for_proxy() {
        let thread_id = ThreadId::new();
        let approval_id = ApprovalId::new();
        let params = serde_json::json!({
            "subagent_proxy": {
                "kind": "approval",
                "task_id": "t1"
            },
            "child_request": {
                "action": "process/start",
                "params": {
                    "approval": { "requirement": "prompt_strict" },
                    "argv": ["cargo", "test"],
                    "cwd": "/tmp/ws"
                }
            }
        });
        let summary = approval_summary_from_params_with_context(
            Some(thread_id),
            Some(approval_id),
            Some("subagent/proxy_approval"),
            &params,
        )
        .expect("summary");
        let expected_cmd = format!("omne approval decide {thread_id} {approval_id} --approve");
        assert_eq!(summary.approve_cmd.as_deref(), Some(expected_cmd.as_str()));
        let expected_deny_cmd = format!("omne approval decide {thread_id} {approval_id} --deny");
        assert_eq!(
            summary.deny_cmd.as_deref(),
            Some(expected_deny_cmd.as_str())
        );
    }

    #[test]
    fn fan_out_auto_apply_summary_reports_error_and_recovery_command_preview() {
        let payload = omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t-auto-apply".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: None,
            isolated_write_patch: None,
            isolated_write_handoff: None,
            isolated_write_auto_apply: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                    enabled: true,
                    attempted: true,
                    applied: false,
                    workspace_cwd: None,
                    target_workspace_cwd: None,
                    check_argv: vec![],
                    apply_argv: vec![],
                    patch_artifact_id: Some("artifact-7".to_string()),
                    patch_read_cmd: None,
                    failure_stage: Some(
                        omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
                    ),
                    recovery_hint: None,
                    recovery_commands: vec![
                        omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData {
                            label: "read_patch_artifact".to_string(),
                            argv: vec![
                                "omne".to_string(),
                                "artifact".to_string(),
                                "read".to_string(),
                                "thread-1".to_string(),
                                "artifact-7".to_string(),
                            ],
                        },
                    ],
                    error: Some("git apply --check failed: patch does not apply".to_string()),
                },
            ),
            status: "completed".to_string(),
            reason: None,
        };

        let summary = fan_out_auto_apply_summary_from_payload(&payload).expect("summary");
        assert_eq!(summary.task_id, "t-auto-apply");
        assert_eq!(summary.status, "error");
        assert_eq!(summary.stage.as_deref(), Some("check_patch"));
        assert_eq!(summary.patch_artifact_id.as_deref(), Some("artifact-7"));
        assert_eq!(summary.recovery_commands, Some(1));
        assert_eq!(
            summary.recovery_1.as_deref(),
            Some("read_patch_artifact: omne artifact read thread-1 artifact-7")
        );

        let text = format_fan_out_auto_apply_summary(&summary);
        assert!(text.contains("task_id=t-auto-apply"));
        assert!(text.contains("status=error"));
        assert!(text.contains("stage=check_patch"));
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_out_auto_apply_when_present() -> anyhow::Result<()> {
        let t1 = test_thread_meta(false, false, false);
        let t2 = test_thread_meta(false, false, false);
        let mut auto_apply_summaries = std::collections::BTreeMap::new();
        auto_apply_summaries.insert(
            t1.thread_id,
            FanOutAutoApplyInboxSummary {
                task_id: "t-auto-apply".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: Some("artifact-7".to_string()),
                recovery_commands: Some(1),
                recovery_1: Some("read_patch_artifact: omne artifact read".to_string()),
                error: Some("git apply --check failed".to_string()),
            },
        );
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1, &t2],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0]["fan_out_auto_apply"]["task_id"].as_str(),
            Some("t-auto-apply")
        );
        assert!(rows[1]["fan_out_auto_apply"].is_null());
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_includes_token_budget_snapshot_fields() -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_remaining = Some(0);
        t1.token_budget_utilization = Some(1.25);
        t1.token_budget_exceeded = Some(true);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["token_budget_limit"].as_u64(), Some(200));
        assert_eq!(rows[0]["token_budget_remaining"].as_u64(), Some(0));
        assert_eq!(rows[0]["token_budget_exceeded"].as_bool(), Some(true));
        assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(false));
        let utilization = rows[0]["token_budget_utilization"]
            .as_f64()
            .expect("token_budget_utilization should be numeric");
        assert!((utilization - 1.25).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_includes_token_budget_warning_active_when_threshold_reached()
    -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_remaining = Some(10);
        t1.token_budget_utilization = Some(1.0);
        t1.token_budget_exceeded = Some(false);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(true));
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_prefers_server_token_budget_warning_active() -> anyhow::Result<()>
    {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_remaining = Some(190);
        t1.token_budget_utilization = Some(0.05);
        t1.token_budget_exceeded = Some(false);
        t1.token_budget_warning_active = Some(true);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(true));
        Ok(())
    }

    #[test]
    fn fan_in_dependency_blocked_summary_reports_blocker_details() {
        let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
            thread_id: "thread-1".to_string(),
            task_count: 2,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            tasks: vec![
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t-upstream".to_string(),
                    title: "upstream".to_string(),
                    thread_id: Some("thread-upstream".to_string()),
                    turn_id: Some("turn-upstream".to_string()),
                    status: "Failed".to_string(),
                    reason: Some("unit tests failed".to_string()),
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t-dependent".to_string(),
                    title: "dependent".to_string(),
                    thread_id: None,
                    turn_id: None,
                    status: "Cancelled".to_string(),
                    reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
                    dependency_blocked: true,
                    dependency_blocker_task_id: Some("t-upstream".to_string()),
                    dependency_blocker_status: Some("Failed".to_string()),
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
            ],
        };

        let summary =
            fan_in_dependency_blocked_summary_from_payload(&payload).expect("blocked summary");
        assert_eq!(summary.task_id, "t-dependent");
        assert_eq!(summary.status, "Cancelled");
        assert_eq!(summary.dependency_blocked_count, 1);
        assert_eq!(summary.task_count, 2);
        assert_eq!(summary.dependency_blocked_ratio, 0.5);
        assert_eq!(summary.blocker_task_id.as_deref(), Some("t-upstream"));
        assert_eq!(summary.blocker_status.as_deref(), Some("Failed"));
        assert_eq!(
            summary.reason.as_deref(),
            Some("blocked by dependency: t-upstream status=Failed")
        );
        assert!(summary.diagnostics_tasks.is_none());

        let text = format_fan_in_dependency_blocked_summary(&summary);
        assert!(text.contains(
            "fan_in_dependency_blocker: task_id=t-dependent status=Cancelled blocked=1/2"
        ));
        assert!(text.contains("blocker_task_id=t-upstream"));
        assert!(text.contains("blocker_status=Failed"));
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_in_dependency_blocker_when_present()
    -> anyhow::Result<()> {
        let t1 = test_thread_meta(false, false, false);
        let t2 = test_thread_meta(false, false, false);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let mut fan_in_blockers = std::collections::BTreeMap::new();
        fan_in_blockers.insert(
            t1.thread_id,
            FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
            },
        );
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1, &t2],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0]["fan_in_dependency_blocker"]["task_id"].as_str(),
            Some("t-dependent")
        );
        assert!(rows[1]["fan_in_dependency_blocker"].is_null());
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_in_result_diagnostics_when_enabled()
    -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.has_fan_in_result_diagnostics = true;
        t1.fan_in_result_diagnostics = Some(FanInResultDiagnosticsInboxSummary {
            task_count: 2,
            diagnostics_tasks: 2,
            diagnostics_matched_completion_total: 5,
            diagnostics_pending_matching_tool_ids_total: 1,
            diagnostics_scan_last_seq_max: 50,
        });
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();

        let rows_with_details = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(
            rows_with_details[0]["fan_in_result_diagnostics"]["diagnostics_tasks"].as_u64(),
            Some(2)
        );

        let rows_without_details = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            false,
        )?;
        assert!(rows_without_details[0]["fan_in_result_diagnostics"].is_null());
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_in_result_diagnostics_from_collected_summaries()
    -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.has_fan_in_result_diagnostics = true;
        t1.fan_in_result_diagnostics = None;
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let mut fan_in_diagnostics = std::collections::BTreeMap::new();
        fan_in_diagnostics.insert(
            t1.thread_id,
            FanInResultDiagnosticsInboxSummary {
                task_count: 3,
                diagnostics_tasks: 2,
                diagnostics_matched_completion_total: 6,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 77,
            },
        );
        let subagent_pending = std::collections::BTreeMap::new();

        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(
            rows[0]["fan_in_result_diagnostics"]["diagnostics_scan_last_seq_max"].as_u64(),
            Some(77)
        );
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_attaches_subagent_pending_when_present() -> anyhow::Result<()> {
        let t1 = test_thread_meta(false, false, false);
        let t2 = test_thread_meta(false, false, false);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let mut subagent_pending = std::collections::BTreeMap::new();
        subagent_pending.insert(
            t1.thread_id,
            SubagentPendingApprovalsSummary {
                total: 3,
                states: std::collections::BTreeMap::from([
                    ("running".to_string(), 2),
                    ("done".to_string(), 1),
                ]),
            },
        );
        let rows = render_inbox_json_threads(
            [&t1, &t2],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["subagent_pending"]["total"].as_u64(), Some(3));
        assert_eq!(
            rows[0]["subagent_pending"]["states"]["running"].as_u64(),
            Some(2)
        );
        assert_eq!(rows[1]["subagent_pending"].as_object(), None);
        Ok(())
    }

    #[test]
    fn build_inbox_json_output_omits_summary_cache_stats_when_absent() -> anyhow::Result<()> {
        let output = build_inbox_json_output(1, 2, vec![], None)?;
        assert_eq!(output["prev_count"].as_u64(), Some(1));
        assert_eq!(output["cur_count"].as_u64(), Some(2));
        assert!(output["threads"].as_array().is_some_and(|rows| rows.is_empty()));
        assert!(output.get("summary_cache_stats").is_none());
        Ok(())
    }

    #[test]
    fn build_inbox_json_output_includes_summary_cache_stats_when_present() -> anyhow::Result<()> {
        let stats = InboxSummaryCacheStats {
            fan_out_meta: 1,
            fan_out_cache_some: 2,
            fan_out_cache_none: 3,
            fan_out_attention: 4,
            fan_out_fetch_some: 5,
            fan_out_fetch_none: 6,
            fan_in_meta: 7,
            fan_in_cache_some: 8,
            fan_in_cache_none: 9,
            fan_in_attention: 10,
            fan_in_fetch_some: 11,
            fan_in_fetch_none: 12,
            fan_in_skip_unblocked: 13,
            fan_in_diag_meta: 14,
            fan_in_diag_cache_some: 15,
            fan_in_diag_cache_none: 16,
            fan_in_diag_attention: 17,
            fan_in_diag_fetch_some: 18,
            fan_in_diag_fetch_none: 19,
            fan_in_diag_skip_absent: 20,
            subagent_meta: 21,
            subagent_cache_some: 22,
            subagent_cache_none: 23,
            subagent_attention_some: 24,
            subagent_attention_none: 25,
            subagent_fetch_some: 26,
            subagent_fetch_none: 27,
            subagent_skip_no_pending: 28,
        };
        let output = build_inbox_json_output(3, 4, vec![], Some(&stats))?;
        assert_eq!(output["summary_cache_stats"]["fan_out_meta"].as_u64(), Some(1));
        assert_eq!(output["summary_cache_stats"]["fan_in_meta"].as_u64(), Some(7));
        assert_eq!(output["summary_cache_stats"]["fan_in_diag_meta"].as_u64(), Some(14));
        assert_eq!(output["summary_cache_stats"]["subagent_meta"].as_u64(), Some(21));
        assert_eq!(
            output["summary_cache_stats"]["subagent_skip_no_pending"].as_u64(),
            Some(28)
        );
        Ok(())
    }

    #[test]
    fn watch_detail_summary_lines_include_auto_apply_and_fan_in_blocker() {
        let auto_apply = FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: Some("git apply failed".to_string()),
        };
        let fan_in_blocker = FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency".to_string()),
        };
        let lines =
            watch_detail_summary_lines(Some(&auto_apply), Some(&fan_in_blocker), None, None);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("summary: fan_out_auto_apply: task_id=t-auto status=error"));
        assert!(lines[1].contains(
            "summary: fan_in_dependency_blocker: task_id=t-dependent status=Cancelled blocked=1/2"
        ));
    }

    #[test]
    fn watch_detail_summary_lines_is_empty_when_no_summaries() {
        let lines = watch_detail_summary_lines(None, None, None, None);
        assert!(lines.is_empty());
    }

    #[test]
    fn format_inbox_summary_cache_stats_includes_all_counters() {
        let stats = InboxSummaryCacheStats {
            fan_out_meta: 1,
            fan_out_cache_some: 2,
            fan_out_cache_none: 3,
            fan_out_attention: 4,
            fan_out_fetch_some: 5,
            fan_out_fetch_none: 6,
            fan_in_meta: 7,
            fan_in_cache_some: 8,
            fan_in_cache_none: 9,
            fan_in_attention: 10,
            fan_in_fetch_some: 11,
            fan_in_fetch_none: 12,
            fan_in_skip_unblocked: 13,
            fan_in_diag_meta: 14,
            fan_in_diag_cache_some: 15,
            fan_in_diag_cache_none: 16,
            fan_in_diag_attention: 17,
            fan_in_diag_fetch_some: 18,
            fan_in_diag_fetch_none: 19,
            fan_in_diag_skip_absent: 20,
            subagent_meta: 21,
            subagent_cache_some: 22,
            subagent_cache_none: 23,
            subagent_attention_some: 24,
            subagent_attention_none: 25,
            subagent_fetch_some: 26,
            subagent_fetch_none: 27,
            subagent_skip_no_pending: 28,
        };
        let line = format_inbox_summary_cache_stats(3, 20, 21, &stats);
        assert!(line.contains("iter=3 prev=20 cur=21"));
        assert!(line.contains(
            "fan_out(meta=1,cache_some=2,cache_none=3,attention=4,fetch_some=5,fetch_none=6)"
        ));
        assert!(line.contains(
            "fan_in(meta=7,cache_some=8,cache_none=9,attention=10,fetch_some=11,fetch_none=12,skip_unblocked=13)"
        ));
        assert!(line.contains(
            "fan_in_diag(meta=14,cache_some=15,cache_none=16,attention=17,fetch_some=18,fetch_none=19,skip_absent=20)"
        ));
        assert!(line.contains(
            "subagent(meta=21,cache_some=22,cache_none=23,attention_some=24,attention_none=25,fetch_some=26,fetch_none=27,skip_no_pending=28)"
        ));
    }

    #[test]
    fn format_watch_summary_refresh_debug_renders_sources() {
        let line = format_watch_summary_refresh_debug(
            7,
            4,
            true,
            false,
            true,
            true,
            SummarySource::Attention,
            SummarySource::Previous,
            SummarySource::Artifact,
            SummarySource::None,
        );
        assert!(line.contains("iter=7 events=4"));
        assert!(line.contains("auto_apply(refresh=true,source=attention)"));
        assert!(line.contains("fan_in(refresh=false,source=previous)"));
        assert!(line.contains("fan_in_diag(refresh=true,source=artifact)"));
        assert!(line.contains("subagent(refresh=true,source=none)"));
    }

    #[test]
    fn build_watch_summary_refresh_debug_json_row_renders_sources() {
        let row = build_watch_summary_refresh_debug_json_row(
            7,
            4,
            true,
            false,
            true,
            true,
            SummarySource::Attention,
            SummarySource::Previous,
            SummarySource::Artifact,
            SummarySource::None,
        );
        assert_eq!(row["kind"].as_str(), Some("watch_summary_refresh_debug"));
        assert_eq!(row["iteration"].as_u64(), Some(7));
        assert_eq!(row["event_count"].as_u64(), Some(4));
        assert_eq!(row["auto_apply"]["refresh"].as_bool(), Some(true));
        assert_eq!(row["auto_apply"]["source"].as_str(), Some("attention"));
        assert_eq!(row["fan_in"]["refresh"].as_bool(), Some(false));
        assert_eq!(row["fan_in"]["source"].as_str(), Some("previous"));
        assert_eq!(row["fan_in_diagnostics"]["refresh"].as_bool(), Some(true));
        assert_eq!(
            row["fan_in_diagnostics"]["source"].as_str(),
            Some("artifact")
        );
        assert_eq!(row["subagent"]["refresh"].as_bool(), Some(true));
        assert_eq!(row["subagent"]["source"].as_str(), Some("none"));
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_cleared_marker() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "summary: fan_out_auto_apply: cleared");
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_only_changed_summary() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "attempted_not_applied".to_string(),
                stage: Some("apply_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: None,
            }),
            fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency".to_string()),
            }),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: previous.fan_in_blocker.clone(),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };

        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("fan_out_auto_apply: task_id=t-auto status=error"));
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_subagent_pending_summary() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 1,
                states: std::collections::BTreeMap::from([("running".to_string(), 1)]),
            }),
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 2,
                states: std::collections::BTreeMap::from([
                    ("done".to_string(), 1),
                    ("running".to_string(), 1),
                ]),
            }),
        };

        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("subagent_pending: total=2"));
        assert!(lines[0].contains("done:1"));
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_fan_in_result_diagnostics_summary() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 2,
                diagnostics_tasks: 2,
                diagnostics_matched_completion_total: 5,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 50,
            }),
            subagent_pending: None,
        };

        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("fan_in_result_diagnostics: tasks=2 diagnostics_tasks=2"));
        assert!(lines[0].contains("matched_completion_total=5"));
    }

    #[test]
    fn watch_detail_summary_json_rows_include_auto_apply_and_fan_in_blocker() {
        let thread_id = ThreadId::new();
        let thread_id_text = thread_id.to_string();
        let auto_apply = FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: Some("git apply failed".to_string()),
        };
        let fan_in_blocker = FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency".to_string()),
        };
        let rows = watch_detail_summary_json_rows(
            thread_id,
            Some(&auto_apply),
            Some(&fan_in_blocker),
            None,
            None,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"].as_str(), Some("watch_detail_summary"));
        assert_eq!(rows[0]["thread_id"].as_str(), Some(thread_id_text.as_str()));
        assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_out_auto_apply"));
        assert_eq!(rows[0]["payload"]["task_id"].as_str(), Some("t-auto"));
        assert_eq!(rows[1]["summary_type"].as_str(), Some("fan_in_dependency_blocker"));
        assert_eq!(rows[1]["payload"]["task_id"].as_str(), Some("t-dependent"));
        assert_eq!(rows[1]["payload"]["dependency_blocked_count"].as_u64(), Some(1));
        assert_eq!(rows[1]["payload"]["task_count"].as_u64(), Some(2));
        assert_eq!(
            rows[1]["payload"]["dependency_blocked_ratio"].as_f64(),
            Some(0.5)
        );
    }

    #[test]
    fn watch_detail_summary_json_rows_is_empty_when_no_summaries() {
        let rows = watch_detail_summary_json_rows(ThreadId::new(), None, None, None, None);
        assert!(rows.is_empty());
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_emits_cleared_marker() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency".to_string()),
            }),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["kind"].as_str(), Some("watch_detail_summary"));
        assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_in_dependency_blocker"));
        assert_eq!(rows[0]["cleared"].as_bool(), Some(true));
        assert_eq!(rows[0]["changed_fields"][0].as_str(), Some("cleared"));
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_includes_changed_fields() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "attempted_not_applied".to_string(),
                stage: Some("apply_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: None,
            }),
            fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency".to_string()),
            }),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: previous.fan_in_blocker.clone(),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };

        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_out_auto_apply"));
        let changed_fields = rows[0]["changed_fields"]
            .as_array()
            .expect("changed_fields array");
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("status")));
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("stage")));
    }

    #[test]
    fn format_fan_in_dependency_blocked_summary_includes_diagnostics_fields() {
        let summary = FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
            diagnostics_tasks: Some(2),
            diagnostics_matched_completion_total: Some(5),
            diagnostics_pending_matching_tool_ids_total: Some(1),
            diagnostics_scan_last_seq_max: Some(50),
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency".to_string()),
        };
        let text = format_fan_in_dependency_blocked_summary(&summary);
        assert!(text.contains("diagnostics_tasks=2"));
        assert!(text.contains("diagnostics_matched_completion_total=5"));
        assert!(text.contains("diagnostics_pending_matching_tool_ids_total=1"));
        assert!(text.contains("diagnostics_scan_last_seq_max=50"));
    }

    #[test]
    fn fan_in_result_diagnostics_summary_reports_without_dependency_blocker() {
        let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
            thread_id: "thread-1".to_string(),
            task_count: 1,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            tasks: vec![omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t1".to_string(),
                title: "first".to_string(),
                thread_id: Some("thread-subagent".to_string()),
                turn_id: Some("turn-subagent".to_string()),
                status: "Completed".to_string(),
                reason: Some("done".to_string()),
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: Some("artifact-1".to_string()),
                result_artifact_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: Some(
                    omne_app_server_protocol::ArtifactFanInSummaryResultArtifactDiagnostics {
                        scan_last_seq: 42,
                        matched_completion_count: 2,
                        pending_matching_tool_ids: 1,
                    },
                ),
                pending_approval: None,
            }],
        };

        let diagnostics = fan_in_result_diagnostics_summary_from_payload(&payload)
            .expect("fan-in diagnostics summary");
        assert_eq!(diagnostics.task_count, 1);
        assert_eq!(diagnostics.diagnostics_tasks, 1);
        assert_eq!(diagnostics.diagnostics_matched_completion_total, 2);
        assert_eq!(diagnostics.diagnostics_pending_matching_tool_ids_total, 1);
        assert_eq!(diagnostics.diagnostics_scan_last_seq_max, 42);

        let text = format_fan_in_result_diagnostics_summary(&diagnostics);
        assert!(text.contains("fan_in_result_diagnostics: tasks=1 diagnostics_tasks=1"));
        assert!(text.contains("matched_completion_total=2"));
        assert!(text.contains("pending_matching_tool_ids_total=1"));
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_includes_subagent_pending_changes() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 1,
                states: std::collections::BTreeMap::from([("running".to_string(), 1)]),
            }),
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 2,
                states: std::collections::BTreeMap::from([
                    ("done".to_string(), 1),
                    ("running".to_string(), 1),
                ]),
            }),
        };

        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary_type"].as_str(), Some("subagent_pending"));
        assert_eq!(rows[0]["payload"]["total"].as_u64(), Some(2));
        let changed_fields = rows[0]["changed_fields"]
            .as_array()
            .expect("changed_fields array");
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("total")));
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("states")));
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_includes_fan_in_result_diagnostics_changes() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 2,
                diagnostics_tasks: 1,
                diagnostics_matched_completion_total: 2,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 42,
            }),
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 2,
                diagnostics_tasks: 2,
                diagnostics_matched_completion_total: 5,
                diagnostics_pending_matching_tool_ids_total: 0,
                diagnostics_scan_last_seq_max: 50,
            }),
            subagent_pending: None,
        };

        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]["summary_type"].as_str(),
            Some("fan_in_result_diagnostics")
        );
        let changed_fields = rows[0]["changed_fields"]
            .as_array()
            .expect("changed_fields array");
        assert!(
            changed_fields
                .iter()
                .any(|value| value.as_str() == Some("diagnostics_tasks"))
        );
        assert!(
            changed_fields.iter().any(|value| {
                value.as_str() == Some("diagnostics_matched_completion_total")
            })
        );
    }

    #[test]
    fn should_emit_watch_detail_summary_emits_for_fan_in_result_diagnostics_only() {
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 1,
                diagnostics_tasks: 1,
                diagnostics_matched_completion_total: 2,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 42,
            }),
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(None, &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_emits_first_non_empty_snapshot() {
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(None, &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_suppresses_unchanged_snapshot() {
        let snapshot = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(!should_emit_watch_detail_summary(Some(&snapshot), &snapshot));
    }

    #[test]
    fn should_emit_watch_detail_summary_emits_when_snapshot_changes() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "attempted_not_applied".to_string(),
                stage: Some("apply_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: None,
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(Some(&previous), &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_suppresses_empty_snapshot() {
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(!should_emit_watch_detail_summary(None, &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_re_emits_after_empty_gap() {
        let non_empty = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let empty = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(Some(&non_empty), &empty));
        assert!(!should_emit_watch_detail_summary(Some(&empty), &empty));
        assert!(should_emit_watch_detail_summary(Some(&empty), &non_empty));
    }

    #[test]
    fn fan_out_auto_apply_summary_omits_applied_payload() {
        let payload = omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t-auto-apply".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: None,
            isolated_write_patch: None,
            isolated_write_handoff: None,
            isolated_write_auto_apply: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                    enabled: true,
                    attempted: true,
                    applied: true,
                    workspace_cwd: None,
                    target_workspace_cwd: None,
                    check_argv: vec![],
                    apply_argv: vec![],
                    patch_artifact_id: None,
                    patch_read_cmd: None,
                    failure_stage: None,
                    recovery_hint: None,
                    recovery_commands: vec![],
                    error: None,
                },
            ),
            status: "completed".to_string(),
            reason: None,
        };

        assert!(fan_out_auto_apply_summary_from_payload(&payload).is_none());
    }

    #[test]
    fn is_fan_out_auto_apply_error_matches_status() {
        let error = FanOutAutoApplyInboxSummary {
            task_id: "t1".to_string(),
            status: "error".to_string(),
            stage: None,
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: None,
        };
        let non_error = FanOutAutoApplyInboxSummary {
            task_id: "t2".to_string(),
            status: "attempted_not_applied".to_string(),
            stage: None,
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: None,
        };
        assert!(is_fan_out_auto_apply_error(&error));
        assert!(!is_fan_out_auto_apply_error(&non_error));
    }
}
