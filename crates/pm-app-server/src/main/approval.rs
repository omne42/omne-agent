async fn handle_approval_decide(
    server: &Server,
    params: ApprovalDecideParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    rt.append_event(pm_protocol::ThreadEventKind::ApprovalDecided {
        approval_id: params.approval_id,
        decision: params.decision,
        remember: params.remember,
        reason: params.reason,
    })
    .await?;
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_approval_list(
    server: &Server,
    params: ApprovalListParams,
) -> anyhow::Result<Value> {
    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let mut requested = BTreeMap::<pm_protocol::ApprovalId, serde_json::Value>::new();
    let mut decided = BTreeMap::<pm_protocol::ApprovalId, serde_json::Value>::new();

    for event in events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action,
                params,
            } => {
                requested.insert(
                    approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "turn_id": turn_id,
                        "action": action,
                        "params": params,
                        "requested_at": ts,
                    }),
                );
            }
            pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember,
                reason,
            } => {
                decided.insert(
                    approval_id,
                    serde_json::json!({
                        "approval_id": approval_id,
                        "decision": decision,
                        "remember": remember,
                        "reason": reason,
                        "decided_at": ts,
                    }),
                );
            }
            _ => {}
        }
    }

    let mut approvals = Vec::new();
    for (id, req) in requested {
        if let Some(decision) = decided.get(&id) {
            if params.include_decided {
                approvals.push(serde_json::json!({
                    "request": req,
                    "decision": decision,
                }));
            }
        } else {
            approvals.push(serde_json::json!({
                "request": req,
                "decision": null,
            }));
        }
    }

    Ok(serde_json::json!({ "approvals": approvals }))
}

async fn ensure_approval(
    server: &Server,
    thread_id: ThreadId,
    approval_id: pm_protocol::ApprovalId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<()> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut found_request: Option<(String, serde_json::Value)> = None;
    let mut found_decision: Option<pm_protocol::ApprovalDecision> = None;

    for event in events {
        match event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: got,
                action,
                params,
                ..
            } if got == approval_id => {
                found_request = Some((action, params));
            }
            pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: got,
                decision,
                ..
            } if got == approval_id => {
                found_decision = Some(decision);
            }
            _ => {}
        }
    }

    let Some((action, params)) = found_request else {
        anyhow::bail!("approval not requested: {}", approval_id);
    };
    if action != expected_action {
        anyhow::bail!(
            "approval action mismatch: expected {}, got {}",
            expected_action,
            action
        );
    }
    if &params != expected_params {
        anyhow::bail!("approval params mismatch for {}", approval_id);
    }

    match found_decision {
        Some(pm_protocol::ApprovalDecision::Approved) => Ok(()),
        Some(pm_protocol::ApprovalDecision::Denied) => {
            anyhow::bail!("approval denied: {}", approval_id)
        }
        None => anyhow::bail!("approval not decided: {}", approval_id),
    }
}

enum ApprovalGate {
    Approved,
    Denied { remembered: bool },
    NeedsApproval { approval_id: pm_protocol::ApprovalId },
}

fn approval_denied_error(remembered: bool) -> &'static str {
    if remembered {
        "approval denied (remembered)"
    } else {
        "approval denied"
    }
}

struct ApprovalRequest<'a> {
    approval_id: Option<pm_protocol::ApprovalId>,
    action: &'a str,
    params: &'a serde_json::Value,
}

async fn gate_approval(
    server: &Server,
    thread_rt: &Arc<ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_policy: pm_protocol::ApprovalPolicy,
    request: ApprovalRequest<'_>,
) -> anyhow::Result<ApprovalGate> {
    if let Some(approval_id) = request.approval_id {
        ensure_approval(
            server,
            thread_id,
            approval_id,
            request.action,
            request.params,
        )
        .await?;
        return Ok(ApprovalGate::Approved);
    }

    if let Some(decision) =
        remembered_approval_decision(server, thread_id, request.action, request.params).await?
    {
        let approval_id = pm_protocol::ApprovalId::new();
        let reason = match decision {
            pm_protocol::ApprovalDecision::Approved => "auto-approved by remembered decision",
            pm_protocol::ApprovalDecision::Denied => "auto-denied by remembered decision",
        };

        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action: request.action.to_string(),
                params: request.params.clone(),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember: false,
                reason: Some(reason.to_string()),
            })
            .await?;

        return match decision {
            pm_protocol::ApprovalDecision::Approved => Ok(ApprovalGate::Approved),
            pm_protocol::ApprovalDecision::Denied => Ok(ApprovalGate::Denied { remembered: true }),
        };
    }

    match approval_policy {
        pm_protocol::ApprovalPolicy::AutoApprove => {
            let approval_id = pm_protocol::ApprovalId::new();
            thread_rt
            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action: request.action.to_string(),
                params: request.params.clone(),
            })
            .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: pm_protocol::ApprovalDecision::Approved,
                    remember: false,
                    reason: Some("auto-approved by policy".to_string()),
                })
                .await?;
            Ok(ApprovalGate::Approved)
        }
        pm_protocol::ApprovalPolicy::Manual => {
            let approval_id = pm_protocol::ApprovalId::new();
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            Ok(ApprovalGate::NeedsApproval { approval_id })
        }
        pm_protocol::ApprovalPolicy::AutoDeny => {
            let approval_id = pm_protocol::ApprovalId::new();
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: pm_protocol::ApprovalDecision::Denied,
                    remember: false,
                    reason: Some("auto-denied by policy".to_string()),
                })
                .await?;
            Ok(ApprovalGate::Denied { remembered: false })
        }
    }
}

fn approval_rule_key(action: &str, params: &serde_json::Value) -> anyhow::Result<String> {
    let obj = params.as_object();
    match action {
        "file/write" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let create_parent_dirs = obj
                .and_then(|o| o.get("create_parent_dirs"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(format!(
                "file/write|path={path}|create_parent_dirs={create_parent_dirs}"
            ))
        }
        "file/delete" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let recursive = obj
                .and_then(|o| o.get("recursive"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(format!("file/delete|path={path}|recursive={recursive}"))
        }
        "fs/mkdir" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let recursive = obj
                .and_then(|o| o.get("recursive"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(format!("fs/mkdir|path={path}|recursive={recursive}"))
        }
        "file/edit" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(format!("file/edit|path={path}"))
        }
        "file/patch" => {
            let path = obj
                .and_then(|o| o.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(format!("file/patch|path={path}"))
        }
        "process/start" => {
            let serialized = serde_json::to_string(params).context("serialize approval params")?;
            Ok(format!("process/start|{serialized}"))
        }
        other => {
            let serialized = serde_json::to_string(params).context("serialize approval params")?;
            Ok(format!("{other}|{serialized}"))
        }
    }
}

async fn remembered_approval_decision(
    server: &Server,
    thread_id: ThreadId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<Option<pm_protocol::ApprovalDecision>> {
    let expected_key = approval_rule_key(expected_action, expected_params)?;
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut requested = HashMap::<pm_protocol::ApprovalId, (String, serde_json::Value)>::new();
    let mut remembered = HashMap::<String, pm_protocol::ApprovalDecision>::new();

    for event in events {
        match event.kind {
            pm_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                action,
                params,
                ..
            } => {
                requested.insert(approval_id, (action, params));
            }
            pm_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember,
                ..
            } => {
                if !remember {
                    continue;
                }
                let Some((action, params)) = requested.get(&approval_id) else {
                    continue;
                };
                let key = approval_rule_key(action, params)?;
                remembered.insert(key, decision);
            }
            _ => {}
        }
    }

    Ok(remembered.get(&expected_key).copied())
}

async fn load_thread_root(
    server: &Server,
    thread_id: ThreadId,
) -> anyhow::Result<(Arc<ThreadRuntime>, PathBuf)> {
    let thread_rt = server.get_or_load_thread(thread_id).await?;
    let thread_cwd = {
        let handle = thread_rt.handle.lock().await;
        handle
            .state()
            .cwd
            .clone()
            .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", thread_id))?
    };
    let thread_root = pm_core::resolve_dir(Path::new(&thread_cwd), Path::new(".")).await?;
    Ok((thread_rt, thread_root))
}

async fn resolve_dir_for_sandbox(
    thread_root: &Path,
    sandbox_policy: pm_protocol::SandboxPolicy,
    input: &Path,
) -> anyhow::Result<PathBuf> {
    match sandbox_policy {
        pm_protocol::SandboxPolicy::DangerFullAccess => {
            pm_core::resolve_dir_unrestricted(thread_root, input).await
        }
        _ => pm_core::resolve_dir(thread_root, input).await,
    }
}

async fn resolve_file_for_sandbox(
    thread_root: &Path,
    sandbox_policy: pm_protocol::SandboxPolicy,
    input: &Path,
    access: pm_core::PathAccess,
    create_parent_dirs: bool,
) -> anyhow::Result<PathBuf> {
    match sandbox_policy {
        pm_protocol::SandboxPolicy::DangerFullAccess => {
            pm_core::resolve_file_unrestricted(thread_root, input, access, create_parent_dirs).await
        }
        _ => pm_core::resolve_file(thread_root, input, access, create_parent_dirs).await,
    }
}
