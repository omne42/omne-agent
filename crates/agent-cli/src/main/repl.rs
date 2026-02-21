use std::io::IsTerminal;

struct ReplState {
    thread_id: ThreadId,
    since_seq: u64,
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run_repl(app: &mut App) -> anyhow::Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        anyhow::bail!("interactive mode requires a TTY (try `omne ask ...` or `omne exec ...`)");
    }

    let cwd = std::env::current_dir()?.display().to_string();
    let thread_result = app.thread_start(Some(cwd.clone())).await?;
    let thread_id: ThreadId = serde_json::from_value(thread_result["thread_id"].clone())
        .context("thread_id missing in result")?;
    let since_seq = thread_result["last_seq"].as_u64().unwrap_or(0);

    app.thread_configure(ThreadConfigureArgs {
        thread_id,
        approval_policy: Some(CliApprovalPolicy::Manual),
        sandbox_policy: None,
        sandbox_writable_roots: None,
        sandbox_network_access: None,
        mode: None,
        model: None,
        openai_base_url: None,
        thinking: None,
    })
    .await?;

    let notification_tx = spawn_repl_notification_hub(app);

    eprintln!("omne cli");
    eprintln!("cwd: {cwd}");
    eprintln!("thread: {thread_id}");
    eprintln!("approval_policy: manual (use `/set approval_policy ...` to change)");
    eprintln!("type `/help` for commands");

    let state_json = app.thread_state(thread_id).await?;
    let since_seq = state_json["last_seq"].as_u64().unwrap_or(since_seq);
    let mut state = ReplState { thread_id, since_seq };

    loop {
        print!("omne[{}]> ", thread_id_short(state.thread_id));
        std::io::stdout().flush().ok();
        let mut line = String::new();
        let bytes = std::io::stdin().read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line == "/exit" || line == "/quit" {
            break;
        }
        if line == "/help" {
            print_repl_help();
            continue;
        }

        if let Some(message) = line.strip_prefix("//") {
            repl_run_turn(
                app,
                &mut state,
                message.trim_start().to_string(),
                notification_tx.as_ref(),
            )
            .await?;
            continue;
        }

        if let Some(cmdline) = line.strip_prefix('/') {
            repl_run_command(app, &mut state, cmdline, notification_tx.as_ref()).await?;
            continue;
        }

        repl_run_turn(app, &mut state, line.to_string(), notification_tx.as_ref()).await?;
    }

    Ok(())
}

fn thread_id_short(thread_id: ThreadId) -> String {
    let s = thread_id.to_string();
    s.chars().take(8).collect::<String>()
}

fn spawn_repl_notification_hub(
    app: &mut App,
) -> Option<tokio::sync::broadcast::Sender<omne_jsonrpc::Notification>> {
    let mut rx = app.take_notifications()?;
    let (tx, _rx0) = tokio::sync::broadcast::channel::<omne_jsonrpc::Notification>(1024);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(note) = rx.recv().await {
            let _ = tx2.send(note);
        }
    });
    Some(tx)
}

async fn repl_run_turn(
    app: &mut App,
    state: &mut ReplState,
    input: String,
    notification_tx: Option<&tokio::sync::broadcast::Sender<omne_jsonrpc::Notification>>,
) -> anyhow::Result<()> {
    let turn_id = app.turn_start(state.thread_id, input, None).await?;
    eprintln!("turn: {turn_id}");

    let saw_delta = Arc::new(AtomicBool::new(false));
    let mut streaming_handle: Option<AbortOnDrop> = None;
    if let Some(tx) = notification_tx {
        let mut rx = tx.subscribe();
        let saw_delta = saw_delta.clone();
        let thread_id_str = state.thread_id.to_string();
        let turn_id_str = turn_id.to_string();
        streaming_handle = Some(AbortOnDrop(tokio::spawn(async move {
            loop {
                let note = match rx.recv().await {
                    Ok(note) => note,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                };
                if note.method != "item/delta" {
                    continue;
                }
                let Some(params) = note.params.as_ref().and_then(Value::as_object) else {
                    continue;
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
        })));
    }

    let mut pending_approvals = Vec::<(ApprovalId, String, Value)>::new();
    let mut decided_approvals = std::collections::HashSet::<ApprovalId>::new();

    loop {
        let resp = app
            .thread_subscribe(state.thread_id, state.since_seq, Some(10_000), Some(1_000))
            .await?;
        state.since_seq = resp.last_seq;

        pending_approvals.clear();
        decided_approvals.clear();

        for event in &resp.events {
            let did_stream = saw_delta.load(Ordering::Relaxed);
            render_event_for_ask(event, did_stream);

            match &event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id: Some(approval_turn_id),
                    action,
                    params,
                } if *approval_turn_id == turn_id => {
                    pending_approvals.push((*approval_id, action.clone(), params.clone()));
                }
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                    decided_approvals.insert(*approval_id);
                }
                omne_protocol::ThreadEventKind::TurnCompleted { turn_id: id, .. } if *id == turn_id => {
                    if did_stream {
                        drop(streaming_handle.take());
                        println!();
                        std::io::stdout().flush().ok();
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        for (approval_id, action, params) in &pending_approvals {
            if decided_approvals.contains(approval_id) {
                continue;
            }
            if saw_delta.load(Ordering::Relaxed) {
                println!();
                std::io::stdout().flush().ok();
            }
            let decision = prompt_approval(approval_id, action, params)?;
            app.approval_decide(
                state.thread_id,
                *approval_id,
                decision.decision,
                decision.remember,
                decision.reason,
            )
            .await?;
        }
    }
}

async fn repl_run_command(
    app: &mut App,
    state: &mut ReplState,
    cmdline: &str,
    _notification_tx: Option<&tokio::sync::broadcast::Sender<omne_jsonrpc::Notification>>,
) -> anyhow::Result<()> {
    let tokens = cmdline.split_whitespace().collect::<Vec<_>>();
    let Some(cmd) = tokens.first().copied() else {
        return Ok(());
    };

    match cmd {
        "thread" => repl_cmd_thread(app, state, &tokens[1..]).await,
        "inbox" => repl_cmd_inbox(app, &tokens[1..]).await,
        "state" => {
            let v = app.thread_state(state.thread_id).await?;
            print_json_or_pretty(false, &v)?;
            Ok(())
        }
        "config" => {
            let v = app.thread_config_explain(state.thread_id).await?;
            print_json_or_pretty(false, &v)?;
            Ok(())
        }
        "set" => repl_cmd_set(app, state, &tokens[1..]).await,
        "approvals" => repl_cmd_approvals(app, state, &tokens[1..]).await,
        "approve" => repl_cmd_approve_or_deny(app, state, true, &tokens[1..]).await,
        "deny" => repl_cmd_approve_or_deny(app, state, false, &tokens[1..]).await,
        _ => {
            eprintln!("unknown command: /{cmd}");
            eprintln!("try /help");
            Ok(())
        }
    }
}

async fn repl_cmd_thread(app: &mut App, state: &mut ReplState, args: &[&str]) -> anyhow::Result<()> {
    match args {
        [] => {
            println!("{}", state.thread_id);
            Ok(())
        }
        ["new"] | ["start"] => repl_thread_start(app, state, None).await,
        ["new", cwd] | ["start", cwd] => repl_thread_start(app, state, Some((*cwd).to_string())).await,
        ["use", thread_id] => {
            let thread_id: ThreadId = (*thread_id).parse().context("parse thread_id")?;
            let result = app.thread_resume(thread_id).await?;
            let since_seq = result["last_seq"].as_u64().unwrap_or(0);
            state.thread_id = thread_id;
            state.since_seq = since_seq;
            eprintln!("thread: {}", state.thread_id);
            Ok(())
        }
        ["list"] => {
            let v = app.thread_list_meta(false).await?;
            print_json_or_pretty(false, &v)?;
            Ok(())
        }
        _ => {
            eprintln!("usage:");
            eprintln!("  /thread");
            eprintln!("  /thread new [cwd]");
            eprintln!("  /thread use <thread_id>");
            eprintln!("  /thread list");
            Ok(())
        }
    }
}

async fn repl_thread_start(
    app: &mut App,
    state: &mut ReplState,
    cwd: Option<String>,
) -> anyhow::Result<()> {
    let cwd = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    });
    let result = app.thread_start(Some(cwd.clone())).await?;
    let thread_id: ThreadId = serde_json::from_value(result["thread_id"].clone())
        .context("thread_id missing in result")?;

    app.thread_configure(ThreadConfigureArgs {
        thread_id,
        approval_policy: Some(CliApprovalPolicy::Manual),
        sandbox_policy: None,
        sandbox_writable_roots: None,
        sandbox_network_access: None,
        mode: None,
        model: None,
        openai_base_url: None,
        thinking: None,
    })
    .await?;

    let state_json = app.thread_state(thread_id).await?;
    let since_seq = state_json["last_seq"]
        .as_u64()
        .unwrap_or_else(|| result["last_seq"].as_u64().unwrap_or(0));
    state.thread_id = thread_id;
    state.since_seq = since_seq;
    eprintln!("thread: {} (cwd={cwd})", state.thread_id);
    Ok(())
}

async fn repl_cmd_inbox(app: &mut App, args: &[&str]) -> anyhow::Result<()> {
    let include_archived = args.contains(&"--include-archived");
    let details = args.contains(&"--details");
    run_inbox(
        app,
        InboxArgs {
            include_archived,
            details,
            watch: false,
            poll_ms: 1_000,
            bell: false,
            debounce_ms: 30_000,
            json: false,
        },
    )
    .await
}

async fn repl_cmd_set(app: &mut App, state: &mut ReplState, args: &[&str]) -> anyhow::Result<()> {
    let Some(key) = args.first().copied() else {
        anyhow::bail!("usage: /set <key> <value>");
    };
    let value = args.get(1).copied().unwrap_or("");
    if value.trim().is_empty() {
        anyhow::bail!("usage: /set {key} <value>");
    }

    let mut cfg = ThreadConfigureArgs {
        thread_id: state.thread_id,
        approval_policy: None,
        sandbox_policy: None,
        sandbox_writable_roots: None,
        sandbox_network_access: None,
        mode: None,
        model: None,
        openai_base_url: None,
        thinking: None,
    };

    match key {
        "approval_policy" => {
            cfg.approval_policy = Some(parse_repl_approval_policy(value)?);
        }
        "sandbox_policy" => {
            cfg.sandbox_policy = Some(parse_repl_sandbox_policy(value)?);
        }
        "sandbox_network_access" => {
            cfg.sandbox_network_access = Some(parse_repl_sandbox_network_access(value)?);
        }
        "mode" => {
            cfg.mode = Some(value.to_string());
        }
        "model" => {
            cfg.model = Some(value.to_string());
        }
        "openai_base_url" => {
            cfg.openai_base_url = Some(value.to_string());
        }
        "thinking" => {
            cfg.thinking = Some(value.to_string());
        }
        _ => {
            anyhow::bail!(
                "unknown key: {key} (try: approval_policy|sandbox_policy|sandbox_network_access|mode|model|openai_base_url|thinking)"
            );
        }
    }

    app.thread_configure(cfg).await?;
    let v = app.thread_state(state.thread_id).await?;
    if let Some(last_seq) = v.get("last_seq").and_then(|v| v.as_u64()) {
        state.since_seq = last_seq;
    }
    print_json_or_pretty(false, &v)?;
    Ok(())
}

fn parse_repl_approval_policy(raw: &str) -> anyhow::Result<CliApprovalPolicy> {
    let norm = raw.trim().to_lowercase().replace('-', "_");
    match norm.as_str() {
        "auto_approve" => Ok(CliApprovalPolicy::AutoApprove),
        "on_request" => Ok(CliApprovalPolicy::OnRequest),
        "manual" => Ok(CliApprovalPolicy::Manual),
        "unless_trusted" => Ok(CliApprovalPolicy::UnlessTrusted),
        "auto_deny" => Ok(CliApprovalPolicy::AutoDeny),
        _ => anyhow::bail!("unknown approval_policy: {raw}"),
    }
}

fn parse_repl_sandbox_policy(raw: &str) -> anyhow::Result<CliSandboxPolicy> {
    let norm = raw.trim().to_lowercase().replace('-', "_");
    match norm.as_str() {
        "read_only" => Ok(CliSandboxPolicy::ReadOnly),
        "workspace_write" => Ok(CliSandboxPolicy::WorkspaceWrite),
        "danger_full_access" => Ok(CliSandboxPolicy::DangerFullAccess),
        _ => anyhow::bail!("unknown sandbox_policy: {raw}"),
    }
}

fn parse_repl_sandbox_network_access(raw: &str) -> anyhow::Result<CliSandboxNetworkAccess> {
    let norm = raw.trim().to_lowercase().replace('-', "_");
    match norm.as_str() {
        "deny" => Ok(CliSandboxNetworkAccess::Deny),
        "allow" => Ok(CliSandboxNetworkAccess::Allow),
        _ => anyhow::bail!("unknown sandbox_network_access: {raw}"),
    }
}

async fn repl_cmd_approvals(app: &mut App, state: &mut ReplState, args: &[&str]) -> anyhow::Result<()> {
    let include_decided = args.contains(&"--include-decided");
    let v = app.approval_list(state.thread_id, include_decided).await?;
    print_json_or_pretty(false, &v)?;
    Ok(())
}

async fn repl_cmd_approve_or_deny(
    app: &mut App,
    state: &mut ReplState,
    approve: bool,
    args: &[&str],
) -> anyhow::Result<()> {
    let Some(id) = args.first().copied() else {
        anyhow::bail!("usage: /{} <approval_id> [--remember] [--reason <text>]", if approve { "approve" } else { "deny" });
    };
    let approval_id: ApprovalId = id.parse().context("parse approval_id")?;
    let remember = args.contains(&"--remember") || args.contains(&"remember");
    let reason = parse_reason_flag(args);
    let decision = if approve {
        ApprovalDecision::Approved
    } else {
        ApprovalDecision::Denied
    };
    app.approval_decide(state.thread_id, approval_id, decision, remember, reason)
        .await?;
    Ok(())
}

fn parse_reason_flag(args: &[&str]) -> Option<String> {
    let mut idx = 0usize;
    while idx < args.len() {
        if args[idx] == "--reason" {
            let rest = args.get(idx + 1..).unwrap_or_default();
            let text = rest.join(" ").trim().to_string();
            return if text.is_empty() { None } else { Some(text) };
        }
        idx += 1;
    }
    None
}

fn print_repl_help() {
    println!(
        r#"omne cli commands:
  /help                         show this help
  /exit | /quit                 exit cli

  /thread                       show current thread id
  /thread new [cwd]             start a new thread (defaults to current dir)
  /thread use <thread_id>       switch to an existing thread
  /thread list                  list threads (JSON)

  /state                        show current thread state (JSON)
  /config                       show config explain (JSON)
  /set <key> <value>            update thread config (and print new state)
    keys: approval_policy | sandbox_policy | sandbox_network_access | mode | model | openai_base_url | thinking

  /inbox [--details] [--include-archived]
  /approvals [--include-decided]
  /approve <approval_id> [--remember] [--reason <text>]
  /deny <approval_id> [--remember] [--reason <text>]

input:
  - plain text is sent as the next turn
  - use '//' to send a message starting with '/' (e.g. '// /plan ...')
"#
    );
}
