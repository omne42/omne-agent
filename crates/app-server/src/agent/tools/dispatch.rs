async fn run_tool_call(
    server: &super::Server,
    thread_id: pm_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: Value,
    cancel: CancellationToken,
    redact_output: bool,
) -> anyhow::Result<ToolCallOutcome> {
    let tool_action = super::hook_tool_name_from_agent_tool(tool_name).unwrap_or(tool_name);

    let pre_hook_contexts = match turn_id {
        Some(turn_id) => {
            super::run_pre_tool_use_hooks(server, thread_id, turn_id, tool_action, &args).await
        }
        None => Vec::new(),
    };

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
            let output = if redact_output {
                redact_tool_output(output)
            } else {
                output
            };
            let post_hook_contexts = match turn_id {
                Some(turn_id) => {
                    super::run_post_tool_use_hooks(server, thread_id, turn_id, tool_action, &args, &output)
                        .await
                }
                None => Vec::new(),
            };
            let hook_messages = hook_contexts_to_messages(&pre_hook_contexts, &post_hook_contexts);
            return Ok(ToolCallOutcome {
                output,
                hook_messages,
            });
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
                let output = serde_json::json!({
                    "denied": true,
                    "approval_id": requested,
                    "decision": outcome.decision,
                    "remember": outcome.remember,
                    "reason": outcome.reason,
                });
                let output = if redact_output {
                    redact_tool_output(output)
                } else {
                    output
                };
                let post_hook_contexts = match turn_id {
                    Some(turn_id) => {
                        super::run_post_tool_use_hooks(
                            server,
                            thread_id,
                            turn_id,
                            tool_action,
                            &args,
                            &output,
                        )
                        .await
                    }
                    None => Vec::new(),
                };
                let hook_messages =
                    hook_contexts_to_messages(&pre_hook_contexts, &post_hook_contexts);
                return Ok(ToolCallOutcome {
                    output,
                    hook_messages,
                });
            }
        }
    }

    Err(AgentTurnError::BudgetExceeded { budget: "retries" }.into())
}

struct ToolCallOutcome {
    output: Value,
    hook_messages: Vec<OpenAiItem>,
}

fn hook_contexts_to_messages(
    pre: &[super::HookAdditionalContext],
    post: &[super::HookAdditionalContext],
) -> Vec<OpenAiItem> {
    let mut out = Vec::new();
    if let Some(item) = hook_contexts_to_message("hooks/pre_tool_use", pre) {
        out.push(item);
    }
    if let Some(item) = hook_contexts_to_message("hooks/post_tool_use", post) {
        out.push(item);
    }
    out
}

fn hook_contexts_to_message(
    label: &str,
    contexts: &[super::HookAdditionalContext],
) -> Option<OpenAiItem> {
    if contexts.is_empty() {
        return None;
    }

    let mut text = String::new();
    text.push_str(&format!("# {label}\n\n"));
    for ctx in contexts {
        text.push_str(&format!(
            "_hook_point: {}_\n_hook_id: {}_\n_path: {}_\n\n",
            ctx.hook_point.as_str(),
            ctx.hook_id,
            ctx.context_path.display()
        ));
        if let Some(summary) = ctx.summary.as_deref() {
            text.push_str(&format!("## {}\n\n", summary.trim()));
        }
        text.push_str(ctx.text.trim());
        text.push_str("\n\n");
    }

    Some(serde_json::json!({
        "type": "message",
        "role": "system",
        "content": [{ "type": "input_text", "text": text }],
    }))
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
        let Some(result) = result.as_ref() else {
            continue;
        };

        let mut record_thread_id = |thread_id: &str| {
            if let Ok(thread_id) = thread_id.parse::<pm_protocol::ThreadId>() {
                spawned_threads.insert(thread_id);
            }
        };

        if let Some(thread_id) = result.get("thread_id").and_then(|value| value.as_str()) {
            record_thread_id(thread_id);
        }

        if let Some(thread_ids) = result.get("thread_ids").and_then(|value| value.as_array()) {
            for thread_id in thread_ids.iter().filter_map(|value| value.as_str()) {
                record_thread_id(thread_id);
            }
        }

        if let Some(tasks) = result.get("tasks").and_then(|value| value.as_array()) {
            for task in tasks {
                if let Some(thread_id) = task.get("thread_id").and_then(|value| value.as_str()) {
                    record_thread_id(thread_id);
                }
            }
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
            let args: AgentSpawnArgs = serde_json::from_value(args)?;
            if args.tasks.is_empty() {
                anyhow::bail!("tasks must not be empty");
            }

            let normalize_optional = |value: Option<String>| {
                value
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            };

            let default_spawn_mode = args.spawn_mode.unwrap_or(AgentSpawnMode::New);
            let default_mode =
                normalize_optional(args.mode).unwrap_or_else(|| "reviewer".to_string());
            let default_workspace_mode =
                args.workspace_mode.unwrap_or(AgentSpawnWorkspaceMode::ReadOnly);
            let default_model = normalize_optional(args.model);
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
                let model = normalize_optional(task.model).or_else(|| default_model.clone());
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
                    model,
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
                    let input_preview = pm_core::redact_text(&truncate_chars(&task.input, 400));
                    serde_json::json!({
                        "id": task.id.clone(),
                        "spawn_mode": spawn_mode_label(task.spawn_mode),
                        "mode": task.mode.clone(),
                        "workspace_mode": workspace_mode_label(task.workspace_mode),
                        "depends_on": task.depends_on.clone(),
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
                "model": default_model,
                "openai_base_url": default_openai_base_url,
            });

            let tool_id = pm_protocol::ToolId::new();
            let (thread_rt, thread_root) = super::load_thread_root(server, thread_id).await?;
            let (approval_policy, mode_name, thread_cwd) = {
                let handle = thread_rt.handle.lock().await;
                let state = handle.state();
                (state.approval_policy, state.mode.clone(), state.cwd.clone())
            };
            let thread_cwd = thread_cwd.ok_or_else(|| {
                anyhow::anyhow!("thread cwd is missing: {}", thread_id)
            })?;

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

            let isolated_tasks = plans
                .iter()
                .filter(|task| matches!(task.workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite))
                .map(|task| task.id.clone())
                .collect::<Vec<_>>();
            if !isolated_tasks.is_empty() {
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

            let max_concurrent_subagents =
                parse_env_usize("CODE_PM_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
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
                            sandbox_policy: Some(pm_protocol::SandboxPolicy::ReadOnly),
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: Some(plan.mode.clone()),
                            model: plan.model.clone(),
                            thinking: None,
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
                        model: plan.model,
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
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Completed,
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

#[derive(Debug)]
struct SubagentSpawnTaskPlan {
    id: String,
    title: String,
    input: String,
    depends_on: Vec<String>,
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    model: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
}

#[derive(Debug, Deserialize)]
struct SpawnedThread {
    thread_id: ThreadId,
    log_path: String,
    last_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubagentTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug)]
struct SubagentSpawnTask {
    id: String,
    title: String,
    input: String,
    depends_on: Vec<String>,
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    model: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
    thread_id: ThreadId,
    log_path: String,
    last_seq: u64,
    turn_id: Option<TurnId>,
    status: SubagentTaskStatus,
    error: Option<String>,
}

struct SubagentSpawnSchedule {
    tasks: Vec<SubagentSpawnTask>,
    by_id: std::collections::HashMap<String, usize>,
    completed: std::collections::HashSet<String>,
    running_by_thread: std::collections::HashMap<ThreadId, (String, TurnId)>,
    external_active: std::collections::HashSet<ThreadId>,
    max_concurrent: usize,
}

impl SubagentSpawnSchedule {
    fn new(
        tasks: Vec<SubagentSpawnTask>,
        external_active: std::collections::HashSet<ThreadId>,
        max_concurrent: usize,
    ) -> Self {
        let mut by_id = std::collections::HashMap::<String, usize>::new();
        let mut completed = std::collections::HashSet::<String>::new();
        let mut running_by_thread = std::collections::HashMap::<ThreadId, (String, TurnId)>::new();

        for (idx, task) in tasks.iter().enumerate() {
            by_id.insert(task.id.clone(), idx);
            match task.status {
                SubagentTaskStatus::Completed | SubagentTaskStatus::Failed => {
                    completed.insert(task.id.clone());
                }
                SubagentTaskStatus::Running => {
                    if let Some(turn_id) = task.turn_id {
                        running_by_thread.insert(task.thread_id, (task.id.clone(), turn_id));
                    }
                }
                SubagentTaskStatus::Pending => {}
            }
        }

        Self {
            tasks,
            by_id,
            completed,
            running_by_thread,
            external_active,
            max_concurrent,
        }
    }

    fn is_done(&self) -> bool {
        self.completed.len() >= self.tasks.len()
    }

    fn available_slots(&self) -> usize {
        if self.max_concurrent == 0 {
            usize::MAX
        } else {
            self.max_concurrent
                .saturating_sub(self.running_by_thread.len() + self.external_active.len())
        }
    }

    async fn start_ready_tasks(&mut self, server: &super::Server) {
        let mut available = self.available_slots();
        if available == 0 {
            return;
        }

        for idx in 0..self.tasks.len() {
            if available == 0 {
                break;
            }
            let task = &mut self.tasks[idx];
            if task.status != SubagentTaskStatus::Pending {
                continue;
            }
            if !task
                .depends_on
                .iter()
                .all(|id| self.completed.contains(id))
            {
                continue;
            }

            let task_id = task.id.clone();
            match start_subagent_turn(server, task).await {
                Ok(turn_id) => {
                    task.turn_id = Some(turn_id);
                    task.status = SubagentTaskStatus::Running;
                    self.running_by_thread
                        .insert(task.thread_id, (task_id, turn_id));
                    available = available.saturating_sub(1);
                }
                Err(err) => {
                    task.status = SubagentTaskStatus::Failed;
                    task.error = Some(err.to_string());
                    self.completed.insert(task_id);
                }
            }
        }
    }

    fn handle_turn_completed(&mut self, thread_id: ThreadId, turn_id: TurnId) {
        if self.external_active.remove(&thread_id) {
            return;
        }
        let Some((task_id, expected_turn_id)) = self.running_by_thread.get(&thread_id).cloned()
        else {
            return;
        };
        if expected_turn_id != turn_id {
            return;
        }
        self.running_by_thread.remove(&thread_id);
        if let Some(idx) = self.by_id.get(&task_id).copied() {
            self.tasks[idx].status = SubagentTaskStatus::Completed;
            self.completed.insert(task_id);
        }
    }

    fn snapshot(&self) -> Vec<Value> {
        self.tasks
            .iter()
            .map(|task| {
                serde_json::json!({
                    "id": task.id.clone(),
                    "title": task.title.clone(),
                    "spawn_mode": spawn_mode_label(task.spawn_mode),
                    "mode": task.mode.clone(),
                    "workspace_mode": workspace_mode_label(task.workspace_mode),
                    "thread_id": task.thread_id,
                    "turn_id": task.turn_id,
                    "log_path": task.log_path.clone(),
                    "last_seq": task.last_seq,
                    "depends_on": task.depends_on.clone(),
                    "expected_artifact_type": task.expected_artifact_type.clone(),
                    "model": task.model.clone(),
                    "openai_base_url": task.openai_base_url.clone(),
                    "status": task_status_label(task.status),
                    "error": task.error.clone(),
                })
            })
            .collect::<Vec<_>>()
    }
}

fn spawn_subagent_scheduler(server: super::Server, mut schedule: SubagentSpawnSchedule) {
    tokio::spawn(async move {
        let mut notify_rx = server.notify_tx.subscribe();
        loop {
            schedule.start_ready_tasks(&server).await;
            if schedule.is_done() {
                return;
            }

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
                    let pm_protocol::ThreadEventKind::TurnCompleted { turn_id, .. } = event.kind else {
                        continue;
                    };
                    schedule.handle_turn_completed(event.thread_id, turn_id);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

fn spawn_mode_label(mode: AgentSpawnMode) -> &'static str {
    match mode {
        AgentSpawnMode::Fork => "fork",
        AgentSpawnMode::New => "new",
    }
}

fn workspace_mode_label(mode: AgentSpawnWorkspaceMode) -> &'static str {
    match mode {
        AgentSpawnWorkspaceMode::ReadOnly => "read_only",
        AgentSpawnWorkspaceMode::IsolatedWrite => "isolated_write",
    }
}

fn task_status_label(status: SubagentTaskStatus) -> &'static str {
    match status {
        SubagentTaskStatus::Pending => "pending",
        SubagentTaskStatus::Running => "running",
        SubagentTaskStatus::Completed => "completed",
        SubagentTaskStatus::Failed => "failed",
    }
}

async fn start_subagent_turn(
    server: &super::Server,
    task: &SubagentSpawnTask,
) -> anyhow::Result<TurnId> {
    let rt = server.get_or_load_thread(task.thread_id).await?;
    let server_arc = Arc::new(server.clone());
    let turn_id = rt
        .start_turn(
            server_arc,
            task.input.clone(),
            None,
            None,
            pm_protocol::TurnPriority::Background,
        )
        .await?;

    let notify_rx = server.notify_tx.subscribe();
    spawn_fan_out_result_writer(
        server.clone(),
        notify_rx,
        task.thread_id,
        turn_id,
        task.id.clone(),
        task.expected_artifact_type.clone(),
    );

    Ok(turn_id)
}

async fn create_new_thread(
    server: &super::Server,
    cwd: &str,
) -> anyhow::Result<SpawnedThread> {
    let handle = server
        .thread_store
        .create_thread(PathBuf::from(cwd))
        .await?;
    let thread_id = handle.thread_id();
    let log_path = handle.log_path().display().to_string();
    let last_seq = handle.last_seq().0;

    let rt = Arc::new(crate::ThreadRuntime::new(handle, server.notify_tx.clone()));
    server.threads.lock().await.insert(thread_id, rt);

    Ok(SpawnedThread {
        thread_id,
        log_path,
        last_seq,
    })
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
            mcp: Arc::new(tokio::sync::Mutex::new(super::super::McpManager::default())),
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
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "workspace_mode": "isolated_write",
                }],
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
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "coder",
                }],
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
                    context_refs: None,
                    attachments: None,
                    priority: pm_protocol::TurnPriority::Foreground,
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
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                }],
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

#[cfg(test)]
mod reference_repo_file_tools_tests {
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
            mcp: Arc::new(tokio::sync::Mutex::new(super::super::McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    #[tokio::test]
    async fn file_glob_excludes_codepm_reference_dir_for_workspace_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".codepm_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".codepm_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = build_test_server(tmp.path().join("pm_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "pattern": "**/*.txt" }),
            None,
        )
        .await?;

        let paths = result["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("hello.txt")));
        assert!(
            !paths
                .iter()
                .any(|p| p.as_str().unwrap_or("").contains(".codepm_data/reference/"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_and_read_can_use_reference_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".codepm_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".codepm_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = build_test_server(tmp.path().join("pm_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let glob = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "root": "reference", "pattern": "**/*.txt" }),
            None,
        )
        .await?;
        let paths = glob["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("ref.txt")));
        assert!(!paths.iter().any(|p| p.as_str() == Some("hello.txt")));

        let read = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await?;
        assert_eq!(read["text"].as_str(), Some("ref\n"));
        assert_eq!(read["root"].as_str(), Some("reference"));
        Ok(())
    }

    #[tokio::test]
    async fn reference_root_fails_closed_when_not_configured() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;

        let server = build_test_server(tmp.path().join("pm_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let err = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await
        .expect_err("expected root=reference to fail when not configured");
        assert!(
            err.to_string().contains("reference repo root")
                || err.to_string().contains(".codepm_data/reference/repo")
        );
        Ok(())
    }
}
