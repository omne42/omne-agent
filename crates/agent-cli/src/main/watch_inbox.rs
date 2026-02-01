async fn run_watch(app: &mut App, args: WatchArgs) -> anyhow::Result<()> {
    let mut since_seq = args.since_seq;
    let mut last_state: Option<&'static str> = None;
    let mut last_bell_at: Option<Instant> = None;
    let mut last_stale_present: Option<bool> = None;
    let mut last_stale_bell_at: Option<Instant> = None;
    let mut suppress_initial_bell = true;

    loop {
        let resp = app
            .thread_subscribe(
                args.thread_id,
                since_seq,
                args.max_events,
                Some(args.wait_ms),
            )
            .await?;
        since_seq = resp.last_seq;

        let mut state_update: Option<&'static str> = None;
        for event in &resp.events {
            if let Some(state) = attention_state_update(event) {
                state_update = Some(state);
            }
            if args.json {
                println!("{}", serde_json::to_string(event)?);
            } else {
                render_event(event);
            }
        }

        if args.bell && !suppress_initial_bell {
            if let Some(state) = state_update {
                maybe_bell(state, args.debounce_ms, &mut last_state, &mut last_bell_at)?;
            }
        }

        if args.bell {
            let att = app.thread_attention(args.thread_id).await?;
            let att: ThreadAttention = serde_json::from_value(att)?;
            let stale_present = !att.stale_processes.is_empty();
            if suppress_initial_bell {
                last_stale_present = Some(stale_present);
            } else {
                maybe_bell_stale(
                    stale_present,
                    args.debounce_ms,
                    &mut last_stale_present,
                    &mut last_stale_bell_at,
                )?;
            }
        }
        suppress_initial_bell = false;

        if resp.timed_out {
            continue;
        }
        if resp.has_more {
            continue;
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ThreadMeta {
    thread_id: ThreadId,
    cwd: String,
    archived: bool,
    #[serde(default)]
    archived_at: Option<String>,
    #[serde(default)]
    archived_reason: Option<String>,
    approval_policy: ApprovalPolicy,
    sandbox_policy: SandboxPolicy,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
    last_seq: u64,
    #[serde(default)]
    active_turn_id: Option<TurnId>,
    #[serde(default)]
    active_turn_interrupt_requested: bool,
    #[serde(default)]
    last_turn_id: Option<TurnId>,
    #[serde(default)]
    last_turn_status: Option<TurnStatus>,
    #[serde(default)]
    last_turn_reason: Option<String>,
    attention_state: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct ThreadListMetaResponse {
    threads: Vec<ThreadMeta>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ThreadAttention {
    thread_id: ThreadId,
    attention_state: String,
    #[serde(default)]
    pending_approvals: Vec<PendingApproval>,
    #[serde(default)]
    running_processes: Vec<RunningProcess>,
    #[serde(default)]
    stale_processes: Vec<StaleProcess>,
    #[serde(default)]
    failed_processes: Vec<ProcessId>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PendingApproval {
    approval_id: ApprovalId,
    #[serde(default)]
    action: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RunningProcess {
    process_id: ProcessId,
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StaleProcess {
    process_id: ProcessId,
    #[serde(default)]
    idle_seconds: u64,
    #[serde(default)]
    last_update_at: Option<String>,
    #[serde(default)]
    stdout_path: Option<String>,
    #[serde(default)]
    stderr_path: Option<String>,
}

async fn run_inbox(app: &mut App, args: InboxArgs) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(args.poll_ms.max(200));

    let mut last_snapshot: std::collections::BTreeMap<ThreadId, ThreadMeta> =
        std::collections::BTreeMap::new();
    let mut bell_state: std::collections::HashMap<ThreadId, (Option<String>, Option<Instant>)> =
        std::collections::HashMap::new();
    let mut stale_bell_state: std::collections::HashMap<ThreadId, (Option<bool>, Option<Instant>)> =
        std::collections::HashMap::new();

    loop {
        let raw = app.thread_list_meta(args.include_archived).await?;
        let resp: ThreadListMetaResponse = serde_json::from_value(raw)?;

        if args.json && !args.watch {
            println!("{}", serde_json::to_string_pretty(&resp)?);
            return Ok(());
        }

        let mut current = std::collections::BTreeMap::<ThreadId, ThreadMeta>::new();
        for thread in resp.threads {
            current.insert(thread.thread_id, thread);
        }

        if !args.watch {
            render_inbox_once(app, &current, args.details, args.json).await?;
            return Ok(());
        }

        render_inbox_changes(app, &last_snapshot, &current, args.details, args.json).await?;
        if args.bell {
            for (thread_id, thread) in &current {
                let state = thread.attention_state.as_str();
                if !matches!(state, "need_approval" | "failed" | "stuck") {
                    bell_state.entry(*thread_id).or_insert((None, None)).0 =
                        Some(thread.attention_state.clone());
                } else {
                    let entry = bell_state.entry(*thread_id).or_insert((None, None));
                    maybe_bell_per_thread(
                        thread_id,
                        &thread.attention_state,
                        args.debounce_ms,
                        &mut entry.0,
                        &mut entry.1,
                    )?;
                }

                if state == "running" {
                    let att = app.thread_attention(*thread_id).await?;
                    let att: ThreadAttention = serde_json::from_value(att)?;
                    let stale_present = !att.stale_processes.is_empty();
                    let entry = stale_bell_state.entry(*thread_id).or_insert((None, None));
                    maybe_bell_stale_per_thread(
                        thread_id,
                        stale_present,
                        args.debounce_ms,
                        &mut entry.0,
                        &mut entry.1,
                    )?;
                } else {
                    stale_bell_state.entry(*thread_id).or_insert((Some(false), None)).0 =
                        Some(false);
                }
            }
        }

        last_snapshot = current;
        tokio::time::sleep(poll_interval).await;
    }
}

async fn render_inbox_once(
    app: &mut App,
    threads: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    details: bool,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        let list = threads.values().collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    println!("threads: {}", threads.len());
    for thread in threads.values() {
        render_thread_row(thread);
        if details {
            let att = app.thread_attention(thread.thread_id).await?;
            let att: ThreadAttention = serde_json::from_value(att)?;
            render_thread_details(&att);
        }
    }
    Ok(())
}

async fn render_inbox_changes(
    app: &mut App,
    prev: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    cur: &std::collections::BTreeMap<ThreadId, ThreadMeta>,
    details: bool,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        let output = serde_json::json!({
            "prev_count": prev.len(),
            "cur_count": cur.len(),
            "threads": cur.values().collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    for (thread_id, meta) in cur {
        let changed = match prev.get(thread_id) {
            Some(old) => {
                old.last_seq != meta.last_seq || old.attention_state != meta.attention_state
            }
            None => true,
        };
        if !changed {
            continue;
        }

        render_thread_row(meta);
        if details {
            let att = app.thread_attention(*thread_id).await?;
            let att: ThreadAttention = serde_json::from_value(att)?;
            render_thread_details(&att);
        }
    }

    for thread_id in prev.keys() {
        if !cur.contains_key(thread_id) {
            println!("thread removed: {thread_id}");
        }
    }

    Ok(())
}

fn render_thread_row(thread: &ThreadMeta) {
    let cwd = shorten_path(&thread.cwd, 60);
    let model = thread.model.as_deref().unwrap_or("-");
    let turn = thread
        .active_turn_id
        .or(thread.last_turn_id)
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".to_string());
    println!(
        "{}  state={}  seq={}  turn={}  model={}  cwd={}",
        thread.thread_id, thread.attention_state, thread.last_seq, turn, model, cwd
    );
}

fn render_thread_details(att: &ThreadAttention) {
    if !att.pending_approvals.is_empty() {
        let ids = att
            .pending_approvals
            .iter()
            .take(3)
            .map(|a| a.approval_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  approvals: {} ({ids}{})",
            att.pending_approvals.len(),
            if att.pending_approvals.len() > 3 {
                ", ..."
            } else {
                ""
            }
        );
    }
    if !att.running_processes.is_empty() {
        let ids = att
            .running_processes
            .iter()
            .take(3)
            .map(|p| p.process_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  processes: {} ({ids}{})",
            att.running_processes.len(),
            if att.running_processes.len() > 3 {
                ", ..."
            } else {
                ""
            }
        );
    }
    if !att.failed_processes.is_empty() {
        let ids = att
            .failed_processes
            .iter()
            .take(3)
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  failed_processes: {} ({ids}{})",
            att.failed_processes.len(),
            if att.failed_processes.len() > 3 {
                ", ..."
            } else {
                ""
            }
        );
    }
    if !att.stale_processes.is_empty() {
        let ids = att
            .stale_processes
            .iter()
            .take(3)
            .map(|p| p.process_id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  stale_processes: {} ({ids}{})",
            att.stale_processes.len(),
            if att.stale_processes.len() > 3 {
                ", ..."
            } else {
                ""
            }
        );
    }
}

fn shorten_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }
    let keep = max_len.saturating_sub(3);
    let tail = path.chars().rev().take(keep).collect::<String>();
    format!("...{}", tail.chars().rev().collect::<String>())
}

fn maybe_bell_per_thread(
    thread_id: &ThreadId,
    state: &str,
    debounce_ms: u64,
    last_state: &mut Option<String>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let now = Instant::now();
    let debounced = last_state.as_deref().is_some_and(|s| s == state)
        && last_bell_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));

    if !debounced {
        eprintln!("attention: {thread_id} -> {state}");
        print!("\x07");
        std::io::stdout().flush().ok();
        *last_bell_at = Some(now);
    }

    *last_state = Some(state.to_string());
    Ok(())
}

fn attention_state_update(event: &ThreadEvent) -> Option<&'static str> {
    match &event.kind {
        omne_agent_protocol::ThreadEventKind::ApprovalRequested { .. } => Some("need_approval"),
        omne_agent_protocol::ThreadEventKind::TurnStarted { .. } => Some("running"),
        omne_agent_protocol::ThreadEventKind::TurnCompleted { status, .. } => match status {
            TurnStatus::Completed => Some("done"),
            TurnStatus::Interrupted => Some("interrupted"),
            TurnStatus::Failed => Some("failed"),
            TurnStatus::Cancelled => Some("cancelled"),
            TurnStatus::Stuck => Some("stuck"),
        },
        omne_agent_protocol::ThreadEventKind::ProcessStarted { .. } => Some("running"),
        omne_agent_protocol::ThreadEventKind::ProcessExited { exit_code, .. } => match exit_code {
            Some(code) if *code != 0 => Some("failed"),
            _ => None,
        },
        _ => None,
    }
}

fn maybe_bell(
    state: &'static str,
    debounce_ms: u64,
    last_state: &mut Option<&'static str>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let should_notify = matches!(state, "need_approval" | "failed" | "stuck");
    if !should_notify {
        *last_state = Some(state);
        return Ok(());
    }

    let now = Instant::now();
    let debounced = last_state.is_some_and(|s| s == state)
        && last_bell_at.is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));

    if !debounced {
        print!("\x07");
        std::io::stdout().flush().ok();
        *last_bell_at = Some(now);
    }

    *last_state = Some(state);
    Ok(())
}

fn maybe_bell_stale(
    stale_present: bool,
    debounce_ms: u64,
    last_stale_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if last_stale_present.is_none() {
        *last_stale_present = Some(stale_present);
        return Ok(());
    }

    if stale_present && last_stale_present == &Some(false) {
        let now = Instant::now();
        let debounced = last_bell_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));
        if !debounced {
            print!("\x07");
            std::io::stdout().flush().ok();
            *last_bell_at = Some(now);
        }
    }

    *last_stale_present = Some(stale_present);
    Ok(())
}

fn maybe_bell_stale_per_thread(
    thread_id: &ThreadId,
    stale_present: bool,
    debounce_ms: u64,
    last_stale_present: &mut Option<bool>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    if last_stale_present.is_none() {
        *last_stale_present = Some(stale_present);
        return Ok(());
    }

    if stale_present && last_stale_present == &Some(false) {
        let now = Instant::now();
        let debounced = last_bell_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(debounce_ms));
        if !debounced {
            eprintln!("attention: {thread_id} -> stale_process");
            print!("\x07");
            std::io::stdout().flush().ok();
            *last_bell_at = Some(now);
        }
    }

    *last_stale_present = Some(stale_present);
    Ok(())
}
