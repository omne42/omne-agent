async fn handle_thread_archive(server: &Server, params: ThreadArchiveParams) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let (already_archived, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.archived, state.active_turn_id)
    };

    if already_archived {
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "archived": true,
            "already_archived": true,
        }));
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
                break;
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

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ThreadArchived {
            reason: reason.clone(),
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "archived": true,
        "already_archived": false,
        "force": params.force,
        "killed_processes": running,
    }))
}

async fn handle_thread_unarchive(server: &Server, params: ThreadUnarchiveParams) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let already_unarchived = {
        let handle = thread_rt.handle.lock().await;
        !handle.state().archived
    };

    if already_unarchived {
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "archived": false,
            "already_unarchived": true,
        }));
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ThreadUnarchived {
            reason: params.reason,
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "archived": false,
        "already_unarchived": false,
    }))
}
