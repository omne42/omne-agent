use super::*;
#[cfg(test)]
use super::attention_and_subscribe::compute_stale_processes;
use omne_git_runtime::{SnapshotKind, SnapshotRecipe, normalize_limits, recipe};

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

pub(super) async fn handle_thread_disk_usage(
    server: &Server,
    params: ThreadDiskUsageParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadDiskUsageResponse> {
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

    Ok(omne_app_server_protocol::ThreadDiskUsageResponse {
        thread_id: params.thread_id,
        thread_dir: thread_dir.display().to_string(),
        events_log_path: events_log_path.display().to_string(),
        events_log_bytes: usage.events_log_bytes,
        artifacts_bytes: usage.artifacts_bytes,
        total_bytes: usage.total_bytes,
        file_count: usage.file_count,
    })
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
    report.push_str("- Use `thread/clear_artifacts` to remove `artifacts/`.\n");
    report.push_str("- Use `thread/delete` to remove the entire thread directory (requires force=true if processes are running).\n");

    report
}

pub(super) async fn handle_thread_disk_report(
    server: &Server,
    params: ThreadDiskReportParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadDiskReportResponse> {
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
    let artifact = serde_json::from_value::<omne_app_server_protocol::ArtifactWriteResponse>(
        artifact,
    )
    .context("parse artifact/write response for thread disk report")?;

    Ok(omne_app_server_protocol::ThreadDiskReportResponse {
        thread_id: params.thread_id,
        disk_usage: omne_app_server_protocol::ThreadDiskUsageSummary {
            events_log_bytes: usage.events_log_bytes,
            artifacts_bytes: usage.artifacts_bytes,
            total_bytes: usage.total_bytes,
            file_count: usage.file_count,
        },
        artifact,
    })
}

pub(super) struct ThreadGitSnapshotSpec {
    pub(super) thread_id: ThreadId,
    pub(super) turn_id: Option<TurnId>,
    pub(super) approval_id: Option<omne_protocol::ApprovalId>,
    pub(super) max_bytes: Option<u64>,
    pub(super) wait_seconds: Option<u64>,
    pub(super) kind: SnapshotKind,
    pub(super) recipe_override: Option<SnapshotRecipe>,
}

fn thread_git_snapshot_denied_error_code(
    detail: &omne_app_server_protocol::ThreadGitSnapshotDeniedDetail,
) -> Option<String> {
    match detail {
        omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(detail) => match detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxPolicyDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxNetworkDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail) => {
                detail.error_code.clone()
            }
        },
        omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(detail) => match detail {
            omne_app_server_protocol::ThreadArtifactDeniedDetail::Denied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(detail) => {
                detail.error_code.clone()
            }
            omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(detail) => {
                detail.error_code.clone()
            }
        },
    }
}

pub(super) async fn handle_thread_git_snapshot(
    server: &Server,
    spec: ThreadGitSnapshotSpec,
) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse> {
    let limits = normalize_limits(spec.max_bytes, spec.wait_seconds);
    let snapshot_recipe = spec.recipe_override.unwrap_or_else(|| recipe(spec.kind));

    let process = handle_process_start(
        server,
        ProcessStartParams {
            thread_id: spec.thread_id,
            turn_id: spec.turn_id,
            approval_id: spec.approval_id,
            argv: snapshot_recipe.argv,
            cwd: None,
            timeout_ms: None,
        },
    )
    .await?;

    if process
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessNeedsApprovalResponse>(
            process,
        )
        .context("parse process needs_approval response for thread git snapshot")?;
        let response = omne_app_server_protocol::ThreadGitSnapshotNeedsApprovalResponse {
            needs_approval: true,
            thread_id: spec.thread_id,
            approval_id: parsed.approval_id,
        };
        return Ok(omne_app_server_protocol::ThreadGitSnapshotRpcResponse::NeedsApproval(response));
    }
    if process
        .get("denied")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let detail =
            serde_json::from_value::<omne_app_server_protocol::ThreadProcessDeniedDetail>(process)
                .context("parse process denied detail for thread git snapshot")?;
        let detail =
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(detail);
        let response = omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id: spec.thread_id,
            error_code: thread_git_snapshot_denied_error_code(&detail),
            detail,
        };
        return Ok(omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response));
    }
    if !process.get("process_id").is_some_and(|v| v.is_string()) {
        anyhow::bail!("unexpected thread git snapshot process/start response shape");
    }

    let process_id: ProcessId =
        serde_json::from_value(process["process_id"].clone()).context("parse process_id")?;
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

    let waited = tokio::time::timeout(Duration::from_secs(limits.wait_seconds), async {
        loop {
            let info = entry.info.lock().await.clone();
            if !matches!(info.status, ProcessStatus::Running) {
                return Ok::<_, anyhow::Error>(info);
            }
            tokio::time::sleep(Duration::from_millis(
                omne_git_runtime::POLL_INTERVAL_MS,
            ))
            .await;
        }
    })
    .await;

    let info = match waited {
        Ok(info) => info?,
        Err(_) => {
            let response = omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse {
                thread_id: spec.thread_id,
                process_id,
                stdout_path,
                stderr_path,
                timed_out: true,
                wait_seconds: limits.wait_seconds,
            };
            return Ok(omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(response));
        }
    };

    if info.exit_code != Some(0) {
        let (stderr_bytes, stderr_truncated) =
            read_rotating_log_prefix(
                Path::new(&stderr_path),
                omne_git_runtime::MAX_STDERR_BYTES,
            )
            .await?;
        let stderr_text = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        let stderr_suffix = if stderr_truncated { " (truncated)" } else { "" };
        anyhow::bail!(
            "{} failed (process_id={}, exit_code={:?}): {}{}",
            snapshot_recipe.summary_dirty,
            process_id,
            info.exit_code,
            stderr_text,
            stderr_suffix
        );
    }

    let (diff_bytes, truncated) =
        read_rotating_log_prefix(Path::new(&stdout_path), limits.max_bytes).await?;
    let diff_text = String::from_utf8_lossy(&diff_bytes).to_string();

    let mut summary = if diff_text.trim().is_empty() {
        snapshot_recipe.summary_clean.to_string()
    } else {
        snapshot_recipe.summary_dirty.to_string()
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
            artifact_type: snapshot_recipe.artifact_type.to_string(),
            summary,
            text: diff_text,
        },
    )
    .await?;

    if artifact
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let parsed =
            serde_json::from_value::<omne_app_server_protocol::ArtifactNeedsApprovalResponse>(
                artifact,
            )
            .context("parse artifact needs_approval response for thread git snapshot")?;
        let response = omne_app_server_protocol::ThreadGitSnapshotNeedsApprovalResponse {
            needs_approval: true,
            thread_id: spec.thread_id,
            approval_id: parsed.approval_id,
        };
        return Ok(omne_app_server_protocol::ThreadGitSnapshotRpcResponse::NeedsApproval(response));
    }
    if artifact
        .get("denied")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let detail =
            serde_json::from_value::<omne_app_server_protocol::ThreadArtifactDeniedDetail>(artifact)
                .context("parse artifact denied detail for thread git snapshot")?;
        let detail =
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(detail);
        let response = omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id: spec.thread_id,
            error_code: thread_git_snapshot_denied_error_code(&detail),
            detail,
        };
        return Ok(omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response));
    }
    let artifact = serde_json::from_value::<omne_app_server_protocol::ArtifactWriteResponse>(
        artifact,
    )
    .context("parse artifact/write response for thread git snapshot")?;

    let response = omne_app_server_protocol::ThreadGitSnapshotResponse {
        thread_id: spec.thread_id,
        process_id,
        stdout_path,
        stderr_path,
        exit_code: info.exit_code,
        truncated,
        max_bytes: limits.max_bytes,
        artifact,
    };
    Ok(omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Ok(
        Box::new(response),
    ))
}

pub(super) async fn handle_thread_diff(
    server: &Server,
    params: ThreadDiffParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse> {
    handle_thread_git_snapshot(
        server,
        ThreadGitSnapshotSpec {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            max_bytes: params.max_bytes,
            wait_seconds: params.wait_seconds,
            kind: SnapshotKind::Diff,
            recipe_override: None,
        },
    )
    .await
}

pub(super) async fn handle_thread_patch(
    server: &Server,
    params: ThreadPatchParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse> {
    handle_thread_git_snapshot(
        server,
        ThreadGitSnapshotSpec {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            max_bytes: params.max_bytes,
            wait_seconds: params.wait_seconds,
            kind: SnapshotKind::Patch,
            recipe_override: None,
        },
    )
    .await
}

async fn read_rotating_log_prefix(
    base_path: &Path,
    max_bytes: u64,
) -> anyhow::Result<(Vec<u8>, bool)> {
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

pub(super) async fn maybe_emit_thread_disk_warning(
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

    #[tokio::test]
    async fn writes_stuck_report_artifact_for_stuck_turn() -> anyhow::Result<()> {
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
                input: "test".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;

        let approval_id = omne_protocol::ApprovalId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: Some(turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({}),
            })
            .await?;

        let tool_id = omne_protocol::ToolId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: Some(turn_id),
                tool: "process/start".to_string(),
                params: None,
            })
            .await?;

        let process_id = ProcessId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: Some(turn_id),
                argv: vec!["sleep".to_string(), "999".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: tmp.path().join("stdout.log").display().to_string(),
                stderr_path: tmp.path().join("stderr.log").display().to_string(),
            })
            .await?;

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                text: "usage snapshot".to_string(),
                model: Some("gpt-5".to_string()),
                response_id: Some("resp_1".to_string()),
                token_usage: Some(serde_json::json!({
                    "total_tokens": 100,
                    "input_tokens": 80,
                    "output_tokens": 20,
                    "cache_input_tokens": 40,
                    "cache_creation_input_tokens": 10
                })),
            })
            .await?;

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: TurnStatus::Stuck,
                reason: Some("budget exceeded: steps".to_string()),
            })
            .await?;

        maybe_write_stuck_report(&server, thread_id, turn_id, Some("budget exceeded: steps"))
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

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: stuck[0].artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read: omne_app_server_protocol::ArtifactReadResponse = serde_json::from_value(read)?;
        assert!(read.text.contains("## Token usage snapshot"));
        assert!(read.text.contains("- total_tokens_used: 100"));
        assert!(read.text.contains("- input_tokens_used: 80"));
        assert!(read.text.contains("- output_tokens_used: 20"));
        assert!(read.text.contains("- cache_input_tokens_used: 40"));
        assert!(read.text.contains("- cache_creation_input_tokens_used: 10"));
        assert!(read.text.contains("- non_cache_input_tokens_used: 40"));
        assert!(read.text.contains("- cache_input_ratio: 50.00%"));
        assert!(read.text.contains("- output_ratio: 20.00%"));
        Ok(())
    }
}
