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
                anyhow::bail!(
                    "timed out waiting for active turn to stop before delete: turn_id={}",
                    turn_id
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    {
        let entries = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .map(|(process_id, entry)| (*process_id, entry.clone()))
                .collect::<Vec<_>>()
        };
        for (process_id, entry) in entries {
            let info = entry.info.lock().await;
            if info.thread_id != params.thread_id {
                continue;
            }
            if matches!(info.status, ProcessStatus::Running) {
                running.push(process_id);
                to_kill.push(entry.clone());
            }
        }
    }

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

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let has_running = {
                let entries = server.processes.lock().await;
                entries.values().any(|entry| {
                    if let Ok(info) = entry.info.try_lock() {
                        info.thread_id == params.thread_id
                            && matches!(info.status, ProcessStatus::Running)
                    } else {
                        true
                    }
                })
            };
            if !has_running {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for thread processes to stop before delete: {:?}",
                    running
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    let _ = remove_mcp_connections_for_thread(server, params.thread_id).await;
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

    server.threads.lock().await.remove(&params.thread_id);
    let to_remove = {
        let entries = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .map(|(process_id, entry)| (*process_id, entry.clone()))
                .collect::<Vec<_>>()
        };
        let mut to_remove = Vec::new();
        for (process_id, entry) in entries {
            let info = entry.info.lock().await;
            if info.thread_id == params.thread_id {
                to_remove.push(process_id);
            }
        }
        to_remove
    };
    {
        let mut entries = server.processes.lock().await;
        for process_id in to_remove {
            entries.remove(&process_id);
        }
    }

    Ok(omne_app_server_protocol::ThreadDeleteResponse {
        thread_id: params.thread_id,
        deleted,
        thread_dir: thread_dir.display().to_string(),
    })
}
