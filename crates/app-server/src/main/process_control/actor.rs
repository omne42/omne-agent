struct ProcessActorArgs {
    server: Arc<Server>,
    thread_rt: Arc<ThreadRuntime>,
    process_id: ProcessId,
    child: tokio::process::Child,
    cmd_rx: mpsc::Receiver<ProcessCommand>,
    stdout_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    stderr_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    execve_gate: Option<ExecveGateHandle>,
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
    cargo_target_dir: Option<PathBuf>,
}

async fn run_process_actor(args: ProcessActorArgs) {
    let ProcessActorArgs {
        server,
        thread_rt,
        process_id,
        mut child,
        mut cmd_rx,
        stdout_task,
        stderr_task,
        mut execve_gate,
        info,
        cargo_target_dir,
    } = args;
    fn try_send_interrupt(child: &tokio::process::Child) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            let Some(pid) = child.id() else {
                anyhow::bail!("process has no pid");
            };
            kill(Pid::from_raw(pid as i32), Signal::SIGINT)
                .with_context(|| format!("send SIGINT to pid {pid}"))?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            anyhow::bail!("process interrupt is not supported on this platform")
        }
    }

    let mut interrupt_reason: Option<String> = None;
    let mut interrupt_logged = false;
    let mut kill_reason: Option<String> = None;
    let mut kill_logged = false;

    async fn cleanup_execve_gate(execve_gate: &mut Option<ExecveGateHandle>) {
        if let Some(gate) = execve_gate.take() {
            shutdown_execve_gate(gate).await;
        }
    }

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { /* sender dropped */ cleanup_execve_gate(&mut execve_gate).await; return; };
                match cmd {
                    ProcessCommand::Interrupt { reason } => {
                        if interrupt_reason.is_none() {
                            interrupt_reason = reason;
                        }
                        if !interrupt_logged {
                            let _ = thread_rt
                                .append_event(omne_agent_protocol::ThreadEventKind::ProcessInterruptRequested {
                                    process_id,
                                    reason: interrupt_reason.clone(),
                                })
                                .await;
                            interrupt_logged = true;
                        }
                        if try_send_interrupt(&child).is_err() {
                            let _ = child.start_kill();
                        }
                    }
                    ProcessCommand::Kill { reason } => {
                        if kill_reason.is_none() {
                            kill_reason = reason;
                        }
                        if !kill_logged {
                            let _ = thread_rt.append_event(omne_agent_protocol::ThreadEventKind::ProcessKillRequested {
                                process_id,
                                reason: kill_reason.clone(),
                            }).await;
                            kill_logged = true;
                        }
                        let _ = child.start_kill();
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(task) = stdout_task {
                    let _ = task.await;
                }
                if let Some(task) = stderr_task {
                    let _ = task.await;
                }

                let thread_id = { info.lock().await.thread_id };

                let exit_code = status.code();
                let exited = thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ProcessExited {
                        process_id,
                        exit_code,
                        reason: kill_reason.clone().or_else(|| interrupt_reason.clone()),
                    })
                    .await;

                if let Ok(event) = exited {
                    if let Ok(ts) = event.timestamp.format(&Rfc3339) {
                        let mut info = info.lock().await;
                        info.status = ProcessStatus::Exited;
                        info.exit_code = exit_code;
                        info.last_update_at = ts;
                    }
                }
                cleanup_execve_gate(&mut execve_gate).await;

                if let Some(target_dir) = cargo_target_dir {
                    let server = server.clone();
                    tokio::spawn(async move {
                        if let Err(err) =
                            maybe_emit_cargo_target_warning(server, thread_id, target_dir).await
                        {
                            tracing::debug!(
                                thread_id = %thread_id,
                                error = %err,
                                "cargo target warning check failed"
                            );
                        }
                    });
                }

                return;
            }
            Ok(None) => {}
            Err(_) => {
                cleanup_execve_gate(&mut execve_gate).await;
                return;
            }
        }
    }
}

#[derive(Debug)]
struct DirDiskUsage {
    total_bytes: u64,
    file_count: usize,
    top_files: Vec<(u64, String)>,
}

fn scan_dir_disk_usage(dir: &Path, top_n: usize) -> anyhow::Result<DirDiskUsage> {
    let mut total_bytes = 0u64;
    let mut file_count = 0usize;
    let mut top_files: Vec<(u64, String)> = Vec::new();

    for entry in WalkDir::new(dir)
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

        if top_n == 0 {
            continue;
        }

        let rel = entry.path().strip_prefix(dir).unwrap_or(entry.path());
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

    Ok(DirDiskUsage {
        total_bytes,
        file_count,
        top_files,
    })
}

fn build_cargo_target_report_markdown(
    target_dir: &Path,
    generated_at: &str,
    warning_threshold_bytes: u64,
    usage: &DirDiskUsage,
) -> String {
    let mut report = String::new();
    report.push_str("# Cargo target dir usage report\n\n");
    report.push_str(&format!("- generated_at: {generated_at}\n"));
    report.push_str(&format!(
        "- warning_threshold_bytes: {warning_threshold_bytes}\n"
    ));
    report.push_str(&format!("- cargo_target_dir: {}\n", target_dir.display()));
    report.push_str(&format!("- total_bytes: {}\n", usage.total_bytes));
    report.push_str(&format!("- file_count: {}\n", usage.file_count));

    if !usage.top_files.is_empty() {
        report.push_str("\n## Top files\n");
        for (size, rel) in &usage.top_files {
            report.push_str(&format!("- {}  {}\n", size, rel));
        }
    }

    report.push_str("\n## Cleanup\n");
    report.push_str("- Consider cleaning the shared target dir:\n");
    report.push_str("  - `cargo clean --target-dir <cargo_target_dir>`\n");
    report.push_str("  - or `CARGO_TARGET_DIR=<cargo_target_dir> cargo clean`\n");
    report.push_str("- If you rely on this cache across sessions, clean during idle windows.\n");

    report
}

async fn maybe_emit_cargo_target_warning(
    server: Arc<Server>,
    thread_id: ThreadId,
    cargo_target_dir: PathBuf,
) -> anyhow::Result<()> {
    let Some(threshold_bytes) = cargo_target_warning_threshold_bytes() else {
        return Ok(());
    };

    let check_debounce = cargo_target_check_debounce();
    let report_debounce = cargo_target_report_debounce();
    let now = tokio::time::Instant::now();
    let key = cargo_target_dir.display().to_string();

    {
        let mut warning = server.cargo_target_warning.lock().await;
        let state = warning
            .entry(key.clone())
            .or_insert_with(|| CargoTargetWarningState {
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

    match tokio::fs::metadata(&cargo_target_dir).await {
        Ok(meta) if meta.is_dir() => {}
        _ => return Ok(()),
    }

    let cargo_target_dir_for_task = cargo_target_dir.clone();
    let usage = tokio::task::spawn_blocking(move || scan_dir_disk_usage(&cargo_target_dir_for_task, 40))
        .await
        .context("join cargo target scan task")??;

    if usage.total_bytes < threshold_bytes {
        return Ok(());
    }

    {
        let mut warning = server.cargo_target_warning.lock().await;
        let state = warning
            .entry(key)
            .or_insert_with(|| CargoTargetWarningState {
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
    let report = build_cargo_target_report_markdown(
        &cargo_target_dir,
        &generated_at,
        threshold_bytes,
        &usage,
    );

    let _artifact = handle_artifact_write(
        &server,
        ArtifactWriteParams {
            thread_id,
            turn_id: None,
            approval_id: None,
            artifact_id: None,
            artifact_type: "cargo_target_report".to_string(),
            summary: "Cargo target dir usage report (warning)".to_string(),
            text: report,
        },
    )
    .await?;

    Ok(())
}
