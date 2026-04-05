pub(crate) const SUBAGENT_PROXY_FORWARDED_REASON_PREFIX: &str = "[subagent_proxy_forwarded]";
pub(crate) const SUBAGENT_PROXY_AUTO_DENIED_REASON_PREFIX: &str = "[subagent_proxy_auto_denied]";

fn decorate_subagent_proxy_forwarded_reason(reason: Option<&str>) -> String {
    let suffix = reason.unwrap_or_default().trim();
    if suffix.is_empty() {
        SUBAGENT_PROXY_FORWARDED_REASON_PREFIX.to_string()
    } else {
        format!("{SUBAGENT_PROXY_FORWARDED_REASON_PREFIX} {suffix}")
    }
}

async fn handle_approval_decide(
    server: &Server,
    params: ApprovalDecideParams,
) -> anyhow::Result<Value> {
    let proxy_route = resolve_pending_approval_proxy(
        &server.thread_store,
        params.thread_id,
        params.approval_id,
    )
    .await?;
    if let Some(proxy_route) = proxy_route {
        ensure_pending_approval(
            &server.thread_store,
            proxy_route.child_thread_id,
            proxy_route.child_approval_id,
        )
        .await?;
        let child_rt = server
            .get_or_load_thread(proxy_route.child_thread_id)
            .await?;
        child_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: proxy_route.child_approval_id,
                decision: params.decision,
                remember: params.remember,
                reason: Some(decorate_subagent_proxy_forwarded_reason(
                    params.reason.as_deref(),
                )),
            })
            .await?;
        let parent_rt = server.get_or_load_thread(params.thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: params.approval_id,
                decision: params.decision,
                remember: params.remember,
                reason: params.reason.clone(),
            })
            .await?;
        return serde_json::to_value(omne_app_server_protocol::ApprovalDecideResponse {
            ok: true,
            forwarded: true,
            child_thread_id: Some(proxy_route.child_thread_id),
            child_approval_id: Some(proxy_route.child_approval_id),
        })
        .context("serialize approval/decide forwarded response");
    }

    let rt = server.get_or_load_thread(params.thread_id).await?;
    rt.append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
        approval_id: params.approval_id,
        decision: params.decision,
        remember: params.remember,
        reason: params.reason,
    })
    .await?;
    serde_json::to_value(omne_app_server_protocol::ApprovalDecideResponse {
        ok: true,
        forwarded: false,
        child_thread_id: None,
        child_approval_id: None,
    })
    .context("serialize approval/decide response")
}

#[derive(Debug, Clone, Copy)]
struct SubagentApprovalProxy {
    child_thread_id: ThreadId,
    child_approval_id: omne_protocol::ApprovalId,
}

#[derive(Clone)]
struct ApprovalRequestRecord {
    turn_id: Option<TurnId>,
    action: String,
    params: Value,
    requested_at: String,
}

#[derive(Clone)]
struct ApprovalDecisionRecord {
    decision: omne_protocol::ApprovalDecision,
    remember: bool,
    reason: Option<String>,
    decided_at: String,
}

#[derive(Default)]
struct ApprovalEventIndex {
    requested: BTreeMap<omne_protocol::ApprovalId, ApprovalRequestRecord>,
    decided: BTreeMap<omne_protocol::ApprovalId, ApprovalDecisionRecord>,
}

impl ApprovalEventIndex {
    fn from_events(events: &[omne_protocol::ThreadEvent]) -> anyhow::Result<Self> {
        let mut out = Self::default();
        for event in events {
            let ts = event.timestamp.format(&Rfc3339)?;
            match &event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action,
                    params,
                } => {
                    out.requested.insert(
                        *approval_id,
                        ApprovalRequestRecord {
                            turn_id: *turn_id,
                            action: action.clone(),
                            params: params.clone(),
                            requested_at: ts,
                        },
                    );
                }
                omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    remember,
                    reason,
                } => {
                    out.decided.insert(
                        *approval_id,
                        ApprovalDecisionRecord {
                            decision: *decision,
                            remember: *remember,
                            reason: reason.clone(),
                            decided_at: ts,
                        },
                    );
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn ensure_pending(
        &self,
        thread_id: ThreadId,
        approval_id: omne_protocol::ApprovalId,
    ) -> anyhow::Result<()> {
        if !self.requested.contains_key(&approval_id) {
            anyhow::bail!("approval not requested: thread_id={thread_id} approval_id={approval_id}");
        }
        if self.decided.contains_key(&approval_id) {
            anyhow::bail!("approval already decided: thread_id={thread_id} approval_id={approval_id}");
        }
        Ok(())
    }
}

fn parse_subagent_approval_proxy(params: &Value) -> Option<SubagentApprovalProxy> {
    let proxy = params.get("subagent_proxy")?.as_object()?;
    if proxy.get("kind").and_then(Value::as_str) != Some("approval") {
        return None;
    }
    let child_thread_id = proxy
        .get("child_thread_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    let child_approval_id = proxy
        .get("child_approval_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    Some(SubagentApprovalProxy {
        child_thread_id,
        child_approval_id,
    })
}

async fn resolve_pending_approval_proxy(
    thread_store: &ThreadStore,
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Option<SubagentApprovalProxy>> {
    let events = thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    let index = ApprovalEventIndex::from_events(&events)?;
    index.ensure_pending(thread_id, approval_id)?;

    let mut proxy: Option<SubagentApprovalProxy> = None;
    for event in events {
        if let omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: got,
            params,
            ..
        } = event.kind
        {
            if got != approval_id {
                continue;
            }
            proxy = parse_subagent_approval_proxy(&params);
        }
    }

    Ok(proxy)
}

async fn ensure_pending_approval(
    thread_store: &ThreadStore,
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<()> {
    let events = thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    let index = ApprovalEventIndex::from_events(&events)?;
    index.ensure_pending(thread_id, approval_id)
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

    let index = ApprovalEventIndex::from_events(&events)?;
    let ApprovalEventIndex { requested, decided } = index;

    let mut approvals = Vec::<omne_app_server_protocol::ApprovalListItem>::new();
    for (id, req) in requested {
        let request = omne_app_server_protocol::ApprovalRequestInfo {
            approval_id: id,
            turn_id: req.turn_id,
            action: req.action.clone(),
            action_id: Some(parse_thread_approval_action_id(&req.action)),
            params: req.params.clone(),
            summary: summarize_pending_approval(&req.params),
            requested_at: req.requested_at,
        };
        if let Some(decision) = decided.get(&id) {
            if params.include_decided {
                approvals.push(omne_app_server_protocol::ApprovalListItem {
                    request,
                    decision: Some(omne_app_server_protocol::ApprovalDecisionInfo {
                        decision: decision.decision,
                        remember: decision.remember,
                        reason: decision.reason.clone(),
                        decided_at: decision.decided_at.clone(),
                    }),
                });
            }
        } else {
            approvals.push(omne_app_server_protocol::ApprovalListItem {
                request,
                decision: None,
            });
        }
    }

    serde_json::to_value(omne_app_server_protocol::ApprovalListResponse { approvals })
        .context("serialize approval/list response")
}

async fn ensure_approval(
    thread_store: &ThreadStore,
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
    expected_action: &str,
    expected_params: &serde_json::Value,
) -> anyhow::Result<()> {
    let events = thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let index = ApprovalEventIndex::from_events(&events)?;
    let Some(request) = index.requested.get(&approval_id) else {
        anyhow::bail!("approval not requested: {}", approval_id);
    };
    if request.action != expected_action {
        anyhow::bail!(
            "approval action mismatch: expected {}, got {}",
            expected_action,
            request.action
        );
    }
    if &request.params != expected_params {
        anyhow::bail!("approval params mismatch for {}", approval_id);
    }

    match index.decided.get(&approval_id).map(|d| d.decision) {
        Some(omne_protocol::ApprovalDecision::Approved) => Ok(()),
        Some(omne_protocol::ApprovalDecision::Denied) => {
            anyhow::bail!("approval denied: {}", approval_id)
        }
        None => anyhow::bail!("approval not decided: {}", approval_id),
    }
}

#[derive(Debug)]
enum ApprovalGate {
    Approved,
    Denied { remembered: bool },
    NeedsApproval { approval_id: omne_protocol::ApprovalId },
}

fn approval_denied_error(remembered: bool) -> &'static str {
    if remembered {
        "approval denied (remembered)"
    } else {
        "approval denied"
    }
}

async fn emit_tool_denied(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
    turn_id: Option<TurnId>,
    action: &str,
    params: Option<Value>,
    error: String,
    result: Value,
) -> anyhow::Result<()> {
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: action.to_string(),
            params,
        })
        .await?;
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Denied,
            structured_error: structured_error_from_result_value(&result),
            error: Some(error),
            result: Some(result),
        })
        .await?;
    Ok(())
}

fn denied_response_with_remembered<T, F>(
    tool_id: omne_protocol::ToolId,
    remembered: Option<bool>,
    context: &'static str,
    build: F,
) -> anyhow::Result<Value>
where
    T: serde::Serialize,
    F: FnOnce(
        omne_protocol::ToolId,
        Option<bool>,
        Option<structured_text_protocol::StructuredTextData>,
        Option<String>,
    ) -> T,
{
    let structured_error = catalog_structured_error_with("approval_denied", |message| {
        if let Some(remembered) = remembered {
            message.try_with_value_arg("remembered", remembered)?;
        }
        Ok(())
    })?;
    let error_code = structured_error_code(&structured_error);
    let response = build(tool_id, remembered, Some(structured_error), error_code);
    serde_json::to_value(response).context(context)
}

fn needs_approval_response_json<T, F>(
    approval_id: omne_protocol::ApprovalId,
    context: &'static str,
    build: F,
) -> anyhow::Result<Value>
where
    T: serde::Serialize,
    F: FnOnce(omne_protocol::ApprovalId) -> T,
{
    let response = build(approval_id);
    serde_json::to_value(response).context(context)
}

async fn enforce_thread_allowed_tools(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
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
    let structured_error = catalog_structured_error_with("allowed_tools_denied", |message| {
        message.try_with_value_arg("tool", tool)?;
        Ok(())
    })?;
    let denied_result = serde_json::json!({
        "tool": tool,
        "allowed_tools": allowed_tools,
        "structured_error": structured_error,
        "error_code": "allowed_tools_denied",
    });

    emit_tool_denied(
        thread_rt,
        tool_id,
        turn_id,
        tool,
        params,
        error,
        denied_result,
    )
    .await?;

    Ok(Some(serde_json::json!({
        "tool_id": tool_id,
        "denied": true,
        "tool": tool,
        "allowed_tools": allowed_tools,
    })))
}

struct ApprovalRequest<'a> {
    approval_id: Option<omne_protocol::ApprovalId>,
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
    approval_policy: omne_protocol::ApprovalPolicy,
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
    exec_policy: &omne_execpolicy::Policy,
    thread_rt: &Arc<ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_policy: omne_protocol::ApprovalPolicy,
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
            omne_protocol::ApprovalPolicy::AutoDeny => {
                let approval_id = omne_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_protocol::ApprovalDecision::Denied,
                        remember: false,
                        reason: Some("auto-denied by policy".to_string()),
                    })
                    .await?;
                Ok(ApprovalGate::Denied { remembered: false })
            }
            _ => {
                let approval_id = omne_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
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
        let approval_id = omne_protocol::ApprovalId::new();
        let reason = match decision {
            omne_protocol::ApprovalDecision::Approved => "auto-approved by remembered decision",
            omne_protocol::ApprovalDecision::Denied => "auto-denied by remembered decision",
        };

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id,
                action: request.action.to_string(),
                params: request.params.clone(),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision,
                remember: false,
                reason: Some(reason.to_string()),
            })
            .await?;

        return match decision {
            omne_protocol::ApprovalDecision::Approved => Ok(ApprovalGate::Approved),
            omne_protocol::ApprovalDecision::Denied => Ok(ApprovalGate::Denied { remembered: true }),
        };
    }

    match approval_policy {
        omne_protocol::ApprovalPolicy::AutoApprove => {
            let approval_id = omne_protocol::ApprovalId::new();
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_protocol::ApprovalDecision::Approved,
                    remember: false,
                    reason: Some("auto-approved by policy".to_string()),
                })
                .await?;
            Ok(ApprovalGate::Approved)
        }
        omne_protocol::ApprovalPolicy::Manual => {
            let approval_id = omne_protocol::ApprovalId::new();
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            Ok(ApprovalGate::NeedsApproval { approval_id })
        }
        omne_protocol::ApprovalPolicy::UnlessTrusted => {
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
                let approval_id = omne_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_protocol::ApprovalDecision::Approved,
                        remember: false,
                        reason: Some("auto-approved by policy (unless_trusted)".to_string()),
                    })
                    .await?;
                Ok(ApprovalGate::Approved)
            } else {
                let approval_id = omne_protocol::ApprovalId::new();
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action: request.action.to_string(),
                        params: request.params.clone(),
                    })
                    .await?;
                Ok(ApprovalGate::NeedsApproval { approval_id })
            }
        }
        omne_protocol::ApprovalPolicy::AutoDeny => {
            let approval_id = omne_protocol::ApprovalId::new();
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    turn_id,
                    action: request.action.to_string(),
                    params: request.params.clone(),
                })
                .await?;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_protocol::ApprovalDecision::Denied,
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
) -> anyhow::Result<Option<omne_protocol::ApprovalDecision>> {
    let expected_key = approval_rule_key(expected_action, expected_params)?;
    let events = thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let mut requested = HashMap::<omne_protocol::ApprovalId, (String, serde_json::Value)>::new();
    let mut remembered = HashMap::<String, omne_protocol::ApprovalDecision>::new();

    for event in events {
        match event.kind {
            omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                action,
                params,
                ..
            } => {
                requested.insert(approval_id, (action, params));
            }
            omne_protocol::ThreadEventKind::ApprovalDecided {
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
mod approval_proxy_tests {
    use super::*;

    #[tokio::test]
    async fn approval_decide_forwards_subagent_proxy_to_child() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let proxy_approval_id = omne_protocol::ApprovalId::new();
        let child_approval_id = omne_protocol::ApprovalId::new();
        let child_rt = server.get_or_load_thread(child_thread_id).await?;
        child_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: None,
                action: "process/start".to_string(),
                params: serde_json::json!({
                    "argv": ["echo", "hi"],
                }),
            })
            .await?;
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "child_thread_id": child_thread_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": {
                            "argv": ["echo", "hi"],
                        },
                    },
                }),
            })
            .await?;

        let result = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved from parent".to_string()),
            },
        )
        .await?;
        let result: omne_app_server_protocol::ApprovalDecideResponse =
            serde_json::from_value(result).context("parse approval/decide test response")?;
        assert!(result.forwarded);
        assert_eq!(result.child_thread_id, Some(child_thread_id));
        assert_eq!(result.child_approval_id, Some(child_approval_id));

        let child_events = server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {child_thread_id}"))?;
        assert!(child_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_protocol::ApprovalDecision::Approved,
                    ..
                } if *approval_id == child_approval_id
            )
        }));

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        assert!(parent_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. }
                    if *approval_id == proxy_approval_id
            )
        }));

        Ok(())
    }

    #[tokio::test]
    async fn approval_decide_rejects_proxy_when_child_approval_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let proxy_approval_id = omne_protocol::ApprovalId::new();
        let child_approval_id = omne_protocol::ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "child_thread_id": child_thread_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": {
                            "argv": ["echo", "hi"],
                        },
                    },
                }),
            })
            .await?;

        let err = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved from parent".to_string()),
            },
        )
        .await
        .expect_err("proxy decision should fail when child approval request is missing");
        assert!(err.to_string().contains("approval not requested"));

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        assert!(!parent_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. }
                    if *approval_id == proxy_approval_id
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn approval_decide_rejects_proxy_when_child_approval_already_decided()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let child_approval_id = omne_protocol::ApprovalId::new();
        let child_rt = server.get_or_load_thread(child_thread_id).await?;
        child_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: None,
                action: "process/start".to_string(),
                params: serde_json::json!({
                    "argv": ["echo", "hi"],
                }),
            })
            .await?;
        child_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: child_approval_id,
                decision: omne_protocol::ApprovalDecision::Denied,
                remember: false,
                reason: Some("denied in child".to_string()),
            })
            .await?;

        let proxy_approval_id = omne_protocol::ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "child_thread_id": child_thread_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": {
                            "argv": ["echo", "hi"],
                        },
                    },
                }),
            })
            .await?;

        let err = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved from parent".to_string()),
            },
        )
        .await
        .expect_err("proxy decision should fail when child approval is already decided");
        assert!(err.to_string().contains("approval already decided"));

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        assert!(!parent_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. }
                    if *approval_id == proxy_approval_id
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn approval_decide_rejects_unknown_approval_id() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let err = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: omne_protocol::ApprovalId::new(),
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approve".to_string()),
            },
        )
        .await
        .expect_err("unknown approval id should be rejected");
        assert!(err.to_string().contains("approval not requested"));
        Ok(())
    }

    #[tokio::test]
    async fn approval_decide_succeeds_for_pending_non_proxy_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let approval_id = omne_protocol::ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: None,
                action: "file/write".to_string(),
                params: serde_json::json!({
                    "path": "a.txt",
                    "content": "hi",
                }),
            })
            .await?;

        let result = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved".to_string()),
            },
        )
        .await?;
        let result: omne_app_server_protocol::ApprovalDecideResponse =
            serde_json::from_value(result).context("parse approval/decide test response")?;
        assert!(result.ok);
        assert!(!result.forwarded);
        assert_eq!(result.child_thread_id, None);
        assert_eq!(result.child_approval_id, None);

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let decided_count = parent_events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id: got,
                        decision: omne_protocol::ApprovalDecision::Approved,
                        ..
                    } if *got == approval_id
                )
            })
            .count();
        assert_eq!(decided_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn approval_decide_rejects_already_decided_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let proxy_approval_id = omne_protocol::ApprovalId::new();
        let child_approval_id = omne_protocol::ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "child_thread_id": child_thread_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": {
                            "argv": ["echo", "hi"],
                        },
                    },
                }),
            })
            .await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved".to_string()),
            })
            .await?;

        let err = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Denied,
                remember: false,
                reason: Some("deny".to_string()),
            },
        )
        .await
        .expect_err("already decided approval id should be rejected");
        assert!(err.to_string().contains("approval already decided"));

        let child_events = server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {child_thread_id}"))?;
        assert!(!child_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_protocol::ApprovalDecision::Denied,
                    ..
                } if *approval_id == child_approval_id
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn approval_decide_rejects_already_decided_non_proxy_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let approval_id = omne_protocol::ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id,
                turn_id: None,
                action: "file/write".to_string(),
                params: serde_json::json!({
                    "path": "a.txt",
                    "content": "hi",
                }),
            })
            .await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id,
                decision: omne_protocol::ApprovalDecision::Denied,
                remember: false,
                reason: Some("deny".to_string()),
            })
            .await?;

        let err = handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approve".to_string()),
            },
        )
        .await
        .expect_err("already decided non-proxy approval id should be rejected");
        assert!(err.to_string().contains("approval already decided"));

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let decided_count = parent_events
            .iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id: got,
                        ..
                    } if *got == approval_id
                )
            })
            .count();
        assert_eq!(decided_count, 1);
        Ok(())
    }
}

#[cfg(test)]
mod approval_prompt_strict_tests {
    use super::*;

    #[tokio::test]
    async fn prompt_strict_forces_manual_even_when_auto_approve() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join(".omne_data")).await?;
        let server = crate::build_test_server_shared(repo_dir.join(".omne_data"));
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
            omne_protocol::ApprovalPolicy::AutoApprove,
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
                omne_protocol::ThreadEventKind::ApprovalRequested { .. } => requested += 1,
                omne_protocol::ThreadEventKind::ApprovalDecided { .. } => decided += 1,
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

        tokio::fs::create_dir_all(repo_dir.join(".omne_data")).await?;
        let server = crate::build_test_server_shared(repo_dir.join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let strict_approval_id = omne_protocol::ApprovalId::new();
        let strict_params = serde_json::json!({
            "path": "foo.txt",
            "create_parent_dirs": true,
            "approval": { "requirement": "prompt_strict" },
        });
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: strict_approval_id,
                turn_id: None,
                action: "file/write".to_string(),
                params: strict_params,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: strict_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
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
            omne_protocol::ApprovalPolicy::Manual,
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
    let thread_root = resolve_thread_root_from_cwd(server, &thread_cwd).await?;
    Ok((thread_rt, thread_root))
}

async fn resolve_thread_root_from_cwd(server: &Server, cwd: &str) -> anyhow::Result<PathBuf> {
    let cwd = Path::new(cwd);
    if cwd.is_absolute() {
        return omne_core::resolve_dir(cwd, Path::new(".")).await;
    }
    omne_core::resolve_dir(&server.cwd, cwd).await
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

#[cfg(test)]
mod load_thread_root_tests {
    use super::*;

    #[tokio::test]
    async fn load_thread_root_resolves_legacy_relative_cwd_against_server_cwd() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let workspace_dir = tmp.path().join("workspace");
        let repo_dir = workspace_dir.join("repo");
        let omne_root = workspace_dir.join(".omne_data");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        let server = Server {
            cwd: workspace_dir.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(omne_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            provider_runtimes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_execpolicy::Policy::empty(),
        };

        let thread_id = ThreadId::new();
        let mut writer = omne_eventlog::EventLogWriter::open(
            thread_id,
            server.thread_store.events_log_path(thread_id),
        )
        .await?;
        writer
            .append(omne_protocol::ThreadEventKind::ThreadCreated {
                cwd: "repo".to_string(),
            })
            .await?;
        drop(writer);

        let (_thread_rt, thread_root) = load_thread_root(&server, thread_id).await?;
        assert_eq!(thread_root, tokio::fs::canonicalize(&repo_dir).await?);
        Ok(())
    }
}

fn tool_denied(error: impl Into<String>, result: Value) -> anyhow::Error {
    anyhow::Error::new(ToolDenied::new(error, result))
}

struct SandboxWriteTarget {
    root: PathBuf,
    rel_path: PathBuf,
}

async fn select_sandbox_write_root(
    thread_root: &Path,
    sandbox_writable_roots: &[String],
    path: &Path,
) -> anyhow::Result<PathBuf> {
    let mut allowed_roots = Vec::with_capacity(1 + sandbox_writable_roots.len());
    allowed_roots.push(
        tokio::fs::canonicalize(thread_root)
            .await
            .with_context(|| format!("canonicalize {}", thread_root.display()))?,
    );
    for root in sandbox_writable_roots {
        let canon = tokio::fs::canonicalize(root)
            .await
            .with_context(|| format!("canonicalize {}", root))?;
        allowed_roots.push(canon);
    }

    allowed_roots
        .into_iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .ok_or_else(|| anyhow::anyhow!("path escapes roots: {}", path.display()))
}

async fn resolve_sandbox_write_target(
    thread_root: &Path,
    sandbox_writable_roots: &[String],
    path: &Path,
) -> anyhow::Result<SandboxWriteTarget> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    let Some(file_name) = path.file_name() else {
        anyhow::bail!("path has no file name: {}", path.display());
    };
    let canon_parent = tokio::fs::canonicalize(parent)
        .await
        .with_context(|| format!("canonicalize {}", parent.display()))?;
    let resolved_path = canon_parent.join(file_name);
    let root = select_sandbox_write_root(thread_root, sandbox_writable_roots, &resolved_path).await?;
    let rel_path = resolved_path
        .strip_prefix(&root)
        .with_context(|| {
            format!(
                "strip {} from {}",
                root.display(),
                resolved_path.display()
            )
        })?
        .to_path_buf();

    Ok(SandboxWriteTarget {
        root,
        rel_path,
    })
}

async fn preview_sandbox_write_target(
    thread_root: &Path,
    sandbox_policy: policy_meta::WriteScope,
    sandbox_writable_roots: &[String],
    input: &Path,
) -> anyhow::Result<SandboxWriteTarget> {
    let preview_path = omne_core::preview_file_for_sandbox_write(
        thread_root,
        sandbox_policy,
        sandbox_writable_roots,
        input,
    )
    .await?;
    resolve_sandbox_write_target(thread_root, sandbox_writable_roots, &preview_path).await
}

#[cfg(test)]
async fn canonical_rel_path_for_write(thread_root: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    Ok(resolve_sandbox_write_target(thread_root, &[], path).await?.rel_path)
}
