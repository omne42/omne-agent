use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use pm_core::{
    Architect, CommandHookRunner, EventBus, FsStorage, HookRunner, HookSpec, Orchestrator, PmPaths,
    PrName, RuleBasedArchitect, SessionId,
};
use pm_git::{RepoManager, RepoRoot, find_repo_root};
use pm_http::serve as serve_http;
use time::format_description::well_known::Rfc3339;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "code-pm")]
#[command(about = "Local Git service + concurrent AI task pipeline (Phase 1 skeleton)", long_about = None)]
struct Cli {
    /// Override `.code_pm` root directory. Relative paths are resolved against repo root.
    #[arg(long, global = true)]
    pm_root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init(InitArgs),
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Serve(ServeArgs),
    Run(Box<RunArgs>),
}

#[derive(Parser, Clone)]
struct InitArgs {
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Subcommand)]
enum RepoCommand {
    Inject {
        #[arg(value_parser = parse_non_empty_trimmed)]
        source: String,
        #[arg(long, value_parser = parse_non_empty_trimmed)]
        name: Option<String>,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    List(RepoListArgs),
}

#[derive(Parser, Clone)]
struct RepoListArgs {
    #[arg(long, default_value_t = false)]
    verbose: bool,
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Subcommand)]
enum SessionCommand {
    List(SessionListArgs),
    Show {
        id: SessionId,
        #[arg(long, default_value_t = false)]
        all: bool,
    },
}

#[derive(Parser, Clone)]
struct SessionListArgs {
    #[arg(long, default_value_t = false)]
    verbose: bool,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Parser, Clone)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:9417")]
    addr: std::net::SocketAddr,
}

#[derive(Parser, Clone)]
struct RunArgs {
    #[arg(long, value_parser = parse_non_empty_trimmed)]
    repo: Option<String>,
    #[arg(long, value_parser = parse_non_empty_trimmed)]
    repo_src: Option<String>,
    #[arg(long, value_parser = parse_non_empty_trimmed)]
    pr_name: String,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    prompt_file: Option<PathBuf>,
    #[arg(long, default_value = "main", value_parser = parse_non_empty_trimmed)]
    base: String,
    #[arg(long)]
    apply_patch: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = 1,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    max_concurrency: u32,
    #[arg(long, default_value_t = false)]
    stream_events: bool,
    #[arg(long, default_value_t = false)]
    stream_events_json: bool,
    #[arg(long, default_value_t = false)]
    strict: bool,
    #[arg(long, default_value_t = false)]
    json: bool,
    /// For Rust repos, also run `cargo test` before committing.
    #[arg(long, default_value_t = false)]
    cargo_test: bool,
    /// Skip merging PR branches into the base branch.
    #[arg(long, default_value_t = false)]
    no_merge: bool,
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["tasks_file", "task"]
    )]
    auto_tasks: bool,
    #[arg(long)]
    tasks_file: Option<PathBuf>,
    #[arg(long)]
    task: Vec<String>,
    #[arg(long)]
    hook_cmd: Option<PathBuf>,
    #[arg(long)]
    hook_arg: Vec<String>,
    #[arg(long)]
    hook_url: Option<String>,
}

