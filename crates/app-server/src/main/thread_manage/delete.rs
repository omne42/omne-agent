async fn handle_thread_delete(
    server: &Server,
    params: ThreadDeleteParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadDeleteResponse> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let (thread_cwd, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.cwd.clone(), state.active_turn_id)
    };
    let reason = Some("thread deleted".to_string());

    if let Some(turn_id) = active_turn_id {
        if !params.force {
            anyhow::bail!(
                "refusing to delete thread with active turn (use force=true): turn_id={}",
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
                    "timed out waiting for active turn to stop before delete: turn_id={}",
                    turn_id
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    let (running, to_kill) = running_thread_process_entries(server, params.thread_id).await;

    if !running.is_empty() && !params.force {
        anyhow::bail!(
            "refusing to delete thread with running processes (use force=true): {:?}",
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
        "delete",
        Duration::from_secs(10),
    )
    .await?;
    cleanup_managed_subagent_worktree(
        server,
        params.thread_id,
        thread_cwd.as_deref(),
        "thread/delete",
    )
    .await;

    let deleted = match tokio::fs::remove_dir_all(&thread_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", thread_dir.display())),
    };

    server.evict_cached_thread(params.thread_id).await;
    server
        .processes
        .lock()
        .await
        .retain(|_, entry| entry.thread_id != params.thread_id);

    Ok(omne_app_server_protocol::ThreadDeleteResponse {
        thread_id: params.thread_id,
        deleted,
        thread_dir: thread_dir.display().to_string(),
    })
}
