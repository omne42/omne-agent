async fn handle_thread_pause(server: &Server, params: ThreadPauseParams) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let (already_paused, archived, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.paused, state.archived, state.active_turn_id)
    };

    if archived {
        anyhow::bail!("refusing to pause an archived thread (unarchive first)");
    }

    if already_paused {
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "paused": true,
            "already_paused": true,
        }));
    }

    let reason = params
        .reason
        .clone()
        .or_else(|| Some("thread paused".to_string()));

    if let Some(turn_id) = active_turn_id {
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

    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ThreadPaused {
            reason: reason.clone(),
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "paused": true,
        "already_paused": false,
        "interrupted_turn_id": active_turn_id,
    }))
}

async fn handle_thread_unpause(server: &Server, params: ThreadUnpauseParams) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let already_unpaused = {
        let handle = thread_rt.handle.lock().await;
        !handle.state().paused
    };

    if already_unpaused {
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "paused": false,
            "already_unpaused": true,
        }));
    }

    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ThreadUnpaused {
            reason: params.reason,
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "paused": false,
        "already_unpaused": false,
    }))
}
