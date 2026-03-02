async fn handle_process_kill(server: &Server, params: ProcessKillParams) -> anyhow::Result<Value> {
    let entry = {
        let entries = server.processes.lock().await;
        entries.get(&params.process_id).cloned()
    };
    let Some(entry) = entry else {
        anyhow::bail!("process not found: {}", params.process_id);
    };
    let info = entry.info.lock().await.clone();

    let (thread_rt, thread_root) = load_thread_root(server, info.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
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

    let _ = entry
        .cmd_tx
        .send(ProcessCommand::Kill {
            reason: params.reason,
        })
        .await;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({ "ok": true })),
        })
        .await?;

    let response = omne_app_server_protocol::ProcessSignalResponse { ok: true };
    serde_json::to_value(response).context("serialize process/kill response")
}

async fn handle_process_interrupt(
    server: &Server,
    params: ProcessInterruptParams,
) -> anyhow::Result<Value> {
    let entry = {
        let entries = server.processes.lock().await;
        entries.get(&params.process_id).cloned()
    };
    let Some(entry) = entry else {
        anyhow::bail!("process not found: {}", params.process_id);
    };
    let info = entry.info.lock().await.clone();

    let (thread_rt, thread_root) = load_thread_root(server, info.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
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

    let _ = entry
        .cmd_tx
        .send(ProcessCommand::Interrupt {
            reason: params.reason,
        })
        .await;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({ "ok": true })),
        })
        .await?;

    let response = omne_app_server_protocol::ProcessSignalResponse { ok: true };
    serde_json::to_value(response).context("serialize process/interrupt response")
}

async fn kill_processes_for_turn(
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

async fn interrupt_processes_for_turn(
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

async fn shutdown_running_processes(server: &Server) {
    let entries = {
        let entries = server.processes.lock().await;
        entries.values().cloned().collect::<Vec<_>>()
    };

    for entry in entries {
        let should_kill = {
            let info = entry.info.lock().await;
            matches!(info.status, ProcessStatus::Running)
        };
        if should_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("app-server shutdown".to_string()),
                })
                .await;
        }
    }

    tokio::time::sleep(Duration::from_millis(200)).await;
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
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;
        Ok(())
    }

    async fn insert_running_process(server: &Server, thread_id: ThreadId) -> ProcessId {
        let process_id = ProcessId::new();
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        let now = "2026-01-01T00:00:00Z".to_string();
        let entry = ProcessEntry {
            info: Arc::new(tokio::sync::Mutex::new(ProcessInfo {
                process_id,
                thread_id,
                turn_id: None,
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
        let process_id = insert_running_process(&server, thread_id).await;

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
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
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
}
