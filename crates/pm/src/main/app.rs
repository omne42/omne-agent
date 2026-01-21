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
            ThreadCommand::Fork { thread_id, json } => {
                let result = app.thread_fork(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Spawn {
                thread_id,
                input,
                model,
                openai_base_url,
                json,
            } => {
                let result = app.thread_spawn(thread_id, input, model, openai_base_url).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Archive {
                thread_id,
                force,
                reason,
            } => {
                let result = app.thread_archive(thread_id, force, reason).await?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Unarchive { thread_id, reason } => {
                let result = app.thread_unarchive(thread_id, reason).await?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Pause { thread_id, reason } => {
                let result = app.thread_pause(thread_id, reason).await?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Unpause { thread_id, reason } => {
                let result = app.thread_unpause(thread_id, reason).await?;
                print_json_or_pretty(true, &result)?;
            }
            ThreadCommand::Delete {
                thread_id,
                force,
                json,
            } => {
                let result = app.thread_delete(thread_id, force).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::ClearArtifacts {
                thread_id,
                force,
                json,
            } => {
                let result = app.thread_clear_artifacts(thread_id, force).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::DiskUsage { thread_id, json } => {
                let result = app.thread_disk_usage(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::DiskReport {
                thread_id,
                top_files,
                json,
            } => {
                let result = app.thread_disk_report(thread_id, top_files).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::HookRun {
                thread_id,
                hook,
                approval_id,
                json,
            } => {
                let result = app.thread_hook_run(thread_id, hook, approval_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Events {
                thread_id,
                since_seq,
                max_events,
                json,
            } => {
                let result = app.thread_events(thread_id, since_seq, max_events).await?;
                if json {
                    println!("{}", serde_json::to_string(&result)?);
                } else {
                    print_json_or_pretty(false, &result)?;
                }
            }
            ThreadCommand::Loaded { json } => {
                let result = app.thread_loaded().await?;
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
            ThreadCommand::State { thread_id, json } => {
                let result = app.thread_state(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::ConfigExplain { thread_id, json } => {
                let result = app.thread_config_explain(thread_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ThreadCommand::Configure(args) => {
                app.thread_configure(args).await?;
            }
        },
        Command::Inbox(args) => {
            run_inbox(&mut app, args).await?;
        }
        Command::Ask(args) => {
            run_ask(&mut app, args).await?;
        }
        Command::Exec(args) => {
            let code = run_exec(&mut app, args).await?;
            std::process::exit(code);
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
                approval_id,
                json,
            } => {
                let result = app.process_inspect(process_id, max_lines, approval_id).await?;
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
                app.process_interrupt(process_id, reason, approval_id).await?;
            }
            ProcessCommand::Kill {
                process_id,
                reason,
                approval_id,
            } => {
                app.process_kill(process_id, reason, approval_id).await?;
            }
        },
        Command::Artifact { command } => match command {
            ArtifactCommand::List {
                thread_id,
                approval_id,
                json,
            } => {
                let result = app.artifact_list(thread_id, approval_id).await?;
                print_json_or_pretty(json, &result)?;
            }
            ArtifactCommand::Read {
                thread_id,
                artifact_id,
                max_bytes,
                approval_id,
                json,
            } => {
                let result = app
                    .artifact_read(thread_id, artifact_id, max_bytes, approval_id)
                    .await?;
                if json {
                    print_json_or_pretty(true, &result)?;
                } else {
                    let text = result["text"].as_str().unwrap_or("");
                    print!("{text}");
                    if !text.ends_with('\n') {
                        println!();
                    }
                    if result["truncated"].as_bool().unwrap_or(false) {
                        eprintln!("[truncated]");
                    }
                }
            }
            ArtifactCommand::Delete {
                thread_id,
                artifact_id,
                approval_id,
                json,
            } => {
                let result = app.artifact_delete(thread_id, artifact_id, approval_id).await?;
                print_json_or_pretty(json, &result)?;
            }
        },
    }

    Ok(())
}
