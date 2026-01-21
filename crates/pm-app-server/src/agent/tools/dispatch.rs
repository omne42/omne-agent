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
        "agent_spawn" => {
            #[derive(Debug, Deserialize)]
            struct ForkResult {
                thread_id: pm_protocol::ThreadId,
                log_path: String,
                last_seq: u64,
            }

            let args: AgentSpawnArgs = serde_json::from_value(args)?;
            if args.input.trim().is_empty() {
                anyhow::bail!("input must not be empty");
            }

            let forked =
                super::handle_thread_fork(server, super::ThreadForkParams { thread_id }).await?;
            let forked: ForkResult = serde_json::from_value(forked)?;

            if args.model.is_some() || args.openai_base_url.is_some() {
                super::handle_thread_configure(
                    server,
                    super::ThreadConfigureParams {
                        thread_id: forked.thread_id,
                        approval_policy: None,
                        sandbox_policy: None,
                        mode: None,
                        model: args.model,
                        openai_base_url: args.openai_base_url,
                    },
                )
                .await?;
            }

            let rt = server.get_or_load_thread(forked.thread_id).await?;
            let server_arc = Arc::new(server.clone());
            let turn_id = rt.start_turn(server_arc, args.input).await?;

            Ok(serde_json::json!({
                "thread_id": forked.thread_id,
                "turn_id": turn_id,
                "log_path": forked.log_path,
                "last_seq": forked.last_seq,
            }))
        }
        _ => anyhow::bail!("unknown tool: {tool_name}"),
    }
}
