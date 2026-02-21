async fn handle_thread_fork(server: &Server, params: ThreadForkParams) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let (cwd, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state
                .cwd
                .clone()
                .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", params.thread_id))?,
            state.active_turn_id,
        )
    };

    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut forked = server.thread_store.create_thread(PathBuf::from(&cwd)).await?;
    let forked_id = forked.thread_id();

    for event in events {
        let kind = event.kind;
        match kind {
            omne_protocol::ThreadEventKind::ThreadCreated { .. } => {}
            omne_protocol::ThreadEventKind::ThreadArchived { .. }
            | omne_protocol::ThreadEventKind::ThreadUnarchived { .. }
            | omne_protocol::ThreadEventKind::ThreadPaused { .. }
            | omne_protocol::ThreadEventKind::ThreadUnpaused { .. } => {}
            kind @ omne_protocol::ThreadEventKind::ThreadConfigUpdated { .. } => {
                forked.append(kind).await?;
            }
            omne_protocol::ThreadEventKind::TurnStarted { turn_id, .. } if active_turn_id == Some(turn_id) => {}
            omne_protocol::ThreadEventKind::ModelRouted { turn_id, .. } if active_turn_id == Some(turn_id) => {}
            omne_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            omne_protocol::ThreadEventKind::TurnCompleted { turn_id, .. } if active_turn_id == Some(turn_id) => {}
            omne_protocol::ThreadEventKind::ApprovalRequested { turn_id: Some(turn_id), .. }
                if active_turn_id == Some(turn_id) => {}
            omne_protocol::ThreadEventKind::AssistantMessage { turn_id: Some(turn_id), .. }
                if active_turn_id == Some(turn_id) => {}
            kind @ omne_protocol::ThreadEventKind::TurnStarted { .. }
            | kind @ omne_protocol::ThreadEventKind::ModelRouted { .. }
            | kind @ omne_protocol::ThreadEventKind::TurnInterruptRequested { .. }
            | kind @ omne_protocol::ThreadEventKind::TurnCompleted { .. }
            | kind @ omne_protocol::ThreadEventKind::ApprovalRequested { .. }
            | kind @ omne_protocol::ThreadEventKind::ApprovalDecided { .. }
            | kind @ omne_protocol::ThreadEventKind::AssistantMessage { .. } => {
                forked.append(kind).await?;
            }
            omne_protocol::ThreadEventKind::ToolStarted { .. }
            | omne_protocol::ThreadEventKind::ToolCompleted { .. }
            | omne_protocol::ThreadEventKind::AgentStep { .. }
            | omne_protocol::ThreadEventKind::ProcessStarted { .. }
            | omne_protocol::ThreadEventKind::ProcessInterruptRequested { .. }
            | omne_protocol::ThreadEventKind::ProcessKillRequested { .. }
            | omne_protocol::ThreadEventKind::ProcessExited { .. }
            | omne_protocol::ThreadEventKind::CheckpointCreated { .. }
            | omne_protocol::ThreadEventKind::CheckpointRestored { .. } => {}
        }
    }

    let log_path = forked.log_path().display().to_string();
    let last_seq = forked.last_seq().0;

    let rt = Arc::new(ThreadRuntime::new(forked, server.notify_tx.clone()));
    server.threads.lock().await.insert(forked_id, rt);

    Ok(serde_json::json!({
        "thread_id": forked_id,
        "log_path": log_path,
        "last_seq": last_seq,
    }))
}
