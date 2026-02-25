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

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = process_unknown_mode_denied_response(
                tool_id,
                info.thread_id,
                &mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_process_tool_denied(
                &thread_rt,
                tool_id,
                params.turn_id,
                "process/kill",
                &approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(result);
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(mode, "process/kill", mode.permissions.process.kill);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = process_mode_denied_response(tool_id, info.thread_id, &mode_name, mode_decision)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/kill",
            &approval_params,
            "mode denies process/kill".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            info.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "process/kill",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = process_denied_response(tool_id, info.thread_id, Some(remembered))?;
                emit_process_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "process/kill",
                    &approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return process_needs_approval_response(info.thread_id, approval_id);
            }
        }
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

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = process_unknown_mode_denied_response(
                tool_id,
                info.thread_id,
                &mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_process_tool_denied(
                &thread_rt,
                tool_id,
                params.turn_id,
                "process/interrupt",
                &approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(result);
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(mode, "process/interrupt", mode.permissions.process.kill);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result =
            process_mode_denied_response(tool_id, info.thread_id, &mode_name, mode_decision)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/interrupt",
            &approval_params,
            "mode denies process/interrupt".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            info.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "process/interrupt",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = process_denied_response(tool_id, info.thread_id, Some(remembered))?;
                emit_process_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "process/interrupt",
                    &approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return process_needs_approval_response(info.thread_id, approval_id);
            }
        }
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
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::{broadcast, mpsc};

    fn build_test_server(omne_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: omne_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(omne_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_execpolicy::Policy::empty(),
        }
    }

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

        let server = build_test_server(tmp.path().join(".omne_data"));
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

        let server = build_test_server(tmp.path().join(".omne_data"));
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

        let server = build_test_server(tmp.path().join(".omne_data"));
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
