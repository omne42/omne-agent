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
