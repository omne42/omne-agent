#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let mut cli = Cli::parse();
    if let Some(command) = take_preconnect_command(&mut cli) {
        match command {
            PreConnectCommand::Init(args) => return run_init(args).await,
            PreConnectCommand::ToolchainBootstrap(args) => {
                return run_toolchain_bootstrap(args).await;
            }
            PreConnectCommand::Reference(command) => return run_reference(&cli, command).await,
            PreConnectCommand::PresetList { json } => return run_preset_list(&cli, json).await,
            PreConnectCommand::PresetShow { file, name, json } => {
                return run_preset_show(&cli, file, name, json).await;
            }
            PreConnectCommand::PresetValidate {
                file,
                name,
                strict,
                json,
            } => {
                return run_preset_validate(&cli, file, name, strict, json).await;
            }
            PreConnectCommand::CommandList { json } => return run_command_list(&cli, json).await,
            PreConnectCommand::CommandShow { name, json } => {
                return run_command_show(&cli, &name, json).await;
            }
            PreConnectCommand::CommandValidate { name, strict, json } => {
                return run_command_validate(&cli, name, strict, json).await;
            }
        }
    }

    let mut app = App::connect(&cli).await?;

    match cli.command {
        None => {
            run_tui(
                &mut app,
                TuiArgs {
                    thread_id: None,
                    include_archived: false,
                },
            )
            .await?;
        }
        Some(Command::Cli) => run_repl(&mut app).await?,
        Some(Command::Init(_)) => unreachable!("handled before App::connect"),
        Some(Command::Toolchain { .. }) => unreachable!("handled before App::connect"),
        Some(Command::Reference { .. }) => unreachable!("handled before App::connect"),
        Some(Command::Preset { ref command }) => {
            run_preset(&cli, &mut app, command.clone()).await?;
        }
        Some(Command::Workflow { ref command }) => {
            if let Err(err) = run_command_run(&cli, &mut app, command).await {
                if let Some(error_code) = command_run_error_code(&err) {
                    eprintln!("[command/run error_code] {error_code}");
                }
                return Err(err);
            }
        }
        Some(Command::Repo { command }) => match command {
            RepoCommand::Search {
                thread_id,
                query,
                regex,
                include_glob,
                max_matches,
                max_bytes_per_file,
                max_files,
                root,
                approval_id,
                json,
            } => {
                let root = root.map(RepoRoot::to_file_root);
                let result = app
                    .repo_search(omne_app_server_protocol::RepoSearchParams {
                        thread_id,
                        turn_id: None,
                        approval_id,
                        root,
                        query,
                        is_regex: regex,
                        include_glob,
                        max_matches,
                        max_bytes_per_file,
                        max_files,
                    })
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize repo/search response")?;
                print_json_or_pretty(json, &result)?;
            }
            RepoCommand::Index {
                thread_id,
                include_glob,
                max_files,
                root,
                approval_id,
                json,
            } => {
                let root = root.map(RepoRoot::to_file_root);
                let result = app
                    .repo_index(omne_app_server_protocol::RepoIndexParams {
                        thread_id,
                        turn_id: None,
                        approval_id,
                        root,
                        include_glob,
                        max_files,
                    })
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize repo/index response")?;
                print_json_or_pretty(json, &result)?;
            }
            RepoCommand::Symbols {
                thread_id,
                include_glob,
                max_files,
                max_bytes_per_file,
                max_symbols,
                root,
                approval_id,
                json,
            } => {
                let root = root.map(RepoRoot::to_file_root);
                let result = app
                    .repo_symbols(omne_app_server_protocol::RepoSymbolsParams {
                        thread_id,
                        turn_id: None,
                        approval_id,
                        root,
                        include_glob,
                        max_files,
                        max_bytes_per_file,
                        max_symbols,
                    })
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize repo/symbols response")?;
                print_json_or_pretty(json, &result)?;
            }
        },
        Some(Command::Mcp { command }) => match command {
            McpCommand::Serve(args) => {
                run_mcp_serve(&mut app, args).await?;
            }
            McpCommand::ListServers {
                thread_id,
                approval_id,
                json,
            } => {
                let result = app
                    .mcp_list_servers(omne_app_server_protocol::McpListServersParams {
                        thread_id,
                        turn_id: None,
                        approval_id,
                    })
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize mcp/list_servers response")?;
                print_json_or_pretty(json, &result)?;
            }
            McpCommand::ListTools {
                thread_id,
                server,
                approval_id,
                json,
            } => {
                let result = app
                    .mcp_list_tools(omne_app_server_protocol::McpListToolsParams {
                        thread_id,
                        turn_id: None,
                        server,
                        approval_id,
                    })
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize mcp/list_tools response")?;
                print_json_or_pretty(json, &result)?;
            }
            McpCommand::ListResources {
                thread_id,
                server,
                approval_id,
                json,
            } => {
                let result = app
                    .mcp_list_resources(omne_app_server_protocol::McpListResourcesParams {
                        thread_id,
                        turn_id: None,
                        server,
                        approval_id,
                    })
                    .await?;
                let result = serde_json::to_value(result)
                    .context("serialize mcp/list_resources response")?;
                print_json_or_pretty(json, &result)?;
            }
            McpCommand::Call {
                thread_id,
                server,
                tool,
                arguments_json,
                approval_id,
                json,
            } => {
                let arguments = match arguments_json {
                    Some(raw) => {
                        Some(serde_json::from_str(&raw).context("parse --arguments-json as JSON")?)
                    }
                    None => None,
                };
                let result = app
                    .mcp_call(omne_app_server_protocol::McpCallParams {
                        thread_id,
                        turn_id: None,
                        server,
                        tool,
                        arguments,
                        approval_id,
                    })
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize mcp/call response")?;
                print_json_or_pretty(json, &result)?;
            }
        },
        Some(Command::Tui(args)) => {
            run_tui(&mut app, args).await?;
        }
        Some(Command::Thread { command }) => match command {
            ThreadCommand::Start { cwd, json } => {
                let cwd = cwd.map(|p| p.display().to_string());
                let result = app.thread_start(cwd).await?;
                ensure_thread_start_auto_hook_ready("thread/start", &result)?;
                let result =
                    serde_json::to_value(result).context("serialize thread/start response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Resume { thread_id, json } => {
                let result = app.thread_resume(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/resume response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Fork { thread_id, json } => {
                let result = app.thread_fork(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/fork response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Spawn {
                thread_id,
                input,
                model,
                openai_base_url,
                json,
            } => {
                let result = app
                    .thread_spawn(thread_id, input, model, openai_base_url)
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/spawn response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Archive {
                thread_id,
                force,
                reason,
            } => {
                let result = app.thread_archive(thread_id, force, reason).await?;
                if let Some(auto_hook) = &result.auto_hook {
                    ensure_auto_hook_ready("thread/archive", "thread/archive auto hook", auto_hook)?;
                }
                let result =
                    serde_json::to_value(result).context("serialize thread/archive response")?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Unarchive { thread_id, reason } => {
                let result = app.thread_unarchive(thread_id, reason).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/unarchive response")?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Pause { thread_id, reason } => {
                let result = app.thread_pause(thread_id, reason).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/pause response")?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Unpause { thread_id, reason } => {
                let result = app.thread_unpause(thread_id, reason).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/unpause response")?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Delete {
                thread_id,
                force,
                json,
            } => {
                let result = app.thread_delete(thread_id, force).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/delete response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::ClearArtifacts {
                thread_id,
                force,
                json,
            } => {
                let result = app.thread_clear_artifacts(thread_id, force).await?;
                let result = serde_json::to_value(result)
                    .context("serialize thread/clear_artifacts response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::DiskUsage { thread_id, json } => {
                let result = app.thread_disk_usage(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/disk_usage response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::DiskReport {
                thread_id,
                top_files,
                json,
            } => {
                let result = app.thread_disk_report(thread_id, top_files).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/disk_report response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Diff {
                thread_id,
                approval_id,
                max_bytes,
                wait_seconds,
                json,
            } => {
                let result = app
                    .thread_diff(thread_id, approval_id, max_bytes, wait_seconds)
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/diff response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Patch {
                thread_id,
                approval_id,
                max_bytes,
                wait_seconds,
                json,
            } => {
                let result = app
                    .thread_patch(thread_id, approval_id, max_bytes, wait_seconds)
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/patch response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::HookRun {
                thread_id,
                hook,
                approval_id,
                json,
            } => {
                let result = app.thread_hook_run(thread_id, hook, approval_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/hook_run response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Events {
                thread_id,
                since_seq,
                max_events,
                kinds,
                json,
            } => {
                let kinds = if kinds.is_empty() { None } else { Some(kinds) };
                let result = app
                    .thread_events(thread_id, since_seq, max_events, kinds)
                    .await?;
                if json {
                    println!("{}", serde_json::to_string(&result)?);
                } else {
                    let value = serde_json::to_value(result)
                        .context("serialize thread/events response")?;
                    print_json_or_pretty(false, &value)?;
                }
            }
            ThreadCommand::Loaded { json } => {
                let result = app.thread_loaded().await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/loaded response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::List { json } => {
                let result = app.thread_list().await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/list response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::ListMeta {
                include_archived,
                include_attention_markers,
                json,
            } => {
                let result = app
                    .thread_list_meta(include_archived, include_attention_markers)
                    .await?;
                if json {
                    let result = serde_json::to_value(result)
                        .context("serialize thread/list_meta response")?;
                    print_json_or_pretty(true, &result)?;
                } else {
                    print_thread_list_meta_plain(&result);
                }
            }
            ThreadCommand::Attention { thread_id, json } => {
                let result = app.thread_attention(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/attention response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::State { thread_id, json } => {
                let result = app.thread_state(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/state response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Usage { thread_id, json } => {
                let result = app.thread_usage(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/usage response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::ConfigExplain { thread_id, json } => {
                let result = app.thread_config_explain(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/config/explain response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Models { thread_id, json } => {
                let result = app.thread_models(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize thread/models response")?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Configure(args) => {
                app.thread_configure(args).await?;
            }
        },
        Some(Command::Checkpoint { command }) => match command {
            CheckpointCommand::Create {
                thread_id,
                label,
                json,
            } => {
                let result = app.checkpoint_create(thread_id, label).await?;
                let result = serde_json::to_value(result)
                    .context("serialize thread/checkpoint/create response")?;
                print_json_or_pretty(json, &result)?;
            }
            CheckpointCommand::List { thread_id, json } => {
                let result = app.checkpoint_list(thread_id).await?;
                let result = serde_json::to_value(result)
                    .context("serialize thread/checkpoint/list response")?;
                print_json_or_pretty(json, &result)?;
            }
            CheckpointCommand::Restore {
                thread_id,
                checkpoint_id,
                approval_id,
                json,
            } => {
                let result = app
                    .checkpoint_restore(thread_id, checkpoint_id, approval_id)
                    .await?;
                let result = serde_json::to_value(result)
                    .context("serialize thread/checkpoint/restore response")?;
                print_json_or_pretty(json, &result)?;
            }
        },
        Some(Command::Inbox(args)) => {
            run_inbox(&mut app, args).await?;
        }
        Some(Command::Ask(args)) => {
            run_ask(&mut app, args).await?;
        }
        Some(Command::Exec(args)) => {
            let code = run_exec(&mut app, args).await?;
            std::process::exit(code);
        }
        Some(Command::Watch(args)) => {
            run_watch(&mut app, args).await?;
        }
        Some(Command::Approval { command }) => match command {
            ApprovalCommand::List {
                thread_id,
                include_decided,
                json,
            } => {
                let result = app.approval_list(thread_id, include_decided).await?;
                let result =
                    serde_json::to_value(result).context("serialize approval/list response")?;
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
        Some(Command::Process { command }) => match command {
            ProcessCommand::List { thread_id, json } => {
                let result = app.process_list(thread_id).await?;
                let result =
                    serde_json::to_value(result).context("serialize process/list response")?;
                print_json_or_pretty(json, &result)?;
            }
            ProcessCommand::Inspect {
                process_id,
                max_lines,
                approval_id,
                json,
            } => {
                let result = app
                    .process_inspect(process_id, max_lines, approval_id)
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize process/inspect response")?;
                print_json_or_pretty(json, &result)?;
            }
            ProcessCommand::Tail {
                process_id,
                stderr,
                max_lines,
                approval_id,
            } => {
                let text = app
                    .process_tail(process_id, stderr, max_lines, approval_id)
                    .await?;
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
                approval_id,
            } => {
                run_process_follow(
                    &mut app,
                    process_id,
                    stderr,
                    since_offset,
                    max_bytes,
                    poll_ms,
                    approval_id,
                )
                .await?;
            }
            ProcessCommand::Interrupt {
                process_id,
                reason,
                approval_id,
            } => {
                app.process_interrupt(process_id, reason, approval_id)
                    .await?;
            }
            ProcessCommand::Kill {
                process_id,
                reason,
                approval_id,
            } => {
                app.process_kill(process_id, reason, approval_id).await?;
            }
        },
        Some(Command::Artifact { command }) => match command {
            ArtifactCommand::List {
                thread_id,
                approval_id,
                json,
            } => {
                let result = app.artifact_list(thread_id, approval_id).await?;
                if json {
                    let result =
                        serde_json::to_value(result).context("serialize artifact/list response")?;
                    print_json_or_pretty(true, &result)?;
                } else {
                    print_artifact_list_plain(&result);
                }
            }
            ArtifactCommand::Read {
                thread_id,
                artifact_id,
                version,
                max_bytes,
                approval_id,
                json,
            } => {
                let result = app
                    .artifact_read(thread_id, artifact_id, version, max_bytes, approval_id)
                    .await?;
                if json {
                    let result =
                        serde_json::to_value(result).context("serialize artifact/read response")?;
                    print_json_or_pretty(true, &result)?;
                } else {
                    let metadata_notice = artifact_read_metadata_notice(&result);
                    let fan_in_summary_notice = artifact_read_fan_in_summary_notice(&result);
                    let fan_out_linkage_issue_notice =
                        artifact_read_fan_out_linkage_issue_notice(&result);
                    let fan_out_linkage_issue_clear_notice =
                        artifact_read_fan_out_linkage_issue_clear_notice(&result);
                    let fan_out_result_notice = artifact_read_fan_out_result_notice(&result);
                    print!("{}", result.text);
                    if !result.text.ends_with('\n') {
                        println!();
                    }
                    if result.truncated {
                        eprintln!("[truncated]");
                    }
                    if let Some(metadata_notice) = metadata_notice {
                        eprintln!("[{metadata_notice}]");
                    }
                    if let Some(fan_in_summary_notice) = fan_in_summary_notice {
                        eprintln!("[{fan_in_summary_notice}]");
                    }
                    if let Some(fan_out_linkage_issue_notice) = fan_out_linkage_issue_notice {
                        eprintln!("[{fan_out_linkage_issue_notice}]");
                    }
                    if let Some(fan_out_linkage_issue_clear_notice) =
                        fan_out_linkage_issue_clear_notice
                    {
                        eprintln!("[{fan_out_linkage_issue_clear_notice}]");
                    }
                    if let Some(fan_out_result_notice) = fan_out_result_notice {
                        eprintln!("[{fan_out_result_notice}]");
                    }
                }
            }
            ArtifactCommand::Versions {
                thread_id,
                artifact_id,
                approval_id,
                json,
            } => {
                let result = app
                    .artifact_versions(thread_id, artifact_id, approval_id)
                    .await?;
                if json {
                    let result = serde_json::to_value(result)
                        .context("serialize artifact/versions response")?;
                    print_json_or_pretty(true, &result)?;
                } else {
                    if result.versions.is_empty() {
                        println!("(no versions)");
                    } else {
                        for version in result.versions {
                            println!("{version}");
                        }
                    }
                }
            }
            ArtifactCommand::Delete {
                thread_id,
                artifact_id,
                approval_id,
                json,
            } => {
                let result = app
                    .artifact_delete(thread_id, artifact_id, approval_id)
                    .await?;
                let result =
                    serde_json::to_value(result).context("serialize artifact/delete response")?;
                print_json_or_pretty(json, &result)?;
            }
        },
    }

    Ok(())
}

enum PreConnectCommand {
    Init(InitArgs),
    ToolchainBootstrap(ToolchainBootstrapArgs),
    Reference(ReferenceCommand),
    PresetList { json: bool },
    PresetShow {
        file: Option<PathBuf>,
        name: Option<String>,
        json: bool,
    },
    PresetValidate {
        file: Option<PathBuf>,
        name: Option<String>,
        strict: bool,
        json: bool,
    },
    CommandList { json: bool },
    CommandShow { name: String, json: bool },
    CommandValidate {
        name: Option<String>,
        strict: bool,
        json: bool,
    },
}

fn take_preconnect_command(cli: &mut Cli) -> Option<PreConnectCommand> {
    let command = cli.command.take()?;

    match command {
        Command::Init(args) => Some(PreConnectCommand::Init(args)),
        Command::Toolchain { command } => match command {
            ToolchainCommand::Bootstrap(args) => Some(PreConnectCommand::ToolchainBootstrap(args)),
        },
        Command::Reference { command } => Some(PreConnectCommand::Reference(command)),
        Command::Preset { command } => match command {
            PresetCommand::List { json } => Some(PreConnectCommand::PresetList { json }),
            PresetCommand::Show { file, name, json } => {
                Some(PreConnectCommand::PresetShow { file, name, json })
            }
            PresetCommand::Validate {
                file,
                name,
                strict,
                json,
            } => Some(PreConnectCommand::PresetValidate {
                file,
                name,
                strict,
                json,
            }),
            other => {
                cli.command = Some(Command::Preset { command: other });
                None
            }
        },
        Command::Workflow { command } => match command {
            CommandCommand::List { json } => Some(PreConnectCommand::CommandList { json }),
            CommandCommand::Show { name, json } => {
                Some(PreConnectCommand::CommandShow { name, json })
            }
            CommandCommand::Validate { name, strict, json } => {
                Some(PreConnectCommand::CommandValidate { name, strict, json })
            }
            CommandCommand::Run(args) => {
                cli.command = Some(Command::Workflow {
                    command: CommandCommand::Run(args),
                });
                None
            }
        },
        other => {
            cli.command = Some(other);
            None
        }
    }
}

fn artifact_read_metadata_notice(
    result: &omne_app_server_protocol::ArtifactReadResponse,
) -> Option<String> {
    if !result.historical {
        return None;
    }
    let metadata_source = result.metadata_source.as_str();
    let mut out = format!("metadata_source={metadata_source}");
    if let Some(reason) = result.metadata_fallback_reason {
        out.push(' ');
        out.push_str("metadata_fallback_reason=");
        out.push_str(reason.as_str());
    }
    Some(out)
}

fn artifact_read_fan_in_summary_notice(
    result: &omne_app_server_protocol::ArtifactReadResponse,
) -> Option<String> {
    let payload = result.fan_in_summary.as_ref()?;
    let pending_approvals = payload
        .tasks
        .iter()
        .filter(|task| task.pending_approval.is_some())
        .count();
    let dependency_blocked = payload
        .tasks
        .iter()
        .filter(|task| task.dependency_blocked)
        .count();
    let dependency_blocker = payload.tasks.iter().find(|task| {
        task.dependency_blocked
            || task
                .dependency_blocker_task_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
            || task
                .dependency_blocker_status
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    });
    let dependency_blocker_detail = dependency_blocker
        .map(|task| {
            let blocker_task_id = task
                .dependency_blocker_task_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("-");
            let blocker_status = task
                .dependency_blocker_status
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("-");
            format!(
                " dependency_blocker_task_id={} dependency_blocker_status={}",
                blocker_task_id, blocker_status
            )
        })
        .unwrap_or_default();
    let mut diagnostics_tasks = 0usize;
    let mut diagnostics_matched_completion_total = 0u64;
    let mut diagnostics_pending_matching_tool_ids_total = 0usize;
    let mut diagnostics_scan_last_seq_max = 0u64;
    for task in &payload.tasks {
        if let Some(diagnostics) = task.result_artifact_diagnostics.as_ref() {
            diagnostics_tasks = diagnostics_tasks.saturating_add(1);
            diagnostics_matched_completion_total = diagnostics_matched_completion_total
                .saturating_add(diagnostics.matched_completion_count);
            diagnostics_pending_matching_tool_ids_total =
                diagnostics_pending_matching_tool_ids_total
                    .saturating_add(diagnostics.pending_matching_tool_ids);
            diagnostics_scan_last_seq_max =
                diagnostics_scan_last_seq_max.max(diagnostics.scan_last_seq);
        }
    }
    let diagnostics_detail = if diagnostics_tasks == 0 {
        String::new()
    } else {
        format!(
            " diagnostics_tasks={} diagnostics_matched_completion_total={} diagnostics_pending_matching_tool_ids_total={} diagnostics_scan_last_seq_max={}",
            diagnostics_tasks,
            diagnostics_matched_completion_total,
            diagnostics_pending_matching_tool_ids_total,
            diagnostics_scan_last_seq_max
        )
    };
    Some(format!(
        "fan_in_summary schema={} tasks={} pending_approvals={} dependency_blocked={}{}{}",
        payload.schema_version,
        payload.task_count,
        pending_approvals,
        dependency_blocked,
        dependency_blocker_detail,
        diagnostics_detail
    ))
}

fn artifact_read_fan_out_linkage_issue_notice(
    result: &omne_app_server_protocol::ArtifactReadResponse,
) -> Option<String> {
    let payload = result.fan_out_linkage_issue.as_ref()?;
    Some(format_fan_out_linkage_issue_notice_from_payload(payload))
}

fn artifact_read_fan_out_linkage_issue_clear_notice(
    result: &omne_app_server_protocol::ArtifactReadResponse,
) -> Option<String> {
    let payload = result.fan_out_linkage_issue_clear.as_ref()?;
    Some(format_fan_out_linkage_issue_clear_notice_from_payload(payload))
}

fn artifact_read_fan_out_result_notice(
    result: &omne_app_server_protocol::ArtifactReadResponse,
) -> Option<String> {
    let payload = result.fan_out_result.as_ref()?;
    Some(format_fan_out_result_notice_from_payload(payload))
}

fn print_artifact_list_plain(result: &omne_app_server_protocol::ArtifactListResponse) {
    for line in artifact_list_plain_lines(result) {
        println!("{line}");
    }
    if result.errors.is_empty() {
        return;
    }

    eprintln!("[artifact/list metadata errors: {}]", result.errors.len());
    for item in result.errors.iter().take(3) {
        eprintln!("- {}: {}", item.path, item.error);
    }
    if result.errors.len() > 3 {
        eprintln!("- ... and {} more", result.errors.len() - 3);
    }
}

fn artifact_list_plain_lines(
    result: &omne_app_server_protocol::ArtifactListResponse,
) -> Vec<String> {
    if result.artifacts.is_empty() {
        return vec!["(no artifacts)".to_string()];
    }

    result
        .artifacts
        .iter()
        .map(|item| {
            format!(
                "{} v{} {} {}",
                item.artifact_id, item.version, item.artifact_type, item.summary
            )
        })
        .collect()
}

fn print_thread_list_meta_plain(result: &omne_app_server_protocol::ThreadListMetaResponse) {
    for line in thread_list_meta_plain_lines(result) {
        println!("{line}");
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct AttentionMarkerSummaryFlags {
    has_plan_ready: bool,
    has_diff_ready: bool,
    has_fan_out_linkage_issue: bool,
    has_fan_out_auto_apply_error: bool,
    has_fan_in_dependency_blocked: bool,
    has_fan_in_result_diagnostics: bool,
    has_subagent_proxy_approval: bool,
    has_token_budget_exceeded: bool,
    has_token_budget_warning: bool,
    has_test_failed: bool,
}

fn attention_marker_parts(flags: AttentionMarkerSummaryFlags) -> Vec<&'static str> {
    let mut marker_parts = Vec::new();
    if flags.has_plan_ready {
        marker_parts.push("plan_ready");
    }
    if flags.has_diff_ready {
        marker_parts.push("diff_ready");
    }
    if flags.has_fan_out_linkage_issue {
        marker_parts.push("fan_out_linkage_issue");
    }
    if flags.has_fan_out_auto_apply_error {
        marker_parts.push("fan_out_auto_apply_error");
    }
    if flags.has_fan_in_dependency_blocked {
        marker_parts.push("fan_in_dependency_blocked");
    }
    if flags.has_fan_in_result_diagnostics {
        marker_parts.push("fan_in_result_diagnostics");
    }
    if flags.has_subagent_proxy_approval {
        marker_parts.push("subagent_proxy_approval");
    }
    if flags.has_token_budget_exceeded {
        marker_parts.push("token_budget_exceeded");
    }
    if flags.has_token_budget_warning {
        marker_parts.push("token_budget_warning");
    }
    if flags.has_test_failed {
        marker_parts.push("test_failed");
    }
    marker_parts
}

fn thread_list_meta_marker_parts(
    thread: &omne_app_server_protocol::ThreadListMetaItem,
    token_budget_warning_threshold_ratio: f64,
) -> Vec<&'static str> {
    let token_budget_warning_active = thread.token_budget_warning_active.unwrap_or_else(|| {
        token_budget_warning_present(
            thread.token_budget_limit,
            thread.token_budget_utilization,
            thread.token_budget_exceeded,
            token_budget_warning_threshold_ratio,
        )
    });
    attention_marker_parts(AttentionMarkerSummaryFlags {
        has_plan_ready: thread.has_plan_ready,
        has_diff_ready: thread.has_diff_ready,
        has_fan_out_linkage_issue: thread.has_fan_out_linkage_issue,
        has_fan_out_auto_apply_error: thread.has_fan_out_auto_apply_error,
        has_fan_in_dependency_blocked: thread.has_fan_in_dependency_blocked,
        has_fan_in_result_diagnostics: thread.has_fan_in_result_diagnostics,
        has_subagent_proxy_approval: thread.pending_subagent_proxy_approvals > 0,
        has_token_budget_exceeded: thread.token_budget_exceeded.unwrap_or(false),
        has_token_budget_warning: token_budget_warning_active,
        has_test_failed: thread.has_test_failed,
    })
}

fn thread_list_meta_plain_lines(
    result: &omne_app_server_protocol::ThreadListMetaResponse,
) -> Vec<String> {
    if result.threads.is_empty() {
        return vec!["(no threads)".to_string()];
    }

    let token_budget_warning_threshold_ratio = parse_token_budget_warning_threshold_ratio_env();
    result
        .threads
        .iter()
        .map(|thread| {
            let turn = thread
                .active_turn_id
                .or(thread.last_turn_id)
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string());
            let model = thread.model.as_deref().unwrap_or("-");
            let cwd = thread.cwd.as_deref().unwrap_or("-");
            let marker_parts =
                thread_list_meta_marker_parts(thread, token_budget_warning_threshold_ratio);

            let markers = if marker_parts.is_empty() {
                "-".to_string()
            } else {
                marker_parts.join(",")
            };
            format_thread_list_meta_plain_line(
                thread,
                &turn,
                model,
                &markers,
                cwd,
            )
        })
        .collect()
}

fn format_thread_list_meta_plain_line(
    thread: &omne_app_server_protocol::ThreadListMetaItem,
    turn: &str,
    model: &str,
    markers: &str,
    cwd: &str,
) -> String {
    let subagent_pending_suffix = if thread.pending_subagent_proxy_approvals > 0 {
        format!(
            " subagent_pending={}",
            thread.pending_subagent_proxy_approvals
        )
    } else {
        String::new()
    };
    format!(
        "{} state={} seq={} turn={} model={} markers={}{} cwd={}",
        thread.thread_id,
        thread.attention_state,
        thread.last_seq,
        turn,
        model,
        markers,
        subagent_pending_suffix,
        cwd
    )
}

#[cfg(test)]
mod app_preconnect_tests {
    use super::*;

    #[test]
    fn preconnect_command_preserves_tui() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "tui"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(preconnect.is_none());
        assert!(matches!(cli.command, Some(Command::Tui(_))));
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_init() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "init"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(preconnect, Some(PreConnectCommand::Init(_))));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_init_minimal_flag() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "init", "--minimal"])?;
        let preconnect = take_preconnect_command(&mut cli);
        match preconnect {
            Some(PreConnectCommand::Init(args)) => {
                assert!(args.minimal);
                assert!(!args.no_spec_templates);
            }
            _ => anyhow::bail!("expected PreConnectCommand::Init"),
        }
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_init_no_spec_templates_flag() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "init", "--no-spec-templates"])?;
        let preconnect = take_preconnect_command(&mut cli);
        match preconnect {
            Some(PreConnectCommand::Init(args)) => {
                assert!(!args.minimal);
                assert!(args.no_spec_templates);
            }
            _ => anyhow::bail!("expected PreConnectCommand::Init"),
        }
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_toolchain_bootstrap() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "toolchain", "bootstrap", "--json"])?;
        let preconnect = take_preconnect_command(&mut cli);
        match preconnect {
            Some(PreConnectCommand::ToolchainBootstrap(args)) => {
                assert!(args.json);
                assert!(!args.strict);
                assert!(args.target_triple.is_none());
            }
            _ => anyhow::bail!("expected PreConnectCommand::ToolchainBootstrap"),
        }
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_command_list() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "command", "list"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::CommandList { json: false })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_command_validate() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "command", "validate"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::CommandValidate {
                name: None,
                strict: false,
                json: false
            })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_command_validate_strict_json() -> anyhow::Result<()> {
        let mut cli =
            Cli::try_parse_from(["omne", "command", "validate", "--name", "plan", "--strict", "--json"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::CommandValidate {
                name: Some(_),
                strict: true,
                json: true
            })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_list() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "preset", "list"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::PresetList { json: false })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_list_json() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "preset", "list", "--json"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::PresetList { json: true })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_show_by_name() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "preset", "show", "--name", "reviewer-safe"])?;
        let preconnect = take_preconnect_command(&mut cli);
        match preconnect {
            Some(PreConnectCommand::PresetShow { file, name, json }) => {
                assert!(file.is_none());
                assert_eq!(name.as_deref(), Some("reviewer-safe"));
                assert!(!json);
            }
            _ => anyhow::bail!("expected PreConnectCommand::PresetShow"),
        }
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_show_json() -> anyhow::Result<()> {
        let mut cli =
            Cli::try_parse_from(["omne", "preset", "show", "--name", "reviewer-safe", "--json"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::PresetShow {
                file: None,
                name: Some(_),
                json: true
            })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_validate_no_selector() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "preset", "validate"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::PresetValidate {
                file: None,
                name: None,
                strict: false,
                json: false
            })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_validate_json() -> anyhow::Result<()> {
        let mut cli =
            Cli::try_parse_from(["omne", "preset", "validate", "--name", "reviewer-safe", "--json"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::PresetValidate {
                file: None,
                name: Some(_),
                strict: false,
                json: true
            })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn preconnect_command_extracts_preset_validate_strict() -> anyhow::Result<()> {
        let mut cli = Cli::try_parse_from(["omne", "preset", "validate", "--strict"])?;
        let preconnect = take_preconnect_command(&mut cli);
        assert!(matches!(
            preconnect,
            Some(PreConnectCommand::PresetValidate {
                file: None,
                name: None,
                strict: true,
                json: false
            })
        ));
        assert!(cli.command.is_none());
        Ok(())
    }

    #[test]
    fn artifact_read_metadata_notice_reports_historical_source_and_reason() {
        let response =
            test_artifact_read_response(true, "latest_fallback", Some("history_metadata_missing"));
        let notice = artifact_read_metadata_notice(&response).expect("notice");
        assert_eq!(
            notice,
            "metadata_source=latest_fallback metadata_fallback_reason=history_metadata_missing"
        );
    }

    #[test]
    fn artifact_read_metadata_notice_skips_non_historical_reads() {
        let response = test_artifact_read_response(false, "latest", None);
        assert!(artifact_read_metadata_notice(&response).is_none());
    }

    #[test]
    fn artifact_read_fan_in_summary_notice_reports_counts() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_in_summary = Some(
            omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
                schema_version: "fan_in_summary.v1".to_string(),
                thread_id: "thread-1".to_string(),
                task_count: 2,
                scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                    env_max_concurrent_subagents: 4,
                    effective_concurrency_limit: 2,
                    priority_aging_rounds: 3,
                },
                tasks: vec![
                    omne_app_server_protocol::ArtifactFanInSummaryTask {
                        task_id: "t1".to_string(),
                        title: "first".to_string(),
                        thread_id: None,
                        turn_id: None,
                        status: "NeedUserInput".to_string(),
                        reason: None,
                        dependency_blocked: false,
                        dependency_blocker_task_id: None,
                        dependency_blocker_status: None,
                        result_artifact_id: None,
                        result_artifact_error: None,
                        result_artifact_error_id: None,
                        result_artifact_diagnostics: None,
                        pending_approval: Some(
                            omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                                approval_id: "approval-1".to_string(),
                                action: "artifact/read".to_string(),
                                summary: None,
                                approve_cmd: Some(
                                    "omne approval decide thread-1 approval-1 --approve".to_string(),
                                ),
                                deny_cmd: Some(
                                    "omne approval decide thread-1 approval-1 --deny".to_string(),
                                ),
                            },
                        ),
                    },
                    omne_app_server_protocol::ArtifactFanInSummaryTask {
                        task_id: "t2".to_string(),
                        title: "second".to_string(),
                        thread_id: None,
                        turn_id: None,
                        status: "Blocked".to_string(),
                        reason: Some("dependency failed".to_string()),
                        dependency_blocked: true,
                        dependency_blocker_task_id: Some("task_a".to_string()),
                        dependency_blocker_status: Some("Failed".to_string()),
                        result_artifact_id: None,
                        result_artifact_error: None,
                        result_artifact_error_id: None,
                        result_artifact_diagnostics: None,
                        pending_approval: None,
                    },
                ],
            },
        );

        let notice = artifact_read_fan_in_summary_notice(&response).expect("notice");
        assert!(notice.contains("schema=fan_in_summary.v1"));
        assert!(notice.contains("tasks=2"));
        assert!(notice.contains("pending_approvals=1"));
        assert!(notice.contains("dependency_blocked=1"));
        assert!(notice.contains("dependency_blocker_task_id=task_a"));
        assert!(notice.contains("dependency_blocker_status=Failed"));
        assert!(!notice.contains("diagnostics_tasks="));
    }

    #[test]
    fn artifact_read_fan_in_summary_notice_uses_dash_when_blocker_fields_missing() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_in_summary = Some(
            omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
                schema_version: "fan_in_summary.v1".to_string(),
                thread_id: "thread-1".to_string(),
                task_count: 1,
                scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                    env_max_concurrent_subagents: 4,
                    effective_concurrency_limit: 2,
                    priority_aging_rounds: 3,
                },
                tasks: vec![omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t1".to_string(),
                    title: "first".to_string(),
                    thread_id: None,
                    turn_id: None,
                    status: "Blocked".to_string(),
                    reason: Some("dependency failed".to_string()),
                    dependency_blocked: true,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                }],
            },
        );

        let notice = artifact_read_fan_in_summary_notice(&response).expect("notice");
        assert!(notice.contains("dependency_blocked=1"));
        assert!(notice.contains("dependency_blocker_task_id=-"));
        assert!(notice.contains("dependency_blocker_status=-"));
    }

    #[test]
    fn artifact_read_fan_in_summary_notice_skips_when_absent() {
        let response = test_artifact_read_response(false, "latest", None);
        assert!(artifact_read_fan_in_summary_notice(&response).is_none());
    }

    #[test]
    fn artifact_read_fan_in_summary_notice_reports_result_artifact_diagnostics() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_in_summary = Some(
            omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
                schema_version: "fan_in_summary.v1".to_string(),
                thread_id: "thread-1".to_string(),
                task_count: 2,
                scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                    env_max_concurrent_subagents: 4,
                    effective_concurrency_limit: 2,
                    priority_aging_rounds: 3,
                },
                tasks: vec![
                    omne_app_server_protocol::ArtifactFanInSummaryTask {
                        task_id: "t1".to_string(),
                        title: "first".to_string(),
                        thread_id: None,
                        turn_id: None,
                        status: "Done".to_string(),
                        reason: None,
                        dependency_blocked: false,
                        dependency_blocker_task_id: None,
                        dependency_blocker_status: None,
                        result_artifact_id: Some("artifact-1".to_string()),
                        result_artifact_error: None,
                        result_artifact_error_id: None,
                        result_artifact_diagnostics: Some(
                            omne_app_server_protocol::ArtifactFanInSummaryResultArtifactDiagnostics {
                                scan_last_seq: 42,
                                matched_completion_count: 2,
                                pending_matching_tool_ids: 1,
                            },
                        ),
                        pending_approval: None,
                    },
                    omne_app_server_protocol::ArtifactFanInSummaryTask {
                        task_id: "t2".to_string(),
                        title: "second".to_string(),
                        thread_id: None,
                        turn_id: None,
                        status: "Done".to_string(),
                        reason: None,
                        dependency_blocked: false,
                        dependency_blocker_task_id: None,
                        dependency_blocker_status: None,
                        result_artifact_id: Some("artifact-2".to_string()),
                        result_artifact_error: None,
                        result_artifact_error_id: None,
                        result_artifact_diagnostics: Some(
                            omne_app_server_protocol::ArtifactFanInSummaryResultArtifactDiagnostics {
                                scan_last_seq: 50,
                                matched_completion_count: 3,
                                pending_matching_tool_ids: 0,
                            },
                        ),
                        pending_approval: None,
                    },
                ],
            },
        );

        let notice = artifact_read_fan_in_summary_notice(&response).expect("notice");
        assert!(notice.contains("diagnostics_tasks=2"));
        assert!(notice.contains("diagnostics_matched_completion_total=5"));
        assert!(notice.contains("diagnostics_pending_matching_tool_ids_total=1"));
        assert!(notice.contains("diagnostics_scan_last_seq_max=50"));
    }

    #[test]
    fn artifact_read_fan_out_linkage_issue_notice_reports_fields() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_linkage_issue = Some(
            omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
                schema_version: "fan_out_linkage_issue.v1".to_string(),
                fan_in_summary_artifact_id: "artifact-1".to_string(),
                issue: "fan-out linkage issue: blocked".to_string(),
                issue_truncated: true,
            },
        );

        let notice = artifact_read_fan_out_linkage_issue_notice(&response).expect("notice");
        assert!(notice.contains("schema=fan_out_linkage_issue.v1"));
        assert!(notice.contains("fan_in_summary_artifact_id=artifact-1"));
        assert!(notice.contains("issue_truncated=true"));
    }

    #[test]
    fn artifact_read_fan_out_linkage_issue_notice_skips_when_absent() {
        let response = test_artifact_read_response(false, "latest", None);
        assert!(artifact_read_fan_out_linkage_issue_notice(&response).is_none());
    }

    #[test]
    fn artifact_read_fan_out_linkage_issue_notice_normalizes_blank_summary_artifact_id() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_linkage_issue = Some(
            omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
                schema_version: "fan_out_linkage_issue.v1".to_string(),
                fan_in_summary_artifact_id: "   ".to_string(),
                issue: "fan-out linkage issue: blocked".to_string(),
                issue_truncated: false,
            },
        );

        let notice = artifact_read_fan_out_linkage_issue_notice(&response).expect("notice");
        assert!(notice.contains("fan_in_summary_artifact_id=-"));
    }

    #[test]
    fn artifact_read_fan_out_linkage_issue_clear_notice_reports_fields() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_linkage_issue_clear = Some(
            omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
                schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
                fan_in_summary_artifact_id: "artifact-1".to_string(),
            },
        );

        let notice = artifact_read_fan_out_linkage_issue_clear_notice(&response).expect("notice");
        assert!(notice.contains("schema=fan_out_linkage_issue_clear.v1"));
        assert!(notice.contains("fan_in_summary_artifact_id=artifact-1"));
    }

    #[test]
    fn artifact_read_fan_out_linkage_issue_clear_notice_skips_when_absent() {
        let response = test_artifact_read_response(false, "latest", None);
        assert!(artifact_read_fan_out_linkage_issue_clear_notice(&response).is_none());
    }

    #[test]
    fn artifact_read_fan_out_linkage_issue_clear_notice_normalizes_blank_summary_artifact_id() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_linkage_issue_clear = Some(
            omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
                schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
                fan_in_summary_artifact_id: "   ".to_string(),
            },
        );

        let notice = artifact_read_fan_out_linkage_issue_clear_notice(&response).expect("notice");
        assert!(notice.contains("fan_in_summary_artifact_id=-"));
    }

    #[test]
    fn artifact_read_fan_out_result_notice_reports_patch_artifact_details() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_result = Some(omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: Some("/tmp/subagent/repo".to_string()),
            isolated_write_patch: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWritePatchStructuredData {
                    artifact_type: Some("patch".to_string()),
                    artifact_id: Some("artifact-1".to_string()),
                    truncated: Some(true),
                    read_cmd: Some("omne artifact read thread-1 artifact-1".to_string()),
                    workspace_cwd: None,
                    error: None,
                },
            ),
            isolated_write_handoff: None,
            isolated_write_auto_apply: None,
            status: "completed".to_string(),
            reason: None,
        });

        let notice = artifact_read_fan_out_result_notice(&response).expect("notice");
        assert!(notice.contains("schema=fan_out_result.v1"));
        assert!(notice.contains("task_id=t1"));
        assert!(notice.contains("status=completed"));
        assert!(notice.contains("workspace_mode=isolated_write"));
        assert!(notice.contains("isolated_write_patch_artifact_id=artifact-1"));
        assert!(notice.contains("isolated_write_patch_truncated=true"));
    }

    #[test]
    fn artifact_read_fan_out_result_notice_reports_patch_error() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_result = Some(omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t2".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-2".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: Some("/tmp/missing".to_string()),
            isolated_write_patch: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWritePatchStructuredData {
                    artifact_type: None,
                    artifact_id: None,
                    truncated: None,
                    read_cmd: None,
                    workspace_cwd: Some("/tmp/missing".to_string()),
                    error: Some("spawn git diff failed".to_string()),
                },
            ),
            isolated_write_handoff: None,
            isolated_write_auto_apply: None,
            status: "completed".to_string(),
            reason: None,
        });

        let notice = artifact_read_fan_out_result_notice(&response).expect("notice");
        assert!(notice.contains("schema=fan_out_result.v1"));
        assert!(notice.contains("task_id=t2"));
        assert!(notice.contains("isolated_write_patch=error"));
    }

    #[test]
    fn artifact_read_fan_out_result_notice_skips_when_absent() {
        let response = test_artifact_read_response(false, "latest", None);
        assert!(artifact_read_fan_out_result_notice(&response).is_none());
    }

    #[test]
    fn artifact_read_fan_out_result_notice_reports_auto_apply_outcome() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_result = Some(omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t3".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-3".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: Some("/tmp/subagent/repo".to_string()),
            isolated_write_patch: None,
            isolated_write_handoff: None,
            isolated_write_auto_apply: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                    enabled: true,
                    attempted: true,
                    applied: true,
                    workspace_cwd: Some("/tmp/subagent/repo".to_string()),
                    target_workspace_cwd: Some("/tmp/parent/repo".to_string()),
                    check_argv: vec![],
                    apply_argv: vec![],
                    patch_artifact_id: None,
                    patch_read_cmd: None,
                    failure_stage: None,
                    recovery_hint: None,
                    recovery_commands: vec![],
                    error: None,
                },
            ),
            status: "completed".to_string(),
            reason: None,
        });

        let notice = artifact_read_fan_out_result_notice(&response).expect("notice");
        assert!(notice.contains("isolated_write_auto_apply=applied"));
    }

    #[test]
    fn artifact_read_fan_out_result_notice_reports_auto_apply_error_stage() {
        let mut response = test_artifact_read_response(false, "latest", None);
        response.fan_out_result = Some(omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t4".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-4".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: Some("/tmp/subagent/repo".to_string()),
            isolated_write_patch: None,
            isolated_write_handoff: None,
            isolated_write_auto_apply: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                    enabled: true,
                    attempted: true,
                    applied: false,
                    workspace_cwd: Some("/tmp/subagent/repo".to_string()),
                    target_workspace_cwd: Some("/tmp/parent/repo".to_string()),
                    check_argv: vec![],
                    apply_argv: vec![],
                    patch_artifact_id: Some("artifact-7".to_string()),
                    patch_read_cmd: Some("omne artifact read thread-1 artifact-7".to_string()),
                    failure_stage: Some(
                        omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
                    ),
                    recovery_hint: Some(
                        "resolve apply-check conflicts in parent workspace, then apply patch manually"
                            .to_string(),
                    ),
                    recovery_commands: vec![
                        omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData {
                            label: "read_patch_artifact".to_string(),
                            argv: vec![
                                "omne".to_string(),
                                "artifact".to_string(),
                                "read".to_string(),
                                "thread-1".to_string(),
                                "artifact-7".to_string(),
                            ],
                        },
                    ],
                    error: Some("git apply --check failed: patch does not apply".to_string()),
                },
            ),
            status: "completed".to_string(),
            reason: None,
        });

        let notice = artifact_read_fan_out_result_notice(&response).expect("notice");
        assert!(notice.contains("isolated_write_auto_apply=error"));
        assert!(notice.contains("isolated_write_auto_apply_stage=check_patch"));
        assert!(notice.contains("isolated_write_auto_apply_patch_artifact_id=artifact-7"));
        assert!(notice.contains("isolated_write_auto_apply_recovery_commands=1"));
    }

    #[test]
    fn artifact_list_plain_lines_renders_rows() {
        let response = test_artifact_list_response(
            serde_json::json!([
                {
                    "artifact_id": "00000000-0000-0000-0000-000000000001",
                    "artifact_type": "plan",
                    "summary": "first",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "version": 2,
                    "content_path": "/tmp/a.md",
                    "size_bytes": 11
                },
                {
                    "artifact_id": "00000000-0000-0000-0000-000000000002",
                    "artifact_type": "diff",
                    "summary": "second",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "version": 1,
                    "content_path": "/tmp/b.md",
                    "size_bytes": 22
                }
            ]),
            serde_json::json!([]),
        );
        assert_eq!(
            artifact_list_plain_lines(&response),
            vec![
                "00000000-0000-0000-0000-000000000001 v2 plan first".to_string(),
                "00000000-0000-0000-0000-000000000002 v1 diff second".to_string()
            ]
        );
    }

    #[test]
    fn artifact_list_plain_lines_handles_empty() {
        let response = test_artifact_list_response(serde_json::json!([]), serde_json::json!([]));
        assert_eq!(artifact_list_plain_lines(&response), vec!["(no artifacts)"]);
    }

    #[test]
    fn thread_list_meta_plain_lines_renders_marker_hints() {
        let response: omne_app_server_protocol::ThreadListMetaResponse =
            serde_json::from_value(serde_json::json!({
                "threads": [
                    {
                        "thread_id": "00000000-0000-0000-0000-000000000111",
                        "cwd": "/tmp/repo",
                        "archived": false,
                        "approval_policy": "on_request",
                        "sandbox_policy": "workspace_write",
                        "last_seq": 42,
                        "active_turn_id": null,
                        "last_turn_id": "00000000-0000-0000-0000-000000000222",
                        "attention_state": "failed",
                        "token_budget_exceeded": false,
                        "token_budget_warning_active": true,
                        "has_plan_ready": true,
                        "has_diff_ready": false,
                        "has_fan_out_linkage_issue": false,
                        "has_fan_out_auto_apply_error": true,
                        "has_fan_in_dependency_blocked": true,
                        "has_fan_in_result_diagnostics": true,
                        "pending_subagent_proxy_approvals": 2,
                        "has_test_failed": false
                    }
                ]
            }))
            .expect("build ThreadListMetaResponse for test");

        let lines = thread_list_meta_plain_lines(&response);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(
            "00000000-0000-0000-0000-000000000111 state=failed seq=42 turn=00000000-0000-0000-0000-000000000222"
        ));
        assert!(lines[0].contains(
            "markers=plan_ready,fan_out_auto_apply_error,fan_in_dependency_blocked,fan_in_result_diagnostics,subagent_proxy_approval,token_budget_warning"
        ));
        assert!(lines[0].contains("subagent_pending=2"));
        assert!(lines[0].contains("cwd=/tmp/repo"));
    }

    #[test]
    fn thread_list_meta_plain_lines_handles_empty() {
        let response = omne_app_server_protocol::ThreadListMetaResponse { threads: vec![] };
        assert_eq!(thread_list_meta_plain_lines(&response), vec!["(no threads)"]);
    }

    #[test]
    fn thread_list_meta_plain_lines_renders_token_budget_exceeded_marker() {
        let response: omne_app_server_protocol::ThreadListMetaResponse =
            serde_json::from_value(serde_json::json!({
                "threads": [
                    {
                        "thread_id": "00000000-0000-0000-0000-000000000333",
                        "cwd": "/tmp/repo",
                        "archived": false,
                        "approval_policy": "on_request",
                        "sandbox_policy": "workspace_write",
                        "last_seq": 9,
                        "active_turn_id": null,
                        "last_turn_id": null,
                        "attention_state": "stuck",
                        "token_budget_exceeded": true,
                        "token_budget_warning_active": false,
                        "has_plan_ready": false,
                        "has_diff_ready": false,
                        "has_fan_out_linkage_issue": false,
                        "has_fan_out_auto_apply_error": false,
                        "has_test_failed": false
                    }
                ]
            }))
            .expect("build ThreadListMetaResponse for test");

        let lines = thread_list_meta_plain_lines(&response);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("markers=token_budget_exceeded"));
    }

    #[test]
    fn thread_list_meta_plain_lines_falls_back_to_budget_snapshot_for_warning_marker() {
        let response: omne_app_server_protocol::ThreadListMetaResponse =
            serde_json::from_value(serde_json::json!({
                "threads": [
                    {
                        "thread_id": "00000000-0000-0000-0000-000000000444",
                        "cwd": "/tmp/repo",
                        "archived": false,
                        "approval_policy": "on_request",
                        "sandbox_policy": "workspace_write",
                        "last_seq": 10,
                        "active_turn_id": null,
                        "last_turn_id": null,
                        "attention_state": "running",
                        "token_budget_limit": 1000,
                        "token_budget_utilization": 0.95,
                        "token_budget_exceeded": false,
                        "has_plan_ready": false,
                        "has_diff_ready": false,
                        "has_fan_out_linkage_issue": false,
                        "has_fan_out_auto_apply_error": false,
                        "has_test_failed": false
                    }
                ]
            }))
            .expect("build ThreadListMetaResponse for test");

        let lines = thread_list_meta_plain_lines(&response);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("markers=token_budget_warning"));
        assert!(!lines[0].contains("subagent_pending="));
    }

    #[test]
    fn thread_list_meta_marker_parts_follow_contract_order() {
        let response: omne_app_server_protocol::ThreadListMetaResponse =
            serde_json::from_value(serde_json::json!({
                "threads": [
                    {
                        "thread_id": "00000000-0000-0000-0000-000000000555",
                        "cwd": "/tmp/repo",
                        "archived": false,
                        "approval_policy": "on_request",
                        "sandbox_policy": "workspace_write",
                        "last_seq": 11,
                        "active_turn_id": null,
                        "last_turn_id": null,
                        "attention_state": "failed",
                        "token_budget_exceeded": true,
                        "token_budget_warning_active": true,
                        "has_plan_ready": true,
                        "has_diff_ready": true,
                        "has_fan_out_linkage_issue": true,
                        "has_fan_out_auto_apply_error": true,
                        "has_fan_in_dependency_blocked": true,
                        "has_fan_in_result_diagnostics": true,
                        "pending_subagent_proxy_approvals": 3,
                        "has_test_failed": true
                    }
                ]
            }))
            .expect("build ThreadListMetaResponse for test");

        let marker_parts = thread_list_meta_marker_parts(&response.threads[0], 0.9);
        assert_eq!(
            marker_parts,
            vec![
                "plan_ready",
                "diff_ready",
                "fan_out_linkage_issue",
                "fan_out_auto_apply_error",
                "fan_in_dependency_blocked",
                "fan_in_result_diagnostics",
                "subagent_proxy_approval",
                "token_budget_exceeded",
                "token_budget_warning",
                "test_failed",
            ]
        );
    }

    #[test]
    fn format_thread_list_meta_plain_line_hides_subagent_suffix_when_zero() {
        let response: omne_app_server_protocol::ThreadListMetaResponse =
            serde_json::from_value(serde_json::json!({
                "threads": [
                    {
                        "thread_id": "00000000-0000-0000-0000-000000000666",
                        "cwd": "/tmp/repo",
                        "archived": false,
                        "approval_policy": "on_request",
                        "sandbox_policy": "workspace_write",
                        "last_seq": 12,
                        "active_turn_id": null,
                        "last_turn_id": null,
                        "attention_state": "running",
                        "pending_subagent_proxy_approvals": 0,
                        "has_plan_ready": false,
                        "has_diff_ready": false,
                        "has_fan_out_linkage_issue": false,
                        "has_fan_out_auto_apply_error": false,
                        "has_fan_in_dependency_blocked": false,
                        "has_fan_in_result_diagnostics": false,
                        "has_test_failed": false
                    }
                ]
            }))
            .expect("build ThreadListMetaResponse for test");

        let line = format_thread_list_meta_plain_line(
            &response.threads[0],
            "-",
            "-",
            "-",
            "/tmp/repo",
        );
        assert_eq!(
            line,
            "00000000-0000-0000-0000-000000000666 state=running seq=12 turn=- model=- markers=- cwd=/tmp/repo"
        );
    }

    #[test]
    fn format_thread_list_meta_plain_line_includes_subagent_suffix_when_positive() {
        let response: omne_app_server_protocol::ThreadListMetaResponse =
            serde_json::from_value(serde_json::json!({
                "threads": [
                    {
                        "thread_id": "00000000-0000-0000-0000-000000000777",
                        "cwd": "/tmp/repo",
                        "archived": false,
                        "approval_policy": "on_request",
                        "sandbox_policy": "workspace_write",
                        "last_seq": 13,
                        "active_turn_id": null,
                        "last_turn_id": null,
                        "attention_state": "failed",
                        "pending_subagent_proxy_approvals": 4,
                        "has_plan_ready": false,
                        "has_diff_ready": false,
                        "has_fan_out_linkage_issue": false,
                        "has_fan_out_auto_apply_error": false,
                        "has_fan_in_dependency_blocked": false,
                        "has_fan_in_result_diagnostics": false,
                        "has_test_failed": false
                    }
                ]
            }))
            .expect("build ThreadListMetaResponse for test");

        let line = format_thread_list_meta_plain_line(
            &response.threads[0],
            "-",
            "-",
            "subagent_proxy_approval",
            "/tmp/repo",
        );
        assert_eq!(
            line,
            "00000000-0000-0000-0000-000000000777 state=failed seq=13 turn=- model=- markers=subagent_proxy_approval subagent_pending=4 cwd=/tmp/repo"
        );
    }

    fn test_artifact_read_response(
        historical: bool,
        metadata_source: &str,
        metadata_fallback_reason: Option<&str>,
    ) -> omne_app_server_protocol::ArtifactReadResponse {
        let mut value = serde_json::json!({
            "tool_id": "00000000-0000-0000-0000-000000000000",
            "metadata": {
                "artifact_id": "00000000-0000-0000-0000-000000000000",
                "artifact_type": "markdown",
                "summary": "test",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "version": 1,
                "content_path": "/tmp/test.md",
                "size_bytes": 1
            },
            "text": "hello",
            "truncated": false,
            "bytes": 1,
            "version": 1,
            "latest_version": 1,
            "historical": historical,
            "metadata_source": metadata_source
        });
        if let Some(reason) = metadata_fallback_reason {
            value["metadata_fallback_reason"] = Value::String(reason.to_string());
        }
        serde_json::from_value(value).expect("build ArtifactReadResponse for test")
    }

    fn test_artifact_list_response(
        artifacts: Value,
        errors: Value,
    ) -> omne_app_server_protocol::ArtifactListResponse {
        serde_json::from_value(serde_json::json!({
            "tool_id": "00000000-0000-0000-0000-000000000000",
            "artifacts": artifacts,
            "errors": errors
        }))
        .expect("build ArtifactListResponse for test")
    }
}
