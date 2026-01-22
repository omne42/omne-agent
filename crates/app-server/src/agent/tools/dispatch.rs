async fn run_tool_call(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: Value,
    cancel: CancellationToken,
) -> anyhow::Result<Value> {
    let mut approval_id: Option<ApprovalId> = None;

    for attempt in 0..3usize {
        if cancel.is_cancelled() {
            return Err(AgentTurnError::Cancelled.into());
        }

        let output = run_tool_call_once(
            server,
            thread_id,
            turn_id,
            tool_name,
            args.clone(),
            approval_id,
        )
        .await?;

        let Some(requested) = parse_needs_approval(&output)? else {
            return Ok(redact_tool_output(output));
        };

        if attempt >= 2 {
            return Err(AgentTurnError::BudgetExceeded {
                budget: "approval_cycles",
            }
            .into());
        }

        let outcome =
            wait_for_approval_outcome(server, thread_id, requested, cancel.clone()).await?;
        match outcome.decision {
            ApprovalDecision::Approved => {
                approval_id = Some(requested);
            }
            ApprovalDecision::Denied => {
                return Ok(serde_json::json!({
                    "denied": true,
                    "approval_id": requested,
                    "decision": outcome.decision,
                    "remember": outcome.remember,
                    "reason": outcome.reason,
                }));
            }
        }
    }

    Err(AgentTurnError::BudgetExceeded { budget: "retries" }.into())
}

async fn active_subagent_threads(
    server: &super::Server,
    parent_thread_id: pm_protocol::ThreadId,
) -> anyhow::Result<Vec<pm_protocol::ThreadId>> {
    let events = server
        .thread_store
        .read_events_since(parent_thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", parent_thread_id))?;

    let mut spawned_tool_ids = std::collections::HashSet::<pm_protocol::ToolId>::new();
    for event in &events {
        if let pm_protocol::ThreadEventKind::ToolStarted { tool_id, tool, .. } = &event.kind
            && tool == "subagent/spawn"
        {
            spawned_tool_ids.insert(*tool_id);
        }
    }

    let mut spawned_threads = std::collections::BTreeSet::<pm_protocol::ThreadId>::new();
    for event in &events {
        let pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status,
            result,
            ..
        } = &event.kind
        else {
            continue;
        };
        if !spawned_tool_ids.contains(tool_id) {
            continue;
        }
        if !matches!(status, pm_protocol::ToolStatus::Completed) {
            continue;
        }
        let Some(thread_id) = result
            .as_ref()
            .and_then(|value| value.get("thread_id"))
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        if let Ok(thread_id) = thread_id.parse::<pm_protocol::ThreadId>() {
            spawned_threads.insert(thread_id);
        }
    }

    let mut active = Vec::new();
    for thread_id in spawned_threads {
        let Some(state) = server.thread_store.read_state(thread_id).await? else {
            continue;
        };
        if state.active_turn_id.is_some() {
            active.push(thread_id);
        }
    }

    Ok(active)
}

async fn run_tool_call_once(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: Value,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<Value> {
    match tool_name {
        "file_read" => {
            let args: FileReadArgs = serde_json::from_value(args)?;
            super::handle_file_read(
                server,
                super::FileReadParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_glob" => {
            let args: FileGlobArgs = serde_json::from_value(args)?;
            super::handle_file_glob(
                server,
                super::FileGlobParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    pattern: args.pattern,
                    max_results: args.max_results,
                },
            )
            .await
        }
        "file_grep" => {
            let args: FileGrepArgs = serde_json::from_value(args)?;
            super::handle_file_grep(
                server,
                super::FileGrepParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    query: args.query,
                    is_regex: args.is_regex,
                    include_glob: args.include_glob,
                    max_matches: args.max_matches,
                    max_bytes_per_file: None,
                    max_files: None,
                },
            )
            .await
        }
        "file_write" => {
            let args: FileWriteArgs = serde_json::from_value(args)?;
            super::handle_file_write(
                server,
                super::FileWriteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    text: args.text,
                    create_parent_dirs: args.create_parent_dirs,
                },
            )
            .await
        }
        "file_patch" => {
            let args: FilePatchArgs = serde_json::from_value(args)?;
            super::handle_file_patch(
                server,
                super::FilePatchParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    patch: args.patch,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_edit" => {
            let args: FileEditArgs = serde_json::from_value(args)?;
            let edits = args
                .edits
                .into_iter()
                .map(|op| super::FileEditOp {
                    old: op.old,
                    new: op.new,
                    expected_replacements: op.expected_replacements,
                })
                .collect();
            super::handle_file_edit(
                server,
                super::FileEditParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    edits,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_delete" => {
            let args: FileDeleteArgs = serde_json::from_value(args)?;
            super::handle_file_delete(
                server,
                super::FileDeleteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    recursive: args.recursive,
                },
            )
            .await
        }
        "fs_mkdir" => {
            let args: FsMkdirArgs = serde_json::from_value(args)?;
            super::handle_fs_mkdir(
                server,
                super::FsMkdirParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    recursive: args.recursive,
                },
            )
            .await
        }
        "process_start" => {
            let args: ProcessStartArgs = serde_json::from_value(args)?;
            super::handle_process_start(
                server,
                super::ProcessStartParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    argv: args.argv,
                    cwd: args.cwd,
                },
            )
            .await
        }
        "process_inspect" => {
            let args: ProcessInspectArgs = serde_json::from_value(args)?;
            super::handle_process_inspect(
                server,
                super::ProcessInspectParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    max_lines: args.max_lines,
                },
            )
            .await
        }
        "process_tail" => {
            let args: ProcessTailArgs = serde_json::from_value(args)?;
            super::handle_process_tail(
                server,
                super::ProcessTailParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    stream: args.stream,
                    max_lines: args.max_lines,
                },
            )
            .await
        }
        "process_follow" => {
            let args: ProcessFollowArgs = serde_json::from_value(args)?;
            super::handle_process_follow(
                server,
                super::ProcessFollowParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    stream: args.stream,
                    since_offset: args.since_offset,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "process_kill" => {
            let args: ProcessKillArgs = serde_json::from_value(args)?;
            super::handle_process_kill(
                server,
                super::ProcessKillParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    reason: args.reason,
                },
            )
            .await
        }
        "artifact_write" => {
            let args: ArtifactWriteArgs = serde_json::from_value(args)?;
            super::handle_artifact_write(
                server,
                super::ArtifactWriteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: None,
                    artifact_type: args.artifact_type,
                    summary: args.summary,
                    text: args.text,
                },
            )
            .await
        }
        "artifact_list" => {
            let _ = args;
            super::handle_artifact_list(
                server,
                super::ArtifactListParams {
                    thread_id,
                    turn_id,
                    approval_id,
                },
            )
            .await
        }
        "artifact_read" => {
            let args: ArtifactReadArgs = serde_json::from_value(args)?;
            super::handle_artifact_read(
                server,
                super::ArtifactReadParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: args.artifact_id.parse()?,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "artifact_delete" => {
            let args: ArtifactDeleteArgs = serde_json::from_value(args)?;
            super::handle_artifact_delete(
                server,
                super::ArtifactDeleteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: args.artifact_id.parse()?,
                },
            )
            .await
        }
        "thread_state" => {
            let args: ThreadStateArgs = serde_json::from_value(args)?;
            let thread_id: pm_protocol::ThreadId = args.thread_id.parse()?;
            let rt = server.get_or_load_thread(thread_id).await?;
            let handle = rt.handle.lock().await;
            let state = handle.state();
            let archived_at = state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok());
            let paused_at = state.paused_at.and_then(|ts| ts.format(&Rfc3339).ok());
            Ok(serde_json::json!({
                "thread_id": handle.thread_id(),
                "cwd": state.cwd,
                "archived": state.archived,
                "archived_at": archived_at,
                "archived_reason": state.archived_reason,
                "paused": state.paused,
                "paused_at": paused_at,
                "paused_reason": state.paused_reason,
                "approval_policy": state.approval_policy,
                "sandbox_policy": state.sandbox_policy,
                "model": state.model,
                "openai_base_url": state.openai_base_url,
                "last_seq": handle.last_seq().0,
                "active_turn_id": state.active_turn_id,
                "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
                "last_turn_id": state.last_turn_id,
                "last_turn_status": state.last_turn_status,
                "last_turn_reason": state.last_turn_reason,
            }))
        }
        "thread_events" => {
            let args: ThreadEventsArgs = serde_json::from_value(args)?;
            let thread_id: pm_protocol::ThreadId = args.thread_id.parse()?;
            let since = EventSeq(args.since_seq);

            let mut events = server
                .thread_store
                .read_events_since(thread_id, since)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

            let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

            let mut has_more = false;
            if let Some(max_events) = args.max_events {
                let max_events = max_events.clamp(1, 50_000);
                if events.len() > max_events {
                    events.truncate(max_events);
                    has_more = true;
                }
            }

            let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

            Ok(serde_json::json!({
                "events": events,
                "last_seq": last_seq,
                "thread_last_seq": thread_last_seq,
                "has_more": has_more,
            }))
        }
        "thread_hook_run" => {
            let args: ThreadHookRunArgs = serde_json::from_value(args)?;
            super::handle_thread_hook_run(
                server,
                super::ThreadHookRunParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    hook: args.hook,
                },
            )
            .await
        }
        "agent_spawn" => {
            #[derive(Debug, Deserialize)]
            struct ForkResult {
                thread_id: pm_protocol::ThreadId,
                log_path: String,
                last_seq: u64,
            }

            let args: AgentSpawnArgs = serde_json::from_value(args)?;
            let input = args.input.trim().to_string();
            if input.is_empty() {
                anyhow::bail!("input must not be empty");
            }

            let task_id = args
                .task_id
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let expected_artifact_type = args
                .expected_artifact_type
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("fan_out_result")
                .to_string();

            let workspace_mode = args
                .workspace_mode
                .unwrap_or(AgentSpawnWorkspaceMode::ReadOnly);
            let workspace_mode_str = match workspace_mode {
                AgentSpawnWorkspaceMode::ReadOnly => "read_only",
                AgentSpawnWorkspaceMode::IsolatedWrite => "isolated_write",
            };
            let child_mode = args
                .mode
                .as_deref()
                .map(|mode| mode.trim())
                .filter(|mode| !mode.is_empty())
                .unwrap_or("reviewer")
                .to_string();

            let input_preview = pm_core::redact_text(&truncate_chars(&input, 400));
            let approval_params = serde_json::json!({
                "input_chars": input.chars().count(),
                "input_preview": input_preview,
                "task_id": task_id.clone(),
                "expected_artifact_type": expected_artifact_type.clone(),
                "mode": child_mode.clone(),
                "workspace_mode": workspace_mode_str,
                "model": args.model.clone(),
                "openai_base_url": args.openai_base_url.clone(),
            });

            let tool_id = pm_protocol::ToolId::new();
            let (thread_rt, thread_root) = super::load_thread_root(server, thread_id).await?;
            let (approval_policy, mode_name) = {
                let handle = thread_rt.handle.lock().await;
                let state = handle.state();
                (state.approval_policy, state.mode.clone())
            };

            let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
            let Some(mode) = catalog.mode(&mode_name) else {
                let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
                let decision = pm_core::modes::Decision::Deny;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some("unknown mode".to_string()),
                        result: Some(serde_json::json!({
                            "mode": mode_name,
                            "decision": decision,
                            "available": available,
                            "load_error": catalog.load_error.clone(),
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "mode": mode_name,
                    "decision": decision,
                    "available": available,
                    "load_error": catalog.load_error,
                }));
            };

            if matches!(workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite) {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some("workspace_mode=isolated_write is not supported yet".to_string()),
                        result: Some(serde_json::json!({
                            "workspace_mode": workspace_mode_str,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "workspace_mode": workspace_mode_str,
                }));
            }

            let base_decision = mode.permissions.subagent.spawn.decision;
            let effective_decision = match mode.tool_overrides.get("subagent/spawn").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };

            if effective_decision == pm_core::modes::Decision::Deny {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some("mode denies subagent/spawn".to_string()),
                        result: Some(serde_json::json!({
                            "mode": mode_name,
                            "decision": effective_decision,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "mode": mode_name,
                    "decision": effective_decision,
                }));
            }

            if let Some(allowed) = mode.permissions.subagent.spawn.allowed_modes.as_ref()
                && !allowed.iter().any(|name| name == &child_mode)
            {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some("mode forbids spawning this subagent mode".to_string()),
                        result: Some(serde_json::json!({
                            "mode": mode_name,
                            "decision": effective_decision,
                            "requested_mode": child_mode,
                            "allowed_modes": allowed,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "mode": mode_name,
                    "decision": effective_decision,
                    "allowed_modes": allowed,
                }));
            }

            let max_concurrent_subagents =
                parse_env_usize("CODE_PM_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
            if max_concurrent_subagents > 0 {
                let active_threads = active_subagent_threads(server, thread_id).await?;
                if active_threads.len() >= max_concurrent_subagents {
                    let active = active_threads.len();
                    let active_thread_ids = active_threads
                        .iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>();

                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                            tool_id,
                            turn_id,
                            tool: "subagent/spawn".to_string(),
                            params: Some(approval_params),
                        })
                        .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some(format!(
                                "max_concurrent_subagents limit reached: active={active}, max={max_concurrent_subagents}"
                            )),
                            result: Some(serde_json::json!({
                                "max_concurrent_subagents": max_concurrent_subagents,
                                "active": active,
                                "active_threads": active_thread_ids,
                            })),
                        })
                        .await?;
                    return Ok(serde_json::json!({
                        "tool_id": tool_id,
                        "denied": true,
                        "max_concurrent_subagents": max_concurrent_subagents,
                        "active": active,
                    }));
                }
            }

            if effective_decision == pm_core::modes::Decision::Prompt {
                match super::gate_approval(
                    server,
                    &thread_rt,
                    thread_id,
                    turn_id,
                    approval_policy,
                    super::ApprovalRequest {
                        approval_id,
                        action: "subagent/spawn",
                        params: &approval_params,
                    },
                )
                .await?
                {
                    super::ApprovalGate::Approved => {}
                    super::ApprovalGate::Denied { remembered } => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id,
                                tool: "subagent/spawn".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some(super::approval_denied_error(remembered).to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": remembered,
                        }));
                    }
                    super::ApprovalGate::NeedsApproval { approval_id } => {
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id,
                    tool: "subagent/spawn".to_string(),
                    params: Some(approval_params),
                })
                .await?;

            let outcome: anyhow::Result<(
                ForkResult,
                TurnId,
                tokio::sync::broadcast::Receiver<String>,
            )> = async {
                let forked = super::handle_thread_fork(server, super::ThreadForkParams { thread_id })
                    .await?;
                let forked: ForkResult = serde_json::from_value(forked)?;

                let notify_rx = server.notify_tx.subscribe();

                super::handle_thread_configure(
                    server,
                    super::ThreadConfigureParams {
                        thread_id: forked.thread_id,
                        approval_policy: None,
                        sandbox_policy: Some(pm_protocol::SandboxPolicy::ReadOnly),
                        sandbox_writable_roots: None,
                        sandbox_network_access: None,
                        mode: Some(child_mode.clone()),
                        model: args.model,
                        openai_base_url: args.openai_base_url,
                    },
                )
                .await?;

                let rt = server.get_or_load_thread(forked.thread_id).await?;
                let server_arc = Arc::new(server.clone());
                let turn_id = rt.start_turn(server_arc, input).await?;

                Ok((forked, turn_id, notify_rx))
            }
            .await;

            match outcome {
                Ok((forked, turn_id, notify_rx)) => {
                    spawn_fan_out_result_writer(
                        server.clone(),
                        notify_rx,
                        forked.thread_id,
                        turn_id,
                        task_id.clone().unwrap_or_else(|| tool_id.to_string()),
                        expected_artifact_type.clone(),
                    );

                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Completed,
                            error: None,
                            result: Some(serde_json::json!({
                                "thread_id": forked.thread_id,
                                "turn_id": turn_id,
                                "log_path": forked.log_path,
                                "last_seq": forked.last_seq,
                            })),
                        })
                        .await?;

                    Ok(serde_json::json!({
                        "tool_id": tool_id,
                        "thread_id": forked.thread_id,
                        "turn_id": turn_id,
                        "log_path": forked.log_path,
                        "last_seq": forked.last_seq,
                    }))
                }
                Err(err) => {
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Failed,
                            error: Some(err.to_string()),
                            result: None,
                        })
                        .await?;
                    Err(err)
                }
            }
        }
        _ => anyhow::bail!("unknown tool: {tool_name}"),
    }
}

fn spawn_fan_out_result_writer(
    server: super::Server,
    mut notify_rx: tokio::sync::broadcast::Receiver<String>,
    thread_id: pm_protocol::ThreadId,
    turn_id: TurnId,
    task_id: String,
    expected_artifact_type: String,
) {
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(line) => {
                    let Ok(val) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if val.get("method").and_then(Value::as_str) != Some("turn/completed") {
                        continue;
                    }
                    let Some(params) = val.get("params") else {
                        continue;
                    };
                    let Ok(event) = serde_json::from_value::<pm_protocol::ThreadEvent>(params.clone())
                    else {
                        continue;
                    };
                    if event.thread_id != thread_id {
                        continue;
                    }
                    let pm_protocol::ThreadEventKind::TurnCompleted {
                        turn_id: completed_turn_id,
                        status,
                        reason,
                    } = event.kind
                    else {
                        continue;
                    };
                    if completed_turn_id != turn_id {
                        continue;
                    }

                    let payload = serde_json::json!({
                        "task_id": task_id,
                        "thread_id": thread_id,
                        "turn_id": turn_id,
                        "status": status,
                        "reason": reason,
                    });
                    let text = match serde_json::to_string_pretty(&payload) {
                        Ok(json) => format!("```json\n{json}\n```\n"),
                        Err(_) => payload.to_string(),
                    };

                    let _ = super::handle_artifact_write(
                        &server,
                        super::ArtifactWriteParams {
                            thread_id,
                            turn_id: Some(turn_id),
                            approval_id: None,
                            artifact_id: None,
                            artifact_type: expected_artifact_type,
                            summary: "fan-out result".to_string(),
                            text,
                        },
                    )
                    .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

#[cfg(test)]
mod agent_spawn_guard_tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn build_test_server(pm_root: PathBuf) -> super::super::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        super::super::Server {
            cwd: pm_root.clone(),
            notify_tx,
            thread_store: super::super::ThreadStore::new(super::super::PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    #[tokio::test]
    async fn agent_spawn_denies_isolated_write_workspace_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "input": "x",
                "workspace_mode": "isolated_write",
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["workspace_mode"].as_str().unwrap_or(""), "isolated_write");
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_denies_disallowed_child_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "input": "x",
                "mode": "coder",
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert!(result["allowed_modes"].as_array().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_enforces_default_max_concurrent_subagents() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(pm_protocol::ThreadEventKind::TurnStarted {
                    turn_id: pm_protocol::TurnId::new(),
                    input: "child".to_string(),
                })
                .await?;
            drop(child);

            let tool_id = pm_protocol::ToolId::new();
            parent
                .append(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "input": "x",
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }
}
