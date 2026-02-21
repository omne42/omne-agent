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
        _ => (repo_root.join(".omne"), PmRootSource::Default),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PmRootSource {
    Default,
    Override,
}

fn legacy_pm_root_warning(
    repo_root: &std::path::Path,
    omne_root: &std::path::Path,
    source: PmRootSource,
) -> Option<String> {
    if source != PmRootSource::Default {
        return None;
    }

    let legacy = repo_root.join(".codex_omne");
    if legacy.is_dir() && !omne_root.is_dir() {
        Some(format!(
            "warning: found legacy omne root `{}` but current default is `{}`; to reuse old data: `omne --omne-root .codex_omne ...` or `mv .codex_omne .omne`",
            legacy.display(),
            omne_root.display()
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
    repo_root: &RepoRoot,
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
    let coder: Arc<dyn omne_core::Coder> = Arc::new(omne_git::GitCoder::default());
    let merger: Arc<dyn omne_core::Merger> = Arc::new(omne_git::GitMerger::default());

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

        let request = omne_core::RunRequest {
            pr_name,
            prompt,
            base_branch: args.base,
            apply_patch: args.apply_patch,
            hook,
            max_concurrency: args.max_concurrency as usize,
            tasks,
            cargo_test: args.cargo_test,
            auto_merge: !args.no_merge,
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

fn sanitize_repo_name_input(value: &str) -> omne_core::RepositoryName {
    let value = value.trim();
    let value = value.trim_end_matches(['/', '\\']);
    let value = value.strip_suffix(".git").unwrap_or(value);
    omne_core::RepositoryName::sanitize(value)
}

#[derive(Debug)]
enum ResolvedRunRepo {
    Load(omne_core::RepositoryName),
    Inject {
        repo_name: omne_core::RepositoryName,
        source: String,
    },
}

fn resolve_run_repo(repo_root: &RepoRoot, args: &RunArgs) -> anyhow::Result<ResolvedRunRepo> {
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
            if !repo_root.is_git_repo {
                anyhow::bail!("missing --repo or --repo-src");
            }
            let repo_name = repo_root
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_repo_name_input)
                .unwrap_or_else(|| omne_core::RepositoryName::sanitize("repo"));
            let source = repo_root.root.to_string_lossy().to_string();
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
        omne_paths: &PmPaths,
        session_paths: &omne_core::SessionPaths,
        result: &omne_core::RunResult,
    ) -> anyhow::Result<()> {
        match hook {
            HookSpec::Command { .. } => {
                self.command
                    .run(hook, omne_paths, session_paths, result)
                    .await
            }
            HookSpec::Webhook { .. } => {
                self.webhook
                    .run(hook, omne_paths, session_paths, result)
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
        omne_paths: &PmPaths,
        session_paths: &omne_core::SessionPaths,
        result: &omne_core::RunResult,
    ) -> anyhow::Result<()> {
        let url = match hook {
            HookSpec::Webhook { url } => url.as_str(),
            HookSpec::Command { .. } => {
                anyhow::bail!("unsupported hook spec: command (expected webhook hook)")
            }
        };

        let session = &result.session;
        let omne_session_dir = omne_paths.session_dir(session.id);
        let tmp_session_dir = session_paths.root();
        let result_json = tmp_session_dir.join("result.json");

        let payload = serde_json::json!({
            "session_id": session.id.to_string(),
            "repo": session.repo.as_str(),
            "pr_name": session.pr_name.as_str(),
            "base_branch": session.base_branch.as_str(),
            "omne_root": omne_paths.root().display().to_string(),
            "session_dir": omne_session_dir.display().to_string(),
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

