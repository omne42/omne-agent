use super::*;

#[cfg(unix)]
fn send_os_signal(pid: u32, signal: nix::sys::signal::Signal) -> anyhow::Result<()> {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    kill(Pid::from_raw(pid as i32), signal)
        .with_context(|| format!("send {signal:?} to pid {pid}"))?;
    Ok(())
}

fn queued_process_signal_response(process_id: ProcessId) -> omne_app_server_protocol::ProcessSignalResponse {
    omne_app_server_protocol::ProcessSignalResponse {
        ok: true,
        accepted: true,
        process_id,
        delivery: omne_app_server_protocol::ProcessSignalDelivery::Queued,
    }
}

async fn send_process_signal(
    entry: &ProcessEntry,
    process_id: ProcessId,
    command: ProcessCommand,
) -> anyhow::Result<()> {
    let is_running = {
        let info = entry.info.lock().await;
        matches!(info.status, ProcessStatus::Running)
    };
    if !is_running {
        anyhow::bail!("process is no longer running: {}", process_id);
    }

    entry
        .cmd_tx
        .send(command)
        .await
        .map_err(|_| anyhow::anyhow!("process is no longer running: {}", process_id))
}

enum ProcessSignalTarget {
    Managed(ProcessEntry, ProcessInfo),
    External(ProcessInfo),
}

async fn load_process_signal_target(
    server: &Server,
    process_id: ProcessId,
) -> anyhow::Result<ProcessSignalTarget> {
    let entry = {
        let entries = server.processes.lock().await;
        entries.get(&process_id).cloned()
    };
    if let Some(entry) = entry {
        let info = entry.info.lock().await.clone();
        return Ok(ProcessSignalTarget::Managed(entry, info));
    }

    let info = resolve_process_info(server, process_id).await?;
    if !matches!(info.status, ProcessStatus::Running) {
        anyhow::bail!("process is no longer running: {}", process_id);
    }
    if !info
        .os_pid
        .is_some_and(|os_pid| super::list::os_process_matches_argv(os_pid, &info.argv))
    {
        anyhow::bail!("process is no longer running: {}", process_id);
    }
    Ok(ProcessSignalTarget::External(info))
}

async fn append_external_signal_request(
    thread_rt: &Arc<ThreadRuntime>,
    process_id: ProcessId,
    command: &ProcessCommand,
) -> anyhow::Result<()> {
    let kind = match command {
        ProcessCommand::Interrupt { reason } => {
            omne_protocol::ThreadEventKind::ProcessInterruptRequested {
                process_id,
                reason: reason.clone(),
            }
        }
        ProcessCommand::Kill { reason } => omne_protocol::ThreadEventKind::ProcessKillRequested {
            process_id,
            reason: reason.clone(),
        },
    };
    thread_rt.append_event(kind).await?;
    Ok(())
}

#[cfg(unix)]
async fn wait_for_os_process_exit(pid: u32, timeout: Duration) -> anyhow::Result<()> {
    use nix::errno::Errno;
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
    use nix::unistd::Pid;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match waitpid(Pid::from_raw(pid as i32), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, _))
            | Ok(WaitStatus::Signaled(_, _, _))
            | Ok(WaitStatus::Stopped(_, _))
            | Ok(WaitStatus::Continued(_))
            | Ok(WaitStatus::PtraceEvent(_, _, _))
            | Ok(WaitStatus::PtraceSyscall(_)) => return Ok(()),
            Ok(WaitStatus::StillAlive) => {}
            Err(Errno::ECHILD) => {}
            Err(err) => return Err(anyhow::anyhow!("waitpid {pid}: {err}")),
        }
        if !super::list::os_process_exists(pid) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for pid {pid} to exit");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(super) async fn handle_process_kill(
    server: &Server,
    params: ProcessKillParams,
) -> anyhow::Result<Value> {
    let target = load_process_signal_target(server, params.process_id).await?;
    let info = match &target {
        ProcessSignalTarget::Managed(_, info) | ProcessSignalTarget::External(info) => info.clone(),
    };

    let (thread_rt, thread_root) = load_thread_root(server, info.thread_id).await?;
    let (approval_policy, mode_name, role_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.role.clone(),
            state.allowed_tools.clone(),
        )
    };

    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "process_id": params.process_id,
        "reason": params.reason.clone(),
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "process/kill",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return process_allowed_tools_denied_response(tool_id, "process/kill", &allowed_tools);
    }

    if let Some(result) = enforce_process_mode_and_approval(
        server,
        ProcessModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: info.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            role_name: &role_name,
            action: "process/kill",
            tool_id,
            approval_params: &approval_params,
        },
        |mode| mode.permissions.process.kill,
    )
    .await?
    {
        return Ok(result);
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "process/kill".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let reason = params.reason;
    let signal_result = match target {
        ProcessSignalTarget::Managed(entry, _) => {
            send_process_signal(
                &entry,
                params.process_id,
                ProcessCommand::Kill {
                    reason: reason.clone(),
                },
            )
            .await
        }
        ProcessSignalTarget::External(info) => {
            append_external_signal_request(
                &thread_rt,
                params.process_id,
                &ProcessCommand::Kill {
                    reason: reason.clone(),
                },
            )
            .await?;
            #[cfg(unix)]
            {
                let os_pid = info
                    .os_pid
                    .ok_or_else(|| anyhow::anyhow!("missing process pid"))?;
                send_os_signal(os_pid, nix::sys::signal::Signal::SIGKILL)?;
                wait_for_os_process_exit(os_pid, Duration::from_secs(5)).await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ProcessExited {
                        process_id: params.process_id,
                        exit_code: None,
                        reason: reason.clone(),
                    })
                    .await
                    .map(|_| ())
            }
            #[cfg(not(unix))]
            {
                let _ = info;
                anyhow::bail!("re-attaching to external processes is not supported on this platform")
            }
        }
    };

    match signal_result {
        Ok(()) => {
            let response = queued_process_signal_response(params.process_id);
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(serde_json::to_value(&response)?),
                })
                .await?;

            serde_json::to_value(response).context("serialize process/kill response")
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: None,
                    error: Some(err.to_string()),
                    result: Some(serde_json::json!({
                        "ok": false,
                        "accepted": false,
                        "delivery": "rejected",
                        "process_id": params.process_id,
                    })),
                })
                .await?;
            Err(err)
        }
    }
}

pub(super) async fn handle_process_interrupt(
    server: &Server,
    params: ProcessInterruptParams,
) -> anyhow::Result<Value> {
    let target = load_process_signal_target(server, params.process_id).await?;
    let info = match &target {
        ProcessSignalTarget::Managed(_, info) | ProcessSignalTarget::External(info) => info.clone(),
    };

    let (thread_rt, thread_root) = load_thread_root(server, info.thread_id).await?;
    let (approval_policy, mode_name, role_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.role.clone(),
            state.allowed_tools.clone(),
        )
    };

    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "process_id": params.process_id,
        "reason": params.reason.clone(),
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "process/interrupt",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return process_allowed_tools_denied_response(tool_id, "process/interrupt", &allowed_tools);
    }

    if let Some(result) = enforce_process_mode_and_approval(
        server,
        ProcessModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: info.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            role_name: &role_name,
            action: "process/interrupt",
            tool_id,
            approval_params: &approval_params,
        },
        |mode| mode.permissions.process.kill,
    )
    .await?
    {
        return Ok(result);
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "process/interrupt".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let reason = params.reason;
    let signal_result = match target {
        ProcessSignalTarget::Managed(entry, _) => {
            send_process_signal(
                &entry,
                params.process_id,
                ProcessCommand::Interrupt {
                    reason: reason.clone(),
                },
            )
            .await
        }
        ProcessSignalTarget::External(info) => {
            append_external_signal_request(
                &thread_rt,
                params.process_id,
                &ProcessCommand::Interrupt {
                    reason: reason.clone(),
                },
            )
            .await?;
            #[cfg(unix)]
            {
                send_os_signal(
                    info.os_pid.ok_or_else(|| anyhow::anyhow!("missing process pid"))?,
                    nix::sys::signal::Signal::SIGINT,
                )
            }
            #[cfg(not(unix))]
            {
                let _ = info;
                anyhow::bail!("re-attaching to external processes is not supported on this platform")
            }
        }
    };

    match signal_result {
        Ok(()) => {
            let response = queued_process_signal_response(params.process_id);
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(serde_json::to_value(&response)?),
                })
                .await?;

            serde_json::to_value(response).context("serialize process/interrupt response")
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: None,
                    error: Some(err.to_string()),
                    result: Some(serde_json::json!({
                        "ok": false,
                        "accepted": false,
                        "delivery": "rejected",
                        "process_id": params.process_id,
                    })),
                })
                .await?;
            Err(err)
        }
    }
}

pub(super) async fn kill_processes_for_turn(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    reason: Option<String>,
) {
    let entries = {
        let entries = server.processes.lock().await;
        entries.values().cloned().collect::<Vec<_>>()
    };

    for entry in entries {
        let should_kill = {
            let info = entry.info.lock().await;
            info.thread_id == thread_id
                && info.turn_id == Some(turn_id)
                && matches!(info.status, ProcessStatus::Running)
        };
        if should_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: reason.clone(),
                })
                .await;
        }
    }
}

pub(super) async fn interrupt_processes_for_turn(
    server: &Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    reason: Option<String>,
) {
    let entries = {
        let entries = server.processes.lock().await;
        entries.values().cloned().collect::<Vec<_>>()
    };

    for entry in entries {
        let should_interrupt = {
            let info = entry.info.lock().await;
            info.thread_id == thread_id
                && info.turn_id == Some(turn_id)
                && matches!(info.status, ProcessStatus::Running)
        };
        if should_interrupt {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Interrupt {
                    reason: reason.clone(),
                })
                .await;
        }
    }
}

pub(super) async fn shutdown_running_processes(server: &Server) {
    let entries = {
        let entries = server.processes.lock().await;
        entries
            .iter()
            .map(|(process_id, entry)| (*process_id, entry.clone()))
            .collect::<Vec<_>>()
    };

    let mut running_process_ids = Vec::new();
    let mut running_entries = Vec::new();
    for (process_id, entry) in entries {
        let should_kill = {
            let info = entry.info.lock().await;
            matches!(info.status, ProcessStatus::Running)
        };
        if should_kill {
            running_process_ids.push(process_id);
            running_entries.push(entry.clone());
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("app-server shutdown".to_string()),
                })
                .await;
        }
    }

    if running_entries.is_empty() {
        return;
    }
    if let Err(err) = wait_for_process_entries_to_complete(
        &running_entries,
        &running_process_ids,
        "app-server shutdown",
        Duration::from_secs(10),
    )
    .await
    {
        tracing::warn!(error = %err, "timed out waiting for processes during app-server shutdown");
    }
}

#[cfg(test)]
mod process_signal_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    async fn configure_thread_mode(
        server: &Server,
        thread_id: ThreadId,
        mode: &str,
    ) -> anyhow::Result<()> {
        handle_thread_configure(
            server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some(mode.to_string()),
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
        Ok(())
    }

    async fn insert_running_process(server: &Server, thread_id: ThreadId) -> ProcessId {
        let process_id = ProcessId::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let mut cmd_rx = cmd_rx;
            while cmd_rx.recv().await.is_some() {}
        });
        let now = "2026-01-01T00:00:00Z".to_string();
        let entry = ProcessEntry {
            thread_id,
            info: Arc::new(tokio::sync::Mutex::new(ProcessInfo {
                process_id,
                thread_id,
                turn_id: None,
                os_pid: None,
                argv: vec!["sleep".to_string(), "999".to_string()],
                cwd: "/tmp".to_string(),
                started_at: now.clone(),
                status: ProcessStatus::Running,
                exit_code: None,
                stdout_path: "/tmp/omne-test.stdout.log".to_string(),
                stderr_path: "/tmp/omne-test.stderr.log".to_string(),
                last_update_at: now,
            })),
            cmd_tx,
            completion: ProcessCompletion::new(),
        };
        server.processes.lock().await.insert(process_id, entry);
        process_id
    }

    async fn insert_process_with_status(
        server: &Server,
        thread_id: ThreadId,
        status: ProcessStatus,
        drop_receiver: bool,
    ) -> ProcessId {
        let process_id = ProcessId::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        if !drop_receiver {
            tokio::spawn(async move {
                let mut cmd_rx = cmd_rx;
                while cmd_rx.recv().await.is_some() {}
            });
        }
        let now = "2026-01-01T00:00:00Z".to_string();
        let entry = ProcessEntry {
            thread_id,
            info: Arc::new(tokio::sync::Mutex::new(ProcessInfo {
                process_id,
                thread_id,
                turn_id: None,
                os_pid: None,
                argv: vec!["sleep".to_string(), "999".to_string()],
                cwd: "/tmp".to_string(),
                started_at: now.clone(),
                status,
                exit_code: None,
                stdout_path: "/tmp/omne-test.stdout.log".to_string(),
                stderr_path: "/tmp/omne-test.stderr.log".to_string(),
                last_update_at: now,
            })),
            cmd_tx,
            completion: ProcessCompletion::new(),
        };
        server.processes.lock().await.insert(process_id, entry);
        process_id
    }

    #[tokio::test]
    async fn process_kill_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
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
      process:
        kill: { decision: allow }
    tool_overrides:
      - tool: "process/kill"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        configure_thread_mode(&server, thread_id, "mode-x").await?;
        let process_id =
            insert_process_with_status(&server, thread_id, ProcessStatus::Running, false).await;

        let result = handle_process_kill(
            &server,
            ProcessKillParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: None,
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
    async fn process_kill_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = insert_running_process(&server, thread_id).await;

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

        let result = handle_process_kill(
            &server,
            ProcessKillParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("process/kill"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn process_kill_fails_for_exited_process() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        configure_thread_mode(&server, thread_id, "code").await?;
        let process_id =
            insert_process_with_status(&server, thread_id, ProcessStatus::Exited, false).await;

        let err = handle_process_kill(
            &server,
            ProcessKillParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: None,
            },
        )
        .await
        .expect_err("exited process should fail");

        assert!(err.to_string().contains("process is no longer running"));
        Ok(())
    }

    #[tokio::test]
    async fn process_kill_returns_queued_response_when_signal_is_accepted() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        configure_thread_mode(&server, thread_id, "code").await?;
        let process_id = insert_running_process(&server, thread_id).await;

        let result = handle_process_kill(
            &server,
            ProcessKillParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: Some("shutdown".to_string()),
            },
        )
        .await?;
        let response: omne_app_server_protocol::ProcessSignalResponse = serde_json::from_value(result)?;

        assert!(response.ok);
        assert!(response.accepted);
        assert_eq!(response.process_id, process_id);
        assert_eq!(
            response.delivery,
            omne_app_server_protocol::ProcessSignalDelivery::Queued
        );
        Ok(())
    }

    #[tokio::test]
    async fn process_interrupt_fails_when_command_channel_is_closed() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        configure_thread_mode(&server, thread_id, "code").await?;
        let process_id =
            insert_process_with_status(&server, thread_id, ProcessStatus::Running, true).await;

        let err = handle_process_interrupt(
            &server,
            ProcessInterruptParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: None,
            },
        )
        .await
        .expect_err("closed command channel should fail");

        assert!(err.to_string().contains("process is no longer running"));
        Ok(())
    }

    #[tokio::test]
    async fn process_kill_reports_exited_history_even_after_registry_eviction()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;
        let rt = server.get_or_load_thread(thread_id).await?;
        let process_id = ProcessId::new();

        rt.append_event(omne_protocol::ThreadEventKind::ProcessStarted {
            process_id,
            turn_id: None,
            os_pid: None,
            argv: vec!["echo".to_string(), "ok".to_string()],
            cwd: "/tmp".to_string(),
            stdout_path: "/tmp/stdout.log".to_string(),
            stderr_path: "/tmp/stderr.log".to_string(),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::ProcessExited {
            process_id,
            exit_code: Some(0),
            reason: Some("completed".to_string()),
        })
        .await?;

        let err = handle_process_kill(
            &server,
            ProcessKillParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: None,
            },
        )
        .await
        .expect_err("evicted exited process should not look missing");

        assert!(err.to_string().contains("process is no longer running"));
        Ok(())
    }

    #[tokio::test]
    async fn process_interrupt_denied_by_tool_override_reports_decision_source()
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
      process:
        kill: { decision: allow }
    tool_overrides:
      - tool: "process/interrupt"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        configure_thread_mode(&server, thread_id, "mode-x").await?;
        let process_id = insert_running_process(&server, thread_id).await;

        let result = handle_process_interrupt(
            &server,
            ProcessInterruptParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: None,
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
    async fn shutdown_running_processes_waits_for_actor_completion() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = ProcessId::new();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
        let completion = ProcessCompletion::new();
        let now = "2026-01-01T00:00:00Z".to_string();
        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id,
                info: Arc::new(tokio::sync::Mutex::new(ProcessInfo {
                    process_id,
                    thread_id,
                    turn_id: None,
                    os_pid: None,
                    argv: vec!["sleep".to_string(), "999".to_string()],
                    cwd: "/tmp".to_string(),
                    started_at: now.clone(),
                    status: ProcessStatus::Running,
                    exit_code: None,
                    stdout_path: "/tmp/omne-test.stdout.log".to_string(),
                    stderr_path: "/tmp/omne-test.stderr.log".to_string(),
                    last_update_at: now,
                })),
                cmd_tx,
                completion: completion.clone(),
            },
        );

        let server_for_actor = server.clone();
        tokio::spawn(async move {
            let command = cmd_rx.recv().await;
            assert!(matches!(command, Some(ProcessCommand::Kill { .. })));
            tokio::time::sleep(Duration::from_millis(250)).await;
            server_for_actor.processes.lock().await.remove(&process_id);
            completion.mark_complete();
        });

        let started = tokio::time::Instant::now();
        shutdown_running_processes(&server).await;

        assert!(started.elapsed() >= Duration::from_millis(200));
        assert!(!server.processes.lock().await.contains_key(&process_id));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_kill_reuses_live_os_pid_without_registry_entry() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let child = std::process::Command::new("sleep").arg("30").spawn()?;
        let os_pid = child.id();
        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        let thread_rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        server.threads.lock().await.insert(thread_id, thread_rt.clone());

        let process_id = ProcessId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                os_pid: Some(os_pid),
                argv: vec!["sleep".to_string(), "30".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: repo_dir.join("stdout.log").display().to_string(),
                stderr_path: repo_dir.join("stderr.log").display().to_string(),
            })
            .await?;

        let result = handle_process_kill(
            &server,
            ProcessKillParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: Some("test".to_string()),
            },
        )
        .await?;
        let response: omne_app_server_protocol::ProcessSignalResponse =
            serde_json::from_value(result)?;
        assert!(response.ok);
        assert!(response.accepted);

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ProcessKillRequested {
                    process_id: got,
                    reason: Some(reason),
                } if *got == process_id && reason == "test"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ProcessExited {
                    process_id: got,
                    reason: Some(reason),
                    ..
                } if *got == process_id && reason == "test"
            )
        }));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_interrupt_reuses_live_os_pid_without_registry_entry() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let child = std::process::Command::new("sleep").arg("30").spawn()?;
        let os_pid = child.id();
        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        let thread_rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        server.threads.lock().await.insert(thread_id, thread_rt.clone());

        let process_id = ProcessId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                os_pid: Some(os_pid),
                argv: vec!["sleep".to_string(), "30".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: repo_dir.join("stdout.log").display().to_string(),
                stderr_path: repo_dir.join("stderr.log").display().to_string(),
            })
            .await?;

        let result = handle_process_interrupt(
            &server,
            ProcessInterruptParams {
                process_id,
                turn_id: None,
                approval_id: None,
                reason: Some("test".to_string()),
            },
        )
        .await?;
        let response: omne_app_server_protocol::ProcessSignalResponse =
            serde_json::from_value(result)?;
        assert!(response.ok);
        assert!(response.accepted);

        let wait_status = tokio::task::spawn_blocking(move || {
            nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(os_pid as i32), None)
        })
        .await??;
        assert!(matches!(
            wait_status,
            nix::sys::wait::WaitStatus::Signaled(
                _,
                nix::sys::signal::Signal::SIGINT,
                _
            )
        ));
        Ok(())
    }
}
