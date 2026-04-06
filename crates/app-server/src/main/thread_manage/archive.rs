async fn collect_thread_process_entries(
    server: &Server,
    thread_id: ThreadId,
) -> Vec<(ProcessId, ProcessEntry)> {
    let entries = server.processes.lock().await;
    entries
        .iter()
        .filter(|(_, entry)| entry.thread_id == thread_id)
        .map(|(process_id, entry)| (*process_id, entry.clone()))
        .collect()
}

async fn running_thread_process_entries(
    server: &Server,
    thread_id: ThreadId,
) -> (Vec<ProcessId>, Vec<ProcessEntry>) {
    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    for (process_id, entry) in collect_thread_process_entries(server, thread_id).await {
        let info = entry.info.lock().await;
        if matches!(info.status, ProcessStatus::Running) {
            running.push(process_id);
            to_kill.push(entry.clone());
        }
    }
    (running, to_kill)
}

async fn running_turn_process_entries(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> Vec<(ProcessId, ProcessEntry)> {
    let mut running = Vec::new();
    for (process_id, entry) in collect_thread_process_entries(server, thread_id).await {
        let info = entry.info.lock().await;
        if info.turn_id == Some(turn_id) && matches!(info.status, ProcessStatus::Running) {
            running.push((process_id, entry.clone()));
        }
    }
    running
}

async fn wait_for_process_entries_to_complete(
    entries: &[ProcessEntry],
    process_ids: &[ProcessId],
    lifecycle: &'static str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let wait_all = async {
        for entry in entries {
            entry.completion.wait().await;
        }
    };
    if tokio::time::timeout(timeout, wait_all).await.is_ok() {
        return Ok(());
    }

    let remaining = entries
        .iter()
        .zip(process_ids.iter())
        .filter_map(|(entry, process_id)| (!entry.completion.is_complete()).then_some(*process_id))
        .collect::<Vec<_>>();
    anyhow::bail!(
        "timed out waiting for thread processes to stop before {lifecycle}: {:?}",
        remaining
    );
}

async fn wait_for_thread_process_entries_to_drain(
    server: &Server,
    thread_id: ThreadId,
    lifecycle: &'static str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = collect_thread_process_entries(server, thread_id).await;
        if remaining.is_empty() {
            return Ok(());
        }
        let timeout_left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout_left.is_zero() {
            let process_ids = remaining
                .iter()
                .map(|(process_id, _)| *process_id)
                .collect::<Vec<_>>();
            anyhow::bail!(
                "timed out waiting for thread processes to stop before {lifecycle}: {:?}",
                process_ids
            );
        }
        let process_ids = remaining
            .iter()
            .map(|(process_id, _)| *process_id)
            .collect::<Vec<_>>();
        let entries = remaining
            .into_iter()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>();
        wait_for_process_entries_to_complete(&entries, &process_ids, lifecycle, timeout_left).await?;
    }
}

async fn stop_turn_processes(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    reason: Option<String>,
    lifecycle: &'static str,
) -> anyhow::Result<()> {
    let running = running_turn_process_entries(server, thread_id, turn_id).await;
    if running.is_empty() {
        return Ok(());
    }

    for (_, entry) in &running {
        let _ = entry
            .cmd_tx
            .send(ProcessCommand::Interrupt {
                reason: reason.clone(),
            })
            .await;
    }

    let entries = running
        .iter()
        .map(|(_, entry)| entry.clone())
        .collect::<Vec<_>>();
    let interrupt_grace = Duration::from_secs(2);
    if tokio::time::timeout(interrupt_grace, async {
        for entry in &entries {
            entry.completion.wait().await;
        }
    })
    .await
    .is_ok()
    {
        return Ok(());
    }

    let survivors = running
        .into_iter()
        .filter(|(_, entry)| !entry.completion.is_complete())
        .collect::<Vec<_>>();
    for (_, entry) in &survivors {
        let _ = entry
            .cmd_tx
            .send(ProcessCommand::Kill {
                reason: reason.clone(),
            })
            .await;
    }
    let survivor_ids = survivors
        .iter()
        .map(|(process_id, _)| *process_id)
        .collect::<Vec<_>>();
    let survivor_entries = survivors
        .into_iter()
        .map(|(_, entry)| entry)
        .collect::<Vec<_>>();
    wait_for_process_entries_to_complete(
        &survivor_entries,
        &survivor_ids,
        lifecycle,
        Duration::from_secs(10),
    )
    .await
}

async fn handle_thread_archive(
    server: &Server,
    params: ThreadArchiveParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadArchiveResponse> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let (already_archived, active_turn_id, thread_cwd) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.archived, state.active_turn_id, state.cwd.clone())
    };

    if already_archived {
        return Ok(omne_app_server_protocol::ThreadArchiveResponse {
            thread_id: params.thread_id,
            archived: true,
            already_archived: true,
            force: None,
            killed_processes: None,
            auto_hook: None,
        });
    }

    let reason = params
        .reason
        .clone()
        .or_else(|| Some("thread archived".to_string()));

    if let Some(turn_id) = active_turn_id {
        if !params.force {
            anyhow::bail!(
                "refusing to archive thread with active turn (use force=true): turn_id={}",
                turn_id
            );
        }

        let _ = thread_rt
            .interrupt_turn(turn_id, reason.clone())
            .await
            .context("interrupt active turn");
        stop_turn_processes(
            server,
            params.thread_id,
            turn_id,
            reason.clone(),
            "archive active turn",
        )
        .await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let done = {
                let handle = thread_rt.handle.lock().await;
                handle.state().active_turn_id.is_none()
            };
            if done {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                thread_rt
                    .force_complete_turn(
                        std::sync::Arc::new(server.clone()),
                        turn_id,
                        omne_protocol::TurnStatus::Interrupted,
                        reason.clone(),
                    )
                    .await;
                let handle = thread_rt.handle.lock().await;
                if handle.state().active_turn_id.is_none() {
                    break;
                }
                anyhow::bail!(
                    "timed out waiting for active turn to stop before archive: turn_id={}",
                    turn_id
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    let (running, to_kill) = running_thread_process_entries(server, params.thread_id).await;

    if !running.is_empty() && !params.force {
        anyhow::bail!(
            "refusing to archive thread with running processes (use force=true): {:?}",
            running
        );
    }

    if params.force {
        for entry in to_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: reason.clone(),
                })
                .await;
        }
    }

    let _ = remove_mcp_connections_for_thread(server, params.thread_id).await;

    wait_for_thread_process_entries_to_drain(
        server,
        params.thread_id,
        "archive",
        Duration::from_secs(10),
    )
    .await?;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ThreadArchived {
            reason: reason.clone(),
        })
        .await?;
    let auto_hook = run_auto_workspace_hook(server, params.thread_id, WorkspaceHookName::Archive).await;
    cleanup_managed_subagent_worktree(
        server,
        params.thread_id,
        thread_cwd.as_deref(),
        "thread/archive",
    )
    .await;

    Ok(omne_app_server_protocol::ThreadArchiveResponse {
        thread_id: params.thread_id,
        archived: true,
        already_archived: false,
        force: Some(params.force),
        killed_processes: Some(running),
        auto_hook: Some(auto_hook),
    })
}

async fn handle_thread_unarchive(
    server: &Server,
    params: ThreadUnarchiveParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadUnarchiveResponse> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let already_unarchived = {
        let handle = thread_rt.handle.lock().await;
        !handle.state().archived
    };

    if already_unarchived {
        return Ok(omne_app_server_protocol::ThreadUnarchiveResponse {
            thread_id: params.thread_id,
            archived: false,
            already_unarchived: true,
        });
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ThreadUnarchived {
            reason: params.reason,
        })
        .await?;

    Ok(omne_app_server_protocol::ThreadUnarchiveResponse {
        thread_id: params.thread_id,
        archived: false,
        already_unarchived: false,
    })
}
