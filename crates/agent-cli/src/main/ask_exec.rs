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
    let (thread_id, mut since_seq) = if let Some(thread_id) = args.thread_id {
        let resumed = app.thread_resume(thread_id).await?;
        (resumed.thread_id, resumed.last_seq)
    } else {
        let cwd = args.cwd.map(|p| p.display().to_string());
        let started = app.thread_start(cwd).await?;
        ensure_thread_start_auto_hook_ready("ask", &started)?;
        (started.thread_id, started.last_seq)
    };

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
            sandbox_writable_roots: None,
            sandbox_network_access: None,
            mode: args.mode,
            model: args.model,
            openai_base_url: args.openai_base_url,
            thinking: None,
            allowed_tools: None,
            clear_allowed_tools: false,
            execpolicy_rules: None,
            clear_execpolicy_rules: false,
        })
        .await?;
    }

    let turn_id = app.turn_start(thread_id, args.input, None).await?;
    eprintln!("thread: {thread_id}");
    eprintln!("turn: {turn_id}");

    let saw_delta = Arc::new(AtomicBool::new(false));
    let mut streaming_handle: Option<tokio::task::JoinHandle<()>> = None;
    if let Some(mut notifications) = app.take_notifications() {
        let saw_delta = saw_delta.clone();
        let thread_id_str = thread_id.to_string();
        let turn_id_str = turn_id.to_string();
        streaming_handle = Some(tokio::spawn(async move {
            let mut thinking_started = false;
            while let Some(note) = notifications.recv().await {
                if note.method != "item/delta" {
                    continue;
                }
                let params = match note.params.as_ref().and_then(Value::as_object) {
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
                let Some(kind) = params.get("kind").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(delta) = params.get("delta").and_then(|v| v.as_str()) else {
                    continue;
                };
                if delta.is_empty() {
                    continue;
                }
                match kind {
                    "output_text" => {
                        saw_delta.store(true, Ordering::Relaxed);
                        print!("{delta}");
                        std::io::stdout().flush().ok();
                    }
                    "thinking" => {
                        if !thinking_started {
                            thinking_started = true;
                            eprint!("\n[thinking]\n");
                        }
                        eprint!("{delta}");
                        std::io::stderr().flush().ok();
                    }
                    "warning" => {
                        eprintln!("\n[warning] {delta}");
                    }
                    _ => {}
                }
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
            if let omne_protocol::ThreadEventKind::ApprovalRequested {
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
                let decision = prompt_approval(thread_id, approval_id, action, params)?;
                app.approval_decide(
                    thread_id,
                    *approval_id,
                    decision.decision,
                    decision.remember,
                    decision.reason,
                )
                .await?;
            }
            if let omne_protocol::ThreadEventKind::TurnCompleted { turn_id: id, .. } = &event.kind
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

type TickFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>>;

async fn run_ask_with_tick<F>(
    app: &mut App,
    args: AskArgs,
    mut tick: F,
) -> anyhow::Result<TurnId>
where
    for<'a> F: FnMut(&'a mut App, ThreadId, TurnId) -> TickFuture<'a>,
{
    let (thread_id, mut since_seq) = if let Some(thread_id) = args.thread_id {
        let resumed = app.thread_resume(thread_id).await?;
        (resumed.thread_id, resumed.last_seq)
    } else {
        let cwd = args.cwd.map(|p| p.display().to_string());
        let started = app.thread_start(cwd).await?;
        ensure_thread_start_auto_hook_ready("ask", &started)?;
        (started.thread_id, started.last_seq)
    };

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
            sandbox_writable_roots: None,
            sandbox_network_access: None,
            mode: args.mode,
            model: args.model,
            openai_base_url: args.openai_base_url,
            thinking: None,
            allowed_tools: None,
            clear_allowed_tools: false,
            execpolicy_rules: None,
            clear_execpolicy_rules: false,
        })
        .await?;
    }

    let turn_id = app.turn_start(thread_id, args.input, None).await?;
    eprintln!("thread: {thread_id}");
    eprintln!("turn: {turn_id}");

    let saw_delta = Arc::new(AtomicBool::new(false));
    let mut streaming_handle: Option<tokio::task::JoinHandle<()>> = None;
    if let Some(mut notifications) = app.take_notifications() {
        let saw_delta = saw_delta.clone();
        let thread_id_str = thread_id.to_string();
        let turn_id_str = turn_id.to_string();
        streaming_handle = Some(tokio::spawn(async move {
            let mut thinking_started = false;
            while let Some(note) = notifications.recv().await {
                if note.method != "item/delta" {
                    continue;
                }
                let params = match note.params.as_ref().and_then(Value::as_object) {
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
                let Some(kind) = params.get("kind").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(delta) = params.get("delta").and_then(|v| v.as_str()) else {
                    continue;
                };
                if delta.is_empty() {
                    continue;
                }
                match kind {
                    "output_text" => {
                        saw_delta.store(true, Ordering::Relaxed);
                        print!("{delta}");
                        std::io::stdout().flush().ok();
                    }
                    "thinking" => {
                        if !thinking_started {
                            thinking_started = true;
                            eprint!("\n[thinking]\n");
                        }
                        eprint!("{delta}");
                        std::io::stderr().flush().ok();
                    }
                    "warning" => {
                        eprintln!("\n[warning] {delta}");
                    }
                    _ => {}
                }
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
            return Ok(turn_id);
        }

        tick(app, thread_id, turn_id).await?;

        let resp = app
            .thread_subscribe(thread_id, since_seq, Some(10_000), Some(1_000))
            .await?;
        since_seq = resp.last_seq;

        for event in &resp.events {
            let did_stream = saw_delta.load(Ordering::Relaxed);
            render_event_for_ask(event, did_stream);
            if let omne_protocol::ThreadEventKind::ApprovalRequested {
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
                let decision = prompt_approval(thread_id, approval_id, action, params)?;
                app.approval_decide(
                    thread_id,
                    *approval_id,
                    decision.decision,
                    decision.remember,
                    decision.reason,
                )
                .await?;
            }
            if let omne_protocol::ThreadEventKind::TurnCompleted { turn_id: id, .. } = &event.kind
                && *id == turn_id
            {
                if did_stream {
                    println!();
                    std::io::stdout().flush().ok();
                }
                if let Some(handle) = streaming_handle.take() {
                    handle.abort();
                }
                return Ok(turn_id);
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
    let (thread_id, mut since_seq) = if let Some(thread_id) = args.thread_id {
        let resumed = app.thread_resume(thread_id).await?;
        (resumed.thread_id, resumed.last_seq)
    } else {
        let cwd = args.cwd.map(|p| p.display().to_string());
        let started = app.thread_start(cwd).await?;
        ensure_thread_start_auto_hook_ready("exec", &started)?;
        (started.thread_id, started.last_seq)
    };

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
            sandbox_writable_roots: None,
            sandbox_network_access: None,
            mode: args.mode,
            model: args.model,
            openai_base_url: args.openai_base_url,
            thinking: None,
            allowed_tools: None,
            clear_allowed_tools: false,
            execpolicy_rules: None,
            clear_execpolicy_rules: false,
        })
        .await?;
    }

    let turn_id = app.turn_start(thread_id, args.input, None).await?;
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
                omne_protocol::ThreadEventKind::AssistantMessage {
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
                omne_protocol::ThreadEventKind::ApprovalRequested {
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
                            let action_label = approval_action_label_from_action(action);
                            eprintln!("approval required: {approval_id} action={action_label}");
                            if let Some(summary) = approval_summary_from_params_with_context(
                                Some(thread_id),
                                Some(*approval_id),
                                Some(action.as_str()),
                                params,
                            ) {
                                if let Some(display) =
                                    approval_summary_display_from_summary(&summary)
                                {
                                    eprintln!("summary: {display}");
                                }
                                for line in approval_quick_command_lines(&summary) {
                                    eprintln!("{line}");
                                }
                            }
                            eprintln!("{}", serde_json::to_string_pretty(params)?);
                        }
                        CliOnApproval::Approve => {
                            app.approval_decide(
                                thread_id,
                                *approval_id,
                                ApprovalDecision::Approved,
                                args.remember,
                                Some("auto-approved by omne exec".to_string()),
                            )
                            .await?;
                        }
                        CliOnApproval::Deny => {
                            app.approval_decide(
                                thread_id,
                                *approval_id,
                                ApprovalDecision::Denied,
                                args.remember,
                                Some("auto-denied by omne exec".to_string()),
                            )
                            .await?;
                        }
                    }
                }
                omne_protocol::ThreadEventKind::TurnCompleted {
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
        omne_protocol::ThreadEventKind::AssistantMessage { text, model, .. } => {
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
    kind: &omne_protocol::ThreadEventKind,
) {
    match kind {
        omne_protocol::ThreadEventKind::ThreadCreated { cwd } => {
            let _ = writeln!(writer, "[{ts}] thread created cwd={cwd}");
        }
        omne_protocol::ThreadEventKind::ThreadArchived { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread archived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadUnarchived { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread unarchived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadPaused { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread paused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadUnpaused { reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] thread unpaused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::TurnStarted { turn_id, input, .. } => {
            let _ = writeln!(writer, "[{ts}] turn started {turn_id}");
            let _ = writeln!(writer, "user: {input}");
        }
        omne_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
            let _ = writeln!(
                writer,
                "[{ts}] turn interrupt requested {turn_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::TurnCompleted {
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
        omne_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            model,
            thinking,
            show_thinking,
            openai_base_url,
            allowed_tools,
            execpolicy_rules,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} sandbox_writable_roots={sandbox_writable_roots:?} sandbox_network_access={sandbox_network_access:?} mode={} model={} thinking={} show_thinking={} openai_base_url={} allowed_tools={allowed_tools:?} execpolicy_rules={execpolicy_rules:?}",
                mode.as_deref().unwrap_or(""),
                model.as_deref().unwrap_or(""),
                thinking.as_deref().unwrap_or(""),
                show_thinking
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                openai_base_url.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id,
            action,
            params,
            ..
        } => {
            let action_label = approval_action_label_from_action(action);
            let summary_hint = approval_summary_from_params(params)
                .and_then(|summary| approval_summary_display_from_summary(&summary))
                .unwrap_or_default();
            let _ = writeln!(
                writer,
                "[{ts}] approval requested {approval_id} action={}{}",
                action_label,
                if summary_hint.is_empty() {
                    "".to_string()
                } else {
                    format!(" summary={summary_hint}")
                }
            );
        }
        omne_protocol::ThreadEventKind::ApprovalDecided {
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
        omne_protocol::ThreadEventKind::ToolStarted { tool, .. } => {
            let _ = writeln!(writer, "[{ts}] tool started {tool}");
        }
        omne_protocol::ThreadEventKind::ToolCompleted { status, error, .. } => {
            let _ = writeln!(
                writer,
                "[{ts}] tool completed status={status:?} error={}",
                error.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ProcessStarted {
            process_id, argv, ..
        } => {
            let _ = writeln!(writer, "[{ts}] process started {process_id} argv={argv:?}");
        }
        omne_protocol::ThreadEventKind::ProcessInterruptRequested {
            process_id, reason, ..
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] process interrupt requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ProcessKillRequested {
            process_id, reason, ..
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] process kill requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ProcessExited {
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
        omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker,
            turn_id,
            artifact_id,
            artifact_type,
            process_id,
            exit_code,
            command,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] attention marker set marker={marker:?} turn_id={} artifact_id={} artifact_type={} process_id={} exit_code={} command={}",
                turn_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                artifact_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                artifact_type.as_deref().unwrap_or(""),
                process_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                command.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker,
            turn_id,
            reason,
        } => {
            let _ = writeln!(
                writer,
                "[{ts}] attention marker cleared marker={marker:?} turn_id={} reason={}",
                turn_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
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
    thread_id: ThreadId,
    approval_id: &ApprovalId,
    action: &str,
    params: &Value,
) -> anyhow::Result<ApprovalPromptDecision> {
    let action_label = approval_action_label_from_action(action);
    eprintln!();
    eprintln!("needs approval: {approval_id}");
    eprintln!("action: {action_label}");
    if action_label != action {
        eprintln!("raw_action: {action}");
    }
    if let Some(summary) = approval_summary_from_params_with_context(
        Some(thread_id),
        Some(*approval_id),
        Some(action),
        params,
    ) {
        if let Some(display) = approval_summary_display_from_summary(&summary) {
            eprintln!("summary: {display}");
        }
        for line in approval_quick_command_lines(&summary) {
            eprintln!("{line}");
        }
    }
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

fn approval_quick_command_lines(
    summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    if let Some(approve_cmd) = approval_approve_cmd_from_summary(summary) {
        lines.push(format!("approve: {approve_cmd}"));
    }
    if let Some(deny_cmd) = approval_deny_cmd_from_summary(summary) {
        lines.push(format!("deny: {deny_cmd}"));
    }
    lines
}

#[cfg(test)]
mod ask_exec_tests {
    use super::*;

    #[test]
    fn approval_quick_command_lines_include_approve_and_deny() {
        let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
            requirement: None,
            argv: None,
            cwd: None,
            process_id: None,
            artifact_type: None,
            path: None,
            server: None,
            tool: None,
            hook: None,
            child_thread_id: None,
            child_turn_id: None,
            child_approval_id: None,
            child_attention_state: None,
            child_last_turn_status: None,
            approve_cmd: Some("omne approval decide t1 a1 --approve".to_string()),
            deny_cmd: Some("omne approval decide t1 a1 --deny".to_string()),
        };
        let lines = approval_quick_command_lines(&summary);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|line| line.contains("approve: ")));
        assert!(lines.iter().any(|line| line.contains("--approve")));
        assert!(lines.iter().any(|line| line.contains("deny: ")));
        assert!(lines.iter().any(|line| line.contains("--deny")));
    }

    #[test]
    fn approval_quick_command_lines_are_empty_without_approve_cmd() {
        let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
            requirement: None,
            argv: None,
            cwd: None,
            process_id: None,
            artifact_type: None,
            path: None,
            server: None,
            tool: None,
            hook: None,
            child_thread_id: None,
            child_turn_id: None,
            child_approval_id: None,
            child_attention_state: None,
            child_last_turn_status: None,
            approve_cmd: None,
            deny_cmd: None,
        };
        let lines = approval_quick_command_lines(&summary);
        assert!(lines.is_empty());
    }
}
