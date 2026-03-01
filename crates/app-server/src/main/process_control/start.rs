async fn handle_process_start(
    server: &Server,
    params: ProcessStartParams,
) -> anyhow::Result<Value> {
    handle_process_start_inner(server, params, None).await
}

async fn handle_process_start_with_env(
    server: &Server,
    params: ProcessStartParams,
    extra_env: &std::collections::BTreeMap<String, String>,
) -> anyhow::Result<Value> {
    handle_process_start_inner(server, params, Some(extra_env)).await
}

async fn handle_process_start_inner(
    server: &Server,
    params: ProcessStartParams,
    extra_env: Option<&std::collections::BTreeMap<String, String>>,
) -> anyhow::Result<Value> {
    if params.argv.is_empty() {
        anyhow::bail!("argv must not be empty");
    }
    if matches!(params.timeout_ms, Some(0)) {
        anyhow::bail!("timeout_ms must be >= 1 when provided");
    }

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (
        approval_policy,
        sandbox_policy,
        sandbox_network_access,
        mode_name,
        allowed_tools,
        thread_execpolicy_rules,
    ) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_network_access,
            state.mode.clone(),
            state.allowed_tools.clone(),
            state.execpolicy_rules.clone(),
        )
    };
    let tool_id = omne_protocol::ToolId::new();
    let mut approval_params = serde_json::json!({
        "argv": params.argv.clone(),
        "cwd": params
            .cwd
            .clone()
            .unwrap_or_else(|| thread_root.display().to_string()),
    });
    if let Some(timeout_ms) = params.timeout_ms {
        approval_params["timeout_ms"] = serde_json::json!(timeout_ms);
    }
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "process/start",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return process_allowed_tools_denied_response(tool_id, "process/start", &allowed_tools);
    }

    let cwd_path = if let Some(cwd) = params.cwd.as_deref() {
        omne_core::resolve_dir_for_sandbox(&thread_root, sandbox_policy, Path::new(cwd)).await?
    } else {
        thread_root.clone()
    };
    let cwd_str = cwd_path.display().to_string();
    let start_tool_params = serde_json::json!({
        "argv": params.argv.clone(),
        "cwd": cwd_str.clone(),
    });

    if sandbox_policy == omne_protocol::SandboxPolicy::ReadOnly {
        let result = process_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/start",
            &start_tool_params,
            "sandbox_policy=read_only forbids process/start".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    if sandbox_network_access == omne_protocol::SandboxNetworkAccess::Deny
        && omne_process_runtime::command_uses_network(&params.argv)
    {
        let result = process_sandbox_network_denied_response(tool_id, sandbox_network_access)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/start",
            &start_tool_params,
            "sandbox_network_access=deny forbids this command".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    let mode_ctx = ProcessModeApprovalContext {
        thread_rt: &thread_rt,
        thread_root: &thread_root,
        thread_id: params.thread_id,
        turn_id: params.turn_id,
        approval_id: params.approval_id,
        approval_policy,
        mode_name: &mode_name,
        action: "process/start",
        tool_id,
        approval_params: &start_tool_params,
    };
    let (mode, mode_decision) = match enforce_process_mode_gate(
        &mode_ctx,
        |mode| mode.permissions.command,
    )
    .await?
    {
        ProcessModeGate::Denied(result) => return Ok(*result),
        ProcessModeGate::Allowed {
            mode,
            mode_decision,
        } => (mode, mode_decision),
    };

    let mut effective_exec_policy = server.exec_policy.clone();
    if !mode.command_execpolicy_rules.is_empty() {
        let mode_exec_policy =
            match load_mode_exec_policy(&thread_root, &mode.command_execpolicy_rules).await {
                Ok(policy) => policy,
                Err(err) => {
                    let result = process_execpolicy_load_denied_response(
                        tool_id,
                        &mode_name,
                        "failed to load mode execpolicy rules",
                        err.to_string(),
                    )?;
                    emit_process_tool_denied(
                        &thread_rt,
                        tool_id,
                        params.turn_id,
                        "process/start",
                        &start_tool_params,
                        "failed to load mode execpolicy rules".to_string(),
                        result.clone(),
                    )
                    .await?;
                    return Ok(result);
                }
            };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &mode_exec_policy);
    }
    if !thread_execpolicy_rules.is_empty() {
        let thread_exec_policy = match load_mode_exec_policy(&thread_root, &thread_execpolicy_rules).await {
            Ok(policy) => policy,
            Err(err) => {
                let result = process_execpolicy_load_denied_response(
                    tool_id,
                    &mode_name,
                    "failed to load thread execpolicy rules",
                    err.to_string(),
                )?;
                emit_process_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "process/start",
                    &start_tool_params,
                    "failed to load thread execpolicy rules".to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
        };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &thread_exec_policy);
    }
    let exec_matches = effective_exec_policy.matches_for_command(&params.argv, None);
    let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();

    let effective_exec_decision = match exec_decision {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };

    if effective_exec_decision == ExecDecision::Forbidden {
        let justification = exec_matches.iter().find_map(|m| match m {
            ExecRuleMatch::PrefixRuleMatch {
                decision: ExecDecision::Forbidden,
                justification,
                ..
            } => justification.clone(),
            _ => None,
        });

        let result = process_execpolicy_denied_response(
            tool_id,
            ExecDecision::Forbidden,
            &exec_matches,
            justification,
        )?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/start",
            &start_tool_params,
            "execpolicy forbids this command".to_string(),
            result.clone(),
        )
        .await?;

        return Ok(result);
    }

    let mut approval_params = serde_json::json!({
        "argv": params.argv.clone(),
        "cwd": cwd_str.clone(),
    });
    if let Some(timeout_ms) = params.timeout_ms {
        approval_params["timeout_ms"] = serde_json::json!(timeout_ms);
    }
    if effective_exec_decision == ExecDecision::PromptStrict {
        approval_params["approval"] = serde_json::json!({
            "requirement": "prompt_strict",
            "source": "execpolicy",
        });
    }
    let needs_approval = mode_decision.decision == omne_core::modes::Decision::Prompt
        || matches!(
            effective_exec_decision,
            ExecDecision::Prompt | ExecDecision::PromptStrict
        );
    if needs_approval {
        match gate_approval_with_deps(
            &server.thread_store,
            &effective_exec_policy,
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
                let result = process_denied_response(tool_id, params.thread_id, Some(remembered))?;
                emit_process_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "process/start",
                    &approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return process_needs_approval_response(params.thread_id, approval_id);
            }
        }
    }

    let process_id = ProcessId::new();
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let process_dir = thread_dir
        .join("runtime")
        .join("processes")
        .join(process_id.to_string());
    tokio::fs::create_dir_all(&process_dir)
        .await
        .with_context(|| format!("create dir {}", process_dir.display()))?;

    let stdout_path = process_dir.join("stdout.log");
    let stderr_path = process_dir.join("stderr.log");

    let mut combined_env = std::collections::BTreeMap::<String, String>::new();
    if let Some(extra_env) = extra_env {
        combined_env.extend(extra_env.clone());
    }

    let mut execve_gate: Option<ExecveGateHandle> = None;
    #[cfg(unix)]
    {
        fn is_bash(argv0: &str) -> bool {
            let mut name = Path::new(argv0)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(argv0)
                .to_ascii_lowercase();
            if let Some(stripped) = name.strip_suffix(".exe") {
                name = stripped.to_string();
            }
            name == "bash"
        }

        if is_bash(&params.argv[0])
            && let Ok(wrapper_path) = std::env::var("OMNE_EXECVE_WRAPPER")
        {
            let wrapper_path = wrapper_path.trim().to_string();
            if wrapper_path.is_empty() {
                anyhow::bail!("OMNE_EXECVE_WRAPPER must not be empty");
            }
            if wrapper_path.chars().any(|c| c.is_whitespace()) {
                anyhow::bail!("OMNE_EXECVE_WRAPPER must not contain whitespace");
            }

            let token = omne_protocol::ApprovalId::new().to_string();
            let socket_path = process_dir.join("execve-gate.sock");

            execve_gate = Some(
                spawn_execve_gate(
                    ExecveGateContext {
                        thread_id: params.thread_id,
                        turn_id: params.turn_id,
                        token: token.clone(),
                        thread_root: thread_root.clone(),
                        thread_store: server.thread_store.clone(),
                        exec_policy: server.exec_policy.clone(),
                        thread_rt: thread_rt.clone(),
                    },
                    socket_path.clone(),
                )
                .await?,
            );

            combined_env.insert("BASH_EXEC_WRAPPER".to_string(), wrapper_path);
            combined_env.insert(
                "OMNE_EXECVE_SOCKET".to_string(),
                socket_path.display().to_string(),
            );
            combined_env.insert("OMNE_EXECVE_TOKEN".to_string(), token);
            combined_env.insert("OMNE_THREAD_ID".to_string(), params.thread_id.to_string());
            if let Some(turn_id) = params.turn_id {
                combined_env.insert("OMNE_TURN_ID".to_string(), turn_id.to_string());
            }
        }
    }

    let mut cmd = Command::new(&params.argv[0]);
    cmd.args(params.argv.iter().skip(1));
    cmd.current_dir(&cwd_path);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let combined_env_opt = (!combined_env.is_empty()).then_some(&combined_env);
    if let Some(env) = combined_env_opt {
        cmd.envs(env.iter());
    }
    let effective_env_summary = apply_child_process_hardening(&mut cmd, combined_env_opt)
        .context("apply child process hardening")?;
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            if let Some(gate) = execve_gate.take() {
                shutdown_execve_gate(gate).await;
            }
            return Err(err).with_context(|| format!("spawn {:?}", params.argv));
        }
    };

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
        .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
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

    tokio::spawn(run_process_actor(ProcessActorArgs {
        thread_rt,
        process_id,
        child,
        cmd_rx,
        stdout_task,
        stderr_task,
        execve_gate,
        info: entry.info.clone(),
    }));
    if let Some(timeout_ms) = params.timeout_ms {
        let timeout_cmd_tx = entry.cmd_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(timeout_ms)).await;
            let _ = timeout_cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some(format!("wall_clock_timeout_ms={timeout_ms}")),
                })
                .await;
        });
    }

    let response = omne_app_server_protocol::ProcessStartResponse {
        process_id,
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        effective_env_summary: serde_json::to_value(effective_env_summary)
            .context("serialize process/start effective_env_summary")?,
        timeout_ms: params.timeout_ms,
    };
    serde_json::to_value(response).context("serialize process/start response")
}

async fn resolve_execpolicy_rule_paths(
    thread_root: &Path,
    rules: &[String],
) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for rule in rules {
        let trimmed = rule.trim();
        if trimmed.is_empty() {
            anyhow::bail!("mode execpolicy rule path must not be empty");
        }

        let path = Path::new(trimmed);
        if path.is_absolute() {
            out.push(path.to_path_buf());
        } else {
            let resolved =
                omne_core::resolve_file(thread_root, path, omne_core::PathAccess::Read, false).await?;
            out.push(resolved);
        }
    }
    Ok(out)
}

async fn load_mode_exec_policy(
    thread_root: &Path,
    rules: &[String],
) -> anyhow::Result<omne_execpolicy::Policy> {
    let rule_paths = resolve_execpolicy_rule_paths(thread_root, rules).await?;
    let policy = tokio::task::spawn_blocking(move || {
        omne_execpolicy::execpolicycheck::load_policies(&rule_paths)
    })
    .await
    .context("join mode execpolicy load task")??;
    Ok(policy)
}

fn merge_exec_policies(
    global: &omne_execpolicy::Policy,
    mode: &omne_execpolicy::Policy,
) -> omne_execpolicy::Policy {
    let mut combined = global.clone();
    for rules in mode.rules().values() {
        for rule in rules {
            combined.add_rule(rule.clone());
        }
    }
    combined
}

#[cfg(test)]
mod process_start_tests {
    use super::*;

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
    async fn process_start_denies_network_commands_when_network_access_is_denied()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("curl").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_network_access"].as_str(), Some("deny"));
        assert!(server.processes.lock().await.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_start_allows_network_commands_when_network_access_is_allowed()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("curl").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Allow),
                mode: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
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
                timeout_ms: None,
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

    #[tokio::test]
    async fn process_start_returns_effective_env_summary_for_hardening() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("ok.sh").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;

        let mut extra_env = std::collections::BTreeMap::new();
        extra_env.insert("OPENAI_API_KEY".to_string(), "secret".to_string());
        extra_env.insert("GIT_TERMINAL_PROMPT".to_string(), "1".to_string());
        extra_env.insert("NO_COLOR".to_string(), "0".to_string());
        extra_env.insert("PAGER".to_string(), "less".to_string());

        let result = handle_process_start_with_env(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./ok.sh".to_string()],
                cwd: None,
                timeout_ms: None,
            },
            &extra_env,
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;
        assert_eq!(
            result["effective_env_summary"]["hardening_mode"].as_str(),
            Some("best_effort")
        );
        let scrubbed_keys = result["effective_env_summary"]["scrubbed_keys"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing scrubbed_keys"))?;
        assert!(scrubbed_keys.iter().any(|k| k.as_str() == Some("OPENAI_API_KEY")));
        let injected_defaults = result["effective_env_summary"]["injected_defaults"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("missing injected_defaults"))?;
        assert!(injected_defaults.is_empty());

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

    #[tokio::test]
    async fn process_start_rejects_zero_timeout_ms() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("ok.sh").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;

        let err = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./ok.sh".to_string()],
                cwd: None,
                timeout_ms: Some(0),
            },
        )
        .await
        .expect_err("timeout_ms=0 should be rejected");

        assert!(err.to_string().contains("timeout_ms must be >= 1"));
        Ok(())
    }

    #[tokio::test]
    async fn process_start_kills_process_after_wall_clock_timeout() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["sleep".to_string(), "5".to_string()],
                cwd: None,
                timeout_ms: Some(100),
            },
        )
        .await?;

        assert_eq!(result["timeout_ms"].as_u64(), Some(100));
        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
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
                anyhow::bail!("process did not exit before timeout deadline");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;

        let mut saw_kill_reason = false;
        let mut saw_exit_reason = false;
        for event in events {
            match event.kind {
                omne_protocol::ThreadEventKind::ProcessKillRequested {
                    process_id: pid,
                    reason,
                } if pid == process_id => {
                    if reason.as_deref() == Some("wall_clock_timeout_ms=100") {
                        saw_kill_reason = true;
                    }
                }
                omne_protocol::ThreadEventKind::ProcessExited {
                    process_id: pid,
                    reason,
                    ..
                } if pid == process_id => {
                    if reason.as_deref() == Some("wall_clock_timeout_ms=100") {
                        saw_exit_reason = true;
                    }
                }
                _ => {}
            }
        }

        assert!(saw_kill_reason);
        assert!(saw_exit_reason);
        Ok(())
    }

    #[tokio::test]
    async fn process_start_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  mode-x:
    description: "mode x"
    permissions:
      command:
        decision: allow
    tool_overrides:
      - tool: "process/start"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: None,
                mode: Some("mode-x".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["echo".to_string(), "hi".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_start_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: None,
                mode: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["echo".to_string(), "hi".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("process/start"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        assert!(server.processes.lock().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn process_start_applies_mode_execpolicy_rules() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(repo_dir.join("rules")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  mode-x:
    description: "mode x"
    permissions:
      command:
        decision: allow
        execpolicy_rules: ["rules/mode.rules"]
"#,
        )
        .await?;
        tokio::fs::write(
            repo_dir.join("rules/mode.rules"),
            r#"
prefix_rule(
    pattern = ["./tool"],
    decision = "forbidden",
)
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: None,
                mode: Some("mode-x".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./tool".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("forbidden"));
        assert!(server.processes.lock().await.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_start_denies_when_mode_execpolicy_rules_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  mode-x:
    description: "mode x"
    permissions:
      command:
        decision: allow
        execpolicy_rules: ["rules/missing.rules"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: None,
                mode: Some("mode-x".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./tool".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(
            result["error"].as_str(),
            Some("failed to load mode execpolicy rules")
        );
        assert!(
            !result["details"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .is_empty()
        );
        assert!(server.processes.lock().await.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_start_applies_thread_execpolicy_rules() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("rules")).await?;
        tokio::fs::write(
            repo_dir.join("rules/thread.rules"),
            r#"
prefix_rule(
    pattern = ["./tool"],
    decision = "forbidden",
)
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: None,
                mode: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: Some(vec!["rules/thread.rules".to_string()]),
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./tool".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("forbidden"));
        assert!(server.processes.lock().await.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn process_start_denies_when_thread_execpolicy_rules_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
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
                sandbox_network_access: None,
                mode: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: Some(vec!["rules/missing.rules".to_string()]),
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["./tool".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(
            result["error"].as_str(),
            Some("failed to load thread execpolicy rules")
        );
        assert!(
            !result["details"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .is_empty()
        );
        assert!(server.processes.lock().await.is_empty());

        Ok(())
    }
}
