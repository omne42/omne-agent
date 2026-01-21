#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;

    let env_pm_root = std::env::var_os("CODE_PM_ROOT");
    let (pm_root, pm_root_source) = resolve_pm_root(
        &repo_root.root,
        cli.pm_root.as_deref(),
        env_pm_root.as_deref(),
    );
    if let Some(note) = legacy_pm_root_warning(&repo_root.root, &pm_root, pm_root_source) {
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

