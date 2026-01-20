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
use pm_git::{RepoManager, find_repo_root};
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;

    let env_pm_root = std::env::var_os("CODE_PM_ROOT");
    let (pm_root, pm_root_source) =
        resolve_pm_root(&repo_root, cli.pm_root.as_deref(), env_pm_root.as_deref());
    if let Some(note) = legacy_pm_root_warning(&repo_root, &pm_root, pm_root_source) {
        eprintln!("{note}");
    }
    let pm_paths = PmPaths::new(pm_root.clone());
    let storage = FsStorage::new(pm_paths.data_dir());

    let repo_manager = RepoManager::new(pm_paths.clone());

    match cli.command {
        Command::Init(args) => {
            repo_manager.ensure_layout().await?;
            if args.json {
                let output = serde_json::json!({
                    "pm_root": pm_paths.root().display().to_string(),
                    "repos_dir": pm_paths.repos_dir().display().to_string(),
                    "data_dir": pm_paths.data_dir().display().to_string(),
                    "locks_dir": pm_paths.locks_dir().display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("{}", pm_paths.root().display());
            }
        }
        Command::Repo { command } => match command {
            RepoCommand::Inject { source, name, json } => {
                let repo_name = name
                    .as_deref()
                    .map(sanitize_repo_name_input)
                    .unwrap_or_else(|| RepoManager::default_repo_name_from_source(&source));
                let repo = repo_manager.inject(&repo_name, &source).await?;
                if json {
                    let output = serde_json::json!({
                        "name": repo.name.as_str(),
                        "bare_path": repo.bare_path.display().to_string(),
                        "lock_path": repo.lock_path.display().to_string(),
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    println!("repo: {}", repo.name.as_str());
                    println!("bare: {}", repo.bare_path.display());
                }
            }
            RepoCommand::List(args) => {
                let repos = repo_manager.list_repos().await?;
                if args.verbose {
                    #[derive(serde::Serialize)]
                    struct RepoListItem {
                        name: String,
                        bare_path: String,
                        lock_path: String,
                    }

                    if args.json {
                        let items = repos
                            .iter()
                            .map(|name| RepoListItem {
                                name: name.as_str().to_string(),
                                bare_path: repo_manager
                                    .paths()
                                    .repo_bare_path(name)
                                    .display()
                                    .to_string(),
                                lock_path: repo_manager
                                    .paths()
                                    .repo_lock_path(name)
                                    .display()
                                    .to_string(),
                            })
                            .collect::<Vec<_>>();
                        println!("{}", serde_json::to_string_pretty(&items)?);
                    } else {
                        for name in repos {
                            let bare = repo_manager.paths().repo_bare_path(&name);
                            let lock = repo_manager.paths().repo_lock_path(&name);
                            println!(
                                "{} bare={} lock={}",
                                name.as_str(),
                                bare.display(),
                                lock.display()
                            );
                        }
                    }
                } else if args.json {
                    println!("{}", serde_json::to_string_pretty(&repos)?);
                } else {
                    for name in repos {
                        println!("{}", name.as_str());
                    }
                }
            }
        },
        Command::Session { command } => match command {
            SessionCommand::List(args) => {
                if args.verbose {
                    let sessions = storage.list_session_meta().await?;
                    let sessions = match args.limit {
                        Some(limit) => sessions.into_iter().take(limit).collect::<Vec<_>>(),
                        None => sessions,
                    };
                    if args.json {
                        println!("{}", serde_json::to_string_pretty(&sessions)?);
                    } else {
                        for session in sessions {
                            let created_at = session
                                .created_at
                                .format(&Rfc3339)
                                .unwrap_or_else(|_| session.created_at.to_string());
                            println!(
                                "{} repo={} pr={} base={} created_at={}",
                                session.id,
                                session.repo.as_str(),
                                session.pr_name.as_str(),
                                session.base_branch,
                                created_at
                            );
                        }
                    }
                } else {
                    let sessions = list_sessions(&storage).await?;
                    let sessions = match args.limit {
                        Some(limit) => sessions.into_iter().take(limit).collect::<Vec<_>>(),
                        None => sessions,
                    };
                    if args.json {
                        println!("{}", serde_json::to_string_pretty(&sessions)?);
                    } else {
                        for id in sessions {
                            println!("{id}");
                        }
                    }
                }
            }
            SessionCommand::Show { id, all } => {
                let json = show_session_json(&storage, id, all).await?;
                println!("{json}");
            }
        },
        Command::Serve(args) => {
            if !args.addr.ip().is_loopback() {
                anyhow::bail!("serve is loopback-only; use --addr 127.0.0.1:<port>");
            }
            repo_manager.ensure_layout().await?;
            serve_http(pm_paths.clone(), args.addr).await?;
        }
        Command::Run(args) => {
            run_session(&repo_root, repo_manager, storage, *args).await?;
        }
    }

    Ok(())
}

fn resolve_pm_root(
    repo_root: &std::path::Path,
    cli_root: Option<&std::path::Path>,
    env_root: Option<&OsStr>,
) -> (PathBuf, PmRootSource) {
    let override_root = cli_root.map(std::path::Path::as_os_str).or(env_root);
    match override_root {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            let root = if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            };
            (root, PmRootSource::Override)
        }
        _ => (repo_root.join(".code_pm"), PmRootSource::Default),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PmRootSource {
    Default,
    Override,
}

fn legacy_pm_root_warning(
    repo_root: &std::path::Path,
    pm_root: &std::path::Path,
    source: PmRootSource,
) -> Option<String> {
    if source != PmRootSource::Default {
        return None;
    }

    let legacy = repo_root.join(".codex_pm");
    if legacy.is_dir() && !pm_root.is_dir() {
        Some(format!(
            "warning: found legacy pm root `{}` but current default is `{}`; to reuse old data: `code-pm --pm-root .codex_pm ...` or `mv .codex_pm .code_pm`",
            legacy.display(),
            pm_root.display()
        ))
    } else {
        None
    }
}

async fn list_sessions(storage: &FsStorage) -> anyhow::Result<Vec<SessionId>> {
    storage.list_session_ids().await
}

async fn show_session_json(
    storage: &FsStorage,
    id: SessionId,
    all: bool,
) -> anyhow::Result<String> {
    let value = storage
        .get_session_bundle(id, all)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
    Ok(serde_json::to_string_pretty(&value)?)
}

async fn read_prompt(args: &RunArgs) -> anyhow::Result<String> {
    match (&args.prompt, &args.prompt_file) {
        (Some(text), None) => {
            if text.trim().is_empty() {
                anyhow::bail!("--prompt must not be empty");
            }
            Ok(text.clone())
        }
        (None, Some(path)) => {
            let text = tokio::fs::read_to_string(path).await?;
            if text.trim().is_empty() {
                anyhow::bail!(
                    "--prompt-file content must not be empty: {}",
                    path.display()
                );
            }
            Ok(text)
        }
        (None, None) => anyhow::bail!("missing --prompt or --prompt-file"),
        (Some(_), Some(_)) => anyhow::bail!("use only one of --prompt or --prompt-file"),
    }
}

async fn run_session(
    repo_root: &std::path::Path,
    repo_manager: RepoManager,
    storage: FsStorage,
    mut args: RunArgs,
) -> anyhow::Result<()> {
    let prompt = read_prompt(&args).await?;

    let pr_name = PrName::sanitize(&args.pr_name);
    let tasks = parse_tasks_override(&args).await?;

    let (repo_name, repo) = match resolve_run_repo(repo_root, &args)? {
        ResolvedRunRepo::Load(repo_name) => {
            let repo = repo_manager.load(&repo_name).await?;
            (repo_name, repo)
        }
        ResolvedRunRepo::Inject { repo_name, source } => {
            let repo = repo_manager.inject(&repo_name, &source).await?;
            (repo_name, repo)
        }
    };

    if args.hook_cmd.is_none() && !args.hook_arg.is_empty() {
        anyhow::bail!("--hook-arg requires --hook-cmd");
    }
    if args.hook_cmd.is_some() && args.hook_url.is_some() {
        anyhow::bail!("use only one of --hook-cmd or --hook-url");
    }

    let hook = match (args.hook_cmd.take(), args.hook_url.take()) {
        (None, None) => None,
        (Some(program), None) => Some(HookSpec::Command {
            program,
            args: std::mem::take(&mut args.hook_arg),
        }),
        (None, Some(url)) => {
            let url = url.trim().to_string();
            if url.is_empty() {
                anyhow::bail!("--hook-url must not be empty");
            }
            Some(HookSpec::Webhook { url })
        }
        (Some(_), Some(_)) => unreachable!("validated above"),
    };
    let hook_runner: Arc<dyn HookRunner> = Arc::new(CliHookRunner::new()?);

    let architect: Arc<dyn Architect> = if args.auto_tasks {
        Arc::new(RuleBasedArchitect::default())
    } else {
        Arc::new(TemplateArchitect)
    };
    let coder: Arc<dyn pm_core::Coder> = Arc::new(pm_git::GitCoder::default());
    let merger: Arc<dyn pm_core::Merger> = Arc::new(pm_git::GitMerger::default());

    let events = EventBus::default();
    let stream_events_mode =
        resolve_stream_events_mode(args.stream_events, args.stream_events_json)?;
    let printer = if let Some(stream_events_mode) = stream_events_mode {
        let mut rx = events.subscribe();
        Some(tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => match stream_events_mode {
                        StreamEventsMode::Text => eprintln!("[event] {event}"),
                        StreamEventsMode::Json => match serde_json::to_string(&event) {
                            Ok(json) => eprintln!("{json}"),
                            Err(err) => {
                                let line = serde_json::json!({
                                    "type": "error",
                                    "error": "event_json_serialize",
                                    "message": err.to_string(),
                                });
                                eprintln!("{line}");
                            }
                        },
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                        match stream_events_mode {
                            StreamEventsMode::Text => {
                                eprintln!("[event] dropped {dropped} events (lagged)")
                            }
                            StreamEventsMode::Json => {
                                let line = serde_json::json!({
                                    "type": "error",
                                    "error": "event_lagged",
                                    "dropped": dropped,
                                    "message": format!("dropped {dropped} events due to lag"),
                                });
                                eprintln!("{line}");
                            }
                        }
                        continue;
                    }
                }
            }
        }))
    } else {
        None
    };

    let result = {
        let orchestrator = Orchestrator {
            storage: Arc::new(storage),
            hook_runner,
            events: events.clone(),
            architect,
            coder,
            merger,
        };

        let request = pm_core::RunRequest {
            pr_name,
            prompt,
            base_branch: args.base,
            apply_patch: args.apply_patch,
            hook,
            max_concurrency: args.max_concurrency as usize,
            tasks,
            cargo_test: args.cargo_test,
        };

        orchestrator.run(repo_manager.paths(), repo, request).await
    };

    drop(events);
    if let Some(printer) = printer {
        let _ = printer.await;
    }
    let result = result?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("session: {}", result.session.id);
        println!("repo: {}", repo_name.as_str());
        println!("prs: {}", result.prs.len());
        println!("merged: {}", result.merge.merged);
    }

    if args.strict {
        validate_strict_run_result(&result)?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamEventsMode {
    Text,
    Json,
}

fn parse_non_empty_trimmed(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err("value must not be empty".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn sanitize_repo_name_input(value: &str) -> pm_core::RepositoryName {
    let value = value.trim();
    let value = value.trim_end_matches(['/', '\\']);
    let value = value.strip_suffix(".git").unwrap_or(value);
    pm_core::RepositoryName::sanitize(value)
}

#[derive(Debug)]
enum ResolvedRunRepo {
    Load(pm_core::RepositoryName),
    Inject {
        repo_name: pm_core::RepositoryName,
        source: String,
    },
}

fn resolve_run_repo(
    repo_root: &std::path::Path,
    args: &RunArgs,
) -> anyhow::Result<ResolvedRunRepo> {
    match (&args.repo, &args.repo_src) {
        (Some(name), None) => Ok(ResolvedRunRepo::Load(sanitize_repo_name_input(name))),
        (maybe_name, Some(source)) => {
            let repo_name = maybe_name
                .as_deref()
                .map(sanitize_repo_name_input)
                .unwrap_or_else(|| RepoManager::default_repo_name_from_source(source));
            Ok(ResolvedRunRepo::Inject {
                repo_name,
                source: source.clone(),
            })
        }
        (None, None) => {
            if !repo_root.join(".git").exists() {
                anyhow::bail!("missing --repo or --repo-src");
            }
            let repo_name = repo_root
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_repo_name_input)
                .unwrap_or_else(|| pm_core::RepositoryName::sanitize("repo"));
            let source = repo_root.to_string_lossy().to_string();
            Ok(ResolvedRunRepo::Inject { repo_name, source })
        }
    }
}

fn resolve_stream_events_mode(
    stream_events: bool,
    stream_events_json: bool,
) -> anyhow::Result<Option<StreamEventsMode>> {
    match (stream_events, stream_events_json) {
        (false, false) => Ok(None),
        (true, false) => Ok(Some(StreamEventsMode::Text)),
        (false, true) => Ok(Some(StreamEventsMode::Json)),
        (true, true) => anyhow::bail!("use only one of --stream-events or --stream-events-json"),
    }
}

#[derive(Clone)]
struct CliHookRunner {
    command: CommandHookRunner,
    webhook: WebhookHookRunner,
}

impl CliHookRunner {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            command: CommandHookRunner,
            webhook: WebhookHookRunner::new()?,
        })
    }
}

#[async_trait::async_trait]
impl HookRunner for CliHookRunner {
    async fn run(
        &self,
        hook: &HookSpec,
        pm_paths: &PmPaths,
        session_paths: &pm_core::SessionPaths,
        result: &pm_core::RunResult,
    ) -> anyhow::Result<()> {
        match hook {
            HookSpec::Command { .. } => {
                self.command
                    .run(hook, pm_paths, session_paths, result)
                    .await
            }
            HookSpec::Webhook { .. } => {
                self.webhook
                    .run(hook, pm_paths, session_paths, result)
                    .await
            }
        }
    }
}

#[derive(Clone)]
struct WebhookHookRunner {
    client: reqwest::Client,
}

impl WebhookHookRunner {
    fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self { client })
    }
}

#[async_trait::async_trait]
impl HookRunner for WebhookHookRunner {
    async fn run(
        &self,
        hook: &HookSpec,
        pm_paths: &PmPaths,
        session_paths: &pm_core::SessionPaths,
        result: &pm_core::RunResult,
    ) -> anyhow::Result<()> {
        let url = match hook {
            HookSpec::Webhook { url } => url.as_str(),
            HookSpec::Command { .. } => {
                anyhow::bail!("unsupported hook spec: command (expected webhook hook)")
            }
        };

        let session = &result.session;
        let pm_session_dir = pm_paths.session_dir(session.id);
        let tmp_session_dir = session_paths.root();
        let result_json = tmp_session_dir.join("result.json");

        let payload = serde_json::json!({
            "session_id": session.id.to_string(),
            "repo": session.repo.as_str(),
            "pr_name": session.pr_name.as_str(),
            "base_branch": session.base_branch.as_str(),
            "pm_root": pm_paths.root().display().to_string(),
            "session_dir": pm_session_dir.display().to_string(),
            "tmp_dir": tmp_session_dir.display().to_string(),
            "result_json": result_json.display().to_string(),
            "merged": result.merge.merged,
            "merge_error": result.merge.error.as_deref(),
        });

        let response = self.client.post(url).json(&payload).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("webhook responded with status {status}: {text}");
        }
        Ok(())
    }
}

fn validate_strict_run_result(result: &pm_core::RunResult) -> anyhow::Result<()> {
    let failed: Vec<String> = result
        .prs
        .iter()
        .filter(|pr| matches!(pr.status, pm_core::PullRequestStatus::Failed))
        .map(|pr| pr.id.as_str().to_string())
        .collect();
    if !failed.is_empty() {
        anyhow::bail!(
            "session {} had failed tasks: {}",
            result.session.id,
            failed.join(", ")
        );
    }
    if let Some(error) = result.merge.error.as_deref() {
        anyhow::bail!("session {} merge failed: {}", result.session.id, error);
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct TaskInput {
    id: Option<String>,
    title: String,
    description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum TasksFile {
    List(Vec<TaskInput>),
    Object { tasks: Vec<TaskInput> },
}

async fn parse_tasks_override(args: &RunArgs) -> anyhow::Result<Option<Vec<pm_core::TaskSpec>>> {
    if args.tasks_file.is_some() && !args.task.is_empty() {
        anyhow::bail!("use only one of --tasks-file or --task");
    }

    let override_requested = args.tasks_file.is_some() || !args.task.is_empty();
    if !override_requested {
        return Ok(None);
    }

    let tasks = if let Some(path) = &args.tasks_file {
        let text = tokio::fs::read_to_string(path).await?;
        let parsed: TasksFile = serde_json::from_str(&text)?;
        match parsed {
            TasksFile::List(tasks) => tasks,
            TasksFile::Object { tasks } => tasks,
        }
    } else if !args.task.is_empty() {
        args.task
            .iter()
            .enumerate()
            .map(|(index, raw)| task_input_from_arg(raw, index))
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    if tasks.is_empty() {
        anyhow::bail!("tasks override provided but empty");
    }

    let mut seen: HashSet<pm_core::TaskId> = HashSet::new();
    let mut specs = Vec::with_capacity(tasks.len());
    for (index, task) in tasks.into_iter().enumerate() {
        let fallback = format!("t{}", index + 1);
        let id_raw = match task.id {
            Some(id) => {
                let trimmed = id.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("task id must not be empty (task index: {})", index + 1);
                }
                trimmed.to_string()
            }
            None => fallback,
        };
        let id = pm_core::TaskId::sanitize(&id_raw);

        if !seen.insert(id.clone()) {
            anyhow::bail!("duplicate task id: {}", id.as_str());
        }

        let title = task.title.trim().to_string();
        if title.is_empty() {
            anyhow::bail!("task title must not be empty (task id: {})", id.as_str());
        }

        specs.push(pm_core::TaskSpec {
            id,
            title,
            description: task.description.and_then(|d| {
                let trimmed = d.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }),
        });
    }

    Ok(Some(specs))
}

fn task_input_from_arg(raw: &str, index: usize) -> anyhow::Result<TaskInput> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("--task value must not be empty");
    }

    let (id, title) = match raw.split_once(':') {
        Some((id, title)) => {
            let id = id.trim();
            if id.is_empty() {
                anyhow::bail!("--task id must not be empty");
            }
            (Some(id.to_string()), title.trim().to_string())
        }
        None => (Some(format!("t{}", index + 1)), raw.to_string()),
    };

    Ok(TaskInput {
        id,
        title,
        description: None,
    })
}

struct TemplateArchitect;

#[async_trait::async_trait]
impl Architect for TemplateArchitect {
    async fn split(&self, session: &pm_core::Session) -> anyhow::Result<Vec<pm_core::TaskSpec>> {
        Ok(vec![pm_core::TaskSpec {
            id: pm_core::TaskId::sanitize("main"),
            title: format!("Implement {}", session.pr_name.as_str()),
            description: Some("Phase 1: template single-task split".to_string()),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::Router;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use pm_core::Storage;
    use serde_json::Value;
    use time::OffsetDateTime;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn list_sessions_returns_sorted_unique_ids() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
        let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

        storage
            .put_json(
                &format!("sessions/{id2}/tasks"),
                &serde_json::json!([{"id":"t1","title":"x"}]),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id1}/session"),
                &serde_json::json!({"ok": true}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id1}/merge"),
                &serde_json::json!({"merged": false}),
            )
            .await?;

        assert_eq!(list_sessions(&storage).await?, vec![id1, id2]);
        Ok(())
    }

    fn make_run_result(
        prs: Vec<pm_core::PullRequest>,
        merge: pm_core::MergeResult,
    ) -> pm_core::RunResult {
        let session_id: SessionId = "00000000-0000-0000-0000-000000000123"
            .parse()
            .expect("valid uuid");
        pm_core::RunResult {
            session: pm_core::Session {
                id: session_id,
                repo: pm_core::RepositoryName::sanitize("repo"),
                pr_name: pm_core::PrName::sanitize("demo"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
            },
            tasks: Vec::new(),
            prs,
            merge,
        }
    }

    #[test]
    fn strict_validation_allows_no_changes_sessions() {
        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::NoChanges,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: false,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: pm_core::CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        );
        assert!(validate_strict_run_result(&result).is_ok());
    }

    #[test]
    fn strict_validation_fails_on_task_failure() {
        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::Failed,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: false,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: pm_core::CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        );
        assert!(validate_strict_run_result(&result).is_err());
    }

    #[test]
    fn strict_validation_fails_on_merge_error() {
        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::Ready,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: false,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: pm_core::CheckSummary::default(),
                error: Some("boom".to_string()),
                error_log_path: None,
            },
        );
        assert!(validate_strict_run_result(&result).is_err());
    }

    #[test]
    fn resolve_stream_events_mode_rejects_conflicts() {
        assert!(resolve_stream_events_mode(true, true).is_err());
    }

    #[test]
    fn cli_rejects_zero_max_concurrency() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--max-concurrency",
            "0",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_run_parses_cargo_test_flag() {
        let cli = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--cargo-test",
        ])
        .unwrap();

        let Command::Run(args) = cli.command else {
            panic!("expected run subcommand");
        };
        assert!(args.cargo_test);
    }

    #[test]
    fn resolve_run_repo_errors_when_missing_flags_outside_git_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some("x".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = resolve_run_repo(repo_root, &args).unwrap_err();
        assert!(err.to_string().contains("missing --repo or --repo-src"));
    }

    #[test]
    fn resolve_run_repo_defaults_to_repo_root_inside_git_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some("x".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let ResolvedRunRepo::Inject { repo_name, source } =
            resolve_run_repo(repo_root, &args).expect("resolve")
        else {
            panic!("expected inject");
        };
        assert_eq!(source, repo_root.to_string_lossy());
        let expected = sanitize_repo_name_input(
            repo_root
                .file_name()
                .and_then(|name| name.to_str())
                .expect("tmp dir has file name"),
        );
        assert_eq!(repo_name, expected);
    }

    #[test]
    fn resolve_run_repo_strips_dot_git_suffix_from_repo_root_dir_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path().join("demo.git");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some("x".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let ResolvedRunRepo::Inject { repo_name, source } =
            resolve_run_repo(&repo_root, &args).expect("resolve")
        else {
            panic!("expected inject");
        };
        assert_eq!(source, repo_root.to_string_lossy());
        assert_eq!(repo_name.as_str(), "demo");
    }

    #[test]
    fn cli_rejects_auto_tasks_with_task_override() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--auto-tasks",
            "--task",
            "t1:foo",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn cli_rejects_auto_tasks_with_tasks_file_override() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--auto-tasks",
            "--tasks-file",
            "tasks.json",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn cli_rejects_empty_pr_name() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            " ",
            "--prompt",
            "x",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_base_branch() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--base",
            " ",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_repo_name() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            " ",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_repo_source() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo-src",
            " ",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_inject_source() {
        let err = Cli::try_parse_from(["code-pm", "repo", "inject", " "])
            .err()
            .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_inject_name() {
        let err = Cli::try_parse_from(["code-pm", "repo", "inject", "src", "--name", " "])
            .err()
            .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_repo_list_parses_json_and_verbose_flags() {
        let cli = Cli::try_parse_from(["code-pm", "repo", "list", "--json", "--verbose"]).unwrap();

        let Command::Repo { command } = cli.command else {
            panic!("expected repo subcommand");
        };
        let RepoCommand::List(args) = command else {
            panic!("expected repo list");
        };
        assert!(args.json);
        assert!(args.verbose);
    }

    #[test]
    fn cli_init_parses_json_flag() {
        let cli = Cli::try_parse_from(["code-pm", "init", "--json"]).unwrap();

        let Command::Init(args) = cli.command else {
            panic!("expected init subcommand");
        };
        assert!(args.json);
    }

    #[test]
    fn cli_repo_inject_parses_json_flag() {
        let cli = Cli::try_parse_from(["code-pm", "repo", "inject", "src", "--json"]).unwrap();

        let Command::Repo { command } = cli.command else {
            panic!("expected repo subcommand");
        };
        let RepoCommand::Inject { json, .. } = command else {
            panic!("expected repo inject");
        };
        assert!(json);
    }

    #[test]
    fn sanitize_repo_name_input_strips_dot_git_suffix() {
        assert_eq!(sanitize_repo_name_input("demo.git").as_str(), "demo");
        assert_eq!(sanitize_repo_name_input("demo.git/").as_str(), "demo");
        assert_eq!(sanitize_repo_name_input(" demo.git/ ").as_str(), "demo");
        assert_eq!(sanitize_repo_name_input("demo").as_str(), "demo");
    }

    #[tokio::test]
    async fn read_prompt_rejects_blank_prompt_arg() {
        let args = RunArgs {
            repo: Some("repo".to_string()),
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some(" \n\t".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = read_prompt(&args).await.unwrap_err();
        assert!(err.to_string().contains("--prompt must not be empty"));
    }

    #[tokio::test]
    async fn read_prompt_rejects_blank_prompt_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("prompt.txt");
        tokio::fs::write(&path, " \n\t").await?;

        let args = RunArgs {
            repo: Some("repo".to_string()),
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: None,
            prompt_file: Some(path),
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = read_prompt(&args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("--prompt-file content must not be empty")
        );
        Ok(())
    }

    #[tokio::test]
    async fn webhook_hook_runner_posts_expected_payload() -> anyhow::Result<()> {
        #[derive(Clone)]
        struct Capture {
            payload: Arc<Mutex<Option<Value>>>,
        }

        async fn handler(State(state): State<Capture>, Json(payload): Json<Value>) -> StatusCode {
            *state.payload.lock().await = Some(payload);
            StatusCode::NO_CONTENT
        }

        let captured = Arc::new(Mutex::new(None));
        let state = Capture {
            payload: Arc::clone(&captured),
        };

        let app = Router::new()
            .route("/hook", post(handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));

        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::Ready,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: true,
                base_branch: "main".to_string(),
                merge_commit: Some("deadbeef".to_string()),
                merged_prs: vec![pm_core::TaskId::sanitize("t1")],
                checks: pm_core::CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        );
        let repo = pm_core::RepositoryName::sanitize("repo");
        let session_paths =
            pm_core::SessionPaths::new_in(tmp.path().join("tmp"), &repo, result.session.id);

        let hook = HookSpec::Webhook {
            url: format!("http://{addr}/hook"),
        };
        let runner = WebhookHookRunner::new()?;
        runner
            .run(&hook, &pm_paths, &session_paths, &result)
            .await?;

        let payload = captured
            .lock()
            .await
            .take()
            .expect("webhook handler must capture payload");

        assert_eq!(payload["session_id"], result.session.id.to_string());
        assert_eq!(payload["repo"], result.session.repo.as_str());
        assert_eq!(payload["pr_name"], result.session.pr_name.as_str());
        assert_eq!(payload["base_branch"], result.session.base_branch.as_str());
        assert_eq!(payload["merged"], result.merge.merged);
        assert_eq!(payload["merge_error"], Value::Null);

        assert_eq!(payload["pm_root"], pm_paths.root().display().to_string());
        assert_eq!(
            payload["session_dir"],
            pm_paths
                .session_dir(result.session.id)
                .display()
                .to_string()
        );
        assert_eq!(
            payload["tmp_dir"],
            session_paths.root().display().to_string()
        );
        assert_eq!(
            payload["result_json"],
            session_paths
                .root()
                .join("result.json")
                .display()
                .to_string()
        );

        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn show_session_prefers_result_by_default() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id: SessionId = "00000000-0000-0000-0000-000000000123".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id, "stage": "session"}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/result"),
                &serde_json::json!({"id": id, "stage": "result"}),
            )
            .await?;

        let json = show_session_json(&storage, id, false).await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(value["result"]["stage"], "result");
        assert!(value.get("session").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn show_session_falls_back_when_result_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id: SessionId = "00000000-0000-0000-0000-000000000456".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id, "stage": "session"}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/tasks"),
                &serde_json::json!([{"id":"t1","title":"x"}]),
            )
            .await?;

        let json = show_session_json(&storage, id, false).await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(value["session"]["stage"], "session");
        assert_eq!(value["tasks"][0]["id"], "t1");
        assert!(value.get("result").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn show_session_all_includes_all_keys() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id: SessionId = "00000000-0000-0000-0000-000000000789".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/tasks"),
                &serde_json::json!([{"id":"t1"}]),
            )
            .await?;
        storage
            .put_json(&format!("sessions/{id}/prs"), &serde_json::json!([]))
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/merge"),
                &serde_json::json!({"merged": true}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/result"),
                &serde_json::json!({"id": id}),
            )
            .await?;

        let json = show_session_json(&storage, id, true).await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        for key in ["session", "tasks", "prs", "merge", "result"] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
        Ok(())
    }

    #[test]
    fn resolve_pm_root_defaults_to_repo_root_dot_code_pm() {
        let repo_root = PathBuf::from("/repo");
        assert_eq!(
            resolve_pm_root(&repo_root, None, None),
            (repo_root.join(".code_pm"), PmRootSource::Default)
        );
    }

    #[test]
    fn resolve_pm_root_prefers_cli_override() {
        let repo_root = PathBuf::from("/repo");
        let cli = PathBuf::from("cli-root");
        let env = std::ffi::OsString::from("env-root");
        assert_eq!(
            resolve_pm_root(&repo_root, Some(&cli), Some(env.as_os_str())),
            (repo_root.join("cli-root"), PmRootSource::Override)
        );
    }

    #[test]
    fn resolve_pm_root_resolves_relative_env_to_repo_root() {
        let repo_root = PathBuf::from("/repo");
        let env = std::ffi::OsString::from("state");
        assert_eq!(
            resolve_pm_root(&repo_root, None, Some(env.as_os_str())),
            (repo_root.join("state"), PmRootSource::Override)
        );
    }

    #[tokio::test]
    async fn parse_tasks_override_rejects_empty_id_in_tasks_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("tasks.json");
        tokio::fs::write(&path, r#"[{"id":"","title":"x"}]"#).await?;

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: None,
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: Some(path),
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = parse_tasks_override(&args).await.unwrap_err();
        assert!(err.to_string().contains("task id must not be empty"));
        Ok(())
    }

    #[tokio::test]
    async fn parse_tasks_override_rejects_empty_id_in_task_arg() -> anyhow::Result<()> {
        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: None,
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            auto_tasks: false,
            tasks_file: None,
            task: vec![":x".to_string()],
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = parse_tasks_override(&args).await.unwrap_err();
        assert!(err.to_string().contains("--task id must not be empty"));
        Ok(())
    }

    #[test]
    fn legacy_pm_root_warning_emits_notice_for_default_root_when_legacy_dir_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".codex_pm")).expect("create legacy dir");

        let (pm_root, source) = resolve_pm_root(repo_root, None, None);
        assert_eq!(source, PmRootSource::Default);
        assert!(legacy_pm_root_warning(repo_root, &pm_root, source).is_some());
    }

    #[test]
    fn legacy_pm_root_warning_skips_when_new_dir_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".codex_pm")).expect("create legacy dir");
        std::fs::create_dir_all(repo_root.join(".code_pm")).expect("create new dir");

        let (pm_root, source) = resolve_pm_root(repo_root, None, None);
        assert_eq!(source, PmRootSource::Default);
        assert!(legacy_pm_root_warning(repo_root, &pm_root, source).is_none());
    }

    #[test]
    fn legacy_pm_root_warning_skips_when_override_root_used() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".codex_pm")).expect("create legacy dir");

        let override_root = repo_root.join("custom-root");
        let (pm_root, source) = resolve_pm_root(repo_root, Some(&override_root), None);
        assert_eq!(source, PmRootSource::Override);
        assert!(legacy_pm_root_warning(repo_root, &pm_root, source).is_none());
    }
}
