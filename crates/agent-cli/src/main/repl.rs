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
    ensure_thread_start_auto_hook_ready("repl", &thread_result)?;
    let thread_id = thread_result.thread_id;
    let since_seq = thread_result.last_seq;

    app.thread_configure(ThreadConfigureArgs {
        thread_id,
        approval_policy: Some(CliApprovalPolicy::Manual),
        sandbox_policy: None,
        sandbox_writable_roots: None,
        sandbox_network_access: None,
        mode: None,
        role: None,
        model: None,
        clear_model: false,
        openai_base_url: None,
        clear_openai_base_url: false,
        thinking: None,
        clear_thinking: false,
        show_thinking: None,
        clear_show_thinking: false,
        allowed_tools: None,
        clear_allowed_tools: false,
        execpolicy_rules: None,
        clear_execpolicy_rules: false,
    })
    .await?;

    let notification_tx = spawn_repl_notification_hub(app);

    eprintln!("omne cli");
    eprintln!("cwd: {cwd}");
    eprintln!("thread: {thread_id}");
    eprintln!("approval_policy: manual (use `/set approval_policy ...` to change)");
    eprintln!("type `/help` for commands");

    let state_json = app.thread_state(thread_id).await?;
    let since_seq = state_json.last_seq.max(since_seq);
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
            let mut thinking_started = false;
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
            let decision = prompt_approval(state.thread_id, approval_id, action, params)?;
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
            let v = serde_json::to_value(v).context("serialize thread/state response")?;
            print_json_or_pretty(false, &v)?;
            Ok(())
        }
        "config" => {
            let v = app.thread_config_explain(state.thread_id).await?;
            let v = serde_json::to_value(v).context("serialize thread/config/explain response")?;
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
            let since_seq = result.last_seq;
            state.thread_id = thread_id;
            state.since_seq = since_seq;
            eprintln!("thread: {}", state.thread_id);
            Ok(())
        }
        ["list"] => {
            let v = app.thread_list_meta(false, false).await?;
            let v = serde_json::to_value(v).context("serialize thread/list_meta response")?;
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
    ensure_thread_start_auto_hook_ready("repl", &result)?;
    let thread_id = result.thread_id;

    app.thread_configure(ThreadConfigureArgs {
        thread_id,
        approval_policy: Some(CliApprovalPolicy::Manual),
        sandbox_policy: None,
        sandbox_writable_roots: None,
        sandbox_network_access: None,
        mode: None,
        role: None,
        model: None,
        clear_model: false,
        openai_base_url: None,
        clear_openai_base_url: false,
        thinking: None,
        clear_thinking: false,
        show_thinking: None,
        clear_show_thinking: false,
        allowed_tools: None,
        clear_allowed_tools: false,
        execpolicy_rules: None,
        clear_execpolicy_rules: false,
    })
    .await?;

    let state_json = app.thread_state(thread_id).await?;
    let since_seq = state_json
        .last_seq
        .max(result.last_seq);
    state.thread_id = thread_id;
    state.since_seq = since_seq;
    eprintln!("thread: {} (cwd={cwd})", state.thread_id);
    Ok(())
}

async fn repl_cmd_inbox(app: &mut App, args: &[&str]) -> anyhow::Result<()> {
    let include_archived = args.contains(&"--include-archived");
    let only_fan_out_linkage_issue = args.contains(&"--only-fan-out-linkage-issue");
    let only_fan_out_auto_apply_error = args.contains(&"--only-fan-out-auto-apply-error");
    let only_fan_in_dependency_blocked = args.contains(&"--only-fan-in-dependency-blocked");
    let only_fan_in_result_diagnostics = args.contains(&"--only-fan-in-result-diagnostics");
    let only_token_budget_exceeded = args.contains(&"--only-token-budget-exceeded");
    let only_token_budget_warning = args.contains(&"--only-token-budget-warning");
    let only_subagent_proxy_approval = args.contains(&"--only-subagent-proxy-approval");
    let details = args.contains(&"--details");
    let debug_summary_cache = args.contains(&"--debug-summary-cache");
    run_inbox(
        app,
        InboxArgs {
            include_archived,
            only_fan_out_linkage_issue,
            only_fan_out_auto_apply_error,
            only_fan_in_dependency_blocked,
            only_fan_in_result_diagnostics,
            only_token_budget_exceeded,
            only_token_budget_warning,
            only_subagent_proxy_approval,
            details,
            watch: false,
            poll_ms: 1_000,
            bell: false,
            debounce_ms: 30_000,
            debug_summary_cache,
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
        role: None,
        model: None,
        clear_model: false,
        openai_base_url: None,
        clear_openai_base_url: false,
        thinking: None,
        clear_thinking: false,
        show_thinking: None,
        clear_show_thinking: false,
        allowed_tools: None,
        clear_allowed_tools: false,
        execpolicy_rules: None,
        clear_execpolicy_rules: false,
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
            if value.eq_ignore_ascii_case("clear") {
                cfg.clear_model = true;
            } else {
                cfg.model = Some(value.to_string());
            }
        }
        "openai_base_url" => {
            if value.eq_ignore_ascii_case("clear") {
                cfg.clear_openai_base_url = true;
            } else {
                cfg.openai_base_url = Some(value.to_string());
            }
        }
        "thinking" => {
            if value.eq_ignore_ascii_case("clear") {
                cfg.clear_thinking = true;
            } else {
                cfg.thinking = Some(value.to_string());
            }
        }
        "show_thinking" => {
            if value.eq_ignore_ascii_case("clear") {
                cfg.clear_show_thinking = true;
            } else {
                cfg.show_thinking = Some(matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                ));
            }
        }
        "allowed_tools" => match parse_repl_list_setting(value) {
            ReplListSetting::Set(tools) => {
                cfg.allowed_tools = Some(tools);
            }
            ReplListSetting::Clear => {
                cfg.clear_allowed_tools = true;
            }
        },
        "execpolicy_rules" => match parse_repl_execpolicy_rules(value) {
            ReplListSetting::Set(rules) => {
                cfg.execpolicy_rules = Some(rules);
            }
            ReplListSetting::Clear => {
                cfg.clear_execpolicy_rules = true;
            }
        },
        _ => {
            anyhow::bail!(
                "unknown key: {key} (try: approval_policy|sandbox_policy|sandbox_network_access|mode|model|openai_base_url|thinking|show_thinking|allowed_tools|execpolicy_rules)"
            );
        }
    }

    app.thread_configure(cfg).await?;
    let v = app.thread_state(state.thread_id).await?;
    state.since_seq = v.last_seq;
    let v = serde_json::to_value(v).context("serialize thread/state response")?;
    print_json_or_pretty(false, &v)?;
    Ok(())
}

fn parse_repl_approval_policy(raw: &str) -> anyhow::Result<CliApprovalPolicy> {
    let norm = raw.trim().to_lowercase().replace('-', "_");
    match norm.as_str() {
        "auto_approve" => Ok(CliApprovalPolicy::AutoApprove),
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
        "full_access" => Ok(CliSandboxPolicy::FullAccess),
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

enum ReplListSetting {
    Set(Vec<String>),
    Clear,
}

fn parse_repl_list_setting(raw: &str) -> ReplListSetting {
    let trimmed = raw.trim();
    if matches!(trimmed.to_ascii_lowercase().as_str(), "clear" | "none" | "null") {
        return ReplListSetting::Clear;
    }
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for part in trimmed.split(',') {
        let rule = part.trim();
        if rule.is_empty() {
            continue;
        }
        let rule = rule.to_string();
        if seen.insert(rule.clone()) {
            out.push(rule);
        }
    }
    ReplListSetting::Set(out)
}

fn parse_repl_execpolicy_rules(raw: &str) -> ReplListSetting {
    parse_repl_list_setting(raw)
}

async fn repl_cmd_approvals(app: &mut App, state: &mut ReplState, args: &[&str]) -> anyhow::Result<()> {
    let include_decided = args.contains(&"--include-decided");
    let parsed = app.approval_list(state.thread_id, include_decided).await?;
    print_repl_approvals(state.thread_id, &parsed);
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

fn approval_decision_label(value: ApprovalDecision) -> &'static str {
    match value {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::Denied => "denied",
    }
}

fn print_repl_approvals(
    thread_id: ThreadId,
    resp: &omne_app_server_protocol::ApprovalListResponse,
) {
    if resp.approvals.is_empty() {
        println!("no approvals");
        return;
    }
    for item in &resp.approvals {
        println!("{}", format_repl_approval_line(thread_id, item));
    }
}

fn format_repl_approval_line(
    thread_id: ThreadId,
    item: &omne_app_server_protocol::ApprovalListItem,
) -> String {
    let req = &item.request;
    let action = approval_action_label_from_parts(req.action_id, Some(req.action.as_str()));
    let mut line = format!(
        "approval_id={} action={} requested_at={}",
        req.approval_id, action, req.requested_at
    );
    if let Some(turn_id) = req.turn_id {
        line.push_str(&format!(" turn_id={turn_id}"));
    }

    let summary = req.summary.clone().or_else(|| {
        approval_summary_from_params_with_context(
            Some(thread_id),
            Some(req.approval_id),
            Some(req.action.as_str()),
            &req.params,
        )
    });
    let mut summary_has_approve_cmd = false;
    if let Some(summary) = summary.as_ref() {
        if let Some(display) = approval_summary_display_from_summary(summary) {
            summary_has_approve_cmd = display.contains("approve_cmd=");
            line.push_str(&format!(" summary={display}"));
        }
        if let Some(approve_cmd) = approval_approve_cmd_from_summary(summary)
            && !summary_has_approve_cmd
        {
            line.push_str(&format!(" approve_cmd={approve_cmd}"));
        }
        if let Some(deny_cmd) = approval_deny_cmd_from_summary(summary) {
            line.push_str(&format!(" deny_cmd={deny_cmd}"));
        }
    }

    if let Some(decision) = &item.decision {
        line.push_str(&format!(
            " decision={} decided_at={} remember={}",
            approval_decision_label(decision.decision),
            decision.decided_at,
            decision.remember
        ));
        if let Some(reason) = decision.reason.as_deref().filter(|s| !s.trim().is_empty()) {
            line.push_str(&format!(" reason={reason}"));
        }
    }
    line
}

fn print_repl_help() {
    println!("{}", repl_help_text());
}

fn repl_help_text() -> &'static str {
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
    keys: approval_policy | sandbox_policy | sandbox_network_access | mode | model | openai_base_url | thinking | show_thinking | allowed_tools | execpolicy_rules
    allowed_tools value: comma-separated tool names, or clear|none|null
    execpolicy_rules value: comma-separated paths, or clear|none|null

  /inbox [--details] [--include-archived] [--only-fan-out-linkage-issue] [--only-fan-out-auto-apply-error] [--only-fan-in-dependency-blocked] [--only-fan-in-result-diagnostics] [--only-token-budget-exceeded] [--only-token-budget-warning] [--only-subagent-proxy-approval] [--debug-summary-cache]
  /approvals [--include-decided]
  /approve <approval_id> [--remember] [--reason <text>]
  /deny <approval_id> [--remember] [--reason <text>]

tooling (default model-facing):
  - facade tools: workspace, process, thread, artifact (integration optional)
  - each facade supports {"op":"help"} for quickstart + advanced usage
  - topic help: {"op":"help","topic":"<op>"}

input:
  - plain text is sent as the next turn
  - use '//' to send a message starting with '/' (e.g. '// /plan ...')
"#
}

#[cfg(test)]
mod repl_tests {
    use super::*;

    #[test]
    fn format_repl_approval_line_adds_proxy_approve_and_deny_commands_from_context() {
        let thread_id = ThreadId::new();
        let approval_id = ApprovalId::new();
        let item = omne_app_server_protocol::ApprovalListItem {
            request: omne_app_server_protocol::ApprovalRequestInfo {
                approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                action_id: Some(
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                ),
                params: serde_json::json!({}),
                summary: None,
                requested_at: "2026-01-01T00:00:00Z".to_string(),
            },
            decision: None,
        };

        let line = format_repl_approval_line(thread_id, &item);
        let expected_approve = format!("omne approval decide {thread_id} {approval_id} --approve");
        let expected_deny = format!("omne approval decide {thread_id} {approval_id} --deny");
        assert!(line.contains("summary="));
        assert!(line.contains(&expected_approve));
        assert!(line.contains("deny_cmd="));
        assert!(line.contains(&expected_deny));
    }

    #[test]
    fn repl_help_mentions_facade_help_first_usage() {
        let help = repl_help_text();
        assert!(help.contains("show_thinking"));
        assert!(help.contains("facade tools: workspace, process, thread, artifact"));
        assert!(help.contains("{\"op\":\"help\"}"));
        assert!(help.contains("{\"op\":\"help\",\"topic\":\"<op>\"}"));
    }
}
