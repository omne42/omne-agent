use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

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
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
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

#[derive(Subcommand)]
enum RepoCommand {
    Inject {
        source: String,
        #[arg(long)]
        name: Option<String>,
    },
    List,
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
}

#[derive(Parser, Clone)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:9417")]
    addr: std::net::SocketAddr,
}

#[derive(Parser, Clone)]
struct RunArgs {
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    repo_src: Option<String>,
    #[arg(long)]
    pr_name: String,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    prompt_file: Option<PathBuf>,
    #[arg(long, default_value = "main")]
    base: String,
    #[arg(long)]
    apply_patch: Option<PathBuf>,
    #[arg(long, default_value_t = 1)]
    max_concurrency: usize,
    #[arg(long, default_value_t = false)]
    stream_events: bool,
    #[arg(long, default_value_t = false)]
    strict: bool,
    #[arg(long, default_value_t = false)]
    json: bool,
    #[arg(long, default_value_t = false)]
    auto_tasks: bool,
    #[arg(long)]
    tasks_file: Option<PathBuf>,
    #[arg(long)]
    task: Vec<String>,
    #[arg(long)]
    hook_cmd: Option<PathBuf>,
    #[arg(long)]
    hook_arg: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;

    let pm_root = repo_root.join(".code_pm");
    let pm_paths = PmPaths::new(pm_root.clone());
    let storage = FsStorage::new(pm_paths.data_dir());

    let repo_manager = RepoManager::new(pm_paths.clone());

    match cli.command {
        Command::Init => {
            repo_manager.ensure_layout().await?;
            println!("{}", pm_paths.root().display());
        }
        Command::Repo { command } => match command {
            RepoCommand::Inject { source, name } => {
                let repo_name = name
                    .as_deref()
                    .map(pm_core::RepositoryName::sanitize)
                    .unwrap_or_else(|| RepoManager::default_repo_name_from_source(&source));
                let repo = repo_manager.inject(&repo_name, &source).await?;
                println!("repo: {}", repo.name.as_str());
                println!("bare: {}", repo.bare_path.display());
            }
            RepoCommand::List => {
                for name in repo_manager.list_repos().await? {
                    println!("{}", name.as_str());
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
                } else {
                    let sessions = list_sessions(&storage).await?;
                    let sessions = match args.limit {
                        Some(limit) => sessions.into_iter().take(limit).collect::<Vec<_>>(),
                        None => sessions,
                    };
                    for id in sessions {
                        println!("{id}");
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
            run_session(repo_manager, storage, *args).await?;
        }
    }

    Ok(())
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

async fn run_session(
    repo_manager: RepoManager,
    storage: FsStorage,
    args: RunArgs,
) -> anyhow::Result<()> {
    let prompt = match (&args.prompt, &args.prompt_file) {
        (Some(text), None) => text.clone(),
        (None, Some(path)) => tokio::fs::read_to_string(path).await?,
        (None, None) => anyhow::bail!("missing --prompt or --prompt-file"),
        (Some(_), Some(_)) => anyhow::bail!("use only one of --prompt or --prompt-file"),
    };

    let pr_name = PrName::sanitize(&args.pr_name);
    let tasks = parse_tasks_override(&args).await?;

    let (repo_name, repo) = match (&args.repo, &args.repo_src) {
        (Some(name), None) => {
            let repo_name = pm_core::RepositoryName::sanitize(name);
            let repo = repo_manager.load(&repo_name).await?;
            (repo_name, repo)
        }
        (maybe_name, Some(source)) => {
            let repo_name = maybe_name
                .as_deref()
                .map(pm_core::RepositoryName::sanitize)
                .unwrap_or_else(|| RepoManager::default_repo_name_from_source(source));
            let repo = repo_manager.inject(&repo_name, source).await?;
            (repo_name, repo)
        }
        (None, None) => anyhow::bail!("missing --repo or --repo-src"),
    };

    let hook = args.hook_cmd.map(|program| HookSpec::Command {
        program,
        args: args.hook_arg,
    });
    let hook_runner: Arc<dyn HookRunner> = Arc::new(CommandHookRunner);

    let architect: Arc<dyn Architect> = if args.auto_tasks {
        Arc::new(RuleBasedArchitect::default())
    } else {
        Arc::new(TemplateArchitect)
    };
    let coder: Arc<dyn pm_core::Coder> = Arc::new(pm_git::GitCoder::default());
    let merger: Arc<dyn pm_core::Merger> = Arc::new(pm_git::GitMerger::default());

    let events = EventBus::default();
    let printer = if args.stream_events {
        let mut rx = events.subscribe();
        Some(tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => eprintln!("[event] {event}"),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
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
            max_concurrency: args.max_concurrency,
            tasks,
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
        let id_raw = task.id.unwrap_or(fallback);
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
        Some((id, title)) => (Some(id.trim().to_string()), title.trim().to_string()),
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
    use pm_core::Storage;
    use time::OffsetDateTime;

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
}
