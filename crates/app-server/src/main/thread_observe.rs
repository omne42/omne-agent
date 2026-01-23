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

    let mut requested = BTreeMap::<pm_protocol::ApprovalId, serde_json::Value>::new();
    let mut decided = HashSet::<pm_protocol::ApprovalId>::new();

    for event in &events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match &event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
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
            pm_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
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
            Some(pm_protocol::TurnStatus::Completed) => "done",
            Some(pm_protocol::TurnStatus::Interrupted) => "interrupted",
            Some(pm_protocol::TurnStatus::Failed) => "failed",
            Some(pm_protocol::TurnStatus::Cancelled) => "cancelled",
            Some(pm_protocol::TurnStatus::Stuck) => "stuck",
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
        return Some("CODE_PM_AGENT_MAX_STEPS");
    }
    if reason.contains("budget exceeded: tool_calls") {
        return Some("CODE_PM_AGENT_MAX_TOOL_CALLS");
    }
    if reason.contains("budget exceeded: turn_seconds") {
        return Some("CODE_PM_AGENT_MAX_TURN_SECONDS");
    }
    if reason.contains("openai request timed out") {
        return Some("CODE_PM_AGENT_MAX_OPENAI_REQUEST_SECONDS");
    }
    if reason.contains("token budget exceeded:") {
        return Some("CODE_PM_AGENT_MAX_TOTAL_TOKENS");
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

    let mut last_approval_in_turn: Option<(pm_protocol::ApprovalId, String)> = None;
    let mut last_approval_any: Option<(pm_protocol::ApprovalId, String)> = None;
    let mut last_tool_in_turn: Option<(pm_protocol::ToolId, String)> = None;
    let mut last_tool_any: Option<(pm_protocol::ToolId, String)> = None;
    let mut started_processes = Vec::<ProcessStartInfo>::new();
    let mut exited = HashSet::<ProcessId>::new();

    for event in &events {
        match &event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
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
            pm_protocol::ThreadEventKind::ToolStarted {
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
            pm_protocol::ThreadEventKind::ProcessStarted {
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
            pm_protocol::ThreadEventKind::ProcessExited { process_id, .. } => {
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
    md.push_str(&format!("- pm thread attention {thread_id}\n"));
    md.push_str(&format!("- pm approval list {thread_id}\n"));
    md.push_str(&format!("- pm process list --thread-id {thread_id}\n"));
    if let Some(process) = &process {
        md.push_str(&format!("- pm process tail {}\n", process.process_id));
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
    let mut threads = Vec::<Value>::new();

    for thread_id in thread_ids {
        let Some(state) = server.thread_store.read_state(thread_id).await? else {
            continue;
        };

        if state.archived && !params.include_archived {
            continue;
        }

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
                Some(pm_protocol::TurnStatus::Completed) => "done",
                Some(pm_protocol::TurnStatus::Interrupted) => "interrupted",
                Some(pm_protocol::TurnStatus::Failed) => "failed",
                Some(pm_protocol::TurnStatus::Cancelled) => "cancelled",
                Some(pm_protocol::TurnStatus::Stuck) => "stuck",
                None => "idle",
            }
        };

        threads.push(serde_json::json!({
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
        }));
    }

    Ok(serde_json::json!({ "threads": threads }))
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

#[derive(Debug)]
struct ThreadDiskUsage {
    total_bytes: u64,
    events_log_bytes: u64,
    artifacts_bytes: u64,
    file_count: usize,
    top_files: Vec<(u64, String)>,
}

fn scan_thread_disk_usage(
    thread_dir: &Path,
    events_log_path: &Path,
    top_n: usize,
) -> anyhow::Result<ThreadDiskUsage> {
    let artifacts_dir = thread_dir.join("artifacts");

    let events_log_bytes = std::fs::metadata(events_log_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let mut total_bytes = 0u64;
    let mut artifacts_bytes = 0u64;
    let mut file_count = 0usize;
    let mut top_files: Vec<(u64, String)> = Vec::new();

    for entry in WalkDir::new(thread_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_type().is_symlink())
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let meta = entry.metadata()?;
        let size = meta.len();
        file_count += 1;
        total_bytes = total_bytes.saturating_add(size);
        if entry.path().starts_with(&artifacts_dir) {
            artifacts_bytes = artifacts_bytes.saturating_add(size);
        }

        if top_n == 0 {
            continue;
        }

        let rel = entry
            .path()
            .strip_prefix(thread_dir)
            .unwrap_or(entry.path());
        let rel = rel.to_string_lossy().to_string();

        if top_files.len() < top_n {
            top_files.push((size, rel));
            top_files.sort_by_key(|(b, _)| *b);
            continue;
        }
        if let Some((smallest, _)) = top_files.first() {
            if size > *smallest {
                top_files[0] = (size, rel);
                top_files.sort_by_key(|(b, _)| *b);
            }
        }
    }

    top_files.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(ThreadDiskUsage {
        total_bytes,
        events_log_bytes,
        artifacts_bytes,
        file_count,
        top_files,
    })
}

async fn handle_thread_disk_usage(
    server: &Server,
    params: ThreadDiskUsageParams,
) -> anyhow::Result<Value> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let events_log_path = server.thread_store.events_log_path(params.thread_id);

    match tokio::fs::metadata(&thread_dir).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => anyhow::bail!("thread dir is not a directory: {}", thread_dir.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("thread not found: {}", params.thread_id)
        }
        Err(err) => return Err(err).with_context(|| format!("stat {}", thread_dir.display())),
    }

    let thread_dir_for_task = thread_dir.clone();
    let events_log_path_for_task = events_log_path.clone();
    let usage = tokio::task::spawn_blocking(move || {
        scan_thread_disk_usage(&thread_dir_for_task, &events_log_path_for_task, 0)
    })
    .await
    .context("join disk usage task")??;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "thread_dir": thread_dir.display().to_string(),
        "events_log_path": events_log_path.display().to_string(),
        "events_log_bytes": usage.events_log_bytes,
        "artifacts_bytes": usage.artifacts_bytes,
        "total_bytes": usage.total_bytes,
        "file_count": usage.file_count,
    }))
}

fn build_thread_disk_report_markdown(
    thread_id: ThreadId,
    generated_at: &str,
    warning_threshold_bytes: Option<u64>,
    thread_dir: &Path,
    events_log_path: &Path,
    usage: &ThreadDiskUsage,
) -> String {
    let mut report = String::new();
    report.push_str("# Thread disk usage report\n\n");
    report.push_str(&format!("- thread_id: {thread_id}\n"));
    report.push_str(&format!("- generated_at: {generated_at}\n"));
    if let Some(threshold) = warning_threshold_bytes {
        report.push_str(&format!("- warning_threshold_bytes: {threshold}\n"));
    }
    report.push_str(&format!("- thread_dir: {}\n", thread_dir.display()));
    report.push_str(&format!(
        "- events_log_path: {}\n",
        events_log_path.display()
    ));
    report.push_str(&format!("- total_bytes: {}\n", usage.total_bytes));
    report.push_str(&format!("- artifacts_bytes: {}\n", usage.artifacts_bytes));
    report.push_str(&format!("- events_log_bytes: {}\n", usage.events_log_bytes));
    report.push_str(&format!("- file_count: {}\n", usage.file_count));

    if !usage.top_files.is_empty() {
        report.push_str("\n## Top files\n");
        for (size, rel) in &usage.top_files {
            report.push_str(&format!("- {}  {}\n", size, rel));
        }
    }

    report.push_str("\n## Cleanup\n");
    report.push_str("- Use `thread/clear_artifacts` to remove `artifacts/` (requires force=true if processes are running).\n");
    report.push_str("- Use `thread/delete` to remove the entire thread directory (requires force=true if processes are running).\n");

    report
}

async fn handle_thread_disk_report(
    server: &Server,
    params: ThreadDiskReportParams,
) -> anyhow::Result<Value> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let events_log_path = server.thread_store.events_log_path(params.thread_id);

    match tokio::fs::metadata(&thread_dir).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => anyhow::bail!("thread dir is not a directory: {}", thread_dir.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("thread not found: {}", params.thread_id)
        }
        Err(err) => return Err(err).with_context(|| format!("stat {}", thread_dir.display())),
    }

    let top_n = params.top_files.unwrap_or(40).min(200);
    let thread_dir_for_task = thread_dir.clone();
    let events_log_path_for_task = events_log_path.clone();
    let usage = tokio::task::spawn_blocking(move || {
        scan_thread_disk_usage(&thread_dir_for_task, &events_log_path_for_task, top_n)
    })
    .await
    .context("join disk report task")??;

    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let report = build_thread_disk_report_markdown(
        params.thread_id,
        &now,
        None,
        &thread_dir,
        &events_log_path,
        &usage,
    );

    let artifact = handle_artifact_write(
        server,
        ArtifactWriteParams {
            thread_id: params.thread_id,
            turn_id: None,
            approval_id: None,
            artifact_id: None,
            artifact_type: "disk_report".to_string(),
            summary: "Thread disk usage report".to_string(),
            text: report,
        },
    )
    .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "disk_usage": {
            "events_log_bytes": usage.events_log_bytes,
            "artifacts_bytes": usage.artifacts_bytes,
            "total_bytes": usage.total_bytes,
            "file_count": usage.file_count,
        },
        "artifact": artifact,
    }))
}

const DEFAULT_THREAD_DIFF_MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_THREAD_DIFF_MAX_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_THREAD_DIFF_WAIT_SECONDS: u64 = 30;
const MAX_THREAD_DIFF_WAIT_SECONDS: u64 = 10 * 60;
const THREAD_DIFF_POLL_INTERVAL_MS: u64 = 50;
const THREAD_DIFF_MAX_STDERR_BYTES: u64 = 32 * 1024;

struct ThreadGitSnapshotSpec {
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<pm_protocol::ApprovalId>,
    max_bytes: Option<u64>,
    wait_seconds: Option<u64>,
    argv: Vec<String>,
    artifact_type: &'static str,
    summary_clean: &'static str,
    summary_dirty: &'static str,
}

async fn handle_thread_git_snapshot(
    server: &Server,
    spec: ThreadGitSnapshotSpec,
) -> anyhow::Result<Value> {
    let max_bytes = spec
        .max_bytes
        .unwrap_or(DEFAULT_THREAD_DIFF_MAX_BYTES)
        .min(MAX_THREAD_DIFF_MAX_BYTES);
    let wait_seconds = spec
        .wait_seconds
        .unwrap_or(DEFAULT_THREAD_DIFF_WAIT_SECONDS)
        .min(MAX_THREAD_DIFF_WAIT_SECONDS);

    let process = handle_process_start(
        server,
        ProcessStartParams {
            thread_id: spec.thread_id,
            turn_id: spec.turn_id,
            approval_id: spec.approval_id,
            argv: spec.argv,
            cwd: None,
        },
    )
    .await?;

    if !process.get("process_id").is_some_and(|v| v.is_string()) {
        return Ok(process);
    }

    let process_id: ProcessId = serde_json::from_value(process["process_id"].clone())
        .context("parse process_id")?;
    let stdout_path = process["stdout_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing stdout_path"))?
        .to_string();
    let stderr_path = process["stderr_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing stderr_path"))?
        .to_string();

    let entry = {
        let processes = server.processes.lock().await;
        processes
            .get(&process_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("process not found: {}", process_id))?
    };

    let waited = tokio::time::timeout(Duration::from_secs(wait_seconds), async {
        loop {
            let info = entry.info.lock().await.clone();
            if !matches!(info.status, ProcessStatus::Running) {
                return Ok::<_, anyhow::Error>(info);
            }
            tokio::time::sleep(Duration::from_millis(THREAD_DIFF_POLL_INTERVAL_MS)).await;
        }
    })
    .await;

    let info = match waited {
        Ok(info) => info?,
        Err(_) => {
            return Ok(serde_json::json!({
                "thread_id": spec.thread_id,
                "process_id": process_id,
                "stdout_path": stdout_path,
                "stderr_path": stderr_path,
                "timed_out": true,
                "wait_seconds": wait_seconds,
            }));
        }
    };

    if info.exit_code != Some(0) {
        let (stderr_bytes, stderr_truncated) = read_rotating_log_prefix(
            Path::new(&stderr_path),
            THREAD_DIFF_MAX_STDERR_BYTES,
        )
        .await?;
        let stderr_text = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        let stderr_suffix = if stderr_truncated { " (truncated)" } else { "" };
        anyhow::bail!(
            "{} failed (process_id={}, exit_code={:?}): {}{}",
            spec.summary_dirty,
            process_id,
            info.exit_code,
            stderr_text,
            stderr_suffix
        );
    }

    let (diff_bytes, truncated) = read_rotating_log_prefix(Path::new(&stdout_path), max_bytes).await?;
    let diff_text = String::from_utf8_lossy(&diff_bytes).to_string();

    let mut summary = if diff_text.trim().is_empty() {
        spec.summary_clean.to_string()
    } else {
        spec.summary_dirty.to_string()
    };
    if truncated {
        summary.push_str(" (truncated)");
    }

    let artifact = handle_artifact_write(
        server,
        ArtifactWriteParams {
            thread_id: spec.thread_id,
            turn_id: spec.turn_id,
            approval_id: None,
            artifact_id: None,
            artifact_type: spec.artifact_type.to_string(),
            summary,
            text: diff_text,
        },
    )
    .await?;

    if artifact.get("needs_approval").is_some() || artifact.get("denied").is_some() {
        return Ok(artifact);
    }

    Ok(serde_json::json!({
        "thread_id": spec.thread_id,
        "process_id": process_id,
        "stdout_path": stdout_path,
        "stderr_path": stderr_path,
        "exit_code": info.exit_code,
        "truncated": truncated,
        "max_bytes": max_bytes,
        "artifact": artifact,
    }))
}

async fn handle_thread_diff(server: &Server, params: ThreadDiffParams) -> anyhow::Result<Value> {
    handle_thread_git_snapshot(
        server,
        ThreadGitSnapshotSpec {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            max_bytes: params.max_bytes,
            wait_seconds: params.wait_seconds,
            argv: vec![
                "git".to_string(),
                "--no-pager".to_string(),
                "diff".to_string(),
                "--no-ext-diff".to_string(),
                "--no-textconv".to_string(),
                "--no-color".to_string(),
            ],
            artifact_type: "diff",
            summary_clean: "git diff (clean)",
            summary_dirty: "git diff",
        },
    )
    .await
}

async fn handle_thread_patch(server: &Server, params: ThreadPatchParams) -> anyhow::Result<Value> {
    handle_thread_git_snapshot(
        server,
        ThreadGitSnapshotSpec {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            max_bytes: params.max_bytes,
            wait_seconds: params.wait_seconds,
            argv: vec![
                "git".to_string(),
                "--no-pager".to_string(),
                "diff".to_string(),
                "--no-ext-diff".to_string(),
                "--no-textconv".to_string(),
                "--no-color".to_string(),
                "--binary".to_string(),
                "--patch".to_string(),
            ],
            artifact_type: "patch",
            summary_clean: "git patch (clean)",
            summary_dirty: "git patch",
        },
    )
    .await
}

async fn read_rotating_log_prefix(base_path: &Path, max_bytes: u64) -> anyhow::Result<(Vec<u8>, bool)> {
    let files = list_rotating_log_files(base_path).await?;
    if files.is_empty() {
        return Ok((Vec::new(), false));
    }

    let mut out = Vec::new();
    let mut remaining = max_bytes as usize;
    let mut truncated = false;

    for file_path in files {
        if remaining == 0 {
            truncated = true;
            break;
        }

        let len = match tokio::fs::metadata(&file_path).await {
            Ok(meta) => usize::try_from(meta.len()).unwrap_or(usize::MAX),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("stat {}", file_path.display())),
        };

        if len > remaining {
            truncated = true;
        }

        let mut file = tokio::fs::File::open(&file_path)
            .await
            .with_context(|| format!("open {}", file_path.display()))?;
        let mut buf = vec![0u8; remaining.min(8192)];
        while remaining > 0 {
            let read_len = buf.len().min(remaining);
            let n = file
                .read(&mut buf[..read_len])
                .await
                .with_context(|| format!("read {}", file_path.display()))?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            remaining = remaining.saturating_sub(n);
        }

        if truncated {
            break;
        }
    }

    Ok((out, truncated))
}

async fn maybe_emit_thread_disk_warning(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<()> {
    let Some(threshold_bytes) = thread_disk_warning_threshold_bytes() else {
        return Ok(());
    };
    let check_debounce = thread_disk_check_debounce();
    let report_debounce = thread_disk_report_debounce();
    let now = tokio::time::Instant::now();

    {
        let mut disk_warning = server.disk_warning.lock().await;
        let state = disk_warning
            .entry(thread_id)
            .or_insert_with(|| DiskWarningState {
                last_checked_at: None,
                last_reported_at: None,
            });
        if let Some(last) = state.last_checked_at
            && now.duration_since(last) < check_debounce
        {
            return Ok(());
        }
        state.last_checked_at = Some(now);
    }

    let thread_dir = server.thread_store.thread_dir(thread_id);
    let events_log_path = server.thread_store.events_log_path(thread_id);

    match tokio::fs::metadata(&thread_dir).await {
        Ok(meta) if meta.is_dir() => {}
        _ => return Ok(()),
    }

    let thread_dir_for_task = thread_dir.clone();
    let events_log_path_for_task = events_log_path.clone();
    let usage = tokio::task::spawn_blocking(move || {
        scan_thread_disk_usage(&thread_dir_for_task, &events_log_path_for_task, 40)
    })
    .await
    .context("join disk warning scan task")??;

    if usage.total_bytes < threshold_bytes {
        return Ok(());
    }

    {
        let mut disk_warning = server.disk_warning.lock().await;
        let state = disk_warning
            .entry(thread_id)
            .or_insert_with(|| DiskWarningState {
                last_checked_at: Some(now),
                last_reported_at: None,
            });

        if let Some(last) = state.last_reported_at
            && now.duration_since(last) < report_debounce
        {
            return Ok(());
        }
        state.last_reported_at = Some(now);
    }

    let generated_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let report = build_thread_disk_report_markdown(
        thread_id,
        &generated_at,
        Some(threshold_bytes),
        &thread_dir,
        &events_log_path,
        &usage,
    );

    let _artifact = handle_artifact_write(
        server,
        ArtifactWriteParams {
            thread_id,
            turn_id: None,
            approval_id: None,
            artifact_id: None,
            artifact_type: "disk_report".to_string(),
            summary: "Thread disk usage report (warning)".to_string(),
            text: report,
        },
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod stale_process_tests {
    use super::*;

    #[tokio::test]
    async fn stale_processes_use_started_at_when_no_logs_exist() -> anyhow::Result<()> {
        let now = OffsetDateTime::now_utc();
        let started_at = (now - time::Duration::hours(1)).format(&Rfc3339)?;

        let tmp = tempfile::tempdir()?;
        let missing_stdout = tmp.path().join("missing_stdout.log");
        let missing_stderr = tmp.path().join("missing_stderr.log");

        let process = ProcessInfo {
            process_id: ProcessId::new(),
            thread_id: ThreadId::new(),
            turn_id: None,
            argv: vec!["sleep".to_string(), "999".to_string()],
            cwd: tmp.path().display().to_string(),
            started_at: started_at.clone(),
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: missing_stdout.display().to_string(),
            stderr_path: missing_stderr.display().to_string(),
            last_update_at: started_at.clone(),
        };

        let stale = compute_stale_processes(&[process], Duration::from_secs(300)).await?;
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].last_update_at, started_at);
        assert!(stale[0].idle_seconds >= 3590);
        Ok(())
    }

    #[tokio::test]
    async fn stale_processes_ignore_running_processes_with_recent_output() -> anyhow::Result<()> {
        let now = OffsetDateTime::now_utc();
        let started_at = (now - time::Duration::hours(1)).format(&Rfc3339)?;

        let tmp = tempfile::tempdir()?;
        let stdout_path = tmp.path().join("stdout.log");
        let stderr_path = tmp.path().join("stderr.log");
        tokio::fs::write(&stdout_path, "hello\n").await?;
        tokio::fs::write(&stderr_path, "world\n").await?;

        let process = ProcessInfo {
            process_id: ProcessId::new(),
            thread_id: ThreadId::new(),
            turn_id: None,
            argv: vec!["echo".to_string(), "hi".to_string()],
            cwd: tmp.path().display().to_string(),
            started_at,
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
            last_update_at: now.format(&Rfc3339)?,
        };

        let stale = compute_stale_processes(&[process], Duration::from_secs(60)).await?;
        assert!(stale.is_empty());
        Ok(())
    }
}

#[cfg(test)]
mod stuck_report_tests {
    use super::*;

    fn build_test_server(pm_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: pm_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    #[tokio::test]
    async fn writes_stuck_report_artifact_for_stuck_turn() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "test".to_string(),
                context_refs: None,
            })
            .await?;

        let approval_id = pm_protocol::ApprovalId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: Some(turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({}),
            })
            .await?;

        let tool_id = pm_protocol::ToolId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: Some(turn_id),
                tool: "process/start".to_string(),
                params: None,
            })
            .await?;

        let process_id = ProcessId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: Some(turn_id),
                argv: vec!["sleep".to_string(), "999".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: tmp.path().join("stdout.log").display().to_string(),
                stderr_path: tmp.path().join("stderr.log").display().to_string(),
            })
            .await?;

        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: TurnStatus::Stuck,
                reason: Some("budget exceeded: steps".to_string()),
            })
            .await?;

        maybe_write_stuck_report(
            &server,
            thread_id,
            turn_id,
            Some("budget exceeded: steps"),
        )
        .await?;

        let value = handle_artifact_list(
            &server,
            ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;

        let artifacts: Vec<ArtifactMetadata> = serde_json::from_value(value["artifacts"].clone())?;
        let stuck = artifacts
            .iter()
            .filter(|meta| meta.artifact_type == "stuck_report")
            .filter(|meta| {
                meta.provenance
                    .as_ref()
                    .and_then(|p| p.turn_id)
                    .is_some_and(|id| id == turn_id)
            })
            .collect::<Vec<_>>();
        assert_eq!(stuck.len(), 1);
        Ok(())
    }
}
