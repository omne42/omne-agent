use super::*;
use omne_eventlog::ThreadState;

pub(super) async fn handle_thread_attention(
    server: &Server,
    params: ThreadAttentionParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadAttentionResponse> {
    let rt = server.get_or_load_thread(params.thread_id).await?;

    let (
        last_seq,
        active_turn_id,
        active_turn_interrupt_requested,
        last_turn_id,
        last_turn_status,
        last_turn_reason,
        archived,
        archived_at,
        archived_reason,
        paused,
        paused_at,
        paused_reason,
        failed_processes,
        approval_policy,
        sandbox_policy,
        model,
        openai_base_url,
        cwd,
        total_tokens_used,
    ) = {
        let handle = rt.handle.lock().await;
        let state = handle.state();
        (
            handle.last_seq().0,
            state.active_turn_id,
            state.active_turn_interrupt_requested,
            state.last_turn_id,
            state.last_turn_status,
            state.last_turn_reason.clone(),
            state.archived,
            state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok()),
            state.archived_reason.clone(),
            state.paused,
            state.paused_at.and_then(|ts| ts.format(&Rfc3339).ok()),
            state.paused_reason.clone(),
            state.failed_processes.iter().copied().collect::<Vec<_>>(),
            state.approval_policy,
            state.sandbox_policy,
            state.model.clone(),
            state.openai_base_url.clone(),
            state.cwd.clone(),
            state.total_tokens_used,
        )
    };

    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut requested =
        BTreeMap::<omne_protocol::ApprovalId, omne_app_server_protocol::ThreadAttentionPendingApproval>::new();
    let mut decided = HashSet::<omne_protocol::ApprovalId>::new();

    for event in &events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match &event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action,
                params: approval_params,
            } => {
                requested.insert(
                    *approval_id,
                    omne_app_server_protocol::ThreadAttentionPendingApproval {
                        approval_id: *approval_id,
                        turn_id: *turn_id,
                        action: Some(action.clone()),
                        action_id: Some(parse_thread_approval_action_id(action)),
                        params: Some(approval_params.clone()),
                        summary: summarize_pending_approval_with_context(
                            Some(params.thread_id),
                            Some(*approval_id),
                            Some(action.as_str()),
                            approval_params,
                        ),
                        requested_at: Some(ts),
                    },
                );
            }
            omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                decided.insert(*approval_id);
            }
            _ => {}
        }
    }

    let mut pending_approvals = requested
        .into_iter()
        .filter(|(id, _)| !decided.contains(id))
        .map(|(_, v)| v)
        .collect::<Vec<_>>();
    enrich_pending_approvals_with_child_thread_state(server, &mut pending_approvals).await;

    let processes = handle_process_list(
        server,
        ProcessListParams {
            thread_id: Some(params.thread_id),
        },
    )
    .await?;

    let running_process_infos = processes
        .into_iter()
        .filter(|p| matches!(p.status, ProcessStatus::Running))
        .collect::<Vec<_>>();

    let stale_processes = match process_idle_window() {
        Some(idle_window) => compute_stale_processes(&running_process_infos, idle_window).await?,
        None => Vec::new(),
    };
    let running_processes = running_process_infos
        .into_iter()
        .map(|p| omne_app_server_protocol::ThreadAttentionRunningProcess {
            process_id: p.process_id,
            argv: p.argv,
            status: Some(
                match p.status {
                    ProcessStatus::Running => "running",
                    ProcessStatus::Exited => "exited",
                    ProcessStatus::Abandoned => "abandoned",
                }
                .to_string(),
            ),
        })
        .collect::<Vec<_>>();
    let attention_markers =
        build_attention_markers(server, params.thread_id, &events).await?;
    let has_plan_ready = attention_markers.plan_ready.is_some();
    let has_diff_ready = attention_markers.diff_ready.is_some();
    let has_fan_out_linkage_issue = attention_markers.fan_out_linkage_issue.is_some();
    let has_fan_out_auto_apply_error = attention_markers.fan_out_auto_apply_error.is_some();
    let fan_out_auto_apply = latest_fan_out_auto_apply_summary(server, params.thread_id).await?;
    let fan_in_dependency_blocker =
        latest_fan_in_dependency_blocked_summary(server, params.thread_id).await?;
    let has_fan_in_dependency_blocked = fan_in_dependency_blocker.is_some();
    let fan_in_result_diagnostics =
        latest_fan_in_result_diagnostics_summary(server, params.thread_id).await?;
    let has_fan_in_result_diagnostics = fan_in_result_diagnostics.is_some();
    let has_test_failed = attention_markers.test_failed.is_some();
    let (
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
    ) = thread_token_budget_snapshot(total_tokens_used, token_budget_warning_threshold_ratio());

    let attention_state = compute_attention_state(
        archived,
        !pending_approvals.is_empty(),
        !failed_processes.is_empty(),
        has_fan_out_auto_apply_error,
        has_fan_out_linkage_issue,
        active_turn_id.is_some() || !running_processes.is_empty(),
        paused,
        last_turn_status,
        false,
    );

    Ok(omne_app_server_protocol::ThreadAttentionResponse {
        thread_id: params.thread_id,
        cwd,
        archived,
        archived_at,
        archived_reason,
        paused,
        paused_at,
        paused_reason,
        failed_processes,
        approval_policy,
        sandbox_policy,
        model,
        openai_base_url,
        last_seq,
        active_turn_id,
        active_turn_interrupt_requested,
        last_turn_id,
        last_turn_status,
        last_turn_reason,
        token_budget_limit,
        token_budget_remaining,
        token_budget_utilization,
        token_budget_exceeded,
        token_budget_warning_active,
        attention_state: attention_state.to_string(),
        pending_approvals,
        running_processes,
        stale_processes: stale_processes.into_iter().map(Into::into).collect::<Vec<_>>(),
        attention_markers: attention_markers.into(),
        has_plan_ready,
        has_diff_ready,
        has_fan_out_linkage_issue,
        has_fan_out_auto_apply_error,
        fan_out_auto_apply,
        has_fan_in_dependency_blocked,
        fan_in_dependency_blocker,
        has_fan_in_result_diagnostics,
        fan_in_result_diagnostics,
        has_test_failed,
    })
}

fn compute_attention_state(
    archived: bool,
    has_pending_approvals: bool,
    has_failed_processes: bool,
    has_fan_out_auto_apply_error: bool,
    has_fan_out_linkage_issue: bool,
    has_running_activity: bool,
    paused: bool,
    last_turn_status: Option<omne_protocol::TurnStatus>,
    archived_first: bool,
) -> &'static str {
    if archived_first && archived {
        return "archived";
    }
    if has_pending_approvals {
        return "need_approval";
    }
    if has_failed_processes || has_fan_out_auto_apply_error || has_fan_out_linkage_issue {
        return "failed";
    }
    if has_running_activity {
        return "running";
    }
    if paused {
        return "paused";
    }
    if !archived_first && archived {
        return "archived";
    }
    match last_turn_status {
        Some(omne_protocol::TurnStatus::Completed) => "done",
        Some(omne_protocol::TurnStatus::Interrupted) => "interrupted",
        Some(omne_protocol::TurnStatus::Failed) => "failed",
        Some(omne_protocol::TurnStatus::Cancelled) => "cancelled",
        Some(omne_protocol::TurnStatus::Stuck) => "stuck",
        None => "idle",
    }
}

pub(super) fn summarize_pending_approval(
    params: &serde_json::Value,
) -> Option<omne_app_server_protocol::ThreadAttentionPendingApprovalSummary> {
    summarize_pending_approval_with_context(None, None, None, params)
}

fn summarize_pending_approval_with_context(
    thread_id: Option<ThreadId>,
    approval_id: Option<omne_protocol::ApprovalId>,
    action: Option<&str>,
    params: &serde_json::Value,
) -> Option<omne_app_server_protocol::ThreadAttentionPendingApprovalSummary> {
    let obj = params.as_object()?;
    let child_request = obj
        .get("child_request")
        .and_then(serde_json::Value::as_object);
    let child_params = child_request
        .and_then(|child| child.get("params"))
        .and_then(serde_json::Value::as_object);
    let proxy = obj
        .get("subagent_proxy")
        .and_then(serde_json::Value::as_object);
    let source = child_params.unwrap_or(obj);

    let requirement = source
        .get("approval")
        .and_then(|v| v.get("requirement"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let argv = source
        .get("argv")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty());
    let cwd = source
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let process_id = source
        .get("process_id")
        .and_then(|v| serde_json::from_value::<ProcessId>(v.clone()).ok());
    let artifact_type = source
        .get("artifact_type")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let path = source
        .get("path")
        .or_else(|| source.get("target_path"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let server = source
        .get("server")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let tool = source
        .get("tool")
        .or_else(|| child_request.and_then(|child| child.get("action")))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let hook = source
        .get("hook")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let child_thread_id = proxy
        .and_then(|v| v.get("child_thread_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<ThreadId>().ok());
    let child_turn_id = proxy
        .and_then(|v| v.get("child_turn_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<TurnId>().ok());
    let child_approval_id = proxy
        .and_then(|v| v.get("child_approval_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse::<omne_protocol::ApprovalId>().ok());
    let approve_cmd = match (thread_id, approval_id, action) {
        (Some(thread_id), Some(approval_id), Some("subagent/proxy_approval")) => Some(format!(
            "omne approval decide {thread_id} {approval_id} --approve"
        )),
        _ => None,
    };
    let deny_cmd = approve_cmd
        .as_deref()
        .and_then(|command| command.strip_suffix(" --approve"))
        .map(|base| format!("{base} --deny"));

    let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
        requirement,
        argv,
        cwd,
        process_id,
        artifact_type,
        path,
        server,
        tool,
        hook,
        child_thread_id,
        child_turn_id,
        child_approval_id,
        child_attention_state: None,
        child_last_turn_status: None,
        approve_cmd,
        deny_cmd,
    };

    if summary.requirement.is_none()
        && summary.argv.is_none()
        && summary.cwd.is_none()
        && summary.process_id.is_none()
        && summary.artifact_type.is_none()
        && summary.path.is_none()
        && summary.server.is_none()
        && summary.tool.is_none()
        && summary.hook.is_none()
        && summary.child_thread_id.is_none()
        && summary.child_turn_id.is_none()
        && summary.child_approval_id.is_none()
        && summary.child_attention_state.is_none()
        && summary.child_last_turn_status.is_none()
        && summary.approve_cmd.is_none()
        && summary.deny_cmd.is_none()
    {
        None
    } else {
        Some(summary)
    }
}

#[derive(Debug, Clone)]
struct ChildThreadAttentionSnapshot {
    attention_state: String,
    last_turn_status: Option<TurnStatus>,
}

async fn enrich_pending_approvals_with_child_thread_state(
    server: &Server,
    pending_approvals: &mut [omne_app_server_protocol::ThreadAttentionPendingApproval],
) {
    let child_thread_ids = pending_approvals
        .iter()
        .filter_map(|pending| pending.summary.as_ref())
        .filter_map(|summary| summary.child_thread_id)
        .collect::<HashSet<_>>();
    if child_thread_ids.is_empty() {
        return;
    }

    let mut snapshots = HashMap::<ThreadId, ChildThreadAttentionSnapshot>::new();
    for child_thread_id in child_thread_ids {
        let events = match server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await
        {
            Ok(Some(events)) => events,
            Ok(None) => continue,
            Err(err) => {
                tracing::debug!(
                    thread_id = %child_thread_id,
                    error = %err,
                    "skip child thread summary enrichment: failed to read events"
                );
                continue;
            }
        };

        let mut child_state = ThreadState::new(child_thread_id);
        let mut apply_failed = false;
        for event in &events {
            if let Err(err) = child_state.apply(event) {
                tracing::debug!(
                    thread_id = %child_thread_id,
                    event_seq = event.seq.0,
                    error = %err,
                    "skip child thread summary enrichment: failed to apply event"
                );
                apply_failed = true;
                break;
            }
        }
        if apply_failed {
            continue;
        }

        snapshots.insert(
            child_thread_id,
            ChildThreadAttentionSnapshot {
                attention_state: child_attention_state_from_state(&child_state).to_string(),
                last_turn_status: child_state.last_turn_status,
            },
        );
    }

    for pending in pending_approvals.iter_mut() {
        let Some(summary) = pending.summary.as_mut() else {
            continue;
        };
        let Some(child_thread_id) = summary.child_thread_id else {
            continue;
        };
        let Some(snapshot) = snapshots.get(&child_thread_id) else {
            continue;
        };
        summary.child_attention_state = Some(snapshot.attention_state.clone());
        summary.child_last_turn_status = snapshot.last_turn_status;
    }
}

fn child_attention_state_from_state(state: &ThreadState) -> &'static str {
    if state.archived {
        "archived"
    } else if !state.pending_approvals.is_empty() {
        "need_approval"
    } else if !state.failed_processes.is_empty() {
        "failed"
    } else if state.active_turn_id.is_some() || !state.running_processes.is_empty() {
        "running"
    } else if state.paused {
        "paused"
    } else {
        match state.last_turn_status {
            Some(omne_protocol::TurnStatus::Completed) => "done",
            Some(omne_protocol::TurnStatus::Interrupted) => "interrupted",
            Some(omne_protocol::TurnStatus::Failed) => "failed",
            Some(omne_protocol::TurnStatus::Cancelled) => "cancelled",
            Some(omne_protocol::TurnStatus::Stuck) => "stuck",
            None => "idle",
        }
    }
}

pub(super) fn parse_thread_approval_action_id(
    action: &str,
) -> omne_app_server_protocol::ThreadApprovalActionId {
    match action {
        "artifact/write" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactWrite,
        "artifact/list" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactList,
        "artifact/read" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactRead,
        "artifact/versions" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactVersions,
        "artifact/delete" => omne_app_server_protocol::ThreadApprovalActionId::ArtifactDelete,
        "file/read" => omne_app_server_protocol::ThreadApprovalActionId::FileRead,
        "file/write" => omne_app_server_protocol::ThreadApprovalActionId::FileWrite,
        "file/edit" => omne_app_server_protocol::ThreadApprovalActionId::FileEdit,
        "file/patch" => omne_app_server_protocol::ThreadApprovalActionId::FilePatch,
        "file/delete" => omne_app_server_protocol::ThreadApprovalActionId::FileDelete,
        "file/glob" => omne_app_server_protocol::ThreadApprovalActionId::FileGlob,
        "file/grep" => omne_app_server_protocol::ThreadApprovalActionId::FileGrep,
        "fs/mkdir" => omne_app_server_protocol::ThreadApprovalActionId::FsMkdir,
        "process/start" => omne_app_server_protocol::ThreadApprovalActionId::ProcessStart,
        "process/kill" => omne_app_server_protocol::ThreadApprovalActionId::ProcessKill,
        "process/interrupt" => omne_app_server_protocol::ThreadApprovalActionId::ProcessInterrupt,
        "process/tail" => omne_app_server_protocol::ThreadApprovalActionId::ProcessTail,
        "process/follow" => omne_app_server_protocol::ThreadApprovalActionId::ProcessFollow,
        "process/inspect" => omne_app_server_protocol::ThreadApprovalActionId::ProcessInspect,
        "process/execve" => omne_app_server_protocol::ThreadApprovalActionId::ProcessExecve,
        "repo/search" => omne_app_server_protocol::ThreadApprovalActionId::RepoSearch,
        "repo/index" => omne_app_server_protocol::ThreadApprovalActionId::RepoIndex,
        "repo/symbols" => omne_app_server_protocol::ThreadApprovalActionId::RepoSymbols,
        "mcp/list_servers" => omne_app_server_protocol::ThreadApprovalActionId::McpListServers,
        "mcp/list_tools" => omne_app_server_protocol::ThreadApprovalActionId::McpListTools,
        "mcp/list_resources" => {
            omne_app_server_protocol::ThreadApprovalActionId::McpListResources
        }
        "mcp/call" => omne_app_server_protocol::ThreadApprovalActionId::McpCall,
        "thread/checkpoint/restore" => {
            omne_app_server_protocol::ThreadApprovalActionId::ThreadCheckpointRestore
        }
        "subagent/proxy_approval" => {
            omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval
        }
        _ => omne_app_server_protocol::ThreadApprovalActionId::Unknown,
    }
}

#[derive(Debug, Serialize)]
pub(super) struct StaleProcessInfo {
    pub(super) process_id: ProcessId,
    pub(super) idle_seconds: u64,
    pub(super) last_update_at: String,
    pub(super) stdout_path: String,
    pub(super) stderr_path: String,
}

impl From<AttentionArtifactMarker> for omne_app_server_protocol::ThreadAttentionArtifactMarker {
    fn from(value: AttentionArtifactMarker) -> Self {
        Self {
            set_at: value.set_at,
            artifact_id: value.artifact_id,
            artifact_type: value.artifact_type,
            turn_id: value.turn_id,
        }
    }
}

impl From<AttentionTestFailedMarker> for omne_app_server_protocol::ThreadAttentionTestFailedMarker {
    fn from(value: AttentionTestFailedMarker) -> Self {
        Self {
            set_at: value.set_at,
            process_id: value.process_id,
            turn_id: value.turn_id,
            exit_code: value.exit_code,
            command: value.command,
        }
    }
}

impl From<AttentionStateMarker> for omne_app_server_protocol::ThreadAttentionStateMarker {
    fn from(value: AttentionStateMarker) -> Self {
        Self {
            set_at: value.set_at,
            turn_id: value.turn_id,
        }
    }
}

impl From<AttentionMarkers> for omne_app_server_protocol::ThreadAttentionMarkers {
    fn from(value: AttentionMarkers) -> Self {
        Self {
            plan_ready: value.plan_ready.map(Into::into),
            diff_ready: value.diff_ready.map(Into::into),
            fan_out_linkage_issue: value.fan_out_linkage_issue.map(Into::into),
            fan_out_auto_apply_error: value.fan_out_auto_apply_error.map(Into::into),
            test_failed: value.test_failed.map(Into::into),
            token_budget_warning: value.token_budget_warning.map(Into::into),
            token_budget_exceeded: value.token_budget_exceeded.map(Into::into),
        }
    }
}

impl From<StaleProcessInfo> for omne_app_server_protocol::ThreadAttentionStaleProcess {
    fn from(value: StaleProcessInfo) -> Self {
        Self {
            process_id: value.process_id,
            idle_seconds: value.idle_seconds,
            last_update_at: value.last_update_at,
            stdout_path: value.stdout_path,
            stderr_path: value.stderr_path,
        }
    }
}

pub(super) async fn compute_stale_processes(
    running_processes: &[ProcessInfo],
    idle_window: Duration,
) -> anyhow::Result<Vec<StaleProcessInfo>> {
    let idle_window_seconds = idle_window.as_secs();
    if idle_window_seconds == 0 {
        return Ok(Vec::new());
    }

    let now = OffsetDateTime::now_utc();
    let mut stale = Vec::new();

    for process in running_processes {
        let last_update_at = last_process_output_at(process).await?;
        let idle_seconds = (now - last_update_at).whole_seconds().max(0) as u64;
        if idle_seconds < idle_window_seconds {
            continue;
        }

        stale.push(StaleProcessInfo {
            process_id: process.process_id,
            idle_seconds,
            last_update_at: last_update_at.format(&Rfc3339)?,
            stdout_path: process.stdout_path.clone(),
            stderr_path: process.stderr_path.clone(),
        });
    }

    Ok(stale)
}

async fn last_process_output_at(process: &ProcessInfo) -> anyhow::Result<OffsetDateTime> {
    let now = OffsetDateTime::now_utc();
    let started_at = OffsetDateTime::parse(&process.started_at, &Rfc3339).unwrap_or(now);

    let stdout_base = PathBuf::from(&process.stdout_path);
    let stderr_base = PathBuf::from(&process.stderr_path);

    let stdout_at = latest_rotating_log_mtime(&stdout_base).await?;
    let stderr_at = latest_rotating_log_mtime(&stderr_base).await?;

    Ok(match (stdout_at, stderr_at) {
        (Some(stdout_at), Some(stderr_at)) => stdout_at.max(stderr_at),
        (Some(stdout_at), None) => stdout_at,
        (None, Some(stderr_at)) => stderr_at,
        (None, None) => started_at,
    })
}

async fn latest_rotating_log_mtime(base_path: &Path) -> anyhow::Result<Option<OffsetDateTime>> {
    let files = list_rotating_log_files(base_path).await?;
    let mut latest: Option<OffsetDateTime> = None;

    for file in files {
        let meta = match tokio::fs::metadata(&file).await {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("metadata {}", file.display())),
        };

        let Ok(modified) = meta.modified() else {
            continue;
        };
        let modified = OffsetDateTime::from(modified);
        latest = Some(latest.map_or(modified, |prev| prev.max(modified)));
    }

    Ok(latest)
}

#[derive(Debug, Clone, Serialize)]
struct AttentionArtifactMarker {
    set_at: String,
    artifact_id: ArtifactId,
    artifact_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<TurnId>,
}

#[derive(Debug, Clone, Serialize)]
struct AttentionTestFailedMarker {
    set_at: String,
    process_id: ProcessId,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<TurnId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AttentionStateMarker {
    set_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<TurnId>,
}

#[derive(Debug, Default, Clone, Serialize)]
struct AttentionMarkers {
    #[serde(skip_serializing_if = "Option::is_none")]
    plan_ready: Option<AttentionArtifactMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_ready: Option<AttentionArtifactMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fan_out_linkage_issue: Option<AttentionArtifactMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fan_out_auto_apply_error: Option<AttentionArtifactMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_failed: Option<AttentionTestFailedMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_budget_warning: Option<AttentionStateMarker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_budget_exceeded: Option<AttentionStateMarker>,
}

#[derive(Debug, Clone)]
struct ProcessStartMarkerInfo {
    turn_id: Option<TurnId>,
    argv: Vec<String>,
}

async fn build_attention_markers(
    server: &Server,
    thread_id: ThreadId,
    events: &[ThreadEvent],
) -> anyhow::Result<AttentionMarkers> {
    let mut markers = AttentionMarkers::default();
    let mut explicit_plan_marker_seen = false;
    let mut explicit_diff_marker_seen = false;
    let mut explicit_fan_out_linkage_issue_marker_seen = false;
    let mut explicit_fan_out_auto_apply_error_marker_seen = false;
    let mut explicit_test_failed_marker_seen = false;
    let mut explicit_token_budget_warning_marker_seen = false;
    let mut explicit_token_budget_exceeded_marker_seen = false;
    let mut started = HashMap::<ProcessId, ProcessStartMarkerInfo>::new();

    for event in events {
        match &event.kind {
            omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker,
                turn_id,
                artifact_id,
                artifact_type,
                process_id,
                exit_code,
                command,
            } => match marker {
                omne_protocol::AttentionMarkerKind::PlanReady => {
                    explicit_plan_marker_seen = true;
                    let Some(artifact_id) = artifact_id else {
                        continue;
                    };
                    let Some(artifact_type) = artifact_type.as_ref() else {
                        continue;
                    };
                    markers.plan_ready = Some(AttentionArtifactMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        artifact_id: *artifact_id,
                        artifact_type: artifact_type.clone(),
                        turn_id: *turn_id,
                    });
                }
                omne_protocol::AttentionMarkerKind::DiffReady => {
                    explicit_diff_marker_seen = true;
                    let Some(artifact_id) = artifact_id else {
                        continue;
                    };
                    let Some(artifact_type) = artifact_type.as_ref() else {
                        continue;
                    };
                    markers.diff_ready = Some(AttentionArtifactMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        artifact_id: *artifact_id,
                        artifact_type: artifact_type.clone(),
                        turn_id: *turn_id,
                    });
                }
                omne_protocol::AttentionMarkerKind::FanOutLinkageIssue => {
                    explicit_fan_out_linkage_issue_marker_seen = true;
                    let Some(artifact_id) = artifact_id else {
                        continue;
                    };
                    let Some(artifact_type) = artifact_type.as_ref() else {
                        continue;
                    };
                    markers.fan_out_linkage_issue = Some(AttentionArtifactMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        artifact_id: *artifact_id,
                        artifact_type: artifact_type.clone(),
                        turn_id: *turn_id,
                    });
                }
                omne_protocol::AttentionMarkerKind::FanOutAutoApplyError => {
                    explicit_fan_out_auto_apply_error_marker_seen = true;
                    let Some(artifact_id) = artifact_id else {
                        continue;
                    };
                    let Some(artifact_type) = artifact_type.as_ref() else {
                        continue;
                    };
                    markers.fan_out_auto_apply_error = Some(AttentionArtifactMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        artifact_id: *artifact_id,
                        artifact_type: artifact_type.clone(),
                        turn_id: *turn_id,
                    });
                }
                omne_protocol::AttentionMarkerKind::TestFailed => {
                    explicit_test_failed_marker_seen = true;
                    let Some(process_id) = process_id else {
                        continue;
                    };
                    markers.test_failed = Some(AttentionTestFailedMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        process_id: *process_id,
                        turn_id: *turn_id,
                        exit_code: *exit_code,
                        command: command.clone(),
                    });
                }
                omne_protocol::AttentionMarkerKind::TokenBudgetWarning => {
                    explicit_token_budget_warning_marker_seen = true;
                    markers.token_budget_warning = Some(AttentionStateMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        turn_id: *turn_id,
                    });
                }
                omne_protocol::AttentionMarkerKind::TokenBudgetExceeded => {
                    explicit_token_budget_exceeded_marker_seen = true;
                    markers.token_budget_exceeded = Some(AttentionStateMarker {
                        set_at: event.timestamp.format(&Rfc3339)?,
                        turn_id: *turn_id,
                    });
                }
            },
            omne_protocol::ThreadEventKind::AttentionMarkerCleared { marker, .. } => match marker {
                omne_protocol::AttentionMarkerKind::PlanReady => {
                    explicit_plan_marker_seen = true;
                    markers.plan_ready = None;
                }
                omne_protocol::AttentionMarkerKind::DiffReady => {
                    explicit_diff_marker_seen = true;
                    markers.diff_ready = None;
                }
                omne_protocol::AttentionMarkerKind::FanOutLinkageIssue => {
                    explicit_fan_out_linkage_issue_marker_seen = true;
                    markers.fan_out_linkage_issue = None;
                }
                omne_protocol::AttentionMarkerKind::FanOutAutoApplyError => {
                    explicit_fan_out_auto_apply_error_marker_seen = true;
                    markers.fan_out_auto_apply_error = None;
                }
                omne_protocol::AttentionMarkerKind::TestFailed => {
                    explicit_test_failed_marker_seen = true;
                    markers.test_failed = None;
                }
                omne_protocol::AttentionMarkerKind::TokenBudgetWarning => {
                    explicit_token_budget_warning_marker_seen = true;
                    markers.token_budget_warning = None;
                }
                omne_protocol::AttentionMarkerKind::TokenBudgetExceeded => {
                    explicit_token_budget_exceeded_marker_seen = true;
                    markers.token_budget_exceeded = None;
                }
            },
            omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id,
                argv,
                ..
            } => {
                started.insert(
                    *process_id,
                    ProcessStartMarkerInfo {
                        turn_id: *turn_id,
                        argv: argv.clone(),
                    },
                );
            }
            omne_protocol::ThreadEventKind::ProcessExited {
                process_id,
                exit_code,
                ..
            } => {
                let code = exit_code.unwrap_or_default();
                if code == 0 {
                    continue;
                }
                let Some(start) = started.get(process_id) else {
                    continue;
                };
                if !looks_like_test_command(&start.argv) {
                    continue;
                }
                if explicit_test_failed_marker_seen {
                    continue;
                }
                let marker = AttentionTestFailedMarker {
                    set_at: event.timestamp.format(&Rfc3339)?,
                    process_id: *process_id,
                    turn_id: start.turn_id,
                    exit_code: *exit_code,
                    command: process_command_label(&start.argv),
                };
                markers.test_failed = Some(marker);
            }
            _ => {}
        }
    }

    if (!explicit_plan_marker_seen && markers.plan_ready.is_none())
        || (!explicit_diff_marker_seen && markers.diff_ready.is_none())
        || (!explicit_fan_out_linkage_issue_marker_seen && markers.fan_out_linkage_issue.is_none())
        || (!explicit_fan_out_auto_apply_error_marker_seen
            && markers.fan_out_auto_apply_error.is_none())
        || (!explicit_token_budget_warning_marker_seen && markers.token_budget_warning.is_none())
        || (!explicit_token_budget_exceeded_marker_seen && markers.token_budget_exceeded.is_none())
    {
        let artifacts = list_thread_artifact_metadata(server, thread_id).await?;
        let mut fan_out_auto_apply_fallback_resolved = false;
        for meta in &artifacts {
            if !explicit_plan_marker_seen && markers.plan_ready.is_none() && meta.artifact_type == "plan" {
                markers.plan_ready = Some(attention_artifact_marker(meta)?);
            }
            if !explicit_diff_marker_seen
                && markers.diff_ready.is_none()
                && (meta.artifact_type == "diff" || meta.artifact_type == "patch")
            {
                markers.diff_ready = Some(attention_artifact_marker(meta)?);
            }
            if !explicit_fan_out_linkage_issue_marker_seen
                && markers.fan_out_linkage_issue.is_none()
                && meta.artifact_type == "fan_out_linkage_issue"
            {
                markers.fan_out_linkage_issue = Some(attention_artifact_marker(meta)?);
            }
            if !explicit_fan_out_auto_apply_error_marker_seen
                && !fan_out_auto_apply_fallback_resolved
                && markers.fan_out_auto_apply_error.is_none()
                && meta.artifact_type == "fan_out_result"
            {
                match infer_fan_out_auto_apply_error_from_result_artifact(meta).await {
                    Ok(Some(has_auto_apply_error)) => {
                        fan_out_auto_apply_fallback_resolved = true;
                        if has_auto_apply_error {
                            markers.fan_out_auto_apply_error = Some(attention_artifact_marker(meta)?);
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            artifact_id = %meta.artifact_id,
                            content_path = %meta.content_path,
                            error = %err,
                            "skip fan_out_result auto-apply fallback parse error"
                        );
                    }
                }
            }
            if markers.plan_ready.is_some()
                && markers.diff_ready.is_some()
                && markers.fan_out_linkage_issue.is_some()
                && (markers.fan_out_auto_apply_error.is_some()
                    || explicit_fan_out_auto_apply_error_marker_seen
                    || fan_out_auto_apply_fallback_resolved)
            {
                break;
            }
        }
    }

    Ok(markers)
}

fn attention_artifact_marker(meta: &ArtifactMetadata) -> anyhow::Result<AttentionArtifactMarker> {
    Ok(AttentionArtifactMarker {
        set_at: meta.updated_at.format(&Rfc3339)?,
        artifact_id: meta.artifact_id,
        artifact_type: meta.artifact_type.clone(),
        turn_id: meta.provenance.as_ref().and_then(|p| p.turn_id),
    })
}

async fn infer_fan_out_auto_apply_error_from_result_artifact(
    meta: &ArtifactMetadata,
) -> anyhow::Result<Option<bool>> {
    if meta.artifact_type != "fan_out_result" {
        return Ok(None);
    }
    let text = match tokio::fs::read_to_string(&meta.content_path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", meta.content_path)),
    };
    let Some(payload) = parse_fan_out_result_structured_data(text.as_str()) else {
        return Ok(None);
    };
    let has_auto_apply_error = payload
        .isolated_write_auto_apply
        .as_ref()
        .and_then(|auto_apply| auto_apply.error.as_ref())
        .is_some_and(|error| !error.trim().is_empty());
    Ok(Some(has_auto_apply_error))
}

async fn latest_fan_out_auto_apply_summary(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<Option<omne_app_server_protocol::ThreadFanOutAutoApplySummary>> {
    let artifacts = list_thread_artifact_metadata(server, thread_id).await?;
    infer_fan_out_auto_apply_summary_from_artifacts(&artifacts).await
}

async fn infer_fan_out_auto_apply_summary_from_artifacts(
    artifacts: &[ArtifactMetadata],
) -> anyhow::Result<Option<omne_app_server_protocol::ThreadFanOutAutoApplySummary>> {
    for meta in artifacts {
        if meta.artifact_type != "fan_out_result" {
            continue;
        }
        match infer_fan_out_auto_apply_summary_from_result_artifact(meta).await {
            Ok(Some(summary)) => return Ok(summary),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    artifact_id = %meta.artifact_id,
                    content_path = %meta.content_path,
                    error = %err,
                    "skip fan_out_result auto-apply summary parse error"
                );
            }
        }
    }
    Ok(None)
}

async fn infer_fan_out_auto_apply_summary_from_result_artifact(
    meta: &ArtifactMetadata,
) -> anyhow::Result<Option<Option<omne_app_server_protocol::ThreadFanOutAutoApplySummary>>> {
    if meta.artifact_type != "fan_out_result" {
        return Ok(None);
    }
    let text = match tokio::fs::read_to_string(&meta.content_path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", meta.content_path)),
    };
    let Some(payload) = parse_fan_out_result_structured_data(text.as_str()) else {
        return Ok(None);
    };
    Ok(Some(fan_out_auto_apply_summary_from_payload(&payload)))
}

fn fan_out_auto_apply_summary_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutResultStructuredData,
) -> Option<omne_app_server_protocol::ThreadFanOutAutoApplySummary> {
    omne_app_server_protocol::fan_out_auto_apply_summary_from_payload(payload, 120)
}

async fn latest_fan_in_dependency_blocked_summary(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<Option<omne_app_server_protocol::ThreadFanInDependencyBlockedSummary>> {
    let artifacts = list_thread_artifact_metadata(server, thread_id).await?;
    let (dependency_blocked, _) = infer_fan_in_summary_signals_from_artifacts(&artifacts).await?;
    Ok(dependency_blocked)
}

async fn latest_fan_in_result_diagnostics_summary(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<Option<omne_app_server_protocol::ThreadFanInResultDiagnosticsSummary>> {
    let artifacts = list_thread_artifact_metadata(server, thread_id).await?;
    let (_, diagnostics) = infer_fan_in_summary_signals_from_artifacts(&artifacts).await?;
    Ok(diagnostics)
}

type FanInSummarySignals = (
    Option<omne_app_server_protocol::ThreadFanInDependencyBlockedSummary>,
    Option<omne_app_server_protocol::ThreadFanInResultDiagnosticsSummary>,
);

async fn infer_fan_in_summary_signals_from_artifacts(
    artifacts: &[ArtifactMetadata],
) -> anyhow::Result<FanInSummarySignals> {
    for meta in artifacts {
        if meta.artifact_type != "fan_in_summary" {
            continue;
        }
        match infer_fan_in_summary_signals_from_summary_artifact(meta).await {
            Ok(Some(summary)) => return Ok(summary),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    artifact_id = %meta.artifact_id,
                    content_path = %meta.content_path,
                    error = %err,
                    "skip fan_in_summary parse error"
                );
            }
        }
    }
    Ok((None, None))
}

async fn infer_fan_in_summary_signals_from_summary_artifact(
    meta: &ArtifactMetadata,
) -> anyhow::Result<Option<FanInSummarySignals>> {
    if meta.artifact_type != "fan_in_summary" {
        return Ok(None);
    }
    let text = match tokio::fs::read_to_string(&meta.content_path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", meta.content_path)),
    };
    let Some(payload) = parse_fan_in_summary_structured_data(text.as_str()) else {
        return Ok(None);
    };
    Ok(Some((
        fan_in_dependency_blocked_summary_from_payload(&payload),
        fan_in_result_diagnostics_summary_from_payload(&payload),
    )))
}

fn fan_in_dependency_blocked_summary_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanInSummaryStructuredData,
) -> Option<omne_app_server_protocol::ThreadFanInDependencyBlockedSummary> {
    omne_app_server_protocol::fan_in_dependency_blocked_summary_from_payload(payload, 280)
}

fn fan_in_result_diagnostics_summary_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanInSummaryStructuredData,
) -> Option<omne_app_server_protocol::ThreadFanInResultDiagnosticsSummary> {
    omne_app_server_protocol::fan_in_result_diagnostics_summary_from_payload(payload)
}

pub(super) fn process_command_label(argv: &[String]) -> Option<String> {
    let first = argv.first()?.trim();
    if first.is_empty() {
        return None;
    }
    if first.eq_ignore_ascii_case("cargo")
        && let Some(second) = argv.get(1)
        && !second.trim().is_empty()
    {
        return Some(format!("{first} {}", second.trim()));
    }
    Some(first.to_string())
}

pub(super) fn looks_like_test_command(argv: &[String]) -> bool {
    let Some(first_raw) = argv.first() else {
        return false;
    };
    let first = first_raw.trim().to_ascii_lowercase();
    if first.is_empty() {
        return false;
    }

    if first == "cargo" {
        return argv
            .get(1)
            .map(|v| v.trim().eq_ignore_ascii_case("test"))
            .unwrap_or(false);
    }
    if first == "go" {
        return argv
            .get(1)
            .map(|v| v.trim().eq_ignore_ascii_case("test"))
            .unwrap_or(false);
    }
    if first == "npm" || first == "pnpm" || first == "yarn" || first == "bun" {
        return argv
            .get(1)
            .map(|v| v.trim().eq_ignore_ascii_case("test"))
            .unwrap_or(false);
    }

    matches!(
        first.as_str(),
        "pytest" | "vitest" | "jest" | "ctest" | "nose2" | "nosetests"
    )
}

async fn list_thread_artifact_metadata(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<Vec<ArtifactMetadata>> {
    let dir = user_artifacts_dir_for_thread(server, thread_id);
    let mut artifacts = Vec::<ArtifactMetadata>::new();

    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(artifacts),
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };

    loop {
        let Some(entry) = read_dir
            .next_entry()
            .await
            .with_context(|| format!("read {}", dir.display()))?
        else {
            break;
        };
        let ty = entry
            .file_type()
            .await
            .with_context(|| format!("stat {}", entry.path().display()))?;
        if !ty.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".metadata.json") {
            continue;
        }
        match read_artifact_metadata(&path).await {
            Ok(meta) => artifacts.push(meta),
            Err(err) => tracing::warn!(path = %path.display(), error = %err, "skip bad artifact metadata"),
        }
    }

    artifacts.sort_by(|a, b| {
        b.updated_at
            .unix_timestamp_nanos()
            .cmp(&a.updated_at.unix_timestamp_nanos())
            .then_with(|| b.artifact_id.cmp(&a.artifact_id))
    });

    Ok(artifacts)
}

#[cfg(test)]
mod attention_marker_tests {
    use super::*;

    static TOKEN_BUDGET_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: test-only temporary env override guarded by TOKEN_BUDGET_ENV_LOCK.
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }

        fn update(&self, value: &str) {
            // SAFETY: test-only temporary env override guarded by TOKEN_BUDGET_ENV_LOCK.
            unsafe { std::env::set_var(self.key, value) };
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                // SAFETY: restoring process env in test teardown.
                unsafe { std::env::set_var(self.key, previous) };
            } else {
                // SAFETY: restoring process env in test teardown.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    fn agent_step_event(response_id: &str, total_tokens: u64) -> omne_protocol::ThreadEventKind {
        omne_protocol::ThreadEventKind::AgentStep {
            turn_id: TurnId::new(),
            step: 1,
            model: "gpt-5".to_string(),
            response_id: response_id.to_string(),
            text: Some("step".to_string()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            token_usage: Some(serde_json::json!({
                "total_tokens": total_tokens,
            })),
            warnings_count: None,
        }
    }

    fn token_budget_marker_sequence(events: &[ThreadEvent]) -> Vec<&'static str> {
        let mut out = Vec::new();
        for event in events {
            match &event.kind {
                omne_protocol::ThreadEventKind::AttentionMarkerSet { marker, .. } => match marker {
                    omne_protocol::AttentionMarkerKind::TokenBudgetWarning => {
                        out.push("set_warning");
                    }
                    omne_protocol::AttentionMarkerKind::TokenBudgetExceeded => {
                        out.push("set_exceeded");
                    }
                    _ => {}
                },
                omne_protocol::ThreadEventKind::AttentionMarkerCleared { marker, .. } => match marker {
                    omne_protocol::AttentionMarkerKind::TokenBudgetWarning => {
                        out.push("clear_warning");
                    }
                    omne_protocol::AttentionMarkerKind::TokenBudgetExceeded => {
                        out.push("clear_exceeded");
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        out
    }

    fn fan_out_result_text(auto_apply_error: Option<&str>) -> String {
        let payload = serde_json::json!({
            "schema_version": omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1,
            "task_id": "task-1",
            "thread_id": "thread-1",
            "turn_id": "turn-1",
            "workspace_mode": "isolated_write",
            "status": "completed",
            "isolated_write_auto_apply": {
                "enabled": true,
                "attempted": true,
                "applied": auto_apply_error.is_none(),
                "error": auto_apply_error
            }
        });
        format!("## fan-out result\n\n```json\n{payload}\n```")
    }

    fn fan_in_summary_text(dependency_blocked: bool, with_diagnostics: bool) -> String {
        let payload = omne_workflow_spec::FanInSummaryStructuredData::new(
            "thread-1".to_string(),
            omne_workflow_spec::FanInSchedulingStructuredData {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            vec![omne_workflow_spec::FanInTaskStructuredData {
                task_id: "t-dependent".to_string(),
                title: "dependent".to_string(),
                thread_id: Some("thread-dependent".to_string()),
                turn_id: Some("turn-dependent".to_string()),
                status: if dependency_blocked {
                    "Cancelled".to_string()
                } else {
                    "Completed".to_string()
                },
                reason: if dependency_blocked {
                    Some("blocked by dependency: t-upstream status=Failed".to_string())
                } else {
                    Some("all good".to_string())
                },
                dependency_blocked,
                dependency_blocker_task_id: if dependency_blocked {
                    Some("t-upstream".to_string())
                } else {
                    None
                },
                dependency_blocker_status: if dependency_blocked {
                    Some("Failed".to_string())
                } else {
                    None
                },
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: with_diagnostics.then_some(
                    omne_workflow_spec::FanInResultArtifactDiagnosticsStructuredData {
                        scan_last_seq: 42,
                        matched_completion_count: 2,
                        pending_matching_tool_ids: 1,
                    },
                ),
                pending_approval: None,
            }],
        );
        let structured_json = serde_json::to_string_pretty(&payload)
            .expect("fan_in_summary structured data should serialize");
        format!("# Fan-in Summary\n\n## Structured Data\n\n```json\n{structured_json}\n```\n")
    }

    async fn write_raw_fan_out_result_artifact(
        server: &Server,
        thread_id: ThreadId,
        artifact_id: ArtifactId,
        summary: &str,
        text: &str,
        updated_at: OffsetDateTime,
    ) -> anyhow::Result<()> {
        let (content_path, metadata_path) = user_artifact_paths(server, thread_id, artifact_id);
        write_file_atomic(&content_path, text.as_bytes()).await?;
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "fan_out_result".to_string(),
            summary: summary.to_string(),
            preview: Some(infer_artifact_preview("fan_out_result")),
            created_at: updated_at,
            updated_at,
            version: 1,
            content_path: content_path.display().to_string(),
            size_bytes: text.len() as u64,
            provenance: Some(ArtifactProvenance {
                thread_id,
                turn_id: None,
                tool_id: None,
                process_id: None,
            }),
        };
        let metadata_bytes =
            serde_json::to_vec_pretty(&metadata).context("serialize test artifact metadata")?;
        write_file_atomic(&metadata_path, &metadata_bytes).await?;
        Ok(())
    }

    async fn write_raw_fan_in_summary_artifact(
        server: &Server,
        thread_id: ThreadId,
        artifact_id: ArtifactId,
        summary: &str,
        text: &str,
        updated_at: OffsetDateTime,
    ) -> anyhow::Result<()> {
        let (content_path, metadata_path) = user_artifact_paths(server, thread_id, artifact_id);
        write_file_atomic(&content_path, text.as_bytes()).await?;
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "fan_in_summary".to_string(),
            summary: summary.to_string(),
            preview: Some(infer_artifact_preview("fan_in_summary")),
            created_at: updated_at,
            updated_at,
            version: 1,
            content_path: content_path.display().to_string(),
            size_bytes: text.len() as u64,
            provenance: Some(ArtifactProvenance {
                thread_id,
                turn_id: None,
                tool_id: None,
                process_id: None,
            }),
        };
        let metadata_bytes =
            serde_json::to_vec_pretty(&metadata).context("serialize test artifact metadata")?;
        write_file_atomic(&metadata_path, &metadata_bytes).await?;
        Ok(())
    }

    #[test]
    fn summarize_pending_approval_extracts_structured_fields() {
        let process_id = ProcessId::new();
        let params = serde_json::json!({
            "approval": { "requirement": "prompt_strict" },
            "argv": ["git", "status"],
            "cwd": "/tmp/repo",
            "process_id": process_id,
            "artifact_type": "diff",
            "path": "src/main.rs",
            "server": "local",
            "tool": "noop",
            "hook": "setup",
        });

        let summary = summarize_pending_approval_with_context(
            Some(ThreadId::new()),
            Some(omne_protocol::ApprovalId::new()),
            Some("process/start"),
            &params,
        )
        .expect("expected summary");
        assert_eq!(summary.requirement.as_deref(), Some("prompt_strict"));
        assert_eq!(
            summary.argv,
            Some(vec!["git".to_string(), "status".to_string()])
        );
        assert_eq!(summary.cwd.as_deref(), Some("/tmp/repo"));
        assert_eq!(summary.process_id, Some(process_id));
        assert_eq!(summary.artifact_type.as_deref(), Some("diff"));
        assert_eq!(summary.path.as_deref(), Some("src/main.rs"));
        assert_eq!(summary.server.as_deref(), Some("local"));
        assert_eq!(summary.tool.as_deref(), Some("noop"));
        assert_eq!(summary.hook.as_deref(), Some("setup"));
    }

    #[test]
    fn summarize_pending_approval_returns_none_for_unstructured_params() {
        let params = serde_json::json!({
            "note": "no known keys"
        });
        assert!(
            summarize_pending_approval_with_context(
                Some(ThreadId::new()),
                Some(omne_protocol::ApprovalId::new()),
                Some("process/start"),
                &params
            )
            .is_none()
        );
    }

    #[test]
    fn summarize_pending_approval_extracts_subagent_proxy_child_request_fields() {
        let params = serde_json::json!({
            "subagent_proxy": {
                "kind": "approval",
                "task_id": "t1",
            },
            "child_request": {
                "action": "process/start",
                "params": {
                    "approval": { "requirement": "prompt_strict" },
                    "argv": ["cargo", "test"],
                    "cwd": "/tmp/repo",
                }
            }
        });

        let thread_id = ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let summary = summarize_pending_approval_with_context(
            Some(thread_id),
            Some(approval_id),
            Some("subagent/proxy_approval"),
            &params,
        )
        .expect("expected summary");
        assert_eq!(summary.requirement.as_deref(), Some("prompt_strict"));
        assert_eq!(
            summary.argv,
            Some(vec!["cargo".to_string(), "test".to_string()])
        );
        assert_eq!(summary.cwd.as_deref(), Some("/tmp/repo"));
        assert_eq!(summary.tool.as_deref(), Some("process/start"));
        assert!(summary.child_attention_state.is_none());
        assert!(summary.child_last_turn_status.is_none());
        let expected_cmd = format!("omne approval decide {thread_id} {approval_id} --approve");
        assert_eq!(
            summary.approve_cmd.as_deref(),
            Some(expected_cmd.as_str())
        );
        let expected_deny_cmd = format!("omne approval decide {thread_id} {approval_id} --deny");
        assert_eq!(
            summary.deny_cmd.as_deref(),
            Some(expected_deny_cmd.as_str())
        );
    }

    #[test]
    fn parse_thread_approval_action_id_maps_known_actions() {
        assert_eq!(
            parse_thread_approval_action_id("process/start"),
            omne_app_server_protocol::ThreadApprovalActionId::ProcessStart
        );
        assert_eq!(
            parse_thread_approval_action_id("mcp/call"),
            omne_app_server_protocol::ThreadApprovalActionId::McpCall
        );
        assert_eq!(
            parse_thread_approval_action_id("thread/checkpoint/restore"),
            omne_app_server_protocol::ThreadApprovalActionId::ThreadCheckpointRestore
        );
    }

    #[test]
    fn parse_thread_approval_action_id_maps_unknown_to_unknown_variant() {
        assert_eq!(
            parse_thread_approval_action_id("custom/action"),
            omne_app_server_protocol::ThreadApprovalActionId::Unknown
        );
    }

    #[tokio::test]
    async fn thread_list_meta_counts_pending_subagent_proxy_approvals() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let proxy_approval_id = omne_protocol::ApprovalId::new();
        let normal_approval_id = omne_protocol::ApprovalId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({}),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: normal_approval_id,
                turn_id: None,
                action: "process/start".to_string(),
                params: serde_json::json!({}),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: normal_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: None,
            })
            .await?;

        let list_meta = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: false,
            },
        )
        .await?;
        let row = list_meta
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist");
        assert_eq!(row.pending_subagent_proxy_approvals, 1);

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: None,
            })
            .await?;

        let list_meta_after = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: false,
            },
        )
        .await?;
        let row_after = list_meta_after
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist");
        assert_eq!(row_after.pending_subagent_proxy_approvals, 0);
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_pending_subagent_approval_includes_child_thread_state()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        let child_repo_dir = tmp.path().join("child-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;
        tokio::fs::create_dir_all(&child_repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(parent_repo_dir).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);
        let child_handle = server.thread_store.create_thread(child_repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let child_rt = server.get_or_load_thread(child_thread_id).await?;
        let child_turn_id = TurnId::new();
        child_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_rt
            .append_event(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: TurnStatus::Completed,
                reason: Some("done".to_string()),
            })
            .await?;

        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        let approval_id = omne_protocol::ApprovalId::new();
        let child_approval_id = omne_protocol::ApprovalId::new();
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": {
                            "approval": { "requirement": "prompt_strict" },
                            "argv": ["cargo", "test"],
                        }
                    }
                }),
            })
            .await?;

        let attention = handle_thread_attention(
            &server,
            ThreadAttentionParams {
                thread_id: parent_thread_id,
            },
        )
        .await?;
        assert_eq!(attention.pending_approvals.len(), 1);
        let summary = attention.pending_approvals[0]
            .summary
            .as_ref()
            .expect("pending approval summary should exist");
        assert_eq!(summary.child_thread_id, Some(child_thread_id));
        assert_eq!(summary.child_turn_id, Some(child_turn_id));
        assert_eq!(summary.child_approval_id, Some(child_approval_id));
        assert_eq!(summary.child_attention_state.as_deref(), Some("done"));
        assert_eq!(summary.child_last_turn_status, Some(TurnStatus::Completed));
        Ok(())
    }

    #[tokio::test]
    async fn thread_runtime_token_budget_marker_sequence_tracks_threshold_and_limit_changes()
    -> anyhow::Result<()> {
        let _env_lock = TOKEN_BUDGET_ENV_LOCK.lock().await;
        let limit_env = ScopedEnvVar::set("OMNE_AGENT_MAX_TOTAL_TOKENS", "100");

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt.append_event(agent_step_event("resp_0", 1)).await?;
        thread_rt.append_event(agent_step_event("resp_1", 99)).await?;
        thread_rt.append_event(agent_step_event("resp_2", 1)).await?;

        limit_env.update("110");
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AssistantMessage {
                turn_id: None,
                text: "recompute budget state".to_string(),
                model: None,
                response_id: None,
                token_usage: None,
            })
            .await?;

        limit_env.update("300");
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AssistantMessage {
                turn_id: None,
                text: "recompute budget state again".to_string(),
                model: None,
                response_id: None,
                token_usage: None,
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        assert_eq!(
            token_budget_marker_sequence(&events),
            vec![
                "set_warning",
                "clear_warning",
                "set_exceeded",
                "set_warning",
                "clear_exceeded",
                "clear_warning"
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_includes_plan_diff_and_test_failed_markers() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "marker test".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;

        let process_id = ProcessId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: Some(turn_id),
                argv: vec!["cargo".to_string(), "test".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: tmp.path().join("stdout.log").display().to_string(),
                stderr_path: tmp.path().join("stderr.log").display().to_string(),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessExited {
                process_id,
                exit_code: Some(101),
                reason: Some("test failed".to_string()),
            })
            .await?;

        let plan = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "plan".to_string(),
                summary: "plan".to_string(),
                text: "plan body".to_string(),
            },
        )
        .await?;
        let plan_artifact_id: ArtifactId = serde_json::from_value(plan["artifact_id"].clone())?;

        let patch = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "patch".to_string(),
                summary: "patch".to_string(),
                text: "diff --git a b".to_string(),
            },
        )
        .await?;
        let patch_artifact_id: ArtifactId = serde_json::from_value(patch["artifact_id"].clone())?;

        let linkage_issue = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue".to_string(),
                summary: "fan-out linkage issue".to_string(),
                text: "fan-out linkage issue: task_id=t1 status=Failed".to_string(),
            },
        )
        .await?;
        let linkage_issue_artifact_id: ArtifactId =
            serde_json::from_value(linkage_issue["artifact_id"].clone())?;
        let fan_out_result_artifact_id = ArtifactId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: Some(turn_id),
                artifact_id: Some(fan_out_result_artifact_id),
                artifact_type: Some("fan_out_result".to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                omne_protocol::ThreadEventKind::AttentionMarkerSet {
                    marker: omne_protocol::AttentionMarkerKind::PlanReady,
                    artifact_id: Some(id),
                    ..
                } if id == plan_artifact_id
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                omne_protocol::ThreadEventKind::AttentionMarkerSet {
                    marker: omne_protocol::AttentionMarkerKind::DiffReady,
                    artifact_id: Some(id),
                    ..
                } if id == patch_artifact_id
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                omne_protocol::ThreadEventKind::AttentionMarkerSet {
                    marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                    artifact_id: Some(id),
                    ..
                } if id == linkage_issue_artifact_id
            )
        }));
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert_eq!(
            markers.plan_ready.as_ref().map(|m| m.artifact_id),
            Some(plan_artifact_id)
        );
        assert_eq!(
            markers.diff_ready.as_ref().map(|m| m.artifact_id),
            Some(patch_artifact_id)
        );
        assert_eq!(
            markers.fan_out_linkage_issue.as_ref().map(|m| m.artifact_id),
            Some(linkage_issue_artifact_id)
        );
        assert_eq!(
            markers
                .fan_out_auto_apply_error
                .as_ref()
                .map(|m| m.artifact_id),
            Some(fan_out_result_artifact_id)
        );
        assert_eq!(
            markers.test_failed.as_ref().map(|m| m.process_id),
            Some(process_id)
        );
        assert_eq!(
            markers
                .test_failed
                .as_ref()
                .and_then(|m| m.command.as_deref()),
            Some("cargo test")
        );

        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        assert!(attention.has_plan_ready);
        assert!(attention.has_diff_ready);
        assert!(attention.has_fan_out_linkage_issue);
        assert!(attention.has_fan_out_auto_apply_error);
        assert!(attention.has_test_failed);
        Ok(())
    }

    #[tokio::test]
    async fn thread_list_meta_includes_marker_booleans() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "marker list_meta test".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;

        let process_id = ProcessId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: Some(turn_id),
                argv: vec!["cargo".to_string(), "test".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: tmp.path().join("stdout.log").display().to_string(),
                stderr_path: tmp.path().join("stderr.log").display().to_string(),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessExited {
                process_id,
                exit_code: Some(101),
                reason: Some("test failed".to_string()),
            })
            .await?;

        handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "plan".to_string(),
                summary: "plan".to_string(),
                text: "plan body".to_string(),
            },
        )
        .await?;

        handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue".to_string(),
                summary: "fan-out linkage issue".to_string(),
                text: "fan-out linkage issue: task_id=t1 status=Failed".to_string(),
            },
        )
        .await?;

        handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "patch".to_string(),
                summary: "patch".to_string(),
                text: "diff --git a b".to_string(),
            },
        )
        .await?;

        let auto_apply_error_artifact_id = ArtifactId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: Some(turn_id),
                artifact_id: Some(auto_apply_error_artifact_id),
                artifact_type: Some("fan_out_result".to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;

        let list_meta_without_markers = serde_json::to_value(
            handle_thread_list_meta(
                &server,
                ThreadListMetaParams {
                    include_archived: true,
                    include_attention_markers: false,
                },
            )
            .await?,
        )?;
        let threads = list_meta_without_markers["threads"]
            .as_array()
            .expect("threads should be an array");
        let row = threads
            .iter()
            .find(|value| {
                serde_json::from_value::<ThreadId>(value["thread_id"].clone())
                    .ok()
                    .map(|id| id == thread_id)
                    .unwrap_or(false)
            })
            .expect("thread row should exist");

        assert_eq!(row["has_plan_ready"].as_bool(), Some(true));
        assert_eq!(row["has_diff_ready"].as_bool(), Some(true));
        assert_eq!(row["has_fan_out_linkage_issue"].as_bool(), Some(true));
        assert_eq!(row["has_fan_out_auto_apply_error"].as_bool(), Some(true));
        assert_eq!(row["has_fan_in_dependency_blocked"].as_bool(), Some(false));
        assert_eq!(row["has_fan_in_result_diagnostics"].as_bool(), Some(false));
        assert_eq!(row["has_test_failed"].as_bool(), Some(true));
        assert!(
            row.as_object()
                .is_some_and(|obj| !obj.contains_key("attention_markers"))
        );

        let list_meta_with_markers = serde_json::to_value(
            handle_thread_list_meta(
                &server,
                ThreadListMetaParams {
                    include_archived: true,
                    include_attention_markers: true,
                },
            )
            .await?,
        )?;
        let threads = list_meta_with_markers["threads"]
            .as_array()
            .expect("threads should be an array");
        let row = threads
            .iter()
            .find(|value| {
                serde_json::from_value::<ThreadId>(value["thread_id"].clone())
                    .ok()
                    .map(|id| id == thread_id)
                    .unwrap_or(false)
            })
            .expect("thread row should exist");
        assert_eq!(
            row["attention_markers"]["plan_ready"]["artifact_type"].as_str(),
            Some("plan")
        );
        assert_eq!(
            row["attention_markers"]["diff_ready"]["artifact_type"].as_str(),
            Some("patch")
        );
        assert_eq!(
            row["attention_markers"]["fan_out_linkage_issue"]["artifact_type"].as_str(),
            Some("fan_out_linkage_issue")
        );
        assert_eq!(
            row["attention_markers"]["fan_out_auto_apply_error"]["artifact_type"].as_str(),
            Some("fan_out_result")
        );
        assert!(
            row["attention_markers"]["test_failed"]["process_id"]
                .as_str()
                .is_some()
        );
        assert!(
            row["attention_markers"]["token_budget_warning"]["set_at"]
                .as_str()
                .is_some()
        );
        assert!(
            row["attention_markers"]["token_budget_exceeded"]["set_at"]
                .as_str()
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_and_list_meta_include_token_budget_snapshot() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        rt.append_event(omne_protocol::ThreadEventKind::AgentStep {
            turn_id: TurnId::new(),
            step: 1,
            model: "gpt-5".to_string(),
            response_id: "resp_1".to_string(),
            text: Some("step".to_string()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            token_usage: Some(serde_json::json!({
                "total_tokens": 100,
                "input_tokens": 70,
                "output_tokens": 30,
                "cache_input_tokens": 55,
                "cache_creation_input_tokens": 9
            })),
            warnings_count: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "final".to_string(),
            model: Some("gpt-5".to_string()),
            response_id: Some("resp_1".to_string()),
            token_usage: Some(serde_json::json!({
                "total_tokens": 100,
                "input_tokens": 70,
                "output_tokens": 30,
                "cache_input_tokens": 55,
                "cache_creation_input_tokens": 9
            })),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "final2".to_string(),
            model: Some("gpt-5".to_string()),
            response_id: Some("resp_2".to_string()),
            token_usage: Some(serde_json::json!({
                "input_tokens": 20,
                "output_tokens": 10,
                "cache_input_tokens": 7
            })),
        })
        .await?;

        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        if let Some(limit) = attention.token_budget_limit {
            assert_eq!(
                attention.token_budget_remaining,
                Some(limit.saturating_sub(130)),
                "thread/attention token_budget_remaining should be derived from budget limit"
            );
            assert_eq!(
                attention.token_budget_utilization,
                Some(130.0 / limit as f64),
                "thread/attention token_budget_utilization should be used/limit"
            );
            assert_eq!(
                attention.token_budget_exceeded,
                Some(130 > limit),
                "thread/attention token_budget_exceeded should reflect used>limit"
            );
            assert_eq!(
                attention.token_budget_warning_active,
                Some(130 <= limit && (130.0 / limit as f64) >= token_budget_warning_threshold_ratio()),
                "thread/attention token_budget_warning_active should reflect threshold and exceeded state"
            );
        } else {
            assert_eq!(attention.token_budget_remaining, None);
            assert_eq!(attention.token_budget_utilization, None);
            assert_eq!(attention.token_budget_exceeded, None);
            assert_eq!(attention.token_budget_warning_active, None);
        }

        let list_meta = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: false,
            },
        )
        .await?;
        let row = list_meta
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist");
        assert_eq!(row.token_budget_limit, attention.token_budget_limit);
        assert_eq!(row.token_budget_remaining, attention.token_budget_remaining);
        assert_eq!(row.token_budget_utilization, attention.token_budget_utilization);
        assert_eq!(row.token_budget_exceeded, attention.token_budget_exceeded);
        assert_eq!(
            row.token_budget_warning_active,
            attention.token_budget_warning_active
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_and_list_meta_include_fan_in_dependency_blocked() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        write_raw_fan_in_summary_artifact(
            &server,
            thread_id,
            artifact_id,
            "fan-in summary with dependency blocker",
            &fan_in_summary_text(true, true),
            OffsetDateTime::now_utc(),
        )
        .await?;

        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        assert!(attention.has_fan_in_dependency_blocked);
        assert!(attention.has_fan_in_result_diagnostics);
        let attention_summary = attention
            .fan_in_dependency_blocker
            .as_ref()
            .expect("fan-in dependency blocker summary should exist");
        assert_eq!(attention_summary.task_id, "t-dependent");
        assert_eq!(attention_summary.blocker_task_id.as_deref(), Some("t-upstream"));
        assert_eq!(attention_summary.blocker_status.as_deref(), Some("Failed"));
        assert_eq!(attention_summary.diagnostics_tasks, Some(1));
        assert_eq!(
            attention_summary.diagnostics_matched_completion_total,
            Some(2)
        );
        assert_eq!(
            attention_summary.diagnostics_pending_matching_tool_ids_total,
            Some(1)
        );
        assert_eq!(attention_summary.diagnostics_scan_last_seq_max, Some(42));
        let attention_diagnostics = attention
            .fan_in_result_diagnostics
            .as_ref()
            .expect("fan-in result diagnostics should exist");
        assert_eq!(attention_diagnostics.task_count, 1);
        assert_eq!(attention_diagnostics.diagnostics_tasks, 1);
        assert_eq!(attention_diagnostics.diagnostics_matched_completion_total, 2);
        assert_eq!(
            attention_diagnostics.diagnostics_pending_matching_tool_ids_total,
            1
        );
        assert_eq!(attention_diagnostics.diagnostics_scan_last_seq_max, 42);

        let list_meta = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: false,
            },
        )
        .await?;
        let row = list_meta
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist");
        assert!(row.has_fan_in_dependency_blocked);
        assert!(row.has_fan_in_result_diagnostics);
        let row_summary = row
            .fan_in_dependency_blocker
            .as_ref()
            .expect("fan-in dependency blocker summary should exist in list_meta");
        assert_eq!(row_summary.task_id, "t-dependent");
        assert_eq!(row_summary.blocker_task_id.as_deref(), Some("t-upstream"));
        assert_eq!(row_summary.diagnostics_tasks, Some(1));
        let row_diagnostics = row
            .fan_in_result_diagnostics
            .as_ref()
            .expect("fan-in result diagnostics should exist in list_meta");
        assert_eq!(row_diagnostics.diagnostics_tasks, 1);
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_fan_in_dependency_blocked_prefers_latest_summary() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let now = OffsetDateTime::now_utc();
        write_raw_fan_in_summary_artifact(
            &server,
            thread_id,
            ArtifactId::new(),
            "older blocked summary",
            &fan_in_summary_text(true, true),
            now - time::Duration::hours(1),
        )
        .await?;
        write_raw_fan_in_summary_artifact(
            &server,
            thread_id,
            ArtifactId::new(),
            "newer clear summary",
            &fan_in_summary_text(false, false),
            now,
        )
        .await?;

        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        assert!(!attention.has_fan_in_dependency_blocked);
        assert!(attention.fan_in_dependency_blocker.is_none());
        assert!(!attention.has_fan_in_result_diagnostics);
        assert!(attention.fan_in_result_diagnostics.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_and_list_meta_include_fan_in_result_diagnostics_without_blocker()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        write_raw_fan_in_summary_artifact(
            &server,
            thread_id,
            ArtifactId::new(),
            "fan-in summary with diagnostics only",
            &fan_in_summary_text(false, true),
            OffsetDateTime::now_utc(),
        )
        .await?;

        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        assert!(!attention.has_fan_in_dependency_blocked);
        assert!(attention.fan_in_dependency_blocker.is_none());
        assert!(attention.has_fan_in_result_diagnostics);
        let attention_diagnostics = attention
            .fan_in_result_diagnostics
            .as_ref()
            .expect("fan-in result diagnostics should exist");
        assert_eq!(attention_diagnostics.task_count, 1);
        assert_eq!(attention_diagnostics.diagnostics_tasks, 1);
        assert_eq!(attention_diagnostics.diagnostics_matched_completion_total, 2);
        assert_eq!(
            attention_diagnostics.diagnostics_pending_matching_tool_ids_total,
            1
        );
        assert_eq!(attention_diagnostics.diagnostics_scan_last_seq_max, 42);

        let list_meta = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: false,
            },
        )
        .await?;
        let row = list_meta
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist");
        assert!(!row.has_fan_in_dependency_blocked);
        assert!(row.fan_in_dependency_blocker.is_none());
        assert!(row.has_fan_in_result_diagnostics);
        let row_diagnostics = row
            .fan_in_result_diagnostics
            .as_ref()
            .expect("fan-in result diagnostics should exist in list_meta");
        assert_eq!(row_diagnostics.diagnostics_tasks, 1);
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_falls_back_to_fan_out_auto_apply_error_from_artifact()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            artifact_id,
            "fan-out result with auto apply error",
            &fan_out_result_text(Some("git apply --check failed")),
            OffsetDateTime::now_utc(),
        )
        .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert_eq!(
            markers
                .fan_out_auto_apply_error
                .as_ref()
                .map(|marker| marker.artifact_id),
            Some(artifact_id)
        );
        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        assert!(attention.has_fan_out_auto_apply_error);
        let summary = attention
            .fan_out_auto_apply
            .as_ref()
            .expect("fan-out auto-apply summary should exist");
        assert_eq!(summary.task_id, "task-1");
        assert_eq!(summary.status, "error");
        assert_eq!(summary.stage.as_deref(), None);
        assert_eq!(
            summary.error.as_deref(),
            Some("git apply --check failed")
        );
        let list_meta = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: false,
            },
        )
        .await?;
        let row = list_meta
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist");
        assert!(row.has_fan_out_auto_apply_error);
        let row_summary = row
            .fan_out_auto_apply
            .as_ref()
            .expect("fan-out auto-apply summary should exist in list_meta");
        assert_eq!(row_summary.task_id, "task-1");
        assert_eq!(row_summary.status, "error");
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_fan_out_auto_apply_fallback_prefers_latest_result() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let older_error_artifact = ArtifactId::new();
        let newer_clear_artifact = ArtifactId::new();
        let now = OffsetDateTime::now_utc();
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            older_error_artifact,
            "older error",
            &fan_out_result_text(Some("patch conflict")),
            now - time::Duration::hours(1),
        )
        .await?;
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            newer_clear_artifact,
            "newer clear",
            &fan_out_result_text(None),
            now,
        )
        .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert!(markers.fan_out_auto_apply_error.is_none());
        let attention = handle_thread_attention(&server, ThreadAttentionParams { thread_id }).await?;
        assert!(!attention.has_fan_out_auto_apply_error);
        assert!(attention.fan_out_auto_apply.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_uses_explicit_marker_events_without_fallback_inputs() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        let plan_artifact_id = ArtifactId::new();
        let diff_artifact_id = ArtifactId::new();
        let fan_out_linkage_issue_artifact_id = ArtifactId::new();
        let process_id = ProcessId::new();

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::PlanReady,
                turn_id: Some(turn_id),
                artifact_id: Some(plan_artifact_id),
                artifact_type: Some("plan".to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::DiffReady,
                turn_id: Some(turn_id),
                artifact_id: Some(diff_artifact_id),
                artifact_type: Some("patch".to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                turn_id: Some(turn_id),
                artifact_id: Some(fan_out_linkage_issue_artifact_id),
                artifact_type: Some("fan_out_linkage_issue".to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TestFailed,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: Some(process_id),
                exit_code: Some(101),
                command: Some("cargo test".to_string()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;

        assert_eq!(
            markers.plan_ready.as_ref().map(|m| m.artifact_id),
            Some(plan_artifact_id)
        );
        assert_eq!(
            markers.diff_ready.as_ref().map(|m| m.artifact_id),
            Some(diff_artifact_id)
        );
        assert_eq!(
            markers.fan_out_linkage_issue.as_ref().map(|m| m.artifact_id),
            Some(fan_out_linkage_issue_artifact_id)
        );
        assert_eq!(
            markers.test_failed.as_ref().map(|m| m.process_id),
            Some(process_id)
        );
        assert_eq!(
            markers
                .test_failed
                .as_ref()
                .and_then(|m| m.command.as_deref()),
            Some("cargo test")
        );
        assert_eq!(
            markers.token_budget_warning.as_ref().and_then(|m| m.turn_id),
            Some(turn_id)
        );
        assert_eq!(
            markers.token_budget_exceeded.as_ref().and_then(|m| m.turn_id),
            Some(turn_id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_explicit_marker_cleared_beats_fallback() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        let process_id = ProcessId::new();

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TestFailed,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: Some(process_id),
                exit_code: Some(101),
                command: Some("cargo test".to_string()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::TestFailed,
                turn_id: Some(turn_id),
                reason: Some("test command succeeded".to_string()),
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert!(markers.test_failed.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_token_budget_marker_clear_removes_marker() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id: Some(turn_id),
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
                turn_id: Some(turn_id),
                reason: Some("token budget warning cleared".to_string()),
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert!(markers.token_budget_warning.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_fan_out_auto_apply_explicit_cleared_beats_artifact_fallback()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        let artifact_id = ArtifactId::new();
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            artifact_id,
            "fan-out result with auto apply error",
            &fan_out_result_text(Some("git apply --check failed")),
            OffsetDateTime::now_utc(),
        )
        .await?;

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: Some(turn_id),
                reason: Some("manual clear".to_string()),
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert!(markers.fan_out_auto_apply_error.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_start_turn_emits_fan_out_auto_apply_clear_event() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let old_artifact_id = ArtifactId::new();
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            old_artifact_id,
            "fan-out result with auto apply error",
            &fan_out_result_text(Some("git apply --check failed")),
            OffsetDateTime::now_utc(),
        )
        .await?;

        let started_turn_id = tokio::task::LocalSet::new()
            .run_until({
                let thread_rt = thread_rt.clone();
                let server = server.clone();
                async move {
                    thread_rt
                        .start_turn(
                            server,
                            "start new turn".to_string(),
                            None,
                            None,
                            None,
                            omne_protocol::TurnPriority::Foreground,
                        )
                        .await
                }
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");

        assert!(events.iter().any(|event| matches!(
            &event.kind,
            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id,
                reason,
            } if *turn_id == Some(started_turn_id) && reason.as_deref() == Some("new turn started")
        )));

        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert!(markers.fan_out_auto_apply_error.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn thread_list_meta_start_turn_clears_fan_out_auto_apply_error() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            artifact_id,
            "fan-out result with auto apply error",
            &fan_out_result_text(Some("git apply --check failed")),
            OffsetDateTime::now_utc(),
        )
        .await?;

        let before_start = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: true,
            },
        )
        .await?;
        let before_row = before_start
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist before start");
        assert!(before_row.has_fan_out_auto_apply_error);
        assert!(
            before_row
                .attention_markers
                .as_ref()
                .and_then(|markers| markers.fan_out_auto_apply_error.as_ref())
                .is_some()
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let started_turn_id = tokio::task::LocalSet::new()
            .run_until({
                let thread_rt = thread_rt.clone();
                let server = server.clone();
                async move {
                    thread_rt
                        .start_turn(
                            server,
                            "start new turn".to_string(),
                            None,
                            None,
                            None,
                            omne_protocol::TurnPriority::Foreground,
                        )
                        .await
                }
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id,
                reason,
            } if *turn_id == Some(started_turn_id) && reason.as_deref() == Some("new turn started")
        )));

        let after_start = handle_thread_list_meta(
            &server,
            ThreadListMetaParams {
                include_archived: true,
                include_attention_markers: true,
            },
        )
        .await?;
        let after_row = after_start
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .expect("thread row should exist after start");
        assert!(!after_row.has_fan_out_auto_apply_error);
        assert!(
            after_row
                .attention_markers
                .as_ref()
                .and_then(|markers| markers.fan_out_auto_apply_error.as_ref())
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_fan_out_auto_apply_explicit_set_beats_artifact_fallback()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        let fallback_artifact_id = ArtifactId::new();
        write_raw_fan_out_result_artifact(
            &server,
            thread_id,
            fallback_artifact_id,
            "fan-out result without auto apply error",
            &fan_out_result_text(None),
            OffsetDateTime::now_utc(),
        )
        .await?;

        let explicit_artifact_id = ArtifactId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: Some(turn_id),
                artifact_id: Some(explicit_artifact_id),
                artifact_type: Some("fan_out_result".to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert_eq!(
            markers
                .fan_out_auto_apply_error
                .as_ref()
                .map(|marker| marker.artifact_id),
            Some(explicit_artifact_id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_attention_plan_diff_and_linkage_cleared_beats_artifact_fallback() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();

        handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "plan".to_string(),
                summary: "plan".to_string(),
                text: "plan body".to_string(),
            },
        )
        .await?;
        handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "patch".to_string(),
                summary: "patch".to_string(),
                text: "diff --git a b".to_string(),
            },
        )
        .await?;
        handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id),
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue".to_string(),
                summary: "fan-out linkage issue".to_string(),
                text: "fan-out linkage issue: task_id=t1 status=Failed".to_string(),
            },
        )
        .await?;

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::PlanReady,
                turn_id: Some(turn_id),
                reason: Some("new turn started".to_string()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::DiffReady,
                turn_id: Some(turn_id),
                reason: Some("new turn started".to_string()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                turn_id: Some(turn_id),
                reason: Some("fan-out linkage issue cleared".to_string()),
            })
            .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread should exist");
        let markers = build_attention_markers(&server, thread_id, &events).await?;
        assert!(markers.plan_ready.is_none());
        assert!(markers.diff_ready.is_none());
        assert!(markers.fan_out_linkage_issue.is_none());
        Ok(())
    }
}

pub(super) async fn maybe_write_stuck_report(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    let report = build_stuck_report_markdown(server, thread_id, turn_id, reason).await?;
    let summary = build_stuck_report_summary(reason);

    let _artifact = handle_artifact_write(
        server,
        ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "stuck_report".to_string(),
            summary,
            text: report,
        },
    )
    .await?;

    Ok(())
}

fn build_stuck_report_summary(reason: Option<&str>) -> String {
    let reason = reason.filter(|s| !s.trim().is_empty()).unwrap_or("unknown");
    let hint = stuck_budget_env_hint(reason);
    let reason = truncate_chars(reason, 120);
    let mut summary = format!("Stuck: {reason}");
    if let Some(hint) = hint {
        summary.push_str(&format!(" (consider {hint})"));
    }
    summary
}

fn stuck_budget_env_hint(reason: &str) -> Option<&'static str> {
    if reason.contains("budget exceeded: steps") {
        return Some("OMNE_AGENT_MAX_STEPS");
    }
    if reason.contains("budget exceeded: tool_calls") {
        return Some("OMNE_AGENT_MAX_TOOL_CALLS");
    }
    if reason.contains("budget exceeded: turn_seconds") {
        return Some("OMNE_AGENT_MAX_TURN_SECONDS");
    }
    if reason.contains("openai request timed out") {
        return Some("OMNE_AGENT_MAX_OPENAI_REQUEST_SECONDS");
    }
    if reason.contains("token budget exceeded:") {
        return Some("OMNE_AGENT_MAX_TOTAL_TOKENS");
    }
    None
}

#[derive(Clone, Copy, Debug)]
struct StuckUsageSnapshot {
    total_tokens_used: u64,
    input_tokens_used: u64,
    output_tokens_used: u64,
    cache_input_tokens_used: u64,
    cache_creation_input_tokens_used: u64,
    non_cache_input_tokens_used: u64,
    cache_input_ratio: Option<f64>,
    output_ratio: Option<f64>,
}

fn usage_snapshot_from_events(thread_id: ThreadId, events: &[ThreadEvent]) -> Option<StuckUsageSnapshot> {
    let mut state = ThreadState::new(thread_id);
    for event in events {
        if state.apply(event).is_err() {
            return None;
        }
    }

    if state.total_tokens_used == 0
        && state.input_tokens_used == 0
        && state.output_tokens_used == 0
        && state.cache_input_tokens_used == 0
        && state.cache_creation_input_tokens_used == 0
    {
        return None;
    }

    Some(StuckUsageSnapshot {
        total_tokens_used: state.total_tokens_used,
        input_tokens_used: state.input_tokens_used,
        output_tokens_used: state.output_tokens_used,
        cache_input_tokens_used: state.cache_input_tokens_used,
        cache_creation_input_tokens_used: state.cache_creation_input_tokens_used,
        non_cache_input_tokens_used: state
            .input_tokens_used
            .saturating_sub(state.cache_input_tokens_used),
        cache_input_ratio: stuck_usage_ratio(state.cache_input_tokens_used, state.input_tokens_used),
        output_ratio: stuck_usage_ratio(state.output_tokens_used, state.total_tokens_used),
    })
}

fn stuck_usage_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

fn format_usage_ratio_percent(ratio: Option<f64>) -> String {
    match ratio {
        Some(value) => format!("{:.2}%", value * 100.0),
        None => "n/a".to_string(),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars + 1).collect::<String>();
    if truncated.chars().count() <= max_chars {
        return truncated;
    }
    truncated
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>()
        + "..."
}

async fn build_stuck_report_markdown(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    reason: Option<&str>,
) -> anyhow::Result<String> {
    #[derive(Clone, Debug)]
    struct ProcessStartInfo {
        process_id: ProcessId,
        turn_id: Option<TurnId>,
        stdout_path: String,
        stderr_path: String,
    }

    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
    let usage = usage_snapshot_from_events(thread_id, &events);

    let mut last_approval_in_turn: Option<(omne_protocol::ApprovalId, String)> = None;
    let mut last_approval_any: Option<(omne_protocol::ApprovalId, String)> = None;
    let mut last_tool_in_turn: Option<(omne_protocol::ToolId, String)> = None;
    let mut last_tool_any: Option<(omne_protocol::ToolId, String)> = None;
    let mut started_processes = Vec::<ProcessStartInfo>::new();
    let mut exited = HashSet::<ProcessId>::new();

    for event in &events {
        match &event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: ev_turn_id,
                action,
                ..
            } => {
                last_approval_any = Some((*approval_id, action.clone()));
                if *ev_turn_id == Some(turn_id) {
                    last_approval_in_turn = Some((*approval_id, action.clone()));
                }
            }
            omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: ev_turn_id,
                tool,
                ..
            } => {
                last_tool_any = Some((*tool_id, tool.clone()));
                if *ev_turn_id == Some(turn_id) {
                    last_tool_in_turn = Some((*tool_id, tool.clone()));
                }
            }
            omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: ev_turn_id,
                stdout_path,
                stderr_path,
                ..
            } => {
                started_processes.push(ProcessStartInfo {
                    process_id: *process_id,
                    turn_id: *ev_turn_id,
                    stdout_path: stdout_path.clone(),
                    stderr_path: stderr_path.clone(),
                });
            }
            omne_protocol::ThreadEventKind::ProcessExited { process_id, .. } => {
                exited.insert(*process_id);
            }
            _ => {}
        }
    }

    let last_approval = last_approval_in_turn.or(last_approval_any);
    let last_tool = last_tool_in_turn.or(last_tool_any);

    let last_running_process_in_turn = started_processes.iter().rev().find(|p| {
        p.turn_id == Some(turn_id) && !exited.contains(&p.process_id)
    });
    let last_running_process_any = started_processes
        .iter()
        .rev()
        .find(|p| !exited.contains(&p.process_id));
    let last_process_in_turn = started_processes
        .iter()
        .rev()
        .find(|p| p.turn_id == Some(turn_id));
    let last_process_any = started_processes.iter().next_back();

    let process = last_running_process_in_turn
        .or(last_running_process_any)
        .or(last_process_in_turn)
        .or(last_process_any);

    let mut md = String::new();
    md.push_str("# Stuck report\n\n");

    md.push_str("## What happened\n");
    md.push_str(&format!("- thread_id: {thread_id}\n"));
    md.push_str(&format!("- turn_id: {turn_id}\n"));
    md.push_str("- status: stuck\n");
    md.push_str(&format!(
        "- reason: {}\n",
        reason.unwrap_or_default().trim()
    ));

    if let Some(usage) = usage {
        md.push_str("\n## Token usage snapshot\n");
        md.push_str(&format!("- total_tokens_used: {}\n", usage.total_tokens_used));
        md.push_str(&format!("- input_tokens_used: {}\n", usage.input_tokens_used));
        md.push_str(&format!("- output_tokens_used: {}\n", usage.output_tokens_used));
        md.push_str(&format!(
            "- cache_input_tokens_used: {}\n",
            usage.cache_input_tokens_used
        ));
        md.push_str(&format!(
            "- cache_creation_input_tokens_used: {}\n",
            usage.cache_creation_input_tokens_used
        ));
        md.push_str(&format!(
            "- non_cache_input_tokens_used: {}\n",
            usage.non_cache_input_tokens_used
        ));
        md.push_str(&format!(
            "- cache_input_ratio: {}\n",
            format_usage_ratio_percent(usage.cache_input_ratio)
        ));
        md.push_str(&format!(
            "- output_ratio: {}\n",
            format_usage_ratio_percent(usage.output_ratio)
        ));
    }

    md.push_str("\n## Where to look\n");
    if let Some((approval_id, action)) = &last_approval {
        md.push_str(&format!(
            "- last_approval_id: {approval_id} ({})\n",
            action.trim()
        ));
    }
    if let Some((tool_id, tool)) = &last_tool {
        md.push_str(&format!("- last_tool: {tool} ({tool_id})\n", tool = tool.trim()));
    }
    if let Some(process) = &process {
        md.push_str(&format!("- last_process_id: {}\n", process.process_id));
        md.push_str(&format!("- stdout_path: {}\n", process.stdout_path.trim()));
        md.push_str(&format!("- stderr_path: {}\n", process.stderr_path.trim()));
    }

    md.push_str("\n## Next actions\n");
    md.push_str(&format!("- omne thread attention {thread_id}\n"));
    md.push_str(&format!("- omne approval list {thread_id}\n"));
    md.push_str(&format!("- omne process list --thread-id {thread_id}\n"));
    if let Some(process) = &process {
        md.push_str(&format!("- omne process tail {}\n", process.process_id));
    }
    if let Some(hint) = reason.and_then(stuck_budget_env_hint) {
        md.push_str(&format!("- consider increasing `{hint}`\n"));
    }
    if let Some(usage) = usage {
        md.push_str(&format!(
            "- token usage snapshot: total={} input={} output={} cache_input={}\n",
            usage.total_tokens_used,
            usage.input_tokens_used,
            usage.output_tokens_used,
            usage.cache_input_tokens_used
        ));
        if let Some(cache_ratio) = usage.cache_input_ratio {
            if usage.input_tokens_used >= 100 && cache_ratio < 0.30 {
                md.push_str(&format!(
                    "- cache reuse is low ({:.1}%): keep stable prompt prefixes to improve prompt cache hits\n",
                    cache_ratio * 100.0
                ));
            }
        }
        if let Some(output_ratio) = usage.output_ratio
            && output_ratio > 0.60
        {
            md.push_str(&format!(
                "- output ratio is high ({:.1}%): tighten output length/format constraints\n",
                output_ratio * 100.0
            ));
        }
    }

    Ok(md)
}

fn pending_subagent_proxy_approval_count(events: &[ThreadEvent]) -> usize {
    let mut pending = HashMap::<omne_protocol::ApprovalId, bool>::new();
    for event in events {
        match &event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                action,
                ..
            } => {
                pending.insert(*approval_id, action == "subagent/proxy_approval");
            }
            omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                pending.remove(approval_id);
            }
            _ => {}
        }
    }
    pending.values().filter(|is_subagent| **is_subagent).count()
}

pub(super) async fn handle_thread_list_meta(
    server: &Server,
    params: ThreadListMetaParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadListMetaResponse> {
    let thread_ids = server.thread_store.list_threads().await?;
    let configured_token_budget_limit = configured_total_token_budget_limit();
    let mut threads =
        Vec::<(Option<i128>, ThreadId, omne_app_server_protocol::ThreadListMetaItem)>::new();

    for thread_id in thread_ids {
        let Some(events) = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
        else {
            continue;
        };
        let mut state = ThreadState::new(thread_id);
        for event in &events {
            state.apply(event)?;
        }

        if state.archived && !params.include_archived {
            continue;
        }

        let created_at = events.first().map(|event| event.timestamp);
        let updated_at = events.last().map(|event| event.timestamp);
        let created_at_rfc3339 = created_at.and_then(|ts| ts.format(&Rfc3339).ok());
        let updated_at_rfc3339 = updated_at.and_then(|ts| ts.format(&Rfc3339).ok());

        let first_message = events.iter().find_map(|event| match &event.kind {
            omne_protocol::ThreadEventKind::TurnStarted { input, .. } => Some(input.clone()),
            _ => None,
        });
        let first_message = first_message
            .map(|text| truncate_chars(&omne_core::redact_text(text.trim()), 500))
            .filter(|text| !text.trim().is_empty());
        let title = first_message.as_deref().and_then(|text| {
            let line = text.lines().find(|line| !line.trim().is_empty())?;
            let line = line.trim();
            if line.is_empty() {
                None
            } else {
                Some(truncate_chars(line, 120))
            }
        });

        let archived_at = state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok());
        let attention_markers = build_attention_markers(server, thread_id, &events).await?;
        let has_plan_ready = attention_markers.plan_ready.is_some();
        let has_diff_ready = attention_markers.diff_ready.is_some();
        let has_fan_out_linkage_issue = attention_markers.fan_out_linkage_issue.is_some();
        let has_fan_out_auto_apply_error = attention_markers.fan_out_auto_apply_error.is_some();
        let fan_out_auto_apply = latest_fan_out_auto_apply_summary(server, thread_id).await?;
        let fan_in_dependency_blocker =
            latest_fan_in_dependency_blocked_summary(server, thread_id).await?;
        let has_fan_in_dependency_blocked = fan_in_dependency_blocker.is_some();
        let fan_in_result_diagnostics =
            latest_fan_in_result_diagnostics_summary(server, thread_id).await?;
        let has_fan_in_result_diagnostics = fan_in_result_diagnostics.is_some();
        let pending_subagent_proxy_approvals = pending_subagent_proxy_approval_count(&events);
        let has_test_failed = attention_markers.test_failed.is_some();
        let (
            token_budget_limit,
            token_budget_remaining,
            token_budget_utilization,
            token_budget_exceeded,
            token_budget_warning_active,
        ) = thread_token_budget_snapshot_with_limit(
            state.total_tokens_used,
            configured_token_budget_limit,
            token_budget_warning_threshold_ratio(),
        );

        let attention_state = compute_attention_state(
            state.archived,
            !state.pending_approvals.is_empty(),
            !state.failed_processes.is_empty(),
            has_fan_out_auto_apply_error,
            has_fan_out_linkage_issue,
            state.active_turn_id.is_some() || !state.running_processes.is_empty(),
            state.paused,
            state.last_turn_status,
            true,
        );

        let sort_ts = updated_at
            .or(created_at)
            .map(|ts| ts.unix_timestamp_nanos());
        let mut thread_meta = omne_app_server_protocol::ThreadListMetaItem {
            thread_id,
            cwd: state.cwd,
            archived: state.archived,
            archived_at,
            archived_reason: state.archived_reason,
            approval_policy: state.approval_policy,
            sandbox_policy: state.sandbox_policy,
            model: state.model,
            openai_base_url: state.openai_base_url,
            last_seq: state.last_seq.0,
            active_turn_id: state.active_turn_id,
            active_turn_interrupt_requested: state.active_turn_interrupt_requested,
            last_turn_id: state.last_turn_id,
            last_turn_status: state.last_turn_status,
            last_turn_reason: state.last_turn_reason,
            token_budget_limit,
            token_budget_remaining,
            token_budget_utilization,
            token_budget_exceeded,
            token_budget_warning_active,
            attention_state: attention_state.to_string(),
            has_plan_ready,
            has_diff_ready,
            has_fan_out_linkage_issue,
            has_fan_out_auto_apply_error,
            fan_out_auto_apply,
            has_fan_in_dependency_blocked,
            fan_in_dependency_blocker,
            has_fan_in_result_diagnostics,
            fan_in_result_diagnostics,
            pending_subagent_proxy_approvals,
            has_test_failed,
            created_at: created_at_rfc3339,
            updated_at: updated_at_rfc3339,
            title,
            first_message,
            attention_markers: None,
        };
        if params.include_attention_markers {
            thread_meta.attention_markers = Some(attention_markers.into());
        }
        threads.push((sort_ts, thread_id, thread_meta));
    }

    threads.sort_by(|(a_ts, a_id, _), (b_ts, b_id, _)| match (a_ts, b_ts) {
        (Some(a), Some(b)) => b.cmp(a).then_with(|| a_id.cmp(b_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a_id.cmp(b_id),
    });

    Ok(omne_app_server_protocol::ThreadListMetaResponse {
        threads: threads.into_iter().map(|(_, _, value)| value).collect::<Vec<_>>(),
    })
}

pub(super) async fn handle_thread_subscribe(
    server: &Server,
    params: ThreadSubscribeParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadSubscribeResponse> {
    if let Err(err) = maybe_emit_thread_disk_warning(server, params.thread_id).await {
        tracing::debug!(
            thread_id = %params.thread_id,
            error = %err,
            "disk warning check failed"
        );
    }

    let wait_ms = params.wait_ms.unwrap_or(30_000).min(300_000);
    let poll_interval = Duration::from_millis(200);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);

    let since = EventSeq(params.since_seq);
    let mut timed_out = false;

    loop {
        let events = read_thread_events_since_or_not_found(server, params.thread_id, since).await?;

        let batch = filter_and_paginate_thread_events(
            events,
            since,
            params.kinds.as_deref(),
            params.max_events,
        );

        if !batch.events.is_empty() || wait_ms == 0 {
            return Ok(build_thread_subscribe_response(batch, false));
        }

        if tokio::time::Instant::now() >= deadline {
            timed_out = true;
        }

        if timed_out {
            return Ok(build_thread_subscribe_response(batch, true));
        }

        tokio::time::sleep(poll_interval).await;
    }
}
