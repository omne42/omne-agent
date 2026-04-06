async fn handle_thread_fork(
    server: &Server,
    params: ThreadForkParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadHandleResponse> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let (current_cwd, active_turn_id) = {
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

    let parent_root = resolve_thread_root_from_cwd(server, &current_cwd).await?;
    let cwd = match params.cwd.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        Some(cwd) => resolve_thread_root_from_cwd(server, cwd).await?,
        None => parent_root.clone(),
    };

    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut forked = server.thread_store.create_thread(cwd.clone()).await?;
    let forked_id = forked.thread_id();
    let mut skipped_active_turn_approvals =
        std::collections::HashSet::<omne_protocol::ApprovalId>::new();
    let mut copied_active_turn_started = false;

    for event in events {
        let kind = event.kind;
        match kind {
            omne_protocol::ThreadEventKind::ThreadCreated { .. } => {}
            omne_protocol::ThreadEventKind::ThreadArchived { .. }
            | omne_protocol::ThreadEventKind::ThreadUnarchived { .. }
            | omne_protocol::ThreadEventKind::ThreadPaused { .. }
            | omne_protocol::ThreadEventKind::ThreadUnpaused { .. } => {}
            kind @ omne_protocol::ThreadEventKind::ThreadSystemPromptSnapshot { .. } => {
                forked.append(kind).await?;
            }
            kind @ omne_protocol::ThreadEventKind::ThreadConfigUpdated { .. } => {
                forked
                    .append(rewrite_thread_config_for_fork(kind, &parent_root, &cwd))
                    .await?;
            }
            kind @ omne_protocol::ThreadEventKind::TurnStarted { turn_id, .. }
                if active_turn_id == Some(turn_id) =>
            {
                copied_active_turn_started = true;
                forked.append(kind).await?;
            }
            kind @ omne_protocol::ThreadEventKind::ModelRouted { turn_id, .. }
                if active_turn_id == Some(turn_id) =>
            {
                forked.append(kind).await?;
            }
            omne_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            omne_protocol::ThreadEventKind::TurnCompleted { turn_id, .. } if active_turn_id == Some(turn_id) => {}
            // Only approvals explicitly bound to the active turn are filtered.
            // Turnless approvals (`turn_id=None`) are preserved during fork.
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: Some(turn_id),
                ..
            } if active_turn_id == Some(turn_id) => {
                skipped_active_turn_approvals.insert(approval_id);
            }
            omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. }
                if skipped_active_turn_approvals.contains(&approval_id) => {}
            kind @ omne_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                ..
            } if active_turn_id == Some(turn_id) => {
                forked.append(kind).await?;
            }
            kind @ omne_protocol::ThreadEventKind::TurnStarted { .. }
            | kind @ omne_protocol::ThreadEventKind::ModelRouted { .. }
            | kind @ omne_protocol::ThreadEventKind::TurnInterruptRequested { .. }
            | kind @ omne_protocol::ThreadEventKind::TurnCompleted { .. }
            | kind @ omne_protocol::ThreadEventKind::ApprovalRequested { .. }
            | kind @ omne_protocol::ThreadEventKind::ApprovalDecided { .. }
            | kind @ omne_protocol::ThreadEventKind::AssistantMessage { .. } => {
                forked.append(kind).await?;
            }
            omne_protocol::ThreadEventKind::AttentionMarkerSet { .. }
            | omne_protocol::ThreadEventKind::AttentionMarkerCleared { .. }
            | omne_protocol::ThreadEventKind::ToolStarted { .. }
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

    if let Some(turn_id) = active_turn_id.filter(|_| copied_active_turn_started) {
        forked
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Interrupted,
                reason: Some("thread fork snapshots the active turn and closes it".to_string()),
            })
            .await?;
    }

    let log_path = forked.log_path().display().to_string();
    let last_seq = forked.last_seq().0;

    let rt = Arc::new(ThreadRuntime::new(forked, server.notify_tx.clone()));
    server.threads.lock().await.insert(forked_id, rt);

    Ok(omne_app_server_protocol::ThreadHandleResponse {
        thread_id: forked_id,
        log_path,
        last_seq,
    })
}

fn rewrite_thread_config_for_fork(
    kind: omne_protocol::ThreadEventKind,
    parent_root: &std::path::Path,
    fork_root: &std::path::Path,
) -> omne_protocol::ThreadEventKind {
    let omne_protocol::ThreadEventKind::ThreadConfigUpdated {
        approval_policy,
        sandbox_policy,
        sandbox_writable_roots,
        sandbox_network_access,
        mode,
        role,
        model,
        clear_model,
        thinking,
        clear_thinking,
        show_thinking,
        clear_show_thinking,
        openai_base_url,
        clear_openai_base_url,
        allowed_tools,
        execpolicy_rules,
        clear_execpolicy_rules,
    } = kind
    else {
        return kind;
    };

    let sandbox_writable_roots = sandbox_writable_roots.map(|roots| {
        roots
            .into_iter()
            .map(|root| rebase_fork_workspace_path(&root, parent_root, fork_root, true))
            .collect()
    });
    let execpolicy_rules = execpolicy_rules.map(|rules| {
        rules
            .into_iter()
            .map(|rule| rebase_fork_workspace_path(&rule, parent_root, fork_root, false))
            .collect()
    });

    omne_protocol::ThreadEventKind::ThreadConfigUpdated {
        approval_policy,
        sandbox_policy,
        sandbox_writable_roots,
        sandbox_network_access,
        mode,
        role,
        model,
        clear_model,
        thinking,
        clear_thinking,
        show_thinking,
        clear_show_thinking,
        openai_base_url,
        clear_openai_base_url,
        allowed_tools,
        execpolicy_rules,
        clear_execpolicy_rules,
    }
}

fn rebase_fork_workspace_path(
    value: &str,
    parent_root: &std::path::Path,
    fork_root: &std::path::Path,
    rebase_relative: bool,
) -> String {
    let path = std::path::Path::new(value);
    if path.is_absolute() {
        return path
            .strip_prefix(parent_root)
            .map(|suffix| fork_root.join(suffix).display().to_string())
            .unwrap_or_else(|_| value.to_string());
    }
    if rebase_relative {
        return fork_root.join(path).display().to_string();
    }
    value.to_string()
}
