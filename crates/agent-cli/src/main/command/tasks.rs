fn parse_workflow_tasks(body: &str) -> anyhow::Result<Vec<WorkflowTask>> {
    let mut tasks = Vec::<WorkflowTask>::new();
    let mut seen_ids = BTreeSet::<String>::new();

    let mut current_id = None::<String>;
    let mut current_title = String::new();
    let mut current_body = String::new();

    for line in body.split_inclusive('\n') {
        if let Some((id, title)) = parse_task_header(line) {
            if let Some(id) = current_id.take() {
                tasks.push(WorkflowTask {
                    id,
                    title: std::mem::take(&mut current_title),
                    body: std::mem::take(&mut current_body),
                });
            }

            ensure_valid_var_name(&id, "task id")?;
            if !seen_ids.insert(id.clone()) {
                anyhow::bail!("duplicate task id: {id}");
            }
            current_id = Some(id);
            current_title = title;
            continue;
        }

        if current_id.is_some() {
            current_body.push_str(line);
        }
    }

    if let Some(id) = current_id.take() {
        tasks.push(WorkflowTask {
            id,
            title: current_title,
            body: current_body,
        });
    }

    Ok(tasks)
}

fn parse_task_header(line: &str) -> Option<(String, String)> {
    let trimmed = line
        .trim_end_matches(&['\r', '\n'][..])
        .trim_start();
    let rest = trimmed.strip_prefix("## Task:")?.trim();
    if rest.is_empty() {
        return None;
    }

    let mut split_at = None::<usize>;
    for (idx, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            split_at = Some(idx);
            break;
        }
    }

    let (id, title) = match split_at {
        Some(idx) => (&rest[..idx], rest[idx..].trim()),
        None => (rest, ""),
    };

    Some((id.to_string(), title.to_string()))
}

async fn resolve_thread_cwd(app: &mut App, thread_id: ThreadId) -> anyhow::Result<String> {
    let state = app.thread_state(thread_id).await?;
    let cwd = state
        .get("cwd")
        .and_then(Value::as_str)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("thread cwd missing: {thread_id}"))?;
    Ok(cwd.to_string())
}

async fn run_workflow_fan_out(
    app: &mut App,
    parent_thread_id: ThreadId,
    tasks: &[WorkflowTask],
    fan_in_artifact_id: omne_agent_protocol::ArtifactId,
    subagent_fork: bool,
) -> anyhow::Result<Vec<WorkflowTaskResult>> {
    #[derive(Debug, Deserialize)]
    struct ForkResult {
        thread_id: ThreadId,
        last_seq: u64,
    }

    #[derive(Debug)]
    struct ActiveTask {
        task_id: String,
        title: String,
        thread_id: ThreadId,
        turn_id: TurnId,
        since_seq: u64,
        assistant_text: Option<String>,
    }

    let max_concurrent_subagents = parse_env_usize("OMNE_AGENT_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
    let concurrency_limit = if max_concurrent_subagents == 0 {
        tasks.len().max(1)
    } else {
        max_concurrent_subagents
    };

    let parent_cwd = if subagent_fork {
        None
    } else {
        Some(resolve_thread_cwd(app, parent_thread_id).await?)
    };

    let mut pending_idx = 0usize;
    let mut active = Vec::<ActiveTask>::new();
    let mut finished = Vec::<WorkflowTaskResult>::new();

    let started_at = Instant::now();
    let mut last_progress_print = Instant::now();
    let mut last_progress_artifact_write = Instant::now();

    write_fan_out_progress_artifact(
        app,
        parent_thread_id,
        fan_in_artifact_id,
        tasks.len(),
        &finished,
        &active,
        started_at,
    )
    .await?;

    while finished.len() < tasks.len() {
        while active.len() < concurrency_limit && pending_idx < tasks.len() {
            let task = &tasks[pending_idx];
            pending_idx += 1;

            let spawned = if subagent_fork {
                app.thread_fork(parent_thread_id).await?
            } else {
                let cwd = parent_cwd
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("fan-out parent cwd missing"))?;
                app.thread_start(Some(cwd.to_string())).await?
            };
            let forked: ForkResult = serde_json::from_value(spawned).with_context(|| {
                if subagent_fork {
                    "parse thread/fork".to_string()
                } else {
                    "parse thread/start".to_string()
                }
            })?;

            let _ = app
                .rpc(
                    "thread/configure",
                    serde_json::json!({
                        "thread_id": forked.thread_id,
                        "approval_policy": null,
                        "sandbox_policy": omne_agent_protocol::SandboxPolicy::ReadOnly,
                        "sandbox_writable_roots": null,
                        "sandbox_network_access": null,
                        "mode": "reviewer",
                        "model": null,
                        "openai_base_url": null,
                        "allowed_tools": null,
                    }),
                )
                .await?;

            let mut input = format!(
                "You are a read-only subagent.\nTask: {}{}\n\n",
                task.id,
                if task.title.is_empty() {
                    "".to_string()
                } else {
                    format!(" ({})", task.title)
                }
            );
            let body = task.body.trim();
            if body.is_empty() {
                input.push_str("(no task body)\n");
            } else {
                input.push_str(body);
                input.push('\n');
            }
            input.push_str("\nReturn a concise result.\n");

            let turn_id = app
                .turn_start(
                    forked.thread_id,
                    input,
                    Some(omne_agent_protocol::TurnPriority::Background),
                )
                .await?;
            active.push(ActiveTask {
                task_id: task.id.clone(),
                title: task.title.clone(),
                thread_id: forked.thread_id,
                turn_id,
                since_seq: forked.last_seq,
                assistant_text: None,
            });
        }

        let mut idx = 0usize;
        while idx < active.len() {
            let mut done: Option<(TurnStatus, Option<String>)> = None;
            let thread_id = active[idx].thread_id;
            let turn_id = active[idx].turn_id;

            loop {
                let resp = app
                    .thread_subscribe(thread_id, active[idx].since_seq, Some(1_000), Some(0))
                    .await?;
                active[idx].since_seq = resp.last_seq;

                for event in resp.events {
                    match event.kind {
                        omne_agent_protocol::ThreadEventKind::AssistantMessage {
                            turn_id: Some(msg_turn_id),
                            text,
                            ..
                        } if msg_turn_id == turn_id => {
                            active[idx].assistant_text = Some(text);
                        }
                        omne_agent_protocol::ThreadEventKind::ApprovalRequested { .. } => {
                            anyhow::bail!(
                                "fan-out task needs approval (thread_id={thread_id}); use `omne-agent inbox`"
                            );
                        }
                        omne_agent_protocol::ThreadEventKind::TurnCompleted {
                            turn_id: completed_turn_id,
                            status,
                            reason,
                        } if completed_turn_id == turn_id => {
                            done = Some((status, reason));
                        }
                        _ => {}
                    }
                }

                if !resp.has_more {
                    break;
                }
            }

            if let Some((status, reason)) = done {
                let task = active.remove(idx);
                finished.push(WorkflowTaskResult {
                    task_id: task.task_id,
                    title: task.title,
                    thread_id: task.thread_id,
                    turn_id: task.turn_id,
                    status,
                    reason,
                    assistant_text: task.assistant_text,
                });
                continue;
            }

            idx += 1;
        }

        if finished.len() < tasks.len() {
            if last_progress_print.elapsed() >= Duration::from_secs(1) {
                eprintln!(
                    "[fan-out] completed {}/{} (active={}, max={})",
                    finished.len(),
                    tasks.len(),
                    active.len(),
                    concurrency_limit
                );
                last_progress_print = Instant::now();
            }
            if last_progress_artifact_write.elapsed() >= Duration::from_secs(2) {
                let outcome = write_fan_out_progress_artifact(
                    app,
                    parent_thread_id,
                    fan_in_artifact_id,
                    tasks.len(),
                    &finished,
                    &active,
                    started_at,
                )
                .await;
                if let Err(err) = outcome {
                    eprintln!("[fan-out] progress artifact update failed: {err}");
                } else {
                    last_progress_artifact_write = Instant::now();
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    let outcome = write_fan_out_progress_artifact(
        app,
        parent_thread_id,
        fan_in_artifact_id,
        tasks.len(),
        &finished,
        &active,
        started_at,
    )
    .await;
    if let Err(err) = outcome {
        eprintln!("[fan-out] final progress artifact update failed: {err}");
    }

    let mut by_id = std::collections::HashMap::<String, WorkflowTaskResult>::new();
    for result in finished {
        by_id.insert(result.task_id.clone(), result);
    }

    let mut ordered = Vec::<WorkflowTaskResult>::new();
    for task in tasks {
        if let Some(result) = by_id.remove(&task.id) {
            ordered.push(result);
        }
    }
    Ok(ordered)
}

async fn write_fan_out_progress_artifact<T: std::fmt::Debug>(
    app: &mut App,
    thread_id: ThreadId,
    artifact_id: omne_agent_protocol::ArtifactId,
    total_tasks: usize,
    finished: &[WorkflowTaskResult],
    active: &[T],
    started_at: Instant,
) -> anyhow::Result<()> {
    let elapsed = started_at.elapsed();
    let done = finished.len();

    let eta_seconds = if done == 0 {
        None
    } else {
        let elapsed_secs = elapsed.as_secs_f64();
        let total_secs_estimate = elapsed_secs * (total_tasks as f64) / (done as f64);
        let eta_secs = (total_secs_estimate - elapsed_secs).max(0.0);
        Some(eta_secs.round() as u64)
    };

    let mut text = String::new();
    text.push_str("# Fan-out Progress\n\n");
    text.push_str(&format!("Progress: {done}/{total_tasks}\n\n"));
    text.push_str(&format!("Elapsed: {}s\n\n", elapsed.as_secs()));
    if let Some(eta_seconds) = eta_seconds {
        text.push_str(&format!("ETA (rough): {}s\n\n", eta_seconds));
    }

    text.push_str("Active tasks:\n\n");
    text.push_str("```text\n");
    if active.is_empty() {
        text.push_str("(none)\n");
    } else {
        for entry in active {
            text.push_str(&format!("{entry:?}\n"));
        }
    }
    text.push_str("```\n\n");

    text.push_str("Completed tasks:\n\n");
    if finished.is_empty() {
        text.push_str("- (none)\n");
    } else {
        for result in finished {
            text.push_str(&format!(
                "- {} status={:?} thread_id={} turn_id={}\n",
                result.task_id, result.status, result.thread_id, result.turn_id
            ));
        }
    }

    let v = app
        .rpc(
            "artifact/write",
            serde_json::json!({
                "thread_id": thread_id,
                "turn_id": null,
                "approval_id": null,
                "artifact_id": artifact_id,
                "artifact_type": "fan_in_summary",
                "summary": "fan-in summary",
                "text": text,
            }),
        )
        .await?;
    ensure_approval_and_denial_handled("artifact/write", &v)?;
    Ok(())
}

async fn write_fan_in_summary_artifact(
    app: &mut App,
    thread_id: ThreadId,
    artifact_id: omne_agent_protocol::ArtifactId,
    results: &[WorkflowTaskResult],
) -> anyhow::Result<String> {
    let mut text = String::new();
    text.push_str("# Fan-in Summary\n\n");
    text.push_str(&format!("Tasks: {}\n\n", results.len()));

    text.push_str("| task_id | thread_id | turn_id | status |\n");
    text.push_str("| --- | --- | --- | --- |\n");
    for result in results {
        text.push_str(&format!(
            "| {} | {} | {} | {:?} |\n",
            result.task_id, result.thread_id, result.turn_id, result.status
        ));
    }
    text.push('\n');

    for result in results {
        text.push_str(&format!("## {} {}\n\n", result.task_id, result.title));
        text.push_str(&format!("- status: `{:?}`\n", result.status));
        if let Some(reason) = result.reason.as_deref().filter(|v| !v.trim().is_empty()) {
            text.push_str(&format!("- reason: {}\n", reason));
        }
        if let Some(msg) = result.assistant_text.as_deref().filter(|v| !v.trim().is_empty()) {
            text.push('\n');
            text.push_str("```text\n");
            text.push_str(&truncate_chars(msg, 8_000));
            if msg.chars().count() > 8_000 {
                text.push_str("\n<...truncated...>\n");
            }
            text.push_str("\n```\n\n");
        }
    }

    let v = app
        .rpc(
            "artifact/write",
            serde_json::json!({
                "thread_id": thread_id,
                "turn_id": null,
                "approval_id": null,
                "artifact_id": artifact_id,
                "artifact_type": "fan_in_summary",
                "summary": "fan-in summary",
                "text": text,
            }),
        )
        .await?;
    ensure_approval_and_denial_handled("artifact/write", &v)?;

    Ok(v["content_path"].as_str().unwrap_or("").to_string())
}

