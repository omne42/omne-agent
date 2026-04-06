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

#[cfg(unix)]
#[derive(Debug, Eq, PartialEq)]
enum ExecveWrapperTarget {
    Disabled,
    Supported,
    Unsupported(String),
}

#[cfg(unix)]
fn normalized_execve_shell_name(program: &str) -> String {
    let mut name = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    if let Some(stripped) = name.strip_suffix(".exe") {
        name = stripped.to_string();
    }
    name
}

#[cfg(unix)]
fn resolve_execve_shell_program(
    program: &str,
    path_override: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    if program.contains('/') {
        return Some(std::path::PathBuf::from(program));
    }

    let path_storage;
    let path = if let Some(path) = path_override {
        path
    } else {
        path_storage = std::env::var_os("PATH")?;
        path_storage.as_os_str()
    };
    for base in std::env::split_paths(path) {
        let candidate = base.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn execve_wrapper_target_for_shell(
    argv0: &str,
    path_override: Option<&std::ffi::OsStr>,
) -> ExecveWrapperTarget {
    match normalized_execve_shell_name(argv0).as_str() {
        "bash" | "rbash" => ExecveWrapperTarget::Supported,
        "sh" => {
            let Some(path) = resolve_execve_shell_program(argv0, path_override) else {
                return ExecveWrapperTarget::Unsupported(format!(
                    "execve wrapper requires a bash-compatible shell; failed to resolve {argv0}"
                ));
            };
            let resolved = std::fs::canonicalize(&path).unwrap_or(path);
            let resolved_name = normalized_execve_shell_name(&resolved.display().to_string());
            if matches!(resolved_name.as_str(), "bash" | "rbash") {
                ExecveWrapperTarget::Supported
            } else {
                ExecveWrapperTarget::Unsupported(format!(
                    "execve wrapper requires a bash-compatible shell; {argv0} resolves to {}",
                    resolved.display()
                ))
            }
        }
        _ => ExecveWrapperTarget::Disabled,
    }
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
    let cwd_path = if let Some(cwd) = params.cwd.as_deref() {
        omne_core::resolve_dir_for_sandbox(&thread_root, sandbox_policy, Path::new(cwd)).await?
    } else {
        thread_root.clone()
    };
    let mut cwd_str = cwd_path.display().to_string();
    let requested_cwd_str = params
        .cwd
        .clone()
        .unwrap_or_else(|| thread_root.display().to_string());
    let start_tool_params = serde_json::json!({
        "argv": params.argv.clone(),
        "cwd": cwd_str.clone(),
    });
    let tool_id = omne_protocol::ToolId::new();
    let approval_params =
        build_process_exec_approval_params(&params.argv, &requested_cwd_str, params.timeout_ms, None);
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

    let auth_cwd_str = cwd_str.clone();
    let exec_governance = evaluate_process_exec_governance(
        &ProcessExecGovernanceContext {
            cwd: &cwd_path,
            sandbox_policy,
            sandbox_network_access,
            authorization: ProcessExecAuthorizationContext {
                thread_root: &thread_root,
                thread_store: &server.thread_store,
                thread_rt: &thread_rt,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                approval_id: params.approval_id,
                approval_policy,
                mode_name: &mode_name,
                action: "process/start",
                exec_policy: &server.exec_policy,
                thread_execpolicy_rules: &thread_execpolicy_rules,
                argv: &params.argv,
                unmatched_command_policy: UnmatchedCommandPolicy::Prompt,
            },
        },
        |mode| mode.permissions.command,
        |approval_requirement| {
            let approval_metadata = matches!(
                approval_requirement,
                ProcessExecApprovalRequirement::PromptStrict
            )
            .then_some((
                ProcessExecApprovalSource::ExecPolicy,
                ProcessExecApprovalRequirement::PromptStrict,
            ));
            build_process_exec_approval_params(
                &params.argv,
                &auth_cwd_str,
                params.timeout_ms,
                approval_metadata,
            )
        },
    )
    .await?;

    match exec_governance {
        ProcessExecGovernance::Allowed => {}
        ProcessExecGovernance::NeedsApproval { approval_id } => {
            return process_needs_approval_response(params.thread_id, approval_id);
        }
        ProcessExecGovernance::Denied(ProcessExecGovernanceDenied::ApprovalDenied {
            remembered,
        }) => {
            let approval_params = build_process_exec_approval_params(
                &params.argv,
                &auth_cwd_str,
                params.timeout_ms,
                None,
            );
            let result = process_denied_response(tool_id, params.thread_id, Some(remembered))?;
            return emit_process_tool_denied_response(
                &thread_rt,
                tool_id,
                params.turn_id,
                "process/start",
                &approval_params,
                approval_denied_error(remembered).to_string(),
                result,
            )
            .await;
        }
        ProcessExecGovernance::Denied(denied) => {
            let error = process_exec_governance_denied_reason(&denied, "process/start");
            let result = match denied {
                ProcessExecGovernanceDenied::SandboxPolicyReadOnly => {
                    process_sandbox_policy_denied_response(tool_id, sandbox_policy)?
                }
                ProcessExecGovernanceDenied::SandboxNetworkDenied => {
                    process_sandbox_network_denied_response(tool_id, sandbox_network_access)?
                }
                ProcessExecGovernanceDenied::GatewayDenied(_) => {
                    process_denied_response(tool_id, params.thread_id, None)?
                }
                ProcessExecGovernanceDenied::UnknownMode {
                    available,
                    load_error,
                } => process_unknown_mode_denied_response(
                    tool_id,
                    params.thread_id,
                    &mode_name,
                    available,
                    load_error,
                )?,
                ProcessExecGovernanceDenied::ModeDenied { mode_decision } => {
                    process_mode_denied_response(tool_id, params.thread_id, &mode_name, mode_decision)?
                }
                ProcessExecGovernanceDenied::ExecPolicyLoad { error, details, .. } => {
                    process_execpolicy_load_denied_response(tool_id, &mode_name, &error, details)?
                }
                ProcessExecGovernanceDenied::ExecPolicyForbidden {
                    matches,
                    justification,
                } => process_execpolicy_denied_response(
                    tool_id,
                    ExecDecision::Forbidden,
                    &matches,
                    justification,
                )?,
                ProcessExecGovernanceDenied::ApprovalDenied { .. } => unreachable!(),
            };
            return emit_process_tool_denied_response(
                &thread_rt,
                tool_id,
                params.turn_id,
                "process/start",
                &start_tool_params,
                error,
                result,
            )
            .await;
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
    combined_env.insert(
        "AGENT_EXEC_GATEWAY_WORKSPACE_ROOT".to_string(),
        thread_root.display().to_string(),
    );

    let mut execve_gate: Option<ExecveGateHandle> = None;
    #[cfg(unix)]
    if let Ok(wrapper_path) = std::env::var("OMNE_EXECVE_WRAPPER") {
        match execve_wrapper_target_for_shell(&params.argv[0], None) {
            ExecveWrapperTarget::Disabled => {}
            ExecveWrapperTarget::Unsupported(reason) => {
                let result = process_denied_response(tool_id, params.thread_id, None)?;
                emit_process_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "process/start",
                    &start_tool_params,
                    reason,
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
            ExecveWrapperTarget::Supported => {
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
    }

    let resolved_request = process_exec_gateway_request(&params.argv, &cwd_path, &thread_root)
        .map(|request| process_exec_gateway().resolve_request(&request));
    let mut cmd = Command::new(
        resolved_request
            .as_ref()
            .filter(|_| !Path::new(&params.argv[0]).is_absolute())
            .map(|request| request.program.as_os_str())
            .unwrap_or_else(|| std::ffi::OsStr::new(&params.argv[0])),
    );
    cmd.args(params.argv.iter().skip(1));
    cmd.current_dir(
        resolved_request
            .as_ref()
            .map(|request| request.cwd.as_path())
            .unwrap_or(cwd_path.as_path()),
    );
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let combined_env_opt = (!combined_env.is_empty()).then_some(&combined_env);
    if let Some(env) = combined_env_opt {
        cmd.envs(env.iter());
    }
    if let Err(err) = prepare_process_exec_gateway_command(
        &params.argv,
        &cwd_path,
        &thread_root,
        sandbox_policy,
        cmd.as_std_mut(),
    )
    {
        if let Some(gate) = execve_gate.take() {
            shutdown_execve_gate(gate).await;
        }
        let result = process_denied_response(tool_id, params.thread_id, None)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/start",
            &start_tool_params,
            process_exec_gateway_error_reason(&err),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }
    if let Some(prepared_cwd) = cmd.as_std().get_current_dir() {
        cwd_str = prepared_cwd.display().to_string();
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
        server: server.clone(),
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
                omne_core::resolve_file(thread_root, path, omne_core::PathAccess::Read, false)
                    .await?;
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
        omne_execpolicy::load_policies(&rule_paths)
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

    async fn wait_for_process_exit(
        server: &Server,
        process_id: ProcessId,
        timeout: Duration,
    ) -> anyhow::Result<ProcessInfo> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let info = match resolve_process_info(server, process_id).await {
                Ok(info) => info,
                Err(err) if err.to_string().contains("process not found") => {
                    if tokio::time::Instant::now() > deadline {
                        return Err(err);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }
                Err(err) => return Err(err),
            };

            if matches!(info.status, ProcessStatus::Exited) {
                return Ok(info);
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("process did not exit in time");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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
    async fn process_start_denies_wrapped_network_commands_when_network_access_is_denied()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        for argv in [
            vec![
                "env".to_string(),
                "FOO=bar".to_string(),
                "curl".to_string(),
                "https://example.com".to_string(),
            ],
            vec![
                "python".to_string(),
                "-c".to_string(),
                "import requests; requests.get('https://example.com')".to_string(),
            ],
            vec![
                "bash".to_string(),
                "-lc".to_string(),
                "echo ok && curl https://example.com".to_string(),
            ],
        ] {
            let result = handle_process_start(
                &server,
                ProcessStartParams {
                    thread_id,
                    turn_id: None,
                    approval_id: None,
                    argv,
                    cwd: None,
                    timeout_ms: None,
                },
            )
            .await?;

            assert!(result["denied"].as_bool().unwrap_or(false));
            assert_eq!(result["sandbox_network_access"].as_str(), Some("deny"));
        }

        assert!(server.processes.lock().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn process_start_allows_local_path_invocations_when_network_access_is_denied()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("tool").as_path(), "#!/bin/sh\nexit 0\n").await?;

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
                argv: vec!["./tool".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;
        let _ = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn process_start_unmatched_execpolicy_requests_approval_when_mode_allows()
    -> anyhow::Result<()> {
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
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("mode-x".to_string()),
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["echo".to_string(), "ok".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        assert_eq!(result["needs_approval"].as_bool(), Some(true));
        let expected_thread_id = thread_id.to_string();
        assert_eq!(result["thread_id"].as_str(), Some(expected_thread_id.as_str()));
        assert!(result["approval_id"].as_str().is_some());
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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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

        let _ = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn process_start_allows_cwd_outside_workspace_with_full_access() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let outside_dir = tmp.path().join("outside");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::create_dir_all(&outside_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: Some(policy_meta::WriteScope::FullAccess),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let result = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    "exit 0".to_string(),
                ],
                cwd: Some(outside_dir.display().to_string()),
                timeout_ms: None,
            },
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;

        let _ = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;

        Ok(())
    }

    #[tokio::test]
    async fn process_start_full_access_still_enforces_allowed_tools() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_executable_sh(repo_dir.join("curl").as_path(), "#!/bin/sh\nexit 0\n").await?;

        let mut exec_policy = omne_execpolicy::Policy::empty();
        exec_policy.add_prefix_rule(&["./curl".to_string()], ExecDecision::Forbidden)?;

        let mut server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        server.exec_policy = exec_policy;

        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoDeny),
                sandbox_policy: Some(policy_meta::WriteScope::FullAccess),
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: Some(Some(vec!["file/read".to_string()])),
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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

        assert_eq!(result["denied"], serde_json::json!(true));
        assert_eq!(result["tool"].as_str(), Some("process/start"));
        assert_eq!(result["allowed_tools"], serde_json::json!(["file/read"]));

        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_start_uses_gateway_prepared_canonical_cwd() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let real_dir = repo_dir.join("real");
        let link_dir = repo_dir.join("link");
        tokio::fs::create_dir_all(&real_dir).await?;
        symlink(&real_dir, &link_dir)?;

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
                argv: vec!["/bin/pwd".to_string()],
                cwd: Some(link_dir.display().to_string()),
                timeout_ms: None,
            },
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;
        let stdout_path = result["stdout_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing stdout_path"))?;

        let info = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;
        let stdout = tokio::fs::read_to_string(stdout_path).await?;
        let expected_cwd = real_dir.canonicalize()?;

        assert_eq!(stdout.trim(), expected_cwd.display().to_string());
        assert_eq!(info.cwd, expected_cwd.display().to_string());

        Ok(())
    }

    #[tokio::test]
    async fn process_start_denies_cwd_outside_workspace_with_workspace_write() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let outside_dir = tmp.path().join("outside");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::create_dir_all(&outside_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let err = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    "exit 0".to_string(),
                ],
                cwd: Some(outside_dir.display().to_string()),
                timeout_ms: None,
            },
        )
        .await
        .expect_err("workspace_write should not allow cwd outside thread root");

        assert!(err.to_string().contains("path escapes root"));
        assert!(server.processes.lock().await.is_empty());

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
        assert!(
            scrubbed_keys
                .iter()
                .any(|k| k.as_str() == Some("OPENAI_API_KEY"))
        );
        let injected_defaults = result["effective_env_summary"]["injected_defaults"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("missing injected_defaults"))?;
        assert!(injected_defaults.is_empty());

        let _ = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;

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
    async fn process_start_evicts_exited_entries_after_exit() -> anyhow::Result<()> {
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
                argv: vec!["echo".to_string(), "hi".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;
        let _ = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        loop {
            let present = server.processes.lock().await.contains_key(&process_id);
            if !present {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("process entry was not evicted in time");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

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

        let _ = wait_for_process_exit(&server, process_id, Duration::from_secs(3)).await?;

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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: Some(vec!["rules/thread.rules".to_string()]),
            clear_execpolicy_rules: false,
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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: Some(vec!["rules/missing.rules".to_string()]),
            clear_execpolicy_rules: false,
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

    #[tokio::test]
    async fn process_start_applies_exec_gateway_prepare_to_spawned_command() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

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
                argv: vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    "test -n \"$AGENT_EXEC_GATEWAY_WORKSPACE_ROOT\"".to_string(),
                ],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;

        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;

        let info = wait_for_process_exit(&server, process_id, Duration::from_secs(2)).await?;
        assert_eq!(info.exit_code, Some(0));

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn execve_wrapper_supports_sh_when_it_resolves_to_bash() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir()?;
        let bash_path = dir.path().join("bash");
        std::fs::write(&bash_path, "#!/bin/sh\nexit 0\n")?;
        let mut perms = std::fs::metadata(&bash_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bash_path, perms)?;
        std::os::unix::fs::symlink(&bash_path, dir.path().join("sh"))?;

        let target = execve_wrapper_target_for_shell(
            "sh",
            Some(dir.path().as_os_str()),
        );
        assert_eq!(target, ExecveWrapperTarget::Supported);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn execve_wrapper_rejects_sh_when_it_resolves_to_non_bash_shell() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir()?;
        let dash_path = dir.path().join("dash");
        std::fs::write(&dash_path, "#!/bin/sh\nexit 0\n")?;
        let mut perms = std::fs::metadata(&dash_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dash_path, perms)?;
        std::os::unix::fs::symlink(&dash_path, dir.path().join("sh"))?;

        let target = execve_wrapper_target_for_shell(
            "sh",
            Some(dir.path().as_os_str()),
        );
        assert!(matches!(
            target,
            ExecveWrapperTarget::Unsupported(reason)
                if reason.contains("requires a bash-compatible shell")
        ));
        Ok(())
    }
}
