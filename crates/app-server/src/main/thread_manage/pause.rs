#[cfg(test)]
const THREAD_PAUSE_WAIT_TIMEOUT: Duration = Duration::from_millis(200);
#[cfg(not(test))]
const THREAD_PAUSE_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
const THREAD_PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(20);
#[cfg(not(test))]
const THREAD_PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(200);

async fn handle_thread_pause(
    server: &Server,
    params: ThreadPauseParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadPauseResponse> {
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
        return Ok(omne_app_server_protocol::ThreadPauseResponse {
            thread_id: params.thread_id,
            paused: true,
            already_paused: true,
            interrupted_turn_id: None,
        });
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
        stop_turn_processes(
            server,
            params.thread_id,
            turn_id,
            reason.clone(),
            "pause active turn",
            ClosedProcessCommandPolicy::WaitForExplicitStop,
        )
        .await?;

        let deadline = tokio::time::Instant::now() + THREAD_PAUSE_WAIT_TIMEOUT;
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
                    "timed out waiting for active turn to stop before pause: turn_id={}",
                    turn_id
                );
            }
            tokio::time::sleep(THREAD_PAUSE_POLL_INTERVAL).await;
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ThreadPaused {
            reason: reason.clone(),
        })
        .await?;

    Ok(omne_app_server_protocol::ThreadPauseResponse {
        thread_id: params.thread_id,
        paused: true,
        already_paused: false,
        interrupted_turn_id: active_turn_id,
    })
}

async fn handle_thread_unpause(
    server: &Server,
    params: ThreadUnpauseParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadUnpauseResponse> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let already_unpaused = {
        let handle = thread_rt.handle.lock().await;
        !handle.state().paused
    };

    if already_unpaused {
        return Ok(omne_app_server_protocol::ThreadUnpauseResponse {
            thread_id: params.thread_id,
            paused: false,
            already_unpaused: true,
        });
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ThreadUnpaused {
            reason: params.reason,
        })
        .await?;

    Ok(omne_app_server_protocol::ThreadUnpauseResponse {
        thread_id: params.thread_id,
        paused: false,
        already_unpaused: false,
    })
}
