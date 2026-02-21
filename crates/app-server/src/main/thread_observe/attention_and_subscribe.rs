use omne_eventlog::ThreadState;

async fn handle_thread_attention(
    server: &Server,
    params: ThreadAttentionParams,
) -> anyhow::Result<Value> {
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
        )
    };

    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut requested = BTreeMap::<omne_protocol::ApprovalId, serde_json::Value>::new();
    let mut decided = HashSet::<omne_protocol::ApprovalId>::new();

    for event in &events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match &event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action,
                params,
            } => {
                requested.insert(
                    *approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "turn_id": turn_id,
                        "action": action,
                        "params": params,
                        "requested_at": ts,
                    }),
                );
            }
            omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                decided.insert(*approval_id);
            }
            _ => {}
        }
    }

    let pending_approvals = requested
        .into_iter()
        .filter(|(id, _)| !decided.contains(id))
        .map(|(_, v)| v)
        .collect::<Vec<_>>();

    let processes = handle_process_list(
        server,
        ProcessListParams {
            thread_id: Some(params.thread_id),
        },
    )
    .await?;

    let running_processes = processes
        .into_iter()
        .filter(|p| matches!(p.status, ProcessStatus::Running))
        .collect::<Vec<_>>();

    let stale_processes = match process_idle_window() {
        Some(idle_window) => compute_stale_processes(&running_processes, idle_window).await?,
        None => Vec::new(),
    };

    let attention_state = if !pending_approvals.is_empty() {
        "need_approval"
    } else if !failed_processes.is_empty() {
        "failed"
    } else if active_turn_id.is_some() || !running_processes.is_empty() {
        "running"
    } else if paused {
        "paused"
    } else if archived {
        "archived"
    } else {
        match last_turn_status {
            Some(omne_protocol::TurnStatus::Completed) => "done",
            Some(omne_protocol::TurnStatus::Interrupted) => "interrupted",
            Some(omne_protocol::TurnStatus::Failed) => "failed",
            Some(omne_protocol::TurnStatus::Cancelled) => "cancelled",
            Some(omne_protocol::TurnStatus::Stuck) => "stuck",
            None => "idle",
        }
    };

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "cwd": cwd,
        "archived": archived,
        "archived_at": archived_at,
        "archived_reason": archived_reason,
        "paused": paused,
        "paused_at": paused_at,
        "paused_reason": paused_reason,
        "failed_processes": failed_processes,
        "approval_policy": approval_policy,
        "sandbox_policy": sandbox_policy,
        "model": model,
        "openai_base_url": openai_base_url,
        "last_seq": last_seq,
        "active_turn_id": active_turn_id,
        "active_turn_interrupt_requested": active_turn_interrupt_requested,
        "last_turn_id": last_turn_id,
        "last_turn_status": last_turn_status,
        "last_turn_reason": last_turn_reason,
        "attention_state": attention_state,
        "pending_approvals": pending_approvals,
        "running_processes": running_processes,
        "stale_processes": stale_processes,
    }))
}

#[derive(Debug, Serialize)]
struct StaleProcessInfo {
    process_id: ProcessId,
    idle_seconds: u64,
    last_update_at: String,
    stdout_path: String,
    stderr_path: String,
}

async fn compute_stale_processes(
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

async fn maybe_write_stuck_report(
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

    Ok(md)
}

async fn handle_thread_list_meta(
    server: &Server,
    params: ThreadListMetaParams,
) -> anyhow::Result<Value> {
    let thread_ids = server.thread_store.list_threads().await?;
    let mut threads = Vec::<(Option<i128>, ThreadId, Value)>::new();

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

        let attention_state = if state.archived {
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
        };

        let sort_ts = updated_at
            .or(created_at)
            .map(|ts| ts.unix_timestamp_nanos());
        threads.push((sort_ts, thread_id, serde_json::json!({
            "thread_id": thread_id,
            "cwd": state.cwd,
            "archived": state.archived,
            "archived_at": archived_at,
            "archived_reason": state.archived_reason,
            "approval_policy": state.approval_policy,
            "sandbox_policy": state.sandbox_policy,
            "model": state.model,
            "openai_base_url": state.openai_base_url,
            "last_seq": state.last_seq.0,
            "active_turn_id": state.active_turn_id,
            "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
            "last_turn_id": state.last_turn_id,
            "last_turn_status": state.last_turn_status,
            "last_turn_reason": state.last_turn_reason,
            "attention_state": attention_state,
            "created_at": created_at_rfc3339,
            "updated_at": updated_at_rfc3339,
            "title": title,
            "first_message": first_message,
        })));
    }

    threads.sort_by(|(a_ts, a_id, _), (b_ts, b_id, _)| match (a_ts, b_ts) {
        (Some(a), Some(b)) => b.cmp(a).then_with(|| a_id.cmp(b_id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a_id.cmp(b_id),
    });

    Ok(serde_json::json!({
        "threads": threads.into_iter().map(|(_, _, value)| value).collect::<Vec<_>>(),
    }))
}

async fn handle_thread_subscribe(
    server: &Server,
    params: ThreadSubscribeParams,
) -> anyhow::Result<Value> {
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
        let mut events = server
            .thread_store
            .read_events_since(params.thread_id, since)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

        let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

        let mut has_more = false;
        if let Some(max_events) = params.max_events {
            let max_events = max_events.clamp(1, 50_000);
            if events.len() > max_events {
                events.truncate(max_events);
                has_more = true;
            }
        }

        let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

        if !events.is_empty() || wait_ms == 0 {
            return Ok(serde_json::json!({
                "events": events,
                "last_seq": last_seq,
                "thread_last_seq": thread_last_seq,
                "has_more": has_more,
                "timed_out": false,
            }));
        }

        if tokio::time::Instant::now() >= deadline {
            timed_out = true;
        }

        if timed_out {
            return Ok(serde_json::json!({
                "events": events,
                "last_seq": last_seq,
                "thread_last_seq": thread_last_seq,
                "has_more": has_more,
                "timed_out": true,
            }));
        }

        tokio::time::sleep(poll_interval).await;
    }
}

