async fn handle_thread_configure(
    server: &Server,
    params: ThreadConfigureParams,
) -> anyhow::Result<Value> {
    let (rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (
        current_approval_policy,
        current_sandbox_policy,
        current_mode,
        current_model,
        current_openai_base_url,
    ) = {
        let handle = rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.mode.clone(),
            state.model.clone(),
            state.openai_base_url.clone(),
        )
    };

    let approval_policy = params.approval_policy.unwrap_or(current_approval_policy);
    let mode = params
        .mode
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    if let Some(mode) = mode.as_deref() {
        let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
        if catalog.mode(mode).is_none() {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            anyhow::bail!("unknown mode: {mode} (available: {available})");
        }
    }
    let model = params.model.filter(|s| !s.trim().is_empty());
    let openai_base_url = params.openai_base_url.filter(|s| !s.trim().is_empty());

    let changed = approval_policy != current_approval_policy
        || params
            .sandbox_policy
            .is_some_and(|p| p != current_sandbox_policy)
        || mode.as_ref().is_some_and(|m| m != &current_mode)
        || model.as_ref() != current_model.as_ref()
        || openai_base_url.as_ref() != current_openai_base_url.as_ref();

    if changed {
        rt.append_event(pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy: params.sandbox_policy,
            mode,
            model,
            openai_base_url,
        })
        .await?;
    }
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_thread_config_explain(
    server: &Server,
    params: ThreadConfigExplainParams,
) -> anyhow::Result<Value> {
    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let thread_cwd = events
        .iter()
        .find_map(|event| match &event.kind {
            pm_protocol::ThreadEventKind::ThreadCreated { cwd, .. } => Some(cwd.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", params.thread_id))?;
    let thread_root = pm_core::resolve_dir(Path::new(&thread_cwd), Path::new(".")).await?;
    let mode_catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;

    let default_model = "gpt-4.1".to_string();
    let default_openai_base_url = "https://api.openai.com".to_string();
    let default_mode = "coder".to_string();

    let mut effective_approval_policy = pm_protocol::ApprovalPolicy::AutoApprove;
    let mut effective_sandbox_policy = pm_protocol::SandboxPolicy::WorkspaceWrite;
    let mut effective_mode = default_mode.clone();
    let mut effective_model = default_model.clone();
    let mut effective_openai_base_url = default_openai_base_url.clone();
    let mut layers = vec![serde_json::json!({
        "source": "default",
        "approval_policy": effective_approval_policy,
        "sandbox_policy": effective_sandbox_policy,
        "mode": effective_mode,
        "model": effective_model,
        "openai_base_url": effective_openai_base_url,
    })];

    let env_model = std::env::var("CODE_PM_OPENAI_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_openai_base_url = std::env::var("CODE_PM_OPENAI_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    if env_model.is_some() || env_openai_base_url.is_some() {
        if let Some(model) = env_model {
            effective_model = model;
        }
        if let Some(openai_base_url) = env_openai_base_url {
            effective_openai_base_url = openai_base_url;
        }
        layers.push(serde_json::json!({
            "source": "env",
            "model": effective_model,
            "openai_base_url": effective_openai_base_url,
        }));
    }

    for event in events {
        if let pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            mode,
            model,
            openai_base_url,
        } = event.kind
        {
            let ts = event.timestamp.format(&Rfc3339)?;
            effective_approval_policy = approval_policy;
            if let Some(policy) = sandbox_policy {
                effective_sandbox_policy = policy;
            }
            if let Some(mode) = mode {
                effective_mode = mode;
            }
            if let Some(model) = model {
                effective_model = model;
            }
            if let Some(openai_base_url) = openai_base_url {
                effective_openai_base_url = openai_base_url;
            }
            layers.push(serde_json::json!({
                "source": "thread",
                "seq": event.seq.0,
                "timestamp": ts,
                "approval_policy": approval_policy,
                "sandbox_policy": effective_sandbox_policy,
                "mode": effective_mode,
                "model": effective_model,
                "openai_base_url": effective_openai_base_url,
            }));
        }
    }

    let (mode_catalog_source, mode_catalog_path) = match &mode_catalog.source {
        pm_core::modes::ModeCatalogSource::Builtin => ("builtin", None),
        pm_core::modes::ModeCatalogSource::Env(path) => ("env", Some(path.display().to_string())),
        pm_core::modes::ModeCatalogSource::Project(path) => {
            ("project", Some(path.display().to_string()))
        }
    };
    let available_modes = mode_catalog
        .mode_names()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let effective_mode_name = effective_mode.clone();
    let effective_mode_def = mode_catalog.mode(&effective_mode).map(|mode| {
        serde_json::json!({
            "name": effective_mode_name,
            "description": mode.description.as_str(),
            "permissions": {
                "read": mode.permissions.read,
                "edit": {
                    "decision": mode.permissions.edit.decision,
                    "allow_globs": &mode.permissions.edit.allow_globs,
                    "deny_globs": &mode.permissions.edit.deny_globs,
                },
                "command": mode.permissions.command,
                "process": {
                    "inspect": mode.permissions.process.inspect,
                    "kill": mode.permissions.process.kill,
                    "interact": mode.permissions.process.interact,
                },
                "artifact": mode.permissions.artifact,
                "browser": mode.permissions.browser,
            },
            "tool_overrides": &mode.tool_overrides,
        })
    });

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "effective": {
            "approval_policy": effective_approval_policy,
            "sandbox_policy": effective_sandbox_policy,
            "mode": effective_mode,
            "model": effective_model,
            "openai_base_url": effective_openai_base_url,
        },
        "mode_catalog": {
            "source": mode_catalog_source,
            "path": mode_catalog_path,
            "load_error": mode_catalog.load_error,
            "modes": available_modes,
        },
        "effective_mode_def": effective_mode_def,
        "layers": layers,
    }))
}

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

    let mut forked = server
        .thread_store
        .create_thread(PathBuf::from(&cwd))
        .await?;
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
            pm_protocol::ThreadEventKind::TurnStarted { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::TurnCompleted { turn_id, .. }
                if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::ApprovalRequested {
                turn_id: Some(turn_id),
                ..
            } if active_turn_id == Some(turn_id) => {}
            pm_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                ..
            } if active_turn_id == Some(turn_id) => {}
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

async fn handle_thread_archive(
    server: &Server,
    params: ThreadArchiveParams,
) -> anyhow::Result<Value> {
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
        .append_event(pm_protocol::ThreadEventKind::ThreadArchived {
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

async fn handle_thread_unarchive(
    server: &Server,
    params: ThreadUnarchiveParams,
) -> anyhow::Result<Value> {
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
        .append_event(pm_protocol::ThreadEventKind::ThreadUnarchived {
            reason: params.reason,
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "archived": false,
        "already_unarchived": false,
    }))
}

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
        .append_event(pm_protocol::ThreadEventKind::ThreadPaused {
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

async fn handle_thread_unpause(
    server: &Server,
    params: ThreadUnpauseParams,
) -> anyhow::Result<Value> {
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
        .append_event(pm_protocol::ThreadEventKind::ThreadUnpaused {
            reason: params.reason,
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "paused": false,
        "already_unpaused": false,
    }))
}

async fn handle_thread_delete(
    server: &Server,
    params: ThreadDeleteParams,
) -> anyhow::Result<Value> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);

    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    let mut to_remove = Vec::<ProcessId>::new();
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
            to_remove.push(process_id);
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
                    reason: Some("thread deleted".to_string()),
                })
                .await;
        }
    }

    server.threads.lock().await.remove(&params.thread_id);
    {
        let mut entries = server.processes.lock().await;
        for process_id in to_remove {
            entries.remove(&process_id);
        }
    }

    let deleted = match tokio::fs::remove_dir_all(&thread_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", thread_dir.display())),
    };

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "deleted": deleted,
        "thread_dir": thread_dir.display().to_string(),
    }))
}

async fn handle_thread_clear_artifacts(
    server: &Server,
    params: ThreadClearArtifactsParams,
) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

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
            "refusing to clear artifacts with running processes (use force=true): {:?}",
            running
        );
    }

    if params.force {
        for entry in to_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("artifacts cleared".to_string()),
                })
                .await;
        }
    }

    let tool_id = pm_protocol::ToolId::new();
    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: None,
            tool: "thread/clear_artifacts".to_string(),
            params: Some(serde_json::json!({
                "force": params.force,
            })),
        })
        .await?;

    let artifacts_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts");
    let removed = match tokio::fs::remove_dir_all(&artifacts_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", artifacts_dir.display())),
    };

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "removed": removed,
                "artifacts_dir": artifacts_dir.display().to_string(),
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "removed": removed,
        "artifacts_dir": artifacts_dir.display().to_string(),
    }))
}

