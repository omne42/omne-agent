use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use omne_protocol::{
    ApprovalDecision, ApprovalId, ApprovalPolicy, ArtifactId, CheckpointId, ProcessId,
    ThreadEvent, ThreadEventKindTag, ThreadId, TurnId, TurnStatus, THREAD_EVENT_KIND_TAGS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "omne")]
#[command(about = "OmneAgent v0.2.0 agent CLI (drives omne-app-server)", long_about = None)]
struct Cli {
    /// Override project data root directory (default: `./.omne_data/`).
    #[arg(long, global = true)]
    omne_root: Option<PathBuf>,

    /// Override `omne-app-server` binary path.
    #[arg(long, global = true)]
    app_server: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH", global = true)]
    execpolicy_rules: Vec<PathBuf>,

    /// When omitted, starts the full-screen TUI.
    #[command(subcommand)]
    command: Option<Command>,
}

fn thread_event_kind_value_parser(raw: &str) -> Result<ThreadEventKindTag, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    ThreadEventKindTag::from_str(&normalized).map_err(|_| {
        format!(
            "unsupported thread event kind: {} (supported: {})",
            raw,
            THREAD_EVENT_KIND_TAGS.join(", ")
        )
    })
}

#[derive(Subcommand)]
enum Command {
    /// Initialize `./.omne_data/` in the current project.
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
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Model {
        #[command(subcommand)]
        command: ModelCommand,
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
    /// Start an interactive CLI (REPL-style).
    #[command(alias = "repl")]
    Cli,
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
    /// List available commands under `./.omne_data/spec/commands/`.
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
    /// Validate command file(s); exits non-zero when any command is invalid.
    Validate {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = false)]
        strict: bool,
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

    /// Parse `## Task: <id> <title>` sections in the command body and run them as parallel
    /// read-only subagent turns before starting the main turn.
    #[arg(long, default_value_t = false)]
    fan_out: bool,

    /// When used with `--fan-out`, start the main turn without waiting for all fan-out tasks to
    /// finish. The `fan_in_summary` artifact will be updated while the main turn runs.
    #[arg(long, default_value_t = false)]
    fan_out_early_return: bool,

    /// Resume an existing thread instead of creating a new one.
    #[arg(long)]
    thread_id: Option<ThreadId>,

    /// Working directory for a newly created thread.
    #[arg(long)]
    cwd: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// Add or update a provider profile (merge update, no full-file overwrite).
    #[command(alias = "set")]
    Add(Box<ProviderAddArgs>),
    /// List configured providers.
    #[command(alias = "ls")]
    List(ProviderListArgs),
    /// Show one provider configuration.
    Show(ProviderShowArgs),
    /// Delete one provider configuration.
    #[command(alias = "rm")]
    Delete(ProviderDeleteArgs),
}

#[derive(Subcommand)]
enum ModelCommand {
    /// Add or update a model profile (merge update, no full-file overwrite).
    #[command(alias = "set")]
    Add(ModelAddArgs),
    /// List configured models.
    #[command(alias = "ls")]
    List(ModelListArgs),
    /// Show one model configuration.
    Show(ModelShowArgs),
    /// Delete one model configuration.
    #[command(alias = "rm")]
    Delete(ModelDeleteArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ConfigScope {
    Auto,
    Workspace,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ProviderNamespace {
    Openai,
    Google,
    Gemini,
    Claude,
    Anthropic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ProviderApiArg {
    OpenaiChatCompletions,
    OpenaiResponses,
    GeminiGenerateContent,
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
enum ProviderAuthTypeArg {
    ApiKeyEnv,
    QueryParamEnv,
    HttpHeaderEnv,
    Command,
}

#[derive(clap::Args)]
struct ProviderAddArgs {
    /// Provider key under `<namespace>.providers.<name>`.
    name: String,

    /// Provider namespace (protocol family namespace in config).
    #[arg(long, value_enum, default_value_t = ProviderNamespace::Openai)]
    namespace: ProviderNamespace,

    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    /// Provider base URL.
    #[arg(long)]
    base_url: Option<String>,

    /// Provider default model id.
    #[arg(long)]
    default_model: Option<String>,

    /// Upstream protocol used against provider.
    #[arg(long, value_enum)]
    upstream_api: Option<ProviderApiArg>,

    /// Normalized response protocol for Omne-facing path.
    #[arg(long, value_enum)]
    normalize_to: Option<ProviderApiArg>,

    /// Explicit normalized endpoint path (example: `/v1/chat/completions`).
    #[arg(long)]
    normalize_endpoint: Option<String>,

    /// Authentication mode.
    #[arg(long, value_enum, default_value_t = ProviderAuthTypeArg::ApiKeyEnv)]
    auth_type: ProviderAuthTypeArg,

    /// Env key candidates (comma-separated or repeated).
    #[arg(long = "auth-key", value_delimiter = ',')]
    auth_keys: Vec<String>,

    /// Query param name for `query_param_env` auth (default: `key`).
    #[arg(long)]
    auth_param: Option<String>,

    /// Header name for `http_header_env` auth.
    #[arg(long)]
    auth_header: Option<String>,

    /// Optional auth value prefix (example: `Bearer `).
    #[arg(long)]
    auth_prefix: Option<String>,

    /// Command argv for `command` auth (repeatable or comma-separated).
    #[arg(long = "auth-command", value_delimiter = ',')]
    auth_command: Vec<String>,

    /// Persist as `[openai].provider = "<namespace>.providers.<name>"`.
    #[arg(long, default_value_t = false)]
    set_default: bool,

    /// Also set `[openai].model` from `--default-model` when provided.
    #[arg(long, default_value_t = false)]
    set_default_model: bool,

    /// Override capabilities (pass explicit true/false).
    #[arg(long)]
    tools: Option<bool>,
    #[arg(long)]
    vision: Option<bool>,
    #[arg(long)]
    reasoning: Option<bool>,
    #[arg(long)]
    json_schema: Option<bool>,
    #[arg(long)]
    streaming: Option<bool>,
    #[arg(long)]
    prompt_cache: Option<bool>,

    /// Discover remote model list and update provider model whitelist.
    #[arg(long, default_value_t = false)]
    discover_models: bool,

    /// API key for online discovery only (not persisted in config).
    #[arg(long)]
    api_key: Option<String>,

    /// Also register discovered models into `[openai.models.<id>]`.
    #[arg(long, default_value_t = false)]
    register_models: bool,

    /// Optional cap for imported model count during discovery.
    #[arg(long)]
    model_limit: Option<usize>,

    /// Force interactive wizard mode (default behavior).
    #[arg(long, default_value_t = false)]
    interactive: bool,

    /// Disable interactive wizard mode and run only from provided args.
    #[arg(long, default_value_t = false, conflicts_with = "interactive")]
    no_interactive: bool,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ProviderListArgs {
    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    /// Optional namespace filter.
    #[arg(long, value_enum)]
    namespace: Option<ProviderNamespace>,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ProviderShowArgs {
    /// Provider key name.
    name: String,

    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    /// Provider namespace.
    #[arg(long, value_enum, default_value_t = ProviderNamespace::Openai)]
    namespace: ProviderNamespace,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ProviderDeleteArgs {
    /// Provider key name.
    name: String,

    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    /// Provider namespace.
    #[arg(long, value_enum, default_value_t = ProviderNamespace::Openai)]
    namespace: ProviderNamespace,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ModelAddArgs {
    /// Model key under `[openai.models."<name>"]`.
    name: String,

    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    /// Optional default provider pointer (`[openai].provider`).
    #[arg(long)]
    provider: Option<String>,

    /// Optional fallback provider list (`[openai].fallback_providers`).
    #[arg(long = "fallback-provider", value_delimiter = ',')]
    fallback_providers: Vec<String>,

    /// Set `[openai].model` to this model.
    #[arg(long, default_value_t = false)]
    set_default: bool,

    /// Model thinking intensity.
    #[arg(long)]
    thinking: Option<String>,

    /// Model context window.
    #[arg(long)]
    context_window: Option<u64>,

    /// Auto compact token limit.
    #[arg(long)]
    auto_compact_token_limit: Option<u64>,

    /// Per-model prompt cache hint.
    #[arg(long)]
    prompt_cache: Option<bool>,

    /// Force interactive wizard mode (default behavior).
    #[arg(long, default_value_t = false)]
    interactive: bool,

    /// Disable interactive wizard mode and run only from provided args.
    #[arg(long, default_value_t = false, conflicts_with = "interactive")]
    no_interactive: bool,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ModelListArgs {
    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ModelShowArgs {
    /// Model key under `[openai.models."<name>"]`.
    name: String,

    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(clap::Args)]
struct ModelDeleteArgs {
    /// Model key under `[openai.models."<name>"]`.
    name: String,

    /// Update target location. `auto` prefers workspace when `./.omne_data` exists.
    #[arg(long, value_enum, default_value_t = ConfigScope::Auto)]
    scope: ConfigScope,

    #[arg(long, default_value_t = false)]
    json: bool,
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
    /// List available preset files under `.omne_data/spec/`.
    List {
        #[arg(long, default_value_t = false)]
        json: bool,
    },
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
    /// Show one preset file (resolved by --file or --name).
    Show {
        #[arg(long, conflicts_with = "name", required_unless_present = "name")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        name: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Validate preset file(s); exits non-zero when any preset is invalid.
    Validate {
        #[arg(long, conflicts_with = "name")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file")]
        name: Option<String>,
        #[arg(long, default_value_t = false)]
        strict: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Import a preset file and apply it via `thread/configure`.
    Import {
        thread_id: ThreadId,
        #[arg(long, conflicts_with = "name", required_unless_present = "name")]
        file: Option<PathBuf>,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        name: Option<String>,
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
    fn to_file_root(self) -> omne_app_server_protocol::FileRoot {
        match self {
            Self::Workspace => omne_app_server_protocol::FileRoot::Workspace,
            Self::Reference => omne_app_server_protocol::FileRoot::Reference,
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
    /// Serve OmneAgent as an MCP server over stdio (experimental).
    ///
    /// This exposes a small read-only tool allowlist intended for other MCP clients.
    Serve(McpServeArgs),
    /// List configured MCP servers (from `.omne_data/spec/mcp.json`).
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

#[derive(Parser)]
struct McpServeArgs {
    /// Write one audit artifact per external tool call (default: enabled).
    ///
    /// When not set, a new thread is created in `--audit-cwd` (default: current directory).
    #[arg(long)]
    audit_thread_id: Option<ThreadId>,

    /// Working directory used when creating a new audit thread.
    #[arg(long)]
    audit_cwd: Option<PathBuf>,

    /// Disable audit artifact logging entirely.
    #[arg(long, default_value_t = false)]
    no_audit: bool,
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

    /// Enable project config by default (`.omne_data/config.toml`).
    #[arg(long, default_value_t = false)]
    enable_project_config: bool,

    /// Create `.omne_data/config_local.toml` template (gitignored).
    #[arg(long, default_value_t = false)]
    create_config_local: bool,

    /// Skip creating default command templates under `.omne_data/spec/commands/`.
    #[arg(long, default_value_t = false)]
    no_command_templates: bool,

    /// Skip creating `.omne_data/spec/workspace.yaml` template.
    #[arg(long, default_value_t = false)]
    no_workspace_template: bool,

    /// Skip creating `.omne_data/spec/hooks.yaml` template.
    #[arg(long, default_value_t = false)]
    no_hooks_template: bool,

    /// Skip creating `.omne_data/spec/modes.yaml` template.
    #[arg(long, default_value_t = false)]
    no_modes_template: bool,

    /// Skip creating all default spec templates under `.omne_data/spec/`.
    #[arg(long, default_value_t = false)]
    no_spec_templates: bool,

    /// Minimal bootstrap mode (equivalent to `--no-spec-templates`).
    #[arg(long, default_value_t = false)]
    minimal: bool,
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
        #[arg(long = "kind", value_parser = thread_event_kind_value_parser)]
        kinds: Vec<ThreadEventKindTag>,
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
        include_attention_markers: bool,
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
    Usage {
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
    role: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value_t = false, conflicts_with = "model")]
    clear_model: bool,
    #[arg(long)]
    openai_base_url: Option<String>,
    #[arg(long, default_value_t = false, conflicts_with = "openai_base_url")]
    clear_openai_base_url: bool,
    #[arg(long)]
    thinking: Option<String>,
    #[arg(long, default_value_t = false, conflicts_with = "thinking")]
    clear_thinking: bool,
    #[arg(long, conflicts_with = "clear_show_thinking")]
    show_thinking: Option<bool>,
    #[arg(long, default_value_t = false, conflicts_with = "show_thinking")]
    clear_show_thinking: bool,
    #[arg(
        long = "allowed-tools",
        value_delimiter = ',',
        conflicts_with = "clear_allowed_tools"
    )]
    allowed_tools: Option<Vec<String>>,
    #[arg(long, default_value_t = false, conflicts_with = "allowed_tools")]
    clear_allowed_tools: bool,
    #[arg(
        long = "execpolicy-rules",
        value_delimiter = ',',
        conflicts_with = "clear_execpolicy_rules"
    )]
    execpolicy_rules: Option<Vec<String>>,
    #[arg(long, default_value_t = false, conflicts_with = "execpolicy_rules")]
    clear_execpolicy_rules: bool,
}

#[derive(Parser)]
struct InboxArgs {
    #[arg(long, default_value_t = false)]
    include_archived: bool,
    /// Only show threads with an active fan-out linkage issue marker.
    #[arg(long, default_value_t = false)]
    only_fan_out_linkage_issue: bool,
    /// Only show threads with an active fan-out auto-apply error marker.
    #[arg(long, default_value_t = false)]
    only_fan_out_auto_apply_error: bool,
    /// Only show threads with dependency-blocked fan-in tasks.
    #[arg(long, default_value_t = false)]
    only_fan_in_dependency_blocked: bool,
    /// Only show threads with fan-in result diagnostics summaries.
    #[arg(long, default_value_t = false)]
    only_fan_in_result_diagnostics: bool,
    /// Only show threads with token budget exceeded.
    #[arg(long, default_value_t = false)]
    only_token_budget_exceeded: bool,
    /// Only show threads with token budget warning active.
    #[arg(long, default_value_t = false)]
    only_token_budget_warning: bool,
    /// Only show threads that are currently blocked on subagent proxy approvals.
    #[arg(long, default_value_t = false)]
    only_subagent_proxy_approval: bool,
    /// Print details (pending approvals + running processes).
    #[arg(long, default_value_t = false)]
    details: bool,
    /// Watch for changes and stream updates.
    #[arg(long, default_value_t = false)]
    watch: bool,
    #[arg(long, default_value_t = 1_000)]
    poll_ms: u64,
    /// Emit a terminal bell on attention-worthy changes (`need_approval|failed|stuck`,
    /// stale process, new fan-out/fan-in marker summaries, or token-budget warnings).
    #[arg(long, default_value_t = false)]
    bell: bool,
    /// Debounce window for repeated bell notifications (milliseconds).
    #[arg(long, default_value_t = 30_000)]
    debounce_ms: u64,
    /// Emit summary cache/source debug lines (also enabled by `OMNE_INBOX_SUMMARY_CACHE_DEBUG=1`).
    #[arg(long, default_value_t = false)]
    debug_summary_cache: bool,
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
        artifact_id: omne_protocol::ArtifactId,
        #[arg(long)]
        version: Option<u32>,
        #[arg(long)]
        max_bytes: Option<u64>,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Versions {
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
        #[arg(long)]
        approval_id: Option<ApprovalId>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    Delete {
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
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
    /// Emit a terminal bell on attention-worthy state changes, stale processes, and
    /// fan-out/fan-in summary warnings.
    #[arg(long, default_value_t = false)]
    bell: bool,
    /// Debounce window for repeated bell notifications (milliseconds).
    #[arg(long, default_value_t = 30_000)]
    debounce_ms: u64,
    /// Emit fan-out/fan-in artifact summaries after non-empty event batches.
    #[arg(long, default_value_t = false)]
    details: bool,
    /// Emit summary cache/source debug lines (also enabled by `OMNE_WATCH_SUMMARY_CACHE_DEBUG=1`).
    #[arg(long, default_value_t = false)]
    debug_summary_cache: bool,
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliApprovalPolicy {
    AutoApprove,
    Manual,
    UnlessTrusted,
    AutoDeny,
}

impl From<CliApprovalPolicy> for ApprovalPolicy {
    fn from(value: CliApprovalPolicy) -> Self {
        match value {
            CliApprovalPolicy::AutoApprove => Self::AutoApprove,
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
    FullAccess,
}

impl From<CliSandboxPolicy> for policy_meta::WriteScope {
    fn from(value: CliSandboxPolicy) -> Self {
        match value {
            CliSandboxPolicy::ReadOnly => Self::ReadOnly,
            CliSandboxPolicy::WorkspaceWrite => Self::WorkspaceWrite,
            CliSandboxPolicy::FullAccess => Self::FullAccess,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliSandboxNetworkAccess {
    Deny,
    Allow,
}

impl From<CliSandboxNetworkAccess> for omne_protocol::SandboxNetworkAccess {
    fn from(value: CliSandboxNetworkAccess) -> Self {
        match value {
            CliSandboxNetworkAccess::Deny => Self::Deny,
            CliSandboxNetworkAccess::Allow => Self::Allow,
        }
    }
}

type SubscribeResponse = omne_app_server_protocol::ThreadSubscribeResponse;

#[cfg(test)]
mod preamble_tests {
    use super::*;

    #[test]
    fn thread_usage_parses_thread_id() {
        let cli = Cli::try_parse_from([
            "omne",
            "thread",
            "usage",
            "00000000-0000-0000-0000-000000000000",
        ])
        .expect("thread usage should parse");
        let Command::Thread { command } = cli.command.expect("expected command") else {
            panic!("expected thread command");
        };
        let ThreadCommand::Usage { thread_id, json } = command else {
            panic!("expected thread usage command");
        };
        assert_eq!(
            thread_id.to_string(),
            "00000000-0000-0000-0000-000000000000"
        );
        assert!(!json);
    }

    #[test]
    fn thread_events_accepts_known_kind_filter() {
        let cli = Cli::try_parse_from([
            "omne",
            "thread",
            "events",
            "00000000-0000-0000-0000-000000000000",
            "--kind",
            "attention_marker_set",
        ])
        .expect("known kind should parse");
        let Command::Thread { command } = cli.command.expect("expected command") else {
            panic!("expected thread command");
        };
        let ThreadCommand::Events { kinds, .. } = command else {
            panic!("expected thread events command");
        };
        assert_eq!(kinds, vec![ThreadEventKindTag::AttentionMarkerSet]);
    }

    #[test]
    fn thread_events_rejects_unknown_kind_filter() {
        let err = match Cli::try_parse_from([
            "omne",
            "thread",
            "events",
            "00000000-0000-0000-0000-000000000000",
            "--kind",
            "not_a_real_event_kind",
        ]) {
            Ok(_) => panic!("unknown kind should be rejected"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(msg.contains("not_a_real_event_kind"));
    }

    #[test]
    fn preset_import_accepts_name_selector() {
        let cli = Cli::try_parse_from([
            "omne",
            "preset",
            "import",
            "00000000-0000-0000-0000-000000000000",
            "--name",
            "reviewer-safe",
        ])
        .expect("preset import --name should parse");
        let Command::Preset { command } = cli.command.expect("expected command") else {
            panic!("expected preset command");
        };
        let PresetCommand::Import { file, name, .. } = command else {
            panic!("expected preset import");
        };
        assert!(file.is_none());
        assert_eq!(name.as_deref(), Some("reviewer-safe"));
    }

    #[test]
    fn preset_import_requires_selector() {
        let err = match Cli::try_parse_from([
            "omne",
            "preset",
            "import",
            "00000000-0000-0000-0000-000000000000",
        ]) {
            Ok(_) => panic!("missing selector should be rejected"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(msg.contains("--file"));
        assert!(msg.contains("--name"));
    }

    #[test]
    fn preset_show_accepts_name_selector() {
        let cli = Cli::try_parse_from(["omne", "preset", "show", "--name", "reviewer-safe"])
            .expect("preset show --name should parse");
        let Command::Preset { command } = cli.command.expect("expected command") else {
            panic!("expected preset command");
        };
        let PresetCommand::Show { file, name, .. } = command else {
            panic!("expected preset show");
        };
        assert!(file.is_none());
        assert_eq!(name.as_deref(), Some("reviewer-safe"));
    }

    #[test]
    fn preset_show_requires_selector() {
        let err = match Cli::try_parse_from(["omne", "preset", "show"]) {
            Ok(_) => panic!("missing selector should be rejected"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(msg.contains("--file"));
        assert!(msg.contains("--name"));
    }

    #[test]
    fn preset_validate_accepts_no_selector() {
        let cli = Cli::try_parse_from(["omne", "preset", "validate"])
            .expect("preset validate without selector should parse");
        let Command::Preset { command } = cli.command.expect("expected command") else {
            panic!("expected preset command");
        };
        let PresetCommand::Validate {
            file,
            name,
            strict,
            json,
        } = command
        else {
            panic!("expected preset validate");
        };
        assert!(file.is_none());
        assert!(name.is_none());
        assert!(!strict);
        assert!(!json);
    }

    #[test]
    fn preset_validate_accepts_name_selector() {
        let cli = Cli::try_parse_from(["omne", "preset", "validate", "--name", "reviewer-safe"])
            .expect("preset validate --name should parse");
        let Command::Preset { command } = cli.command.expect("expected command") else {
            panic!("expected preset command");
        };
        let PresetCommand::Validate {
            file, name, strict, ..
        } = command
        else {
            panic!("expected preset validate");
        };
        assert!(file.is_none());
        assert_eq!(name.as_deref(), Some("reviewer-safe"));
        assert!(!strict);
    }

    #[test]
    fn preset_validate_accepts_strict_flag() {
        let cli = Cli::try_parse_from(["omne", "preset", "validate", "--strict"])
            .expect("preset validate --strict should parse");
        let Command::Preset { command } = cli.command.expect("expected command") else {
            panic!("expected preset command");
        };
        let PresetCommand::Validate { strict, .. } = command else {
            panic!("expected preset validate");
        };
        assert!(strict);
    }

    #[test]
    fn command_validate_accepts_no_selector() {
        let cli = Cli::try_parse_from(["omne", "command", "validate"])
            .expect("command validate without selector should parse");
        let Command::Workflow { command } = cli.command.expect("expected command") else {
            panic!("expected command workflow");
        };
        let CommandCommand::Validate { name, strict, json } = command else {
            panic!("expected command validate");
        };
        assert!(name.is_none());
        assert!(!strict);
        assert!(!json);
    }

    #[test]
    fn command_validate_accepts_name_strict_and_json() {
        let cli = Cli::try_parse_from([
            "omne", "command", "validate", "--name", "plan", "--strict", "--json",
        ])
        .expect("command validate with flags should parse");
        let Command::Workflow { command } = cli.command.expect("expected command") else {
            panic!("expected command workflow");
        };
        let CommandCommand::Validate { name, strict, json } = command else {
            panic!("expected command validate");
        };
        assert_eq!(name.as_deref(), Some("plan"));
        assert!(strict);
        assert!(json);
    }

    #[test]
    fn inbox_accepts_debug_summary_cache_flag() {
        let cli = Cli::try_parse_from(["omne", "inbox", "--debug-summary-cache"])
            .expect("inbox --debug-summary-cache should parse");
        let Command::Inbox(args) = cli.command.expect("expected command") else {
            panic!("expected inbox command");
        };
        assert!(args.debug_summary_cache);
    }

    #[test]
    fn inbox_accepts_only_fan_in_result_diagnostics_flag() {
        let cli = Cli::try_parse_from(["omne", "inbox", "--only-fan-in-result-diagnostics"])
            .expect("inbox --only-fan-in-result-diagnostics should parse");
        let Command::Inbox(args) = cli.command.expect("expected command") else {
            panic!("expected inbox command");
        };
        assert!(args.only_fan_in_result_diagnostics);
    }

    #[test]
    fn inbox_accepts_only_token_budget_exceeded_flag() {
        let cli = Cli::try_parse_from(["omne", "inbox", "--only-token-budget-exceeded"])
            .expect("inbox --only-token-budget-exceeded should parse");
        let Command::Inbox(args) = cli.command.expect("expected command") else {
            panic!("expected inbox command");
        };
        assert!(args.only_token_budget_exceeded);
    }

    #[test]
    fn inbox_accepts_only_token_budget_warning_flag() {
        let cli = Cli::try_parse_from(["omne", "inbox", "--only-token-budget-warning"])
            .expect("inbox --only-token-budget-warning should parse");
        let Command::Inbox(args) = cli.command.expect("expected command") else {
            panic!("expected inbox command");
        };
        assert!(args.only_token_budget_warning);
    }

    #[test]
    fn watch_accepts_debug_summary_cache_flag() {
        let cli = Cli::try_parse_from([
            "omne",
            "watch",
            "00000000-0000-0000-0000-000000000000",
            "--debug-summary-cache",
        ])
        .expect("watch --debug-summary-cache should parse");
        let Command::Watch(args) = cli.command.expect("expected command") else {
            panic!("expected watch command");
        };
        assert!(args.debug_summary_cache);
    }

    #[test]
    fn provider_add_parses_namespace_scope_and_auth_keys() {
        let cli = Cli::try_parse_from([
            "omne",
            "provider",
            "add",
            "openrouter",
            "--namespace",
            "google",
            "--scope",
            "workspace",
            "--auth-key",
            "OPENROUTER_API_KEY,OPENAI_API_KEY",
            "--upstream-api",
            "openai_chat_completions",
        ])
        .expect("provider add should parse");
        let Command::Provider { command } = cli.command.expect("expected command") else {
            panic!("expected provider command");
        };
        let ProviderCommand::Add(args) = command else {
            panic!("expected provider add command");
        };
        assert_eq!(args.namespace, ProviderNamespace::Google);
        assert_eq!(args.scope, ConfigScope::Workspace);
        assert_eq!(
            args.upstream_api,
            Some(ProviderApiArg::OpenaiChatCompletions)
        );
        assert_eq!(
            args.auth_keys,
            vec![
                "OPENROUTER_API_KEY".to_string(),
                "OPENAI_API_KEY".to_string()
            ]
        );
    }

    #[test]
    fn provider_set_alias_parses_namespace_scope_and_auth_keys() {
        let cli = Cli::try_parse_from([
            "omne",
            "provider",
            "set",
            "openrouter",
            "--namespace",
            "google",
            "--scope",
            "workspace",
            "--auth-key",
            "OPENROUTER_API_KEY,OPENAI_API_KEY",
            "--upstream-api",
            "openai_chat_completions",
        ])
        .expect("provider set alias should parse");
        let Command::Provider { command } = cli.command.expect("expected command") else {
            panic!("expected provider command");
        };
        let ProviderCommand::Add(args) = command else {
            panic!("expected provider add command");
        };
        assert_eq!(args.namespace, ProviderNamespace::Google);
        assert_eq!(args.scope, ConfigScope::Workspace);
        assert_eq!(
            args.upstream_api,
            Some(ProviderApiArg::OpenaiChatCompletions)
        );
        assert_eq!(
            args.auth_keys,
            vec![
                "OPENROUTER_API_KEY".to_string(),
                "OPENAI_API_KEY".to_string()
            ]
        );
    }

    #[test]
    fn model_add_parses_set_default_and_prompt_cache() {
        let cli = Cli::try_parse_from([
            "omne",
            "model",
            "add",
            "google/gemini-3.1-pro-preview",
            "--set-default",
            "--prompt-cache",
            "true",
        ])
        .expect("model add should parse");
        let Command::Model { command } = cli.command.expect("expected command") else {
            panic!("expected model command");
        };
        let ModelCommand::Add(args) = command else {
            panic!("expected model add command");
        };
        assert!(args.set_default);
        assert_eq!(args.prompt_cache, Some(true));
    }

    #[test]
    fn model_set_alias_parses_set_default_and_prompt_cache() {
        let cli = Cli::try_parse_from([
            "omne",
            "model",
            "set",
            "google/gemini-3.1-pro-preview",
            "--set-default",
            "--prompt-cache",
            "true",
        ])
        .expect("model set alias should parse");
        let Command::Model { command } = cli.command.expect("expected command") else {
            panic!("expected model command");
        };
        let ModelCommand::Add(args) = command else {
            panic!("expected model add command");
        };
        assert!(args.set_default);
        assert_eq!(args.prompt_cache, Some(true));
    }

    #[test]
    fn provider_add_accepts_interactive_flag() {
        let cli = Cli::try_parse_from([
            "omne",
            "provider",
            "add",
            "openrouter",
            "--interactive",
        ])
        .expect("provider add --interactive should parse");
        let Command::Provider { command } = cli.command.expect("expected command") else {
            panic!("expected provider command");
        };
        let ProviderCommand::Add(args) = command else {
            panic!("expected provider add command");
        };
        assert!(args.interactive);
        assert!(!args.no_interactive);
    }

    #[test]
    fn model_add_accepts_interactive_flag() {
        let cli = Cli::try_parse_from(["omne", "model", "add", "gemini-3.1-pro", "--interactive"])
            .expect("model add --interactive should parse");
        let Command::Model { command } = cli.command.expect("expected command") else {
            panic!("expected model command");
        };
        let ModelCommand::Add(args) = command else {
            panic!("expected model add command");
        };
        assert!(args.interactive);
        assert!(!args.no_interactive);
    }

    #[test]
    fn provider_add_accepts_no_interactive_flag() {
        let cli = Cli::try_parse_from([
            "omne",
            "provider",
            "add",
            "openrouter",
            "--no-interactive",
        ])
        .expect("provider add --no-interactive should parse");
        let Command::Provider { command } = cli.command.expect("expected command") else {
            panic!("expected provider command");
        };
        let ProviderCommand::Add(args) = command else {
            panic!("expected provider add command");
        };
        assert!(args.no_interactive);
        assert!(!args.interactive);
    }

    #[test]
    fn model_add_accepts_no_interactive_flag() {
        let cli =
            Cli::try_parse_from(["omne", "model", "add", "gemini-3.1-pro", "--no-interactive"])
                .expect("model add --no-interactive should parse");
        let Command::Model { command } = cli.command.expect("expected command") else {
            panic!("expected model command");
        };
        let ModelCommand::Add(args) = command else {
            panic!("expected model add command");
        };
        assert!(args.no_interactive);
        assert!(!args.interactive);
    }

    #[test]
    fn provider_list_alias_parses() {
        let cli = Cli::try_parse_from(["omne", "provider", "ls", "--namespace", "google"])
            .expect("provider ls should parse");
        let Command::Provider { command } = cli.command.expect("expected command") else {
            panic!("expected provider command");
        };
        let ProviderCommand::List(args) = command else {
            panic!("expected provider list command");
        };
        assert_eq!(args.namespace, Some(ProviderNamespace::Google));
    }

    #[test]
    fn model_delete_alias_parses() {
        let cli = Cli::try_parse_from(["omne", "model", "rm", "gemini-3.1-pro"])
            .expect("model rm should parse");
        let Command::Model { command } = cli.command.expect("expected command") else {
            panic!("expected model command");
        };
        let ModelCommand::Delete(args) = command else {
            panic!("expected model delete command");
        };
        assert_eq!(args.name, "gemini-3.1-pro");
    }
}
