use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use pm_protocol::{
    ApprovalDecision, ApprovalId, ApprovalPolicy, CheckpointId, ProcessId, SandboxPolicy,
    ThreadEvent, ThreadId, TurnId, TurnStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pm")]
#[command(about = "CodePM v0.2.0 agent CLI (drives pm-app-server)", long_about = None)]
struct Cli {
    /// Override project data root directory (default: `./.codepm_data/`).
    #[arg(long, global = true)]
    pm_root: Option<PathBuf>,

    /// Override `pm-app-server` binary path.
    #[arg(long, global = true)]
    app_server: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH", global = true)]
    execpolicy_rules: Vec<PathBuf>,

    /// When omitted, starts an interactive REPL.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize `./.codepm_data/` in the current project.
    Init(InitArgs),
    Reference {
        #[command(subcommand)]
        command: ReferenceCommand,
    },
    Preset {
        #[command(subcommand)]
        command: PresetCommand,
    },
    #[command(name = "command")]
    Workflow {
        #[command(subcommand)]
        command: CommandCommand,
    },
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Start a full-screen TUI (thin client over JSON-RPC).
    Tui(TuiArgs),
    /// Start an interactive REPL.
    Repl,
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    Checkpoint {
        #[command(subcommand)]
        command: CheckpointCommand,
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
enum CommandCommand {
    /// List available commands under `./.codepm_data/spec/commands/`.
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show a command (frontmatter + body).
    Show {
        name: String,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Run a command (executes context steps, then starts a turn).
    Run(CommandRunArgs),
}

#[derive(Parser)]
struct CommandRunArgs {
    name: String,

    /// Template variables: `--var key=value` (repeatable).
    #[arg(long = "var", value_name = "KEY=VALUE")]
    vars: Vec<CommandVar>,

    /// Resume an existing thread instead of creating a new one.
    #[arg(long)]
    thread_id: Option<ThreadId>,

    /// Working directory for a newly created thread.
    #[arg(long)]
    cwd: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct CommandVar {
    key: String,
    value: String,
}

impl std::str::FromStr for CommandVar {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (key, value) = input
            .split_once('=')
            .ok_or_else(|| "expected KEY=VALUE".to_string())?;
        let key = key.trim();
        if key.is_empty() {
            return Err("command var key must not be empty".to_string());
        }
        Ok(Self {
            key: key.to_string(),
            value: value.to_string(),
        })
    }
}

#[derive(clap::Args)]
struct TuiArgs {
    /// Open an existing thread directly (skips thread picker).
    #[arg(long)]
    thread_id: Option<ThreadId>,

    /// Include archived threads in the picker.
    #[arg(long, default_value_t = false)]
    include_archived: bool,
}

#[derive(Subcommand)]
enum ReferenceCommand {
    /// Import a local directory as the project's reference repo snapshot.
    Import {
        /// Source directory to copy from.
        from: PathBuf,
        /// Overwrite any existing reference repo.
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Skip files larger than this (bytes). Default: 10MiB.
        #[arg(long)]
        max_file_bytes: Option<u64>,
        /// Output JSON instead of human-readable text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Show the currently installed reference repo (if any).
    Status {
        /// Output JSON instead of human-readable text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand, Clone)]
enum PresetCommand {
    /// Export the effective thread config as a preset file (no secrets).
    Export {
        thread_id: ThreadId,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Import a preset file and apply it via `thread/configure`.
    Import {
        thread_id: ThreadId,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum RepoRoot {
    Workspace,
    Reference,
}

impl RepoRoot {
    fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Reference => "reference",
        }
    }
}

#[derive(Subcommand)]
enum RepoCommand {
    Search {
        thread_id: ThreadId,
        query: String,
        #[arg(long, default_value_t = false)]
        regex: bool,
        #[arg(long)]
        include_glob: Option<String>,
        #[arg(long)]
        max_matches: Option<usize>,
        #[arg(long)]
        max_bytes_per_file: Option<u64>,
        #[arg(long)]
        max_files: Option<usize>,
        #[arg(long)]
        root: Option<RepoRoot>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Index {
        thread_id: ThreadId,
        #[arg(long)]
        include_glob: Option<String>,
        #[arg(long)]
        max_files: Option<usize>,
        #[arg(long)]
        root: Option<RepoRoot>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Symbols {
        thread_id: ThreadId,
        #[arg(long)]
        include_glob: Option<String>,
        #[arg(long)]
        max_files: Option<usize>,
        #[arg(long)]
        max_bytes_per_file: Option<u64>,
        #[arg(long)]
        max_symbols: Option<usize>,
        #[arg(long)]
        root: Option<RepoRoot>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum McpCommand {
    /// List configured MCP servers (from `.codepm_data/spec/mcp.json`).
    ListServers {
        thread_id: ThreadId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List tools exposed by an MCP server.
    ListTools {
        thread_id: ThreadId,
        server: String,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List resources exposed by an MCP server.
    ListResources {
        thread_id: ThreadId,
        server: String,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Call a tool exposed by an MCP server.
    Call {
        thread_id: ThreadId,
        server: String,
        tool: String,
        #[arg(long)]
        arguments_json: Option<String>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(clap::Args)]
struct InitArgs {
    /// Target directory to initialize (defaults to current working directory).
    #[arg(long)]
    dir: Option<PathBuf>,

    /// Overwrite existing files when present.
    #[arg(long, default_value_t = false)]
    force: bool,

    /// Skip interactive prompts and use defaults.
    #[arg(long, default_value_t = false)]
    yes: bool,

    /// Enable project config by default (`.codepm_data/config.toml`).
    #[arg(long, default_value_t = false)]
    enable_project_config: bool,

    /// Create `.codepm_data/config_local.toml` template (gitignored).
    #[arg(long, default_value_t = false)]
    create_config_local: bool,
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
    Diff {
        thread_id: ThreadId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long)]
        max_bytes: Option<u64>,
        #[arg(long)]
        wait_seconds: Option<u64>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Patch {
        thread_id: ThreadId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long)]
        max_bytes: Option<u64>,
        #[arg(long)]
        wait_seconds: Option<u64>,
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
    Models {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Configure(ThreadConfigureArgs),
}

#[derive(Subcommand)]
enum CheckpointCommand {
    /// Create a checkpoint snapshot of the current workspace.
    Create {
        thread_id: ThreadId,
        #[arg(long)]
        label: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// List available checkpoints for a thread.
    List {
        thread_id: ThreadId,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Restore the workspace from a checkpoint (requires prompt_strict approval).
    Restore {
        thread_id: ThreadId,
        checkpoint_id: CheckpointId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Parser)]
struct ThreadConfigureArgs {
    thread_id: ThreadId,
    #[arg(long)]
    approval_policy: Option<CliApprovalPolicy>,
    #[arg(long)]
    sandbox_policy: Option<CliSandboxPolicy>,
    #[arg(long, value_delimiter = ',')]
    sandbox_writable_roots: Option<Vec<String>>,
    #[arg(long)]
    sandbox_network_access: Option<CliSandboxNetworkAccess>,
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
    OnRequest,
    Manual,
    UnlessTrusted,
    AutoDeny,
}

impl From<CliApprovalPolicy> for ApprovalPolicy {
    fn from(value: CliApprovalPolicy) -> Self {
        match value {
            CliApprovalPolicy::AutoApprove => Self::AutoApprove,
            CliApprovalPolicy::OnRequest => Self::OnRequest,
            CliApprovalPolicy::Manual => Self::Manual,
            CliApprovalPolicy::UnlessTrusted => Self::UnlessTrusted,
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliSandboxNetworkAccess {
    Deny,
    Allow,
}

impl From<CliSandboxNetworkAccess> for pm_protocol::SandboxNetworkAccess {
    fn from(value: CliSandboxNetworkAccess) -> Self {
        match value {
            CliSandboxNetworkAccess::Deny => Self::Deny,
            CliSandboxNetworkAccess::Allow => Self::Allow,
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
