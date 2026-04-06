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

async fn wait_for_thread_process_entries_to_drain(
    server: &Server,
    thread_id: ThreadId,
    lifecycle: &'static str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .filter_map(|(process_id, entry)| {
                    (entry.thread_id == thread_id).then_some(*process_id)
                })
                .collect::<Vec<_>>()
        };
        if remaining.is_empty() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "timed out waiting for thread processes to stop before {lifecycle}: {:?}",
                remaining
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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
        interrupt_processes_for_turn(server, params.thread_id, turn_id, reason.clone()).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        kill_processes_for_turn(server, params.thread_id, turn_id, reason.clone()).await;

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
