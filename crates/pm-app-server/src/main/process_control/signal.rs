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
    let (approval_policy, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone())
    };

    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "process_id": params.process_id,
        "reason": params.reason.clone(),
    });

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "process/kill".to_string(),
                    params: Some(approval_params),
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

    let base_decision = mode.permissions.process.kill;
    let effective_decision = match mode.tool_overrides.get("process/kill").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/kill".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies process/kill".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
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
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "process/kill".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some("approval denied (remembered)".to_string()),
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
                    "thread_id": info.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
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
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({ "ok": true })),
        })
        .await?;

    Ok(serde_json::json!({ "ok": true }))
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
    let (approval_policy, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone())
    };

    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "process_id": params.process_id,
        "reason": params.reason.clone(),
    });

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "process/interrupt".to_string(),
                    params: Some(approval_params),
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

    let base_decision = mode.permissions.process.kill;
    let effective_decision = match mode.tool_overrides.get("process/interrupt").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "process/interrupt".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies process/interrupt".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
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
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "process/interrupt".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
                        error: Some("approval denied (remembered)".to_string()),
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
                    "thread_id": info.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
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
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({ "ok": true })),
        })
        .await?;

    Ok(serde_json::json!({ "ok": true }))
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
