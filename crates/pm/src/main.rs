use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use pm_protocol::{
    ApprovalDecision, ApprovalId, ApprovalPolicy, ProcessId, SandboxPolicy, ThreadEvent, ThreadId,
    TurnId, TurnStatus,
};
use serde::Deserialize;
use serde_json::Value;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pm")]
#[command(about = "CodePM v0.2.0 agent CLI (drives pm-app-server)", long_about = None)]
struct Cli {
    /// Override `.code_pm` root directory.
    #[arg(long, global = true)]
    pm_root: Option<PathBuf>,

    /// Override `pm-app-server` binary path.
    #[arg(long, global = true)]
    app_server: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH", global = true)]
    execpolicy_rules: Vec<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    Ask(AskArgs),
    Watch(WatchArgs),
    Approval {
        #[command(subcommand)]
        command: ApprovalCommand,
    },
    Process {
        #[command(subcommand)]
        command: ProcessCommand,
    },
}

#[derive(Subcommand)]
enum ThreadCommand {
    Start {
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Resume {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    ListMeta {
        #[arg(long, default_value_t = false)]
        include_archived: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Attention {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Configure(ThreadConfigureArgs),
}

#[derive(Parser)]
struct ThreadConfigureArgs {
    thread_id: ThreadId,
    #[arg(long)]
    approval_policy: Option<CliApprovalPolicy>,
    #[arg(long)]
    sandbox_policy: Option<CliSandboxPolicy>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    openai_base_url: Option<String>,
}

#[derive(Subcommand)]
enum ApprovalCommand {
    List {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        include_decided: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Decide {
        thread_id: ThreadId,
        approval_id: ApprovalId,
        #[arg(long, conflicts_with = "deny", default_value_t = false)]
        approve: bool,
        #[arg(long, conflicts_with = "approve", default_value_t = false)]
        deny: bool,
        #[arg(long, default_value_t = false)]
        remember: bool,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProcessCommand {
    List {
        #[arg(long)]
        thread_id: Option<ThreadId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Inspect {
        process_id: ProcessId,
        #[arg(long)]
        max_lines: Option<usize>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Tail {
        process_id: ProcessId,
        #[arg(long, default_value_t = false)]
        stderr: bool,
        #[arg(long)]
        max_lines: Option<usize>,
    },
    Follow {
        process_id: ProcessId,
        #[arg(long, default_value_t = false)]
        stderr: bool,
        #[arg(long, default_value_t = 0)]
        since_offset: u64,
        #[arg(long)]
        max_bytes: Option<u64>,
        #[arg(long, default_value_t = 200)]
        poll_ms: u64,
    },
    Kill {
        process_id: ProcessId,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Parser)]
struct AskArgs {
    /// Resume an existing thread instead of creating a new one.
    #[arg(long)]
    thread_id: Option<ThreadId>,

    /// Working directory for a newly created thread.
    #[arg(long)]
    cwd: Option<PathBuf>,

    #[arg(long)]
    approval_policy: Option<CliApprovalPolicy>,

    #[arg(long)]
    sandbox_policy: Option<CliSandboxPolicy>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    openai_base_url: Option<String>,

    /// Message to send as the next turn.
    #[arg(value_parser = parse_non_empty_trimmed)]
    input: String,
}

#[derive(Parser)]
struct WatchArgs {
    thread_id: ThreadId,
    #[arg(long, default_value_t = 0)]
    since_seq: u64,
    #[arg(long)]
    max_events: Option<usize>,
    #[arg(long, default_value_t = 30_000)]
    wait_ms: u64,
    /// Emit a terminal bell on attention-worthy state changes.
    #[arg(long, default_value_t = false)]
    bell: bool,
    /// Debounce window for repeated bell notifications (milliseconds).
    #[arg(long, default_value_t = 30_000)]
    debounce_ms: u64,
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliApprovalPolicy {
    AutoApprove,
    Manual,
}

impl From<CliApprovalPolicy> for ApprovalPolicy {
    fn from(value: CliApprovalPolicy) -> Self {
        match value {
            CliApprovalPolicy::AutoApprove => Self::AutoApprove,
            CliApprovalPolicy::Manual => Self::Manual,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliSandboxPolicy {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl From<CliSandboxPolicy> for SandboxPolicy {
    fn from(value: CliSandboxPolicy) -> Self {
        match value {
            CliSandboxPolicy::ReadOnly => Self::ReadOnly,
            CliSandboxPolicy::WorkspaceWrite => Self::WorkspaceWrite,
            CliSandboxPolicy::DangerFullAccess => Self::DangerFullAccess,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SubscribeResponse {
    events: Vec<ThreadEvent>,
    last_seq: u64,
    has_more: bool,
    timed_out: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let mut app = App::connect(&cli).await?;

    match cli.command {
        Command::Thread { command } => match command {
            ThreadCommand::Start { cwd, json } => {
                let cwd = cwd.map(|p| p.display().to_string());
                let result = app.thread_start(cwd).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Resume { thread_id, json } => {
                let result = app.thread_resume(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::List { json } => {
                let result = app.thread_list().await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::ListMeta {
                include_archived,
                json,
            } => {
                let result = app.thread_list_meta(include_archived).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Attention { thread_id, json } => {
                let result = app.thread_attention(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Configure(args) => {
                app.thread_configure(args).await?;
            }
        },
        Command::Ask(args) => {
            run_ask(&mut app, args).await?;
        }
        Command::Watch(args) => {
            run_watch(&mut app, args).await?;
        }
        Command::Approval { command } => match command {
            ApprovalCommand::List {
                thread_id,
                include_decided,
                json,
            } => {
                let result = app.approval_list(thread_id, include_decided).await?;
                print_json_or_pretty(json, &result)?;
            }
            ApprovalCommand::Decide {
                thread_id,
                approval_id,
                approve,
                deny,
                remember,
                reason,
            } => {
                let decision = if approve {
                    ApprovalDecision::Approved
                } else if deny {
                    ApprovalDecision::Denied
                } else {
                    anyhow::bail!("must pass exactly one of --approve/--deny");
                };
                app.approval_decide(thread_id, approval_id, decision, remember, reason)
                    .await?;
            }
        },
        Command::Process { command } => match command {
            ProcessCommand::List { thread_id, json } => {
                let result = app.process_list(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ProcessCommand::Inspect {
                process_id,
                max_lines,
                json,
            } => {
                let result = app.process_inspect(process_id, max_lines).await?;
                print_json_or_pretty(json, &result)?;
            }
            ProcessCommand::Tail {
                process_id,
                stderr,
                max_lines,
            } => {
                let text = app.process_tail(process_id, stderr, max_lines).await?;
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            }
            ProcessCommand::Follow {
                process_id,
                stderr,
                since_offset,
                max_bytes,
                poll_ms,
            } => {
                run_process_follow(
                    &mut app,
                    process_id,
                    stderr,
                    since_offset,
                    max_bytes,
                    poll_ms,
                )
                .await?;
            }
            ProcessCommand::Kill { process_id, reason } => {
                app.process_kill(process_id, reason).await?;
            }
        },
    }

    Ok(())
}

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
        || args.model.is_some()
        || args.openai_base_url.is_some()
    {
        app.thread_configure(ThreadConfigureArgs {
            thread_id,
            approval_policy: args.approval_policy,
            sandbox_policy: args.sandbox_policy,
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

    loop {
        if interrupt_rx.try_recv().is_ok() {
            app.turn_interrupt(thread_id, turn_id, Some("ctrl-c".to_string()))
                .await?;
            eprintln!("interrupt requested: {turn_id}");
            return Ok(());
        }

        let resp = app
            .thread_subscribe(thread_id, since_seq, Some(10_000), Some(1_000))
            .await?;
        since_seq = resp.last_seq;

        for event in &resp.events {
            render_event(event);
            if let pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                action,
                params,
                ..
            } = &event.kind
            {
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

async fn run_watch(app: &mut App, args: WatchArgs) -> anyhow::Result<()> {
    let mut since_seq = args.since_seq;
    let mut last_state: Option<&'static str> = None;
    let mut last_bell_at: Option<Instant> = None;
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
            state_update = state_update.or_else(|| attention_state_update(event));
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
        suppress_initial_bell = false;

        if resp.timed_out {
            continue;
        }
        if resp.has_more {
            continue;
        }
    }
}

fn attention_state_update(event: &ThreadEvent) -> Option<&'static str> {
    match &event.kind {
        pm_protocol::ThreadEventKind::ApprovalRequested { .. } => Some("need_approval"),
        pm_protocol::ThreadEventKind::TurnStarted { .. } => Some("running"),
        pm_protocol::ThreadEventKind::TurnCompleted { status, .. } => match status {
            TurnStatus::Completed => Some("done"),
            TurnStatus::Interrupted => Some("interrupted"),
            TurnStatus::Failed => Some("failed"),
            TurnStatus::Cancelled => Some("cancelled"),
        },
        pm_protocol::ThreadEventKind::ProcessStarted { .. } => Some("running"),
        _ => None,
    }
}

fn maybe_bell(
    state: &'static str,
    debounce_ms: u64,
    last_state: &mut Option<&'static str>,
    last_bell_at: &mut Option<Instant>,
) -> anyhow::Result<()> {
    let should_notify = matches!(state, "need_approval" | "failed");
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

async fn run_process_follow(
    app: &mut App,
    process_id: ProcessId,
    stderr: bool,
    mut offset: u64,
    max_bytes: Option<u64>,
    poll_ms: u64,
) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(poll_ms.max(50));
    loop {
        let (text, next_offset, eof) = app
            .process_follow(process_id, stderr, offset, max_bytes)
            .await?;
        offset = next_offset;
        if !text.is_empty() {
            print!("{text}");
            std::io::stdout().flush().ok();
        }

        if eof {
            let status = app.process_status(process_id).await?;
            if status != "running" {
                return Ok(());
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

fn render_event(event: &ThreadEvent) {
    let ts = event
        .timestamp
        .format(&time::format_description::well_known::Rfc3339);
    let ts = ts.unwrap_or_else(|_| "<time>".to_string());
    match &event.kind {
        pm_protocol::ThreadEventKind::ThreadCreated { cwd } => {
            println!("[{ts}] thread created cwd={cwd}");
        }
        pm_protocol::ThreadEventKind::ThreadArchived { reason } => {
            println!(
                "[{ts}] thread archived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnarchived { reason } => {
            println!(
                "[{ts}] thread unarchived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnStarted { turn_id, input } => {
            println!("[{ts}] turn started {turn_id}");
            println!("user: {input}");
        }
        pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
            println!(
                "[{ts}] turn interrupt requested {turn_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } => {
            println!(
                "[{ts}] turn completed {turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            model,
            openai_base_url,
        } => {
            println!(
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} model={} openai_base_url={}",
                model.as_deref().unwrap_or(""),
                openai_base_url.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ApprovalRequested {
            approval_id,
            action,
            ..
        } => {
            println!("[{ts}] approval requested {approval_id} action={action}");
        }
        pm_protocol::ThreadEventKind::ApprovalDecided {
            approval_id,
            decision,
            remember,
            reason,
        } => {
            println!(
                "[{ts}] approval decided {approval_id} decision={decision:?} remember={remember} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ToolStarted { tool, .. } => {
            println!("[{ts}] tool started {tool}");
        }
        pm_protocol::ThreadEventKind::ToolCompleted { status, error, .. } => {
            println!(
                "[{ts}] tool completed status={status:?} error={}",
                error.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::AssistantMessage { text, model, .. } => {
            if let Some(model) = model {
                println!("[{ts}] assistant (model={model}):");
            } else {
                println!("[{ts}] assistant:");
            }
            println!("{text}");
        }
        pm_protocol::ThreadEventKind::ProcessStarted {
            process_id, argv, ..
        } => {
            println!("[{ts}] process started {process_id} argv={argv:?}");
        }
        pm_protocol::ThreadEventKind::ProcessKillRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process kill requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => {
            println!(
                "[{ts}] process exited {process_id} exit_code={} reason={}",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                reason.as_deref().unwrap_or("")
            );
        }
    }
}

struct App {
    rpc: pm_jsonrpc::Client,
}

impl App {
    async fn connect(cli: &Cli) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        let pm_root = cli
            .pm_root
            .clone()
            .or_else(|| std::env::var_os("CODE_PM_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| cwd.join(".code_pm"));

        let server = cli.app_server.clone().unwrap_or_else(|| {
            default_app_server_path().unwrap_or_else(|| PathBuf::from("pm-app-server"))
        });

        let mut argv: Vec<OsString> = Vec::new();
        argv.push("--pm-root".into());
        argv.push(pm_root.into_os_string());
        for path in &cli.execpolicy_rules {
            argv.push("--execpolicy-rules".into());
            argv.push(path.clone().into_os_string());
        }

        let mut rpc = pm_jsonrpc::Client::spawn(server, argv).await?;
        let _ = rpc.request("initialize", serde_json::json!({})).await?;
        let _ = rpc.request("initialized", serde_json::json!({})).await?;
        Ok(Self { rpc })
    }

    async fn rpc(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        Ok(self.rpc.request(method, params).await?)
    }

    async fn thread_start(&mut self, cwd: Option<String>) -> anyhow::Result<Value> {
        self.rpc("thread/start", serde_json::json!({ "cwd": cwd }))
            .await
    }

    async fn thread_resume(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/resume",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_list(&mut self) -> anyhow::Result<Value> {
        self.rpc("thread/list", serde_json::json!({})).await
    }

    async fn thread_list_meta(&mut self, include_archived: bool) -> anyhow::Result<Value> {
        self.rpc(
            "thread/list_meta",
            serde_json::json!({ "include_archived": include_archived }),
        )
        .await
    }

    async fn thread_attention(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/attention",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_configure(&mut self, args: ThreadConfigureArgs) -> anyhow::Result<()> {
        let approval_policy: Option<ApprovalPolicy> = args.approval_policy.map(Into::into);
        let sandbox_policy: Option<SandboxPolicy> = args.sandbox_policy.map(Into::into);
        let _ = self
            .rpc(
                "thread/configure",
                serde_json::json!({
                    "thread_id": args.thread_id,
                    "approval_policy": approval_policy,
                    "sandbox_policy": sandbox_policy,
                    "model": args.model,
                    "openai_base_url": args.openai_base_url,
                }),
            )
            .await?;
        Ok(())
    }

    async fn turn_start(&mut self, thread_id: ThreadId, input: String) -> anyhow::Result<TurnId> {
        let v = self
            .rpc(
                "turn/start",
                serde_json::json!({ "thread_id": thread_id, "input": input }),
            )
            .await?;
        Ok(serde_json::from_value(v["turn_id"].clone()).context("turn_id missing in result")?)
    }

    async fn turn_interrupt(
        &mut self,
        thread_id: ThreadId,
        turn_id: TurnId,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let _ = self
            .rpc(
                "turn/interrupt",
                serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "reason": reason,
                }),
            )
            .await?;
        Ok(())
    }

    async fn thread_subscribe(
        &mut self,
        thread_id: ThreadId,
        since_seq: u64,
        max_events: Option<usize>,
        wait_ms: Option<u64>,
    ) -> anyhow::Result<SubscribeResponse> {
        let v = self
            .rpc(
                "thread/subscribe",
                serde_json::json!({
                    "thread_id": thread_id,
                    "since_seq": since_seq,
                    "max_events": max_events,
                    "wait_ms": wait_ms,
                }),
            )
            .await?;
        Ok(serde_json::from_value(v)?)
    }

    async fn approval_list(
        &mut self,
        thread_id: ThreadId,
        include_decided: bool,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "approval/list",
            serde_json::json!({
                "thread_id": thread_id,
                "include_decided": include_decided,
            }),
        )
        .await
    }

    async fn approval_decide(
        &mut self,
        thread_id: ThreadId,
        approval_id: ApprovalId,
        decision: ApprovalDecision,
        remember: bool,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let _ = self
            .rpc(
                "approval/decide",
                serde_json::json!({
                    "thread_id": thread_id,
                    "approval_id": approval_id,
                    "decision": decision,
                    "remember": remember,
                    "reason": reason,
                }),
            )
            .await?;
        Ok(())
    }

    async fn process_list(&mut self, thread_id: Option<ThreadId>) -> anyhow::Result<Value> {
        self.rpc(
            "process/list",
            serde_json::json!({
                "thread_id": thread_id,
            }),
        )
        .await
    }

    async fn process_inspect(
        &mut self,
        process_id: ProcessId,
        max_lines: Option<usize>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "process/inspect",
            serde_json::json!({
                "process_id": process_id,
                "max_lines": max_lines,
            }),
        )
        .await
    }

    async fn process_tail(
        &mut self,
        process_id: ProcessId,
        stderr: bool,
        max_lines: Option<usize>,
    ) -> anyhow::Result<String> {
        let stream = if stderr { "stderr" } else { "stdout" };
        let v = self
            .rpc(
                "process/tail",
                serde_json::json!({
                    "process_id": process_id,
                    "stream": stream,
                    "max_lines": max_lines,
                }),
            )
            .await?;
        Ok(v["text"].as_str().unwrap_or("").to_string())
    }

    async fn process_follow(
        &mut self,
        process_id: ProcessId,
        stderr: bool,
        since_offset: u64,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<(String, u64, bool)> {
        let stream = if stderr { "stderr" } else { "stdout" };
        let v = self
            .rpc(
                "process/follow",
                serde_json::json!({
                    "process_id": process_id,
                    "stream": stream,
                    "since_offset": since_offset,
                    "max_bytes": max_bytes,
                }),
            )
            .await?;

        let text = v["text"].as_str().unwrap_or("").to_string();
        let next_offset = v["next_offset"].as_u64().unwrap_or(since_offset);
        let eof = v["eof"].as_bool().unwrap_or(true);
        Ok((text, next_offset, eof))
    }

    async fn process_status(&mut self, process_id: ProcessId) -> anyhow::Result<String> {
        let v = self.process_inspect(process_id, Some(0)).await?;
        Ok(v["process"]["status"]
            .as_str()
            .unwrap_or("unknown")
            .to_string())
    }

    async fn process_kill(
        &mut self,
        process_id: ProcessId,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let _ = self
            .rpc(
                "process/kill",
                serde_json::json!({
                    "process_id": process_id,
                    "reason": reason,
                }),
            )
            .await?;
        Ok(())
    }
}

fn default_app_server_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(app_server_exe_name());
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

fn app_server_exe_name() -> &'static str {
    if cfg!(windows) {
        "pm-app-server.exe"
    } else {
        "pm-app-server"
    }
}
