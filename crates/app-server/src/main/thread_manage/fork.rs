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
            pm_protocol::ThreadEventKind::ThreadCreated { .. } => {}
            pm_protocol::ThreadEventKind::ThreadArchived { .. }
            | pm_protocol::ThreadEventKind::ThreadUnarchived { .. }
            | pm_protocol::ThreadEventKind::ThreadPaused { .. }
            | pm_protocol::ThreadEventKind::ThreadUnpaused { .. } => {}
            kind @ pm_protocol::ThreadEventKind::ThreadConfigUpdated { .. } => {
                forked.append(kind).await?;
            }
            pm_protocol::ThreadEventKind::TurnStarted { turn_id, .. } if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::TurnCompleted { turn_id, .. } if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::ApprovalRequested { turn_id: Some(turn_id), .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::AssistantMessage { turn_id: Some(turn_id), .. }
                if active_turn_id == Some(turn_id) => {}
            kind @ pm_protocol::ThreadEventKind::TurnStarted { .. }
            | kind @ pm_protocol::ThreadEventKind::TurnInterruptRequested { .. }
            | kind @ pm_protocol::ThreadEventKind::TurnCompleted { .. }
            | kind @ pm_protocol::ThreadEventKind::ApprovalRequested { .. }
            | kind @ pm_protocol::ThreadEventKind::ApprovalDecided { .. }
            | kind @ pm_protocol::ThreadEventKind::AssistantMessage { .. } => {
                forked.append(kind).await?;
            }
            pm_protocol::ThreadEventKind::ToolStarted { .. }
            | pm_protocol::ThreadEventKind::ToolCompleted { .. }
            | pm_protocol::ThreadEventKind::ProcessStarted { .. }
            | pm_protocol::ThreadEventKind::ProcessInterruptRequested { .. }
            | pm_protocol::ThreadEventKind::ProcessKillRequested { .. }
            | pm_protocol::ThreadEventKind::ProcessExited { .. } => {}
        }
    }

    let log_path = forked.log_path().display().to_string();
    let last_seq = forked.last_seq().0;

    let rt = Arc::new(ThreadRuntime::new(forked, server.out_tx.clone()));
    server.threads.lock().await.insert(forked_id, rt);

    Ok(serde_json::json!({
        "thread_id": forked_id,
        "log_path": log_path,
        "last_seq": last_seq,
    }))
}
