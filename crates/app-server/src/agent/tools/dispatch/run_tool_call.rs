async fn run_tool_call(
    server: &super::Server,
    thread_id: omne_protocol::ThreadId,
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
    parent_thread_id: omne_protocol::ThreadId,
) -> anyhow::Result<Vec<omne_protocol::ThreadId>> {
    let events = server
        .thread_store
        .read_events_since(parent_thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", parent_thread_id))?;

    let mut spawned_tool_ids = std::collections::HashSet::<omne_protocol::ToolId>::new();
    for event in &events {
        if let omne_protocol::ThreadEventKind::ToolStarted { tool_id, tool, .. } = &event.kind
            && tool == "subagent/spawn"
        {
            spawned_tool_ids.insert(*tool_id);
        }
    }

    let mut spawned_threads = std::collections::BTreeSet::<omne_protocol::ThreadId>::new();
    for event in &events {
        let omne_protocol::ThreadEventKind::ToolCompleted {
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
        if !matches!(status, omne_protocol::ToolStatus::Completed) {
            continue;
        }
        let Some(result) = result.as_ref() else {
            continue;
        };

        let mut record_thread_id = |thread_id: &str| {
            if let Ok(thread_id) = thread_id.parse::<omne_protocol::ThreadId>() {
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

