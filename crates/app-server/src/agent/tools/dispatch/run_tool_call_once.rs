async fn run_tool_call_once(
    server: &super::Server,
    thread_id: omne_agent_protocol::ThreadId,
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
                    root: args.root,
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
                    root: args.root,
                    path_prefix: args.path_prefix,
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
                    root: args.root,
                    path_prefix: args.path_prefix,
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
        "repo_search" => {
            let args: RepoSearchArgs = serde_json::from_value(args)?;
            super::handle_repo_search(
                server,
                super::RepoSearchParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    query: args.query,
                    is_regex: args.is_regex,
                    include_glob: args.include_glob,
                    max_matches: args.max_matches,
                    max_bytes_per_file: args.max_bytes_per_file,
                    max_files: args.max_files,
                },
            )
            .await
        }
        "repo_index" => {
            let args: RepoIndexArgs = serde_json::from_value(args)?;
            super::handle_repo_index(
                server,
                super::RepoIndexParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    include_glob: args.include_glob,
                    max_files: args.max_files,
                },
            )
            .await
        }
        "repo_symbols" => {
            let args: RepoSymbolsArgs = serde_json::from_value(args)?;
            super::handle_repo_symbols(
                server,
                super::RepoSymbolsParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    include_glob: args.include_glob,
                    max_files: args.max_files,
                    max_bytes_per_file: args.max_bytes_per_file,
                    max_symbols: args.max_symbols,
                },
            )
            .await
        }
        "mcp_list_servers" => {
            super::handle_mcp_list_servers(
                server,
                super::McpListServersParams {
                    thread_id,
                    turn_id,
                    approval_id,
                },
            )
            .await
        }
        "mcp_list_tools" => {
            let args: McpListToolsArgs = serde_json::from_value(args)?;
            super::handle_mcp_list_tools(
                server,
                super::McpListToolsParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                },
            )
            .await
        }
        "mcp_list_resources" => {
            let args: McpListResourcesArgs = serde_json::from_value(args)?;
            super::handle_mcp_list_resources(
                server,
                super::McpListResourcesParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                },
            )
            .await
        }
        "mcp_call" => {
            let args: McpCallArgs = serde_json::from_value(args)?;
            super::handle_mcp_call(
                server,
                super::McpCallParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                    tool: args.tool,
                    arguments: args.arguments,
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
        "thread_diff" => {
            let args: ThreadDiffArgs = serde_json::from_value(args)?;
            super::handle_thread_diff(
                server,
                super::ThreadDiffParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    max_bytes: args.max_bytes,
                    wait_seconds: args.wait_seconds,
                },
            )
            .await
        }
        "thread_state" => {
            let args: ThreadStateArgs = serde_json::from_value(args)?;
            let thread_id: omne_agent_protocol::ThreadId = args.thread_id.parse()?;
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
            let thread_id: omne_agent_protocol::ThreadId = args.thread_id.parse()?;
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
            let args: AgentSpawnArgs = serde_json::from_value(args)?;
            if args.tasks.is_empty() {
                anyhow::bail!("tasks must not be empty");
            }

            let normalize_optional = |value: Option<String>| {
                value
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            };
            let normalize_thinking = |value: Option<String>| -> anyhow::Result<Option<String>> {
                let Some(value) = value else {
                    return Ok(None);
                };
                let value = value.trim();
                if value.is_empty() {
                    return Ok(None);
                }
                let value = value.to_ascii_lowercase();
                match value.as_str() {
                    "small" | "medium" | "high" | "xhigh" | "unsupported" => Ok(Some(value)),
                    other => anyhow::bail!(
                        "invalid thinking: {other} (expected: small|medium|high|xhigh|unsupported)"
                    ),
                }
            };

            let default_spawn_mode = args.spawn_mode.unwrap_or(AgentSpawnMode::New);
            let default_mode =
                normalize_optional(args.mode).unwrap_or_else(|| "reviewer".to_string());
            let default_workspace_mode =
                args.workspace_mode.unwrap_or(AgentSpawnWorkspaceMode::ReadOnly);
            let default_openai_provider = normalize_optional(args.openai_provider);
            let default_model = normalize_optional(args.model);
            let default_thinking = normalize_thinking(args.thinking)?;
            let default_openai_base_url = normalize_optional(args.openai_base_url);
            let default_expected_artifact_type =
                normalize_optional(args.expected_artifact_type)
                    .unwrap_or_else(|| "fan_out_result".to_string());

            let mut seen_ids = std::collections::BTreeSet::<String>::new();
            let mut plans = Vec::<SubagentSpawnTaskPlan>::new();
            for task in args.tasks {
                let id = task.id.trim().to_string();
                if id.is_empty() {
                    anyhow::bail!("tasks[].id must not be empty");
                }
                if !seen_ids.insert(id.clone()) {
                    anyhow::bail!("duplicate task id: {id}");
                }

                let input = task.input.trim().to_string();
                if input.is_empty() {
                    anyhow::bail!("task input must not be empty (task_id={id})");
                }

                let title = task.title.unwrap_or_default().trim().to_string();

                let mut depends_on = Vec::<String>::new();
                let mut seen_deps = std::collections::BTreeSet::<String>::new();
                for dep in task.depends_on {
                    let dep = dep.trim().to_string();
                    if dep.is_empty() {
                        continue;
                    }
                    if dep == id {
                        anyhow::bail!("task depends_on itself: {id}");
                    }
                    if seen_deps.insert(dep.clone()) {
                        depends_on.push(dep);
                    }
                }

                let spawn_mode = task.spawn_mode.unwrap_or(default_spawn_mode);
                let mode = normalize_optional(task.mode).unwrap_or_else(|| default_mode.clone());
                let workspace_mode = task
                    .workspace_mode
                    .unwrap_or(default_workspace_mode);
                let openai_provider = normalize_optional(task.openai_provider)
                    .or_else(|| default_openai_provider.clone());
                let model = normalize_optional(task.model).or_else(|| default_model.clone());
                let thinking =
                    normalize_thinking(task.thinking)?.or_else(|| default_thinking.clone());
                let openai_base_url =
                    normalize_optional(task.openai_base_url).or_else(|| default_openai_base_url.clone());
                let expected_artifact_type = normalize_optional(task.expected_artifact_type)
                    .unwrap_or_else(|| default_expected_artifact_type.clone());

                plans.push(SubagentSpawnTaskPlan {
                    id,
                    title,
                    input,
                    depends_on,
                    spawn_mode,
                    mode,
                    workspace_mode,
                    openai_provider,
                    model,
                    thinking,
                    openai_base_url,
                    expected_artifact_type,
                });
            }

            let mut by_id = std::collections::HashMap::<String, usize>::new();
            for (idx, task) in plans.iter().enumerate() {
                by_id.insert(task.id.clone(), idx);
            }
            for task in &plans {
                for dep in &task.depends_on {
                    if !by_id.contains_key(dep) {
                        anyhow::bail!("unknown depends_on: {dep} (task_id={})", task.id);
                    }
                }
            }

            let mut indegree = std::collections::HashMap::<String, usize>::new();
            let mut edges = std::collections::HashMap::<String, Vec<String>>::new();
            for task in &plans {
                indegree.insert(task.id.clone(), task.depends_on.len());
                for dep in &task.depends_on {
                    edges.entry(dep.clone()).or_default().push(task.id.clone());
                }
            }
            let mut queue = std::collections::VecDeque::<String>::new();
            for (id, degree) in &indegree {
                if *degree == 0 {
                    queue.push_back(id.clone());
                }
            }
            let mut visited = 0usize;
            while let Some(id) = queue.pop_front() {
                visited += 1;
                if let Some(children) = edges.get(&id) {
                    for child in children {
                        if let Some(degree) = indegree.get_mut(child) {
                            *degree = degree.saturating_sub(1);
                            if *degree == 0 {
                                queue.push_back(child.clone());
                            }
                        }
                    }
                }
            }
            if visited != plans.len() {
                anyhow::bail!("task dependencies contain a cycle");
            }

            let task_previews = plans
                .iter()
                .map(|task| {
                    let input_preview = omne_agent_core::redact_text(&truncate_chars(&task.input, 400));
                    serde_json::json!({
                        "id": task.id.clone(),
                        "spawn_mode": spawn_mode_label(task.spawn_mode),
                        "mode": task.mode.clone(),
                        "workspace_mode": workspace_mode_label(task.workspace_mode),
                        "depends_on": task.depends_on.clone(),
                        "openai_provider": task.openai_provider.clone(),
                        "model": task.model.clone(),
                        "thinking": task.thinking.clone(),
                        "openai_base_url": task.openai_base_url.clone(),
                        "input_chars": task.input.chars().count(),
                        "input_preview": input_preview,
                    })
                })
                .collect::<Vec<_>>();

            let approval_params = serde_json::json!({
                "task_count": plans.len(),
                "tasks": task_previews,
                "default_spawn_mode": spawn_mode_label(default_spawn_mode),
                "default_mode": default_mode,
                "default_workspace_mode": workspace_mode_label(default_workspace_mode),
                "default_expected_artifact_type": default_expected_artifact_type,
                "openai_provider": default_openai_provider,
                "model": default_model,
                "thinking": default_thinking,
                "openai_base_url": default_openai_base_url,
            });

            let tool_id = omne_agent_protocol::ToolId::new();
            let (thread_rt, thread_root) = super::load_thread_root(server, thread_id).await?;
            let (approval_policy, mode_name, thread_cwd) = {
                let handle = thread_rt.handle.lock().await;
                let state = handle.state();
                (state.approval_policy, state.mode.clone(), state.cwd.clone())
            };
            let thread_cwd = thread_cwd.ok_or_else(|| {
                anyhow::anyhow!("thread cwd is missing: {}", thread_id)
            })?;

            let catalog = omne_agent_core::modes::ModeCatalog::load(&thread_root).await;
            let Some(mode) = catalog.mode(&mode_name) else {
                let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
                let decision = omne_agent_core::modes::Decision::Deny;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
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

            let isolated_tasks = plans
                .iter()
                .filter(|task| matches!(task.workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite))
                .map(|task| task.id.clone())
                .collect::<Vec<_>>();
            if !isolated_tasks.is_empty() {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
                        error: Some("workspace_mode=isolated_write is not supported yet".to_string()),
                        result: Some(serde_json::json!({
                            "workspace_mode": workspace_mode_label(AgentSpawnWorkspaceMode::IsolatedWrite),
                            "task_ids": isolated_tasks,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "workspace_mode": workspace_mode_label(AgentSpawnWorkspaceMode::IsolatedWrite),
                    "task_ids": isolated_tasks,
                }));
            }

            let base_decision = mode.permissions.subagent.spawn.decision;
            let effective_decision = match mode.tool_overrides.get("subagent/spawn").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };

            if effective_decision == omne_agent_core::modes::Decision::Deny {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
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

            let max_concurrent_subagents =
                parse_env_usize("OMNE_AGENT_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
            if let Some(allowed) = mode.permissions.subagent.spawn.allowed_modes.as_ref() {
                let mut disallowed = std::collections::BTreeSet::<String>::new();
                for task in &plans {
                    if !allowed.iter().any(|name| name == &task.mode) {
                        disallowed.insert(task.mode.clone());
                    }
                }
                if !disallowed.is_empty() {
                    let requested_modes = disallowed.into_iter().collect::<Vec<_>>();
                    thread_rt
                        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                            tool_id,
                            turn_id,
                            tool: "subagent/spawn".to_string(),
                            params: Some(approval_params),
                        })
                        .await?;
                    thread_rt
                        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: omne_agent_protocol::ToolStatus::Denied,
                            error: Some("mode forbids spawning this subagent mode".to_string()),
                            result: Some(serde_json::json!({
                                "mode": mode_name,
                                "decision": effective_decision,
                                "requested_modes": requested_modes,
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
            }

            let active_threads = if max_concurrent_subagents > 0 {
                active_subagent_threads(server, thread_id).await?
            } else {
                Vec::new()
            };
            if max_concurrent_subagents > 0 && active_threads.len() >= max_concurrent_subagents {
                let active = active_threads.len();
                let active_thread_ids = active_threads
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>();

                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
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

            if effective_decision == omne_agent_core::modes::Decision::Prompt {
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
                            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id,
                                tool: "subagent/spawn".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: omne_agent_protocol::ToolStatus::Denied,
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
                .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id,
                    tool: "subagent/spawn".to_string(),
                    params: Some(approval_params),
                })
                .await?;

            let outcome: anyhow::Result<Vec<Value>> = async {
                let mut tasks = Vec::<SubagentSpawnTask>::with_capacity(plans.len());
                for plan in plans {
                    let spawned = match plan.spawn_mode {
                        AgentSpawnMode::Fork => {
                            let forked = super::handle_thread_fork(
                                server,
                                super::ThreadForkParams { thread_id },
                            )
                            .await?;
                            serde_json::from_value::<SpawnedThread>(forked)?
                        }
                        AgentSpawnMode::New => create_new_thread(server, &thread_cwd).await?,
                    };

                    let approval_override = if matches!(plan.spawn_mode, AgentSpawnMode::New) {
                        Some(approval_policy)
                    } else {
                        None
                    };
                    super::handle_thread_configure(
                        server,
                        super::ThreadConfigureParams {
                            thread_id: spawned.thread_id,
                            approval_policy: approval_override,
                            sandbox_policy: Some(omne_agent_protocol::SandboxPolicy::ReadOnly),
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: Some(plan.mode.clone()),
                            openai_provider: plan.openai_provider.clone(),
                            model: plan.model.clone(),
                            thinking: plan.thinking.clone(),
                            openai_base_url: plan.openai_base_url.clone(),
                            allowed_tools: None,
                        },
                    )
                    .await?;

                    tasks.push(SubagentSpawnTask {
                        id: plan.id,
                        title: plan.title,
                        input: plan.input,
                        depends_on: plan.depends_on,
                        spawn_mode: plan.spawn_mode,
                        mode: plan.mode,
                        workspace_mode: plan.workspace_mode,
                        openai_provider: plan.openai_provider,
                        model: plan.model,
                        thinking: plan.thinking,
                        openai_base_url: plan.openai_base_url,
                        expected_artifact_type: plan.expected_artifact_type,
                        thread_id: spawned.thread_id,
                        log_path: spawned.log_path,
                        last_seq: spawned.last_seq,
                        turn_id: None,
                        status: SubagentTaskStatus::Pending,
	                        error: None,
	                    });
	                }
	                let external_active = active_threads
	                    .into_iter()
	                    .collect::<std::collections::HashSet<_>>();
	                let mut schedule = SubagentSpawnSchedule::new(
                    tasks,
                    external_active,
                    max_concurrent_subagents,
                );
                schedule.start_ready_tasks(server).await;
                let snapshot = schedule.snapshot();
                spawn_subagent_scheduler(server.clone(), schedule);
                Ok(snapshot)
            }
            .await;
            match outcome {
                Ok(tasks) => {
                    thread_rt
                        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: omne_agent_protocol::ToolStatus::Completed,
                            error: None,
                            result: Some(serde_json::json!({
                                "tasks": tasks,
                            })),
                        })
                        .await?;
                    Ok(serde_json::json!({
                        "tool_id": tool_id,
                        "tasks": tasks,
                    }))
                }
                Err(err) => {
                    thread_rt
                        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: omne_agent_protocol::ToolStatus::Failed,
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
