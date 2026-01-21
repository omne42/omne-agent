fn parse_non_empty_trimmed(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("value must not be empty".to_string());
    }
    Ok(trimmed.to_string())
}

fn print_json_or_pretty(json: bool, value: &Value) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
        return Ok(());
    }
    match value {
        Value::Object(_) | Value::Array(_) => {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        _ => println!("{value}"),
    }
    Ok(())
}

async fn run_ask(app: &mut App, args: AskArgs) -> anyhow::Result<()> {
    let thread_result = if let Some(thread_id) = args.thread_id {
        app.thread_resume(thread_id).await?
    } else {
        let cwd = args.cwd.map(|p| p.display().to_string());
        app.thread_start(cwd).await?
    };

    let thread_id: ThreadId = serde_json::from_value(thread_result["thread_id"].clone())
        .context("thread_id missing in result")?;
    let mut since_seq = thread_result["last_seq"].as_u64().unwrap_or(0);

    if args.approval_policy.is_some()
        || args.sandbox_policy.is_some()
        || args.mode.is_some()
        || args.model.is_some()
        || args.openai_base_url.is_some()
    {
        app.thread_configure(ThreadConfigureArgs {
            thread_id,
            approval_policy: args.approval_policy,
            sandbox_policy: args.sandbox_policy,
            mode: args.mode,
            model: args.model,
            openai_base_url: args.openai_base_url,
        })
        .await?;
    }

    let turn_id = app.turn_start(thread_id, args.input).await?;
    eprintln!("thread: {thread_id}");
    eprintln!("turn: {turn_id}");

    let saw_delta = Arc::new(AtomicBool::new(false));
    let mut streaming_handle: Option<tokio::task::JoinHandle<()>> = None;
    if let Some(mut notifications) = app.take_notifications() {
        let saw_delta = saw_delta.clone();
        let thread_id_str = thread_id.to_string();
        let turn_id_str = turn_id.to_string();
        streaming_handle = Some(tokio::spawn(async move {
            while let Some(note) = notifications.recv().await {
                if note.method != "item/delta" {
                    continue;
                }
                let params = match note.params.as_object() {
                    Some(params) => params,
                    None => continue,
                };
                let Some(thread_id) = params.get("thread_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(turn_id) = params.get("turn_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                if thread_id != thread_id_str || turn_id != turn_id_str {
                    continue;
                }
                if params.get("kind").and_then(|v| v.as_str()) != Some("output_text") {
                    continue;
                }
                let Some(delta) = params.get("delta").and_then(|v| v.as_str()) else {
                    continue;
                };
                if delta.is_empty() {
                    continue;
                }
                saw_delta.store(true, Ordering::Relaxed);
                print!("{delta}");
                std::io::stdout().flush().ok();
            }
        }));
    }

    let (interrupt_tx, mut interrupt_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = interrupt_tx.send(()).await;
        }
    });

    loop {
        if interrupt_rx.try_recv().is_ok() {
            app.turn_interrupt(thread_id, turn_id, Some("ctrl-c".to_string()))
                .await?;
            eprintln!("interrupt requested: {turn_id}");
            if let Some(handle) = streaming_handle.take() {
                handle.abort();
            }
            return Ok(());
        }

        let resp = app
            .thread_subscribe(thread_id, since_seq, Some(10_000), Some(1_000))
            .await?;
        since_seq = resp.last_seq;

        for event in &resp.events {
            let did_stream = saw_delta.load(Ordering::Relaxed);
            render_event_for_ask(event, did_stream);
            if let pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: Some(approval_turn_id),
                action,
                params,
            } = &event.kind
                && *approval_turn_id == turn_id
            {
                if did_stream {
                    println!();
                    std::io::stdout().flush().ok();
                }
                let decision = prompt_approval(approval_id, action, params)?;
                app.approval_decide(
                    thread_id,
                    *approval_id,
                    decision.decision,
                    decision.remember,
                    decision.reason,
                )
                .await?;
            }
            if let pm_protocol::ThreadEventKind::TurnCompleted { turn_id: id, .. } = &event.kind
                && *id == turn_id
            {
                if did_stream {
                    println!();
                    std::io::stdout().flush().ok();
                }
                if let Some(handle) = streaming_handle.take() {
                    handle.abort();
                }
                return Ok(());
            }
        }

        if resp.timed_out {
            continue;
        }
        if resp.has_more {
            continue;
        }
    }
}

async fn run_exec(app: &mut App, args: ExecArgs) -> anyhow::Result<i32> {
    let thread_result = if let Some(thread_id) = args.thread_id {
        app.thread_resume(thread_id).await?
    } else {
        let cwd = args.cwd.map(|p| p.display().to_string());
        app.thread_start(cwd).await?
    };

    let thread_id: ThreadId = serde_json::from_value(thread_result["thread_id"].clone())
        .context("thread_id missing in result")?;
    let mut since_seq = thread_result["last_seq"].as_u64().unwrap_or(0);

    if args.approval_policy.is_some()
        || args.sandbox_policy.is_some()
        || args.mode.is_some()
        || args.model.is_some()
        || args.openai_base_url.is_some()
    {
        app.thread_configure(ThreadConfigureArgs {
            thread_id,
            approval_policy: args.approval_policy,
            sandbox_policy: args.sandbox_policy,
            mode: args.mode,
            model: args.model,
            openai_base_url: args.openai_base_url,
        })
        .await?;
    }

    let turn_id = app.turn_start(thread_id, args.input).await?;
    eprintln!("thread: {thread_id}");
    eprintln!("turn: {turn_id}");

    let (interrupt_tx, mut interrupt_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = interrupt_tx.send(()).await;
        }
    });

    let mut handled_approvals = std::collections::HashSet::<ApprovalId>::new();
    let mut assistant_text: Option<String> = None;
    let mut assistant_model: Option<String> = None;
    let mut assistant_response_id: Option<String> = None;
    let mut assistant_token_usage: Option<Value> = None;

    loop {
        if interrupt_rx.try_recv().is_ok() {
            app.turn_interrupt(thread_id, turn_id, Some("ctrl-c".to_string()))
                .await?;
            eprintln!("interrupt requested: {turn_id}");
        }

        let resp = app
            .thread_subscribe(thread_id, since_seq, Some(10_000), Some(1_000))
            .await?;
        since_seq = resp.last_seq;

        for event in &resp.events {
            match &event.kind {
                pm_protocol::ThreadEventKind::AssistantMessage {
                    turn_id: Some(id),
                    text,
                    model,
                    response_id,
                    token_usage,
                } if *id == turn_id => {
                    assistant_text = Some(text.clone());
                    assistant_model = model.clone();
                    assistant_response_id = response_id.clone();
                    assistant_token_usage = token_usage.clone();
                }
                pm_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id: Some(approval_turn_id),
                    action,
                    params,
                } if *approval_turn_id == turn_id && !handled_approvals.contains(approval_id) => {
                    handled_approvals.insert(*approval_id);
                    match args.on_approval {
                        CliOnApproval::Fail => {
                            app.turn_interrupt(
                                thread_id,
                                turn_id,
                                Some(format!("approval required: {approval_id}")),
                            )
                            .await?;
                            eprintln!("approval required: {approval_id} action={action}");
                            eprintln!("{}", serde_json::to_string_pretty(params)?);
                        }
                        CliOnApproval::Approve => {
                            app.approval_decide(
                                thread_id,
                                *approval_id,
                                ApprovalDecision::Approved,
                                args.remember,
                                Some("auto-approved by pm exec".to_string()),
                            )
                            .await?;
                        }
                        CliOnApproval::Deny => {
                            app.approval_decide(
                                thread_id,
                                *approval_id,
                                ApprovalDecision::Denied,
                                args.remember,
                                Some("auto-denied by pm exec".to_string()),
                            )
                            .await?;
                        }
                    }
                }
                pm_protocol::ThreadEventKind::TurnCompleted {
                    turn_id: id,
                    status,
                    reason,
                } if *id == turn_id => {
                    if args.json {
                        let output = serde_json::json!({
                            "thread_id": thread_id,
                            "turn_id": turn_id,
                            "status": status,
                            "reason": reason,
                            "assistant": {
                                "text": assistant_text,
                                "model": assistant_model,
                                "response_id": assistant_response_id,
                                "token_usage": assistant_token_usage,
                            },
                            "last_seq": since_seq,
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    } else if let Some(text) = assistant_text.as_deref() {
                        print!("{text}");
                        if !text.ends_with('\n') {
                            println!();
                        }
                    }
                    return Ok(if *status == TurnStatus::Completed {
                        0
                    } else {
                        1
                    });
                }
                other => {
                    let ts = event
                        .timestamp
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_else(|_| "<time>".to_string());
                    let mut buffered = Vec::<u8>::new();
                    render_event_to(&mut buffered, ts, other);
                    if let Ok(text) = String::from_utf8(buffered) {
                        eprint!("{text}");
                    }
                }
            }
        }

        if resp.timed_out {
            continue;
        }
        if resp.has_more {
            continue;
        }
    }
}

fn render_event_for_ask(event: &ThreadEvent, streamed_assistant: bool) {
    let ts = event
        .timestamp
        .format(&time::format_description::well_known::Rfc3339);
    let ts = ts.unwrap_or_else(|_| "<time>".to_string());
    match &event.kind {
        pm_protocol::ThreadEventKind::AssistantMessage { text, model, .. } => {
            if streamed_assistant {
                return;
            }
            if let Some(model) = model {
                println!("[{ts}] assistant (model={model}):");
            } else {
                println!("[{ts}] assistant:");
            }
            println!("{text}");
        }
        other => {
            let mut buffered = Vec::<u8>::new();
            render_event_to(&mut buffered, ts, other);
            if let Ok(text) = String::from_utf8(buffered) {
                eprint!("{text}");
            }
        }
    }
}

fn render_event_to<W: std::io::Write>(
    writer: &mut W,
    ts: String,
    kind: &pm_protocol::ThreadEventKind,
) {
    match kind {
        pm_protocol::ThreadEventKind::ThreadCreated { cwd } => {
            let _ = writeln!(writer, "[{ts}] thread created cwd={cwd}");
        }
        pm_protocol::ThreadEventKind::ThreadArchived { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread archived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnarchived { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread unarchived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadPaused { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread paused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnpaused { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread unpaused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnStarted { turn_id, input } => {
            let _ = writeln!(writer, "[{ts}] turn started {turn_id}");
            let _ = writeln!(writer, "user: {input}");
        }
        pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] turn interrupt requested {turn_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] turn completed {turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            mode,
            model,
            openai_base_url,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} mode={} model={} openai_base_url={}",
                mode.as_deref().unwrap_or(""),
                model.as_deref().unwrap_or(""),
                openai_base_url.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ApprovalRequested {
            approval_id,
            action,
            ..
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] approval requested {approval_id} action={action}"
            );
        }
        pm_protocol::ThreadEventKind::ApprovalDecided {
            approval_id,
            decision,
            remember,
            reason,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] approval decided {approval_id} decision={decision:?} remember={remember} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ToolStarted { tool, .. } => {
            let _ = writeln!(writer, "[{ts}] tool started {tool}");
        }
        pm_protocol::ThreadEventKind::ToolCompleted { status, error, .. } => {
            let _ = writeln!(
                writer,
                "[{ts}] tool completed status={status:?} error={}",
                error.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessStarted {
            process_id, argv, ..
        } => {
            let _ = writeln!(writer, "[{ts}] process started {process_id} argv={argv:?}");
        }
        pm_protocol::ThreadEventKind::ProcessInterruptRequested {
            process_id, reason, ..
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] process interrupt requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessKillRequested {
            process_id, reason, ..
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] process kill requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] process exited {process_id} exit_code={} reason={}",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                reason.as_deref().unwrap_or("")
            );
        }
        _ => {}
    }
}

struct ApprovalPromptDecision {
    decision: ApprovalDecision,
    remember: bool,
    reason: Option<String>,
}

fn prompt_approval(
    approval_id: &ApprovalId,
    action: &str,
    params: &Value,
) -> anyhow::Result<ApprovalPromptDecision> {
    eprintln!();
    eprintln!("needs approval: {approval_id}");
    eprintln!("action: {action}");
    eprintln!("params: {}", serde_json::to_string_pretty(params)?);

    let decision = loop {
        eprint!("approve? [y/N]: ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let line = line.trim().to_lowercase();
        match line.as_str() {
            "y" | "yes" => break ApprovalDecision::Approved,
            "" | "n" | "no" => break ApprovalDecision::Denied,
            _ => continue,
        }
    };

    let remember = loop {
        eprint!("remember? [y/N]: ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let line = line.trim().to_lowercase();
        match line.as_str() {
            "y" | "yes" => break true,
            "" | "n" | "no" => break false,
            _ => continue,
        }
    };

    let reason = {
        eprint!("reason (optional): ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    Ok(ApprovalPromptDecision {
        decision,
        remember,
        reason,
    })
}

