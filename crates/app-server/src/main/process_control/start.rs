fn command_uses_network(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };

    let mut name = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program.as_str())
        .to_ascii_lowercase();
    if let Some(stripped) = name.strip_suffix(".exe") {
        name = stripped.to_string();
    }

    match name.as_str() {
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "ftp" | "telnet" | "nc" | "ncat"
        | "netcat" | "gh" => true,
        "git" => argv
            .get(1)
            .map(|subcommand| {
                matches!(
                    subcommand.as_str(),
                    "clone" | "fetch" | "pull" | "push" | "ls-remote" | "submodule"
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}

async fn handle_process_start(
    server: &Server,
    params: ProcessStartParams,
) -> anyhow::Result<Value> {
    if params.argv.is_empty() {
        anyhow::bail!("argv must not be empty");
    }

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, sandbox_policy, sandbox_network_access, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_network_access,
            state.mode.clone(),
        )
    };

    let cwd_path = if let Some(cwd) = params.cwd.as_deref() {
        resolve_dir_for_sandbox(&thread_root, sandbox_policy, Path::new(cwd)).await?
    } else {
        thread_root.clone()
    };
    let cwd_str = cwd_path.display().to_string();

    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        let tool_id = pm_protocol::ToolId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/start".to_string(),
                params: Some(serde_json::json!({
                    "argv": params.argv.clone(),
                    "cwd": cwd_str,
                })),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids process/start".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    if sandbox_network_access == pm_protocol::SandboxNetworkAccess::Deny
        && command_uses_network(&params.argv)
    {
        let tool_id = pm_protocol::ToolId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/start".to_string(),
                params: Some(serde_json::json!({
                    "argv": params.argv.clone(),
                    "cwd": cwd_str,
                })),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_network_access=deny forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_network_access": sandbox_network_access,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_network_access": sandbox_network_access,
        }));
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;
            let tool_id = pm_protocol::ToolId::new();

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "process/start".to_string(),
                    params: Some(serde_json::json!({
                        "argv": params.argv,
                        "cwd": cwd_str,
                    })),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.command;
    let effective_mode_decision = match mode.tool_overrides.get("process/start").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_mode_decision == pm_core::modes::Decision::Deny {
        let tool_id = pm_protocol::ToolId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/start".to_string(),
                params: Some(serde_json::json!({
                    "argv": params.argv,
                    "cwd": cwd_str,
                })),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies process/start".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_mode_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_mode_decision,
        }));
    }

    let exec_matches = server.exec_policy.matches_for_command(&params.argv, None);
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();

    let effective_exec_decision = match exec_decision {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };

    if effective_exec_decision == ExecDecision::Forbidden {
        let tool_id = pm_protocol::ToolId::new();
        let exec_matches_json = serde_json::to_value(&exec_matches)?;

        let justification = exec_matches.iter().find_map(|m| match m {
            ExecRuleMatch::PrefixRuleMatch {
                decision: ExecDecision::Forbidden,
                justification,
                ..
            } => justification.clone(),
            _ => None,
        });

        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/start".to_string(),
                params: Some(serde_json::json!({
                    "argv": params.argv,
                    "cwd": cwd_str,
                })),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("execpolicy forbids this command".to_string()),
                result: Some(serde_json::json!({
                    "decision": ExecDecision::Forbidden,
                    "matched_rules": exec_matches_json,
                    "justification": justification,
                })),
            })
            .await?;

        return Ok(serde_json::json!({
            "denied": true,
            "decision": ExecDecision::Forbidden,
            "matched_rules": exec_matches_json,
            "justification": justification,
        }));
    }

    let approval_params = serde_json::json!({
        "argv": params.argv.clone(),
        "cwd": cwd_str.clone(),
    });
    let needs_approval =
        effective_mode_decision == pm_core::modes::Decision::Prompt
            || effective_exec_decision == ExecDecision::Prompt;
    if needs_approval {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "process/start",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let tool_id = pm_protocol::ToolId::new();
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "process/start".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some(approval_denied_error(remembered).to_string()),
                            result: Some(serde_json::json!({
                                "approval_policy": approval_policy,
                            })),
                        })
                        .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    let process_id = ProcessId::new();
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let process_dir = thread_dir
        .join("artifacts")
        .join("processes")
        .join(process_id.to_string());
    tokio::fs::create_dir_all(&process_dir)
        .await
        .with_context(|| format!("create dir {}", process_dir.display()))?;

    let stdout_path = process_dir.join("stdout.log");
    let stderr_path = process_dir.join("stderr.log");

    let mut cmd = Command::new(&params.argv[0]);
    cmd.args(params.argv.iter().skip(1));
    cmd.current_dir(&cwd_path);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    scrub_child_process_env(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {:?}", params.argv))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let max_bytes_per_part = process_log_max_bytes_per_part();

    let stdout_task = if let Some(stdout) = stdout {
        let stdout_path = stdout_path.clone();
        Some(tokio::spawn(async move {
            capture_rotating_log(stdout, stdout_path, max_bytes_per_part).await
        }))
    } else {
        None
    };

    let stderr_task = if let Some(stderr) = stderr {
        let stderr_path = stderr_path.clone();
        Some(tokio::spawn(async move {
            capture_rotating_log(stderr, stderr_path, max_bytes_per_part).await
        }))
    } else {
        None
    };

    let started = thread_rt
        .append_event(pm_protocol::ThreadEventKind::ProcessStarted {
            process_id,
            turn_id: params.turn_id,
            argv: params.argv.clone(),
            cwd: cwd_str.clone(),
            stdout_path: stdout_path.display().to_string(),
            stderr_path: stderr_path.display().to_string(),
        })
        .await?;
    let started_at = started.timestamp.format(&Rfc3339)?;

    let info = ProcessInfo {
        process_id,
        thread_id: params.thread_id,
        turn_id: params.turn_id,
        argv: params.argv.clone(),
        cwd: cwd_str,
        started_at: started_at.clone(),
        status: ProcessStatus::Running,
        exit_code: None,
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        last_update_at: started_at,
    };

    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let entry = ProcessEntry {
        info: Arc::new(tokio::sync::Mutex::new(info)),
        cmd_tx,
    };
    server
        .processes
        .lock()
        .await
        .insert(process_id, entry.clone());

    tokio::spawn(run_process_actor(
        thread_rt,
        process_id,
        child,
        cmd_rx,
        stdout_task,
        stderr_task,
        entry.info.clone(),
    ));

    Ok(serde_json::json!({
        "process_id": process_id,
        "stdout_path": stdout_path.display().to_string(),
        "stderr_path": stderr_path.display().to_string(),
    }))
}

#[cfg(test)]
mod network_access_tests {
    use super::*;

    fn build_test_server(pm_root: PathBuf) -> Server {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        Server {
            cwd: pm_root.clone(),
            out_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    async fn write_executable_sh(path: &Path, script: &str) -> anyhow::Result<()> {
        tokio::fs::write(path, script).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            tokio::fs::set_permissions(path, perms).await?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn process_start_denies_network_commands_when_network_access_is_denied() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("curl").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let server = build_test_server(tmp.path().join(".code_pm"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./curl".to_string()],
                cwd: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_network_access"].as_str(), Some("deny"));
        assert!(server.processes.lock().await.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_start_allows_network_commands_when_network_access_is_allowed() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("curl").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let server = build_test_server(tmp.path().join(".code_pm"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(pm_protocol::SandboxNetworkAccess::Allow),
                mode: None,
                model: None,
                openai_base_url: None,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./curl".to_string()],
                cwd: None,
            },
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let status = {
                let entry = {
                    let processes = server.processes.lock().await;
                    processes
                        .get(&process_id)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("missing process entry"))?
                };
                let info = entry.info.lock().await;
                info.status.clone()
            };

            if matches!(status, ProcessStatus::Exited) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("process did not exit in time");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }
}
