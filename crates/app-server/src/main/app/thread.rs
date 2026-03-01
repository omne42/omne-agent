use super::*;

#[derive(Debug, Deserialize)]
struct ThreadStreamParamsRaw {
    thread_id: ThreadId,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
    #[serde(default)]
    kinds: Option<Vec<String>>,
    #[serde(default)]
    wait_ms: Option<u64>,
}

pub(super) struct ThreadEventBatch {
    pub(super) events: Vec<omne_protocol::ThreadEvent>,
    last_seq: u64,
    thread_last_seq: u64,
    has_more: bool,
}

struct ParsedThreadStreamParams {
    thread_id: ThreadId,
    since_seq: u64,
    max_events: Option<usize>,
    kinds: Option<Vec<omne_protocol::ThreadEventKindTag>>,
    wait_ms: Option<u64>,
}

fn normalize_thread_event_kinds_param(
    id: serde_json::Value,
    kinds: Option<Vec<String>>,
    method: &'static str,
) -> Result<Option<Vec<omne_protocol::ThreadEventKindTag>>, Box<JsonRpcResponse>> {
    let Some(kinds) = kinds else {
        return Ok(None);
    };
    match omne_protocol::normalize_thread_event_kind_filter(&kinds) {
        Ok(requested) => {
            let mut values = requested.into_iter().collect::<Vec<_>>();
            values.sort_by_key(|kind| kind.as_str());
            Ok(Some(values))
        }
        Err(invalid) => Err(Box::new(JsonRpcResponse::err(
            id,
            JSONRPC_INVALID_PARAMS,
            "invalid params",
            Some(serde_json::json!({
                "error": format!(
                    "unsupported {method} kinds: {}",
                    invalid.join(", ")
                ),
                "supported_kinds": omne_protocol::THREAD_EVENT_KIND_TAGS,
            })),
        ))),
    }
}

fn parse_thread_stream_params(
    id: serde_json::Value,
    params: serde_json::Value,
    method: &'static str,
) -> Result<ParsedThreadStreamParams, Box<JsonRpcResponse>> {
    let raw = match serde_json::from_value::<ThreadStreamParamsRaw>(params) {
        Ok(raw) => raw,
        Err(err) => return Err(Box::new(invalid_params(id, err))),
    };
    let kinds = normalize_thread_event_kinds_param(id, raw.kinds, method)?;
    Ok(ParsedThreadStreamParams {
        thread_id: raw.thread_id,
        since_seq: raw.since_seq,
        max_events: raw.max_events,
        kinds,
        wait_ms: raw.wait_ms,
    })
}

pub(super) fn filter_and_paginate_thread_events(
    mut events: Vec<omne_protocol::ThreadEvent>,
    since: EventSeq,
    kinds: Option<&[omne_protocol::ThreadEventKindTag]>,
    max_events: Option<usize>,
) -> ThreadEventBatch {
    let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

    if let Some(kinds) = kinds
        && !kinds.is_empty()
    {
        let requested = kinds
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        events.retain(|event| requested.contains(&event.kind.tag_enum()));
    }

    let mut has_more = false;
    if let Some(max_events) = max_events {
        let max_events = max_events.clamp(1, 50_000);
        if events.len() > max_events {
            events.truncate(max_events);
            has_more = true;
        }
    }

    let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);
    ThreadEventBatch {
        events,
        last_seq,
        thread_last_seq,
        has_more,
    }
}

fn build_thread_events_response(
    batch: ThreadEventBatch,
) -> omne_app_server_protocol::ThreadEventsResponse {
    omne_app_server_protocol::ThreadEventsResponse {
        events: batch.events,
        last_seq: batch.last_seq,
        thread_last_seq: batch.thread_last_seq,
        has_more: batch.has_more,
    }
}

pub(super) fn build_thread_subscribe_response(
    batch: ThreadEventBatch,
    timed_out: bool,
) -> omne_app_server_protocol::ThreadSubscribeResponse {
    omne_app_server_protocol::ThreadSubscribeResponse {
        events: batch.events,
        last_seq: batch.last_seq,
        thread_last_seq: batch.thread_last_seq,
        has_more: batch.has_more,
        timed_out,
    }
}

pub(super) async fn read_thread_events_since_or_not_found(
    server: &Server,
    thread_id: ThreadId,
    since: EventSeq,
) -> anyhow::Result<Vec<omne_protocol::ThreadEvent>> {
    server
        .thread_store
        .read_events_since(thread_id, since)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))
}

fn parse_thread_events_params(
    id: serde_json::Value,
    params: serde_json::Value,
) -> Result<ThreadEventsParams, Box<JsonRpcResponse>> {
    let parsed = parse_thread_stream_params(id, params, "thread/events")?;

    Ok(ThreadEventsParams {
        thread_id: parsed.thread_id,
        since_seq: parsed.since_seq,
        max_events: parsed.max_events,
        kinds: parsed.kinds,
    })
}

fn parse_thread_subscribe_params(
    id: serde_json::Value,
    params: serde_json::Value,
) -> Result<ThreadSubscribeParams, Box<JsonRpcResponse>> {
    let parsed = parse_thread_stream_params(id, params, "thread/subscribe")?;

    Ok(ThreadSubscribeParams {
        thread_id: parsed.thread_id,
        since_seq: parsed.since_seq,
        max_events: parsed.max_events,
        kinds: parsed.kinds,
        wait_ms: parsed.wait_ms,
    })
}

fn usage_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

pub(super) fn configured_total_token_budget_limit() -> Option<u64> {
    std::env::var("OMNE_AGENT_MAX_TOTAL_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn token_budget_snapshot(
    total_tokens_used: u64,
    token_budget_limit: Option<u64>,
) -> (Option<u64>, Option<f64>, Option<bool>) {
    let Some(limit) = token_budget_limit else {
        return (None, None, None);
    };
    let remaining = Some(limit.saturating_sub(total_tokens_used));
    let utilization = usage_ratio(total_tokens_used, limit);
    let exceeded = Some(total_tokens_used > limit);
    (remaining, utilization, exceeded)
}

fn thread_usage_token_budget_warning_snapshot(
    total_tokens_used: u64,
    token_budget_limit: Option<u64>,
    warning_threshold_ratio: f64,
) -> Option<bool> {
    token_budget_limit.map(|_| {
        token_budget_warning_active(
            total_tokens_used,
            token_budget_limit,
            warning_threshold_ratio,
        )
    })
}

type TokenBudgetSnapshot = (
    Option<u64>,
    Option<u64>,
    Option<f64>,
    Option<bool>,
    Option<bool>,
);

pub(super) fn thread_token_budget_snapshot_with_limit(
    total_tokens_used: u64,
    token_budget_limit: Option<u64>,
    warning_threshold_ratio: f64,
) -> TokenBudgetSnapshot {
    let (token_budget_remaining, token_budget_utilization, token_budget_exceeded) =
        token_budget_snapshot(total_tokens_used, token_budget_limit);
    let token_budget_warning_active = thread_usage_token_budget_warning_snapshot(
        total_tokens_used,
        token_budget_limit,
        warning_threshold_ratio,
    );
    (
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
    )
}

pub(super) fn thread_token_budget_snapshot(
    total_tokens_used: u64,
    warning_threshold_ratio: f64,
) -> TokenBudgetSnapshot {
    thread_token_budget_snapshot_with_limit(
        total_tokens_used,
        configured_total_token_budget_limit(),
        warning_threshold_ratio,
    )
}

fn build_thread_usage_response(
    thread_id: ThreadId,
    last_seq: u64,
    total_tokens_used: u64,
    input_tokens_used: u64,
    output_tokens_used: u64,
    cache_input_tokens_used: u64,
    cache_creation_input_tokens_used: u64,
    token_budget_limit: Option<u64>,
    warning_threshold_ratio: f64,
) -> omne_app_server_protocol::ThreadUsageResponse {
    let non_cache_input_tokens_used = input_tokens_used.saturating_sub(cache_input_tokens_used);
    let (
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
    ) = thread_token_budget_snapshot_with_limit(
        total_tokens_used,
        token_budget_limit,
        warning_threshold_ratio,
    );

    omne_app_server_protocol::ThreadUsageResponse {
        thread_id,
        last_seq,
        total_tokens_used,
        input_tokens_used,
        output_tokens_used,
        cache_input_tokens_used,
        cache_creation_input_tokens_used,
        non_cache_input_tokens_used,
        cache_input_ratio: usage_ratio(cache_input_tokens_used, input_tokens_used),
        output_ratio: usage_ratio(output_tokens_used, total_tokens_used),
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
    }
}

async fn handle_thread_start(
    server: &Server,
    params: ThreadStartParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadStartResponse> {
    let cwd = params
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| server.cwd.clone());
    let handle = server.thread_store.create_thread(cwd).await?;
    let thread_id = handle.thread_id();
    let log_path = handle.log_path().display().to_string();
    let last_seq = handle.last_seq().0;
    let rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
    server.threads.lock().await.insert(thread_id, rt);
    let auto_hook = run_auto_workspace_hook(server, thread_id, WorkspaceHookName::Setup).await;
    Ok(omne_app_server_protocol::ThreadStartResponse {
        thread_id,
        log_path,
        last_seq,
        auto_hook,
    })
}

async fn handle_thread_resume(
    server: &Server,
    params: ThreadResumeParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadHandleResponse> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    let handle = rt.handle.lock().await;
    Ok(omne_app_server_protocol::ThreadHandleResponse {
        thread_id: handle.thread_id(),
        log_path: handle.log_path().display().to_string(),
        last_seq: handle.last_seq().0,
    })
}

async fn handle_thread_loaded(
    server: &Server,
) -> anyhow::Result<omne_app_server_protocol::ThreadListResponse> {
    let mut threads = server
        .threads
        .lock()
        .await
        .keys()
        .copied()
        .collect::<Vec<_>>();
    threads.sort_unstable();
    Ok(omne_app_server_protocol::ThreadListResponse { threads })
}

async fn handle_thread_list(
    server: &Server,
) -> anyhow::Result<omne_app_server_protocol::ThreadListResponse> {
    server
        .thread_store
        .list_threads()
        .await
        .map(|threads| omne_app_server_protocol::ThreadListResponse { threads })
}

async fn handle_thread_state(
    server: &Server,
    params: ThreadStateParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadStateResponse> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    let handle = rt.handle.lock().await;
    let state = handle.state();
    let archived_at = state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok());
    let paused_at = state.paused_at.and_then(|ts| ts.format(&Rfc3339).ok());
    let (
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
    ) = thread_token_budget_snapshot(
        state.total_tokens_used,
        token_budget_warning_threshold_ratio(),
    );
    Ok(omne_app_server_protocol::ThreadStateResponse {
        thread_id: handle.thread_id(),
        cwd: state.cwd.clone(),
        archived: state.archived,
        archived_at,
        archived_reason: state.archived_reason.clone(),
        paused: state.paused,
        paused_at,
        paused_reason: state.paused_reason.clone(),
        approval_policy: state.approval_policy,
        sandbox_policy: state.sandbox_policy,
        sandbox_writable_roots: state.sandbox_writable_roots.clone(),
        sandbox_network_access: state.sandbox_network_access,
        mode: state.mode.clone(),
        model: state.model.clone(),
        openai_base_url: state.openai_base_url.clone(),
        allowed_tools: state.allowed_tools.clone(),
        last_seq: handle.last_seq().0,
        active_turn_id: state.active_turn_id,
        active_turn_interrupt_requested: state.active_turn_interrupt_requested,
        last_turn_id: state.last_turn_id,
        last_turn_status: state.last_turn_status,
        last_turn_reason: state.last_turn_reason.clone(),
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
        total_tokens_used: state.total_tokens_used,
        input_tokens_used: state.input_tokens_used,
        output_tokens_used: state.output_tokens_used,
        cache_input_tokens_used: state.cache_input_tokens_used,
        cache_creation_input_tokens_used: state.cache_creation_input_tokens_used,
    })
}

async fn handle_thread_usage(
    server: &Server,
    params: ThreadUsageParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadUsageResponse> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    let handle = rt.handle.lock().await;
    let state = handle.state();
    let token_budget_limit = configured_total_token_budget_limit();
    Ok(build_thread_usage_response(
        handle.thread_id(),
        handle.last_seq().0,
        state.total_tokens_used,
        state.input_tokens_used,
        state.output_tokens_used,
        state.cache_input_tokens_used,
        state.cache_creation_input_tokens_used,
        token_budget_limit,
        token_budget_warning_threshold_ratio(),
    ))
}

async fn handle_thread_events_request(
    server: &Arc<Server>,
    id: &serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params = match parse_thread_events_params(id.clone(), params) {
        Ok(params) => params,
        Err(response) => return *response,
    };
    let since = EventSeq(params.since_seq);
    let result = read_thread_events_since_or_not_found(server, params.thread_id, since)
        .await
        .map(|events| {
            filter_and_paginate_thread_events(
                events,
                since,
                params.kinds.as_deref(),
                params.max_events,
            )
        })
        .map(build_thread_events_response);
    jsonrpc_ok_or_internal(id, result)
}

async fn handle_thread_subscribe_request(
    server: &Arc<Server>,
    id: &serde_json::Value,
    params: serde_json::Value,
) -> JsonRpcResponse {
    let params = match parse_thread_subscribe_params(id.clone(), params) {
        Ok(params) => params,
        Err(response) => return *response,
    };
    jsonrpc_ok_or_internal(id, handle_thread_subscribe(server, params).await)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod thread_usage_budget_tests {
    use super::*;

    #[test]
    fn token_budget_snapshot_disabled_returns_none_fields() {
        let (remaining, utilization, exceeded) = token_budget_snapshot(123, None);
        assert_eq!(remaining, None);
        assert_eq!(utilization, None);
        assert_eq!(exceeded, None);
    }

    #[test]
    fn token_budget_snapshot_under_limit_has_remaining_and_not_exceeded() {
        let (remaining, utilization, exceeded) = token_budget_snapshot(120, Some(200));
        assert_eq!(remaining, Some(80));
        assert_eq!(utilization, Some(0.6));
        assert_eq!(exceeded, Some(false));
    }

    #[test]
    fn token_budget_snapshot_over_limit_saturates_remaining_and_marks_exceeded() {
        let (remaining, utilization, exceeded) = token_budget_snapshot(250, Some(200));
        assert_eq!(remaining, Some(0));
        assert_eq!(utilization, Some(1.25));
        assert_eq!(exceeded, Some(true));
    }

    #[test]
    fn token_budget_snapshot_at_limit_keeps_zero_remaining_without_exceeded() {
        let (remaining, utilization, exceeded) = token_budget_snapshot(200, Some(200));
        assert_eq!(remaining, Some(0));
        assert_eq!(utilization, Some(1.0));
        assert_eq!(exceeded, Some(false));
    }

    #[test]
    fn thread_usage_token_budget_warning_snapshot_disabled_returns_none() {
        assert_eq!(
            thread_usage_token_budget_warning_snapshot(100, None, 0.9),
            None
        );
    }

    #[test]
    fn thread_usage_token_budget_warning_snapshot_threshold_and_exceeded_behavior() {
        assert_eq!(
            thread_usage_token_budget_warning_snapshot(90, Some(100), 0.9),
            Some(true)
        );
        assert_eq!(
            thread_usage_token_budget_warning_snapshot(89, Some(100), 0.9),
            Some(false)
        );
        assert_eq!(
            thread_usage_token_budget_warning_snapshot(101, Some(100), 0.9),
            Some(false)
        );
    }

    #[test]
    fn build_thread_usage_response_marks_warning_at_threshold() {
        let response =
            build_thread_usage_response(ThreadId::new(), 7, 90, 70, 20, 35, 5, Some(100), 0.9);
        assert_eq!(response.last_seq, 7);
        assert_eq!(response.non_cache_input_tokens_used, 35);
        assert_eq!(response.token_budget_remaining, Some(10));
        assert_eq!(response.token_budget_utilization, Some(0.9));
        assert_eq!(response.token_budget_exceeded, Some(false));
        assert_eq!(response.token_budget_warning_active, Some(true));
    }

    #[test]
    fn build_thread_usage_response_disables_warning_when_exceeded() {
        let response =
            build_thread_usage_response(ThreadId::new(), 9, 101, 80, 21, 20, 3, Some(100), 0.9);
        assert_eq!(response.token_budget_remaining, Some(0));
        assert_eq!(response.token_budget_exceeded, Some(true));
        assert_eq!(response.token_budget_warning_active, Some(false));
        assert_eq!(response.token_budget_utilization, Some(1.01));
    }

    #[test]
    fn build_thread_usage_response_without_budget_keeps_budget_fields_empty() {
        let response = build_thread_usage_response(ThreadId::new(), 1, 50, 40, 10, 5, 0, None, 0.9);
        assert_eq!(response.token_budget_limit, None);
        assert_eq!(response.token_budget_remaining, None);
        assert_eq!(response.token_budget_utilization, None);
        assert_eq!(response.token_budget_exceeded, None);
        assert_eq!(response.token_budget_warning_active, None);
    }

    #[test]
    fn thread_token_budget_snapshot_with_limit_reports_limit_and_warning() {
        let (limit, remaining, utilization, exceeded, warning_active) =
            thread_token_budget_snapshot_with_limit(90, Some(100), 0.9);
        assert_eq!(limit, Some(100));
        assert_eq!(remaining, Some(10));
        assert_eq!(utilization, Some(0.9));
        assert_eq!(exceeded, Some(false));
        assert_eq!(warning_active, Some(true));
    }

    #[test]
    fn thread_token_budget_snapshot_with_limit_reports_none_fields_when_unset() {
        let (limit, remaining, utilization, exceeded, warning_active) =
            thread_token_budget_snapshot_with_limit(90, None, 0.9);
        assert_eq!(limit, None);
        assert_eq!(remaining, None);
        assert_eq!(utilization, None);
        assert_eq!(exceeded, None);
        assert_eq!(warning_active, None);
    }
}

pub(super) async fn handle_thread_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    if method == "thread/events" {
        return handle_thread_events_request(server, &id, params).await;
    }
    if method == "thread/subscribe" {
        return handle_thread_subscribe_request(server, &id, params).await;
    }

    dispatch_typed_routes!(id, method, params, {
        "thread/start" => ThreadStartParams => |params| handle_thread_start(server, params),
        "thread/resume" => ThreadResumeParams => |params| handle_thread_resume(server, params),
        "thread/fork" => ThreadForkParams => |params| handle_thread_fork(server, params),
        "thread/archive" => ThreadArchiveParams => |params| handle_thread_archive(server, params),
        "thread/unarchive" => ThreadUnarchiveParams => |params| handle_thread_unarchive(server, params),
        "thread/pause" => ThreadPauseParams => |params| handle_thread_pause(server, params),
        "thread/unpause" => ThreadUnpauseParams => |params| handle_thread_unpause(server, params),
        "thread/delete" => ThreadDeleteParams => |params| handle_thread_delete(server, params),
        "thread/clear_artifacts" => ThreadClearArtifactsParams => |params| handle_thread_clear_artifacts(server, params),
        "thread/list" => Option<ThreadListParams> => |_| handle_thread_list(server),
        "thread/list_meta" => ThreadListMetaParams => |params| handle_thread_list_meta(server, params),
        "thread/loaded" => Option<ThreadLoadedParams> => |_| handle_thread_loaded(server),
        "thread/state" => ThreadStateParams => |params| handle_thread_state(server, params),
        "thread/usage" => ThreadUsageParams => |params| handle_thread_usage(server, params),
        "thread/attention" => ThreadAttentionParams => |params| handle_thread_attention(server, params),
        "thread/disk_usage" => ThreadDiskUsageParams => |params| handle_thread_disk_usage(server, params),
        "thread/disk_report" => ThreadDiskReportParams => |params| handle_thread_disk_report(server, params),
        "thread/diff" => ThreadDiffParams => |params| handle_thread_diff(server, params),
        "thread/patch" => ThreadPatchParams => |params| handle_thread_patch(server, params),
        "thread/checkpoint/create" => ThreadCheckpointCreateParams => |params| handle_thread_checkpoint_create(server, params),
        "thread/checkpoint/list" => ThreadCheckpointListParams => |params| handle_thread_checkpoint_list(server, params),
        "thread/checkpoint/restore" => ThreadCheckpointRestoreParams => |params| handle_thread_checkpoint_restore(server, params),
        "thread/hook_run" => ThreadHookRunParams => |params| handle_thread_hook_run(server, params),
        "thread/configure" => ThreadConfigureParams => |params| handle_thread_configure(server, params),
        "thread/config/explain" => ThreadConfigExplainParams => |params| handle_thread_config_explain(server, params),
        "thread/models" => ThreadModelsParams => |params| handle_thread_models(server, params),
    })
}
