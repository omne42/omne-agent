use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use pm_protocol::{
    ApprovalDecision, ApprovalId, ApprovalPolicy, ProcessId, SandboxPolicy, ThreadEvent, ThreadId,
    TurnId, TurnStatus,
};
use serde::{Deserialize, Serialize};
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
    Inbox(InboxArgs),
    Ask(AskArgs),
    Exec(ExecArgs),
    Watch(WatchArgs),
    Approval {
        #[command(subcommand)]
        command: ApprovalCommand,
    },
    Process {
        #[command(subcommand)]
        command: ProcessCommand,
    },
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommand,
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
    Fork {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Spawn {
        thread_id: ThreadId,
        input: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        openai_base_url: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Archive {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        force: bool,
        #[arg(long)]
        reason: Option<String>,
    },
    Unarchive {
        thread_id: ThreadId,
        #[arg(long)]
        reason: Option<String>,
    },
    Pause {
        thread_id: ThreadId,
        #[arg(long)]
        reason: Option<String>,
    },
    Unpause {
        thread_id: ThreadId,
        #[arg(long)]
        reason: Option<String>,
    },
    Delete {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        force: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    ClearArtifacts {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        force: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    DiskUsage {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    DiskReport {
        thread_id: ThreadId,
        #[arg(long)]
        top_files: Option<usize>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    HookRun {
        thread_id: ThreadId,
        hook: CliWorkspaceHookName,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Events {
        thread_id: ThreadId,
        #[arg(long, default_value_t = 0)]
        since_seq: u64,
        #[arg(long)]
        max_events: Option<usize>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Loaded {
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
    State {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    ConfigExplain {
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
    mode: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    openai_base_url: Option<String>,
}

#[derive(Parser)]
struct InboxArgs {
    #[arg(long, default_value_t = false)]
    include_archived: bool,
    /// Print details (pending approvals + running processes).
    #[arg(long, default_value_t = false)]
    details: bool,
    /// Watch for changes and stream updates.
    #[arg(long, default_value_t = false)]
    watch: bool,
    #[arg(long, default_value_t = 1_000)]
    poll_ms: u64,
    /// Emit a terminal bell when attention becomes `need_approval` or `failed`.
    #[arg(long, default_value_t = false)]
    bell: bool,
    /// Debounce window for repeated bell notifications (milliseconds).
    #[arg(long, default_value_t = 30_000)]
    debounce_ms: u64,
    #[arg(long, default_value_t = false)]
    json: bool,
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
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Tail {
        process_id: ProcessId,
        #[arg(long, default_value_t = false)]
        stderr: bool,
        #[arg(long)]
        max_lines: Option<usize>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
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
        #[arg(long)]
        approval_id: Option<ApprovalId>,
    },
    Interrupt {
        process_id: ProcessId,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
    },
    Kill {
        process_id: ProcessId,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
    },
}

#[derive(Subcommand)]
enum ArtifactCommand {
    List {
        thread_id: ThreadId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Read {
        thread_id: ThreadId,
        artifact_id: pm_protocol::ArtifactId,
        #[arg(long)]
        max_bytes: Option<u64>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Delete {
        thread_id: ThreadId,
        artifact_id: pm_protocol::ArtifactId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
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
    mode: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    openai_base_url: Option<String>,

    /// Message to send as the next turn.
    #[arg(value_parser = parse_non_empty_trimmed)]
    input: String,
}

#[derive(Parser)]
struct ExecArgs {
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
    mode: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    openai_base_url: Option<String>,

    /// Behavior when an approval is requested (exec is non-interactive).
    #[arg(long, value_enum, default_value_t = CliOnApproval::Fail)]
    on_approval: CliOnApproval,

    /// Persist approval decisions within this session.
    #[arg(long, default_value_t = false)]
    remember: bool,

    /// Output a machine-readable JSON summary.
    #[arg(long, default_value_t = false)]
    json: bool,

    /// Message to send as the next turn.
    #[arg(value_parser = parse_non_empty_trimmed)]
    input: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliOnApproval {
    Fail,
    Approve,
    Deny,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliWorkspaceHookName {
    Setup,
    Run,
    Archive,
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
    AutoDeny,
}

impl From<CliApprovalPolicy> for ApprovalPolicy {
    fn from(value: CliApprovalPolicy) -> Self {
        match value {
            CliApprovalPolicy::AutoApprove => Self::AutoApprove,
            CliApprovalPolicy::Manual => Self::Manual,
            CliApprovalPolicy::AutoDeny => Self::AutoDeny,
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
