async fn handle_approval_decide(
    server: &Server,
    params: ApprovalDecideParams,
) -> anyhow::Result<Value> {
    let rt = server.get_or_load_thread(params.thread_id).await?;
    rt.append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
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

    let mut requested = BTreeMap::<omne_agent_protocol::ApprovalId, serde_json::Value>::new();
    let mut decided = BTreeMap::<omne_agent_protocol::ApprovalId, serde_json::Value>::new();

    for event in events {
        let ts = event.timestamp.format(&Rfc3339)?;
        match event.kind {
            omne_agent_protocol::ThreadEventKind::ApprovalRequested {
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
            omne_agent_protocol::ThreadEventKind::ApprovalDecided {
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
    thread_store: &ThreadStore,
    thread_id: ThreadId,
    approval_id: omne_agent_protocol::ApprovalId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<()> {
    let events = thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut found_request: Option<(String, serde_json::Value)> = None;
    let mut found_decision: Option<omne_agent_protocol::ApprovalDecision> = None;

    for event in events {
        match event.kind {
            omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: got,
                action,
                params,
                ..
            } if got == approval_id => {
                found_request = Some((action, params));
            }
            omne_agent_protocol::ThreadEventKind::ApprovalDecided {
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
        Some(omne_agent_protocol::ApprovalDecision::Approved) => Ok(()),
        Some(omne_agent_protocol::ApprovalDecision::Denied) => {
            anyhow::bail!("approval denied: {}", approval_id)
        }
        None => anyhow::bail!("approval not decided: {}", approval_id),
    }
}

#[derive(Debug)]
enum ApprovalGate {
    Approved,
    Denied { remembered: bool },
    NeedsApproval { approval_id: omne_agent_protocol::ApprovalId },
}

fn approval_denied_error(remembered: bool) -> &'static str {
    if remembered {
        "approval denied (remembered)"
    } else {
        "approval denied"
    }
}

async fn enforce_thread_allowed_tools(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_agent_protocol::ToolId,
    turn_id: Option<TurnId>,
    tool: &str,
    params: Option<serde_json::Value>,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Option<Value>> {
    let Some(allowed_tools) = allowed_tools else {
        return Ok(None);
    };
    if allowed_tools.iter().any(|allowed| allowed == tool) {
        return Ok(None);
    }

    let allowed_json = serde_json::to_string(allowed_tools)
        .unwrap_or_else(|_| format!("{allowed_tools:?}"));
    let error = format!("tool {tool} denied by thread allowed_tools={allowed_json}");

    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: tool.to_string(),
            params,
        })
        .await?;
    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_agent_protocol::ToolStatus::Denied,
            error: Some(error),
            result: Some(serde_json::json!({
                "tool": tool,
                "allowed_tools": allowed_tools,
            })),
        })
        .await?;

    Ok(Some(serde_json::json!({
        "tool_id": tool_id,
        "denied": true,
        "tool": tool,
        "allowed_tools": allowed_tools,
    })))
}

struct ApprovalRequest<'a> {
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    action: &'a str,
    params: &'a serde_json::Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApprovalRequirement {
    Prompt,
    PromptStrict,
}

fn approval_requirement(params: &serde_json::Value) -> ApprovalRequirement {
    let requirement = params
        .as_object()
        .and_then(|obj| obj.get("approval"))
        .and_then(|approval| approval.as_object())
        .and_then(|approval| approval.get("requirement"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or("prompt");
    match requirement {
        "prompt_strict" | "promptStrict" => ApprovalRequirement::PromptStrict,
        _ => ApprovalRequirement::Prompt,
    }
}

async fn gate_approval(
    server: &Server,
    thread_rt: &Arc<ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_policy: omne_agent_protocol::ApprovalPolicy,
    request: ApprovalRequest<'_>,
) -> anyhow::Result<ApprovalGate> {
    gate_approval_with_deps(
        &server.thread_store,
        &server.exec_policy,
        thread_rt,
        thread_id,
        turn_id,
        approval_policy,
        request,
    )
    .await
}

async fn gate_approval_with_deps(
    thread_store: &ThreadStore,
    exec_policy: &omne_agent_execpolicy::Policy,
    thread_rt: &Arc<ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_policy: omne_agent_protocol::ApprovalPolicy,
    request: ApprovalRequest<'_>,
) -> anyhow::Result<ApprovalGate> {
    if let Some(approval_id) = request.approval_id {
        ensure_approval(
            thread_store,
            thread_id,
            approval_id,
            request.action,
            request.params,
        )
        .await?;
        return Ok(ApprovalGate::Approved);
    }

    let requirement = approval_requirement(request.params);
    if requirement == ApprovalRequirement::PromptStrict {
        return match approval_policy {
            omne_agent_protocol::ApprovalPolicy::AutoDeny => {
                let approval_id = omne_agent_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_agent_protocol::ApprovalDecision::Denied,
                        remember: false,
                        reason: Some("auto-denied by policy".to_string()),
                    })
                    .await?;
                Ok(ApprovalGate::Denied { remembered: false })
            }
            _ => {
                let approval_id = omne_agent_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                Ok(ApprovalGate::NeedsApproval { approval_id })
            }
        };
    }

    if let Some(decision) =
        remembered_approval_decision(thread_store, thread_id, request.action, request.params).await?
    {
        let approval_id = omne_agent_protocol::ApprovalId::new();
        let reason = match decision {
            omne_agent_protocol::ApprovalDecision::Approved => "auto-approved by remembered decision",
            omne_agent_protocol::ApprovalDecision::Denied => "auto-denied by remembered decision",
        };

        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action: request.action.to_string(),
                params: request.params.clone(),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember: false,
                reason: Some(reason.to_string()),
            })
            .await?;

        return match decision {
            omne_agent_protocol::ApprovalDecision::Approved => Ok(ApprovalGate::Approved),
            omne_agent_protocol::ApprovalDecision::Denied => Ok(ApprovalGate::Denied { remembered: true }),
        };
    }

    match approval_policy {
        omne_agent_protocol::ApprovalPolicy::AutoApprove | omne_agent_protocol::ApprovalPolicy::OnRequest => {
            let approval_id = omne_agent_protocol::ApprovalId::new();
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_agent_protocol::ApprovalDecision::Approved,
                    remember: false,
                    reason: Some("auto-approved by policy".to_string()),
                })
                .await?;
            Ok(ApprovalGate::Approved)
        }
        omne_agent_protocol::ApprovalPolicy::Manual => {
            let approval_id = omne_agent_protocol::ApprovalId::new();
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            Ok(ApprovalGate::NeedsApproval { approval_id })
        }
        omne_agent_protocol::ApprovalPolicy::UnlessTrusted => {
            let trusted = match request.action {
                "process/start" => {
                    let argv = request
                        .params
                        .as_object()
                        .and_then(|o| o.get("argv"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if argv.is_empty() {
                        false
                    } else {
                        let exec_matches = exec_policy.matches_for_command(&argv, None);
                        let exec_decision = exec_matches.iter().map(ExecRuleMatch::decision).max();
                        let effective_exec_decision = match exec_decision {
                            Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
                            Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
                            Some(ExecDecision::Allow) => ExecDecision::Allow,
                            Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
                        };
                        matches!(effective_exec_decision, ExecDecision::Allow)
                    }
                }
                _ => false,
            };

            if trusted {
                let approval_id = omne_agent_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_agent_protocol::ApprovalDecision::Approved,
                        remember: false,
                        reason: Some("auto-approved by policy (unless_trusted)".to_string()),
                    })
                    .await?;
                Ok(ApprovalGate::Approved)
            } else {
                let approval_id = omne_agent_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                Ok(ApprovalGate::NeedsApproval { approval_id })
            }
        }
        omne_agent_protocol::ApprovalPolicy::AutoDeny => {
            let approval_id = omne_agent_protocol::ApprovalId::new();
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_agent_protocol::ApprovalDecision::Denied,
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
    thread_store: &ThreadStore,
    thread_id: ThreadId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<Option<omne_agent_protocol::ApprovalDecision>> {
    let expected_key = approval_rule_key(expected_action, expected_params)?;
    let events = thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut requested = HashMap::<omne_agent_protocol::ApprovalId, (String, serde_json::Value)>::new();
    let mut remembered = HashMap::<String, omne_agent_protocol::ApprovalDecision>::new();

    for event in events {
        match event.kind {
            omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                action,
                params,
                ..
            } => {
                requested.insert(approval_id, (action, params));
            }
            omne_agent_protocol::ThreadEventKind::ApprovalDecided {
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
                if approval_requirement(params) == ApprovalRequirement::PromptStrict {
                    continue;
                }
                let key = approval_rule_key(action, params)?;
                remembered.insert(key, decision);
            }
            _ => {}
        }
    }

    Ok(remembered.get(&expected_key).copied())
}

#[cfg(test)]
mod approval_prompt_strict_tests {
    use super::*;

    fn build_test_server(agent_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: agent_root.clone(),
            notify_tx,
            notify_hub: default_notify_hub(),
            thread_store: ThreadStore::new(AgentPaths::new(agent_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_agent_execpolicy::Policy::empty(),
            db_vfs: None,
        }
    }

    #[tokio::test]
    async fn prompt_strict_forces_manual_even_when_auto_approve() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join(".omne_agent_data")).await?;
        let server = build_test_server(repo_dir.join(".omne_agent_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let params = serde_json::json!({
            "path": "foo.txt",
            "create_parent_dirs": true,
            "approval": { "requirement": "prompt_strict" },
        });

        let gate = gate_approval(
            &server,
            &thread_rt,
            thread_id,
            None,
            omne_agent_protocol::ApprovalPolicy::AutoApprove,
            ApprovalRequest {
                approval_id: None,
                action: "file/write",
                params: &params,
            },
        )
        .await?;

        let ApprovalGate::NeedsApproval { .. } = gate else {
            anyhow::bail!("expected NeedsApproval, got {gate:?}");
        };

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;
        let mut requested = 0usize;
        let mut decided = 0usize;
        for event in events {
            match event.kind {
                omne_agent_protocol::ThreadEventKind::ApprovalRequested { .. } => requested += 1,
                omne_agent_protocol::ThreadEventKind::ApprovalDecided { .. } => decided += 1,
                _ => {}
            }
        }
        assert_eq!(requested, 1);
        assert_eq!(decided, 0);
        Ok(())
    }

    #[tokio::test]
    async fn prompt_strict_decisions_are_not_remembered() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join(".omne_agent_data")).await?;
        let server = build_test_server(repo_dir.join(".omne_agent_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let strict_approval_id = omne_agent_protocol::ApprovalId::new();
        let strict_params = serde_json::json!({
            "path": "foo.txt",
            "create_parent_dirs": true,
            "approval": { "requirement": "prompt_strict" },
        });
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: strict_approval_id,
                turn_id: None,
                action: "file/write".to_string(),
                params: strict_params,
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: strict_approval_id,
                decision: omne_agent_protocol::ApprovalDecision::Approved,
                remember: true,
                reason: Some("test".to_string()),
            })
            .await?;

        let params = serde_json::json!({
            "path": "foo.txt",
            "create_parent_dirs": true,
        });
        let gate = gate_approval(
            &server,
            &thread_rt,
            thread_id,
            None,
            omne_agent_protocol::ApprovalPolicy::Manual,
            ApprovalRequest {
                approval_id: None,
                action: "file/write",
                params: &params,
            },
        )
        .await?;

        let ApprovalGate::NeedsApproval { .. } = gate else {
            anyhow::bail!("expected NeedsApproval, got {gate:?}");
        };
        Ok(())
    }
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
    let thread_root = omne_agent_core::resolve_dir(Path::new(&thread_cwd), Path::new(".")).await?;
    Ok((thread_rt, thread_root))
}

async fn resolve_dir_for_sandbox(
    thread_root: &Path,
    sandbox_policy: omne_agent_protocol::SandboxPolicy,
    input: &Path,
) -> anyhow::Result<PathBuf> {
    match sandbox_policy {
        omne_agent_protocol::SandboxPolicy::DangerFullAccess => {
            omne_agent_core::resolve_dir_unrestricted(thread_root, input).await
        }
        _ => omne_agent_core::resolve_dir(thread_root, input).await,
    }
}

async fn resolve_file_for_sandbox(
    thread_root: &Path,
    sandbox_policy: omne_agent_protocol::SandboxPolicy,
    sandbox_writable_roots: &[String],
    input: &Path,
    access: omne_agent_core::PathAccess,
    create_parent_dirs: bool,
) -> anyhow::Result<PathBuf> {
    match sandbox_policy {
        omne_agent_protocol::SandboxPolicy::DangerFullAccess => {
            omne_agent_core::resolve_file_unrestricted(thread_root, input, access, create_parent_dirs).await
        }
        _ => {
            if matches!(access, omne_agent_core::PathAccess::Write) && !sandbox_writable_roots.is_empty() {
                let writable_roots = sandbox_writable_roots
                    .iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                omne_agent_core::resolve_file_with_writable_roots(
                    thread_root,
                    &writable_roots,
                    input,
                    access,
                    create_parent_dirs,
                )
                .await
            } else {
                omne_agent_core::resolve_file(thread_root, input, access, create_parent_dirs).await
            }
        }
    }
}

#[derive(Debug)]
struct ToolDenied {
    error: String,
    result: Value,
}

impl ToolDenied {
    fn new(error: impl Into<String>, result: Value) -> Self {
        Self {
            error: error.into(),
            result,
        }
    }
}

impl std::fmt::Display for ToolDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for ToolDenied {}

fn tool_denied(error: impl Into<String>, result: Value) -> anyhow::Error {
    anyhow::Error::new(ToolDenied::new(error, result))
}

fn merge_json_object(mut base: Value, extra: &Value) -> Value {
    let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) else {
        return base;
    };
    for (key, value) in extra_obj {
        if key == "tool_id" || key == "denied" {
            continue;
        }
        base_obj.insert(key.clone(), value.clone());
    }
    base
}

async fn canonical_rel_path_for_write(thread_root: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    let Some(file_name) = path.file_name() else {
        anyhow::bail!("path has no file name: {}", path.display());
    };
    let canon_parent = tokio::fs::canonicalize(parent)
        .await
        .with_context(|| format!("canonicalize {}", parent.display()))?;
    omne_agent_core::modes::relative_path_under_root(thread_root, &canon_parent.join(file_name))
}
