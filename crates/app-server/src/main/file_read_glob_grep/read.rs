async fn handle_file_read(server: &Server, params: FileReadParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, sandbox_policy, sandbox_writable_roots, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_bytes = params.max_bytes.unwrap_or(256 * 1024).min(4 * 1024 * 1024);
    let tool_id = omne_agent_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "path": params.path.clone(),
        "max_bytes": max_bytes,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/read",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "file/read".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: Some(serde_json::json!({
                            "root": file_root.as_str(),
                            "reason": "reference repo is not configured",
                        })),
                    })
                    .await?;
                return Err(err);
            }
        },
    };

    let rel_path = omne_agent_core::modes::relative_path_under_root(&root, Path::new(&params.path));
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/read".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_agent_protocol::ToolStatus::Denied,
                error: Some("refusing to read secrets file (.env)".to_string()),
                result: Some(serde_json::json!({
                    "reason": "secrets file is always denied",
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
        }));
    }
    let catalog = omne_agent_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");

            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "file/read".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": omne_agent_core::modes::Decision::Deny,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": omne_agent_core::modes::Decision::Deny,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = match rel_path.as_ref() {
        Ok(rel) if mode.permissions.edit.is_denied(rel) => omne_agent_core::modes::Decision::Deny,
        Ok(_) => mode.permissions.read,
        Err(_) => omne_agent_core::modes::Decision::Deny,
    };
    let effective_decision = match mode.tool_overrides.get("file/read").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == omne_agent_core::modes::Decision::Deny {
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/read".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_agent_protocol::ToolStatus::Denied,
                error: Some("mode denies file/read".to_string()),
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

    if effective_decision == omne_agent_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "file/read",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "file/read".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                    thread_rt
                        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: omne_agent_protocol::ToolStatus::Denied,
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

    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/read".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let db_vfs = server.db_vfs.clone();
    let outcome: anyhow::Result<(String, String, bool, usize)> = async {
        if matches!(file_root, FileRoot::Workspace)
            && let Some(db_vfs) = db_vfs
        {
            let rel = omne_agent_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path))?;
            if rel_path_is_secret(&rel) {
                return Err(tool_denied(
                    "refusing to read secrets file (.env)".to_string(),
                    serde_json::json!({
                        "reason": "secrets file is always denied",
                    }),
                ));
            }

            let base_decision = if mode.permissions.edit.is_denied(&rel) {
                omne_agent_core::modes::Decision::Deny
            } else {
                mode.permissions.read
            };
            let effective_decision = match mode.tool_overrides.get("file/read").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision == omne_agent_core::modes::Decision::Deny {
                return Err(tool_denied(
                    "mode denies file/read".to_string(),
                    serde_json::json!({
                        "mode": mode_name,
                        "decision": effective_decision,
                    }),
                ));
            }

            let normalized = rel.to_string_lossy().replace('\\', "/");
            let resp = match db_vfs
                .read(params.thread_id.to_string(), normalized.clone())
                .await
            {
                Ok(resp) => resp,
                Err(err) if err.is_denied() => {
                    return Err(tool_denied(
                        err.to_string(),
                        serde_json::json!({
                            "db_vfs_code": err.code,
                            "db_vfs_status": err.status.map(|status| status.as_u16()),
                        }),
                    ));
                }
                Err(err) => return Err(anyhow::anyhow!(err)),
            };

            let mut bytes = resp.content.len();
            let mut truncated = false;
            let text = if bytes > max_bytes as usize {
                truncated = true;
                let mut end = max_bytes as usize;
                while end > 0 && !resp.content.is_char_boundary(end) {
                    end = end.saturating_sub(1);
                }
                bytes = end;
                resp.content[..end].to_string()
            } else {
                resp.content
            };

            Ok((normalized, text, truncated, bytes))
        } else {
            let path = match file_root {
                FileRoot::Workspace => {
                    resolve_file_for_sandbox(
                        &thread_root,
                        sandbox_policy,
                        &sandbox_writable_roots,
                        Path::new(&params.path),
                        omne_agent_core::PathAccess::Read,
                        false,
                    )
                    .await?
                }
                FileRoot::Reference => {
                    omne_agent_core::resolve_file(
                        &root,
                        Path::new(&params.path),
                        omne_agent_core::PathAccess::Read,
                        false,
                    )
                    .await?
                }
            };

            let resolved_rel = omne_agent_core::modes::relative_path_under_root(&root, &path)?;
            if rel_path_is_secret(&resolved_rel) {
                return Err(tool_denied(
                    "refusing to read secrets file (.env)".to_string(),
                    serde_json::json!({
                        "reason": "secrets file is always denied",
                    }),
                ));
            }
            let base_decision = if mode.permissions.edit.is_denied(&resolved_rel) {
                omne_agent_core::modes::Decision::Deny
            } else {
                mode.permissions.read
            };
            let effective_decision = match mode.tool_overrides.get("file/read").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision == omne_agent_core::modes::Decision::Deny {
                return Err(tool_denied(
                    "mode denies file/read".to_string(),
                    serde_json::json!({
                        "mode": mode_name,
                        "decision": effective_decision,
                    }),
                ));
            }

            let limit = max_bytes + 1;
            let file = tokio::fs::File::open(&path)
                .await
                .with_context(|| format!("open {}", path.display()))?;
            let mut buf = Vec::new();
            file.take(limit).read_to_end(&mut buf).await?;

            let truncated = buf.len() > max_bytes as usize;
            if truncated {
                buf.truncate(max_bytes as usize);
            }
            let bytes = buf.len();
            let text = String::from_utf8(buf).context("file is not valid utf-8")?;
            Ok((path.to_string_lossy().to_string(), text, truncated, bytes))
        }
    }
    .await;

    match outcome {
        Ok((resolved_path, text, truncated, bytes)) => {
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "bytes": bytes,
                        "truncated": truncated,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": resolved_path,
                "root": file_root.as_str(),
                "text": text,
                "truncated": truncated,
            }))
        }
        Err(err) => {
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
                        error: Some(denied.error.clone()),
                        result: Some(denied.result.clone()),
                    })
                    .await?;
                Ok(merge_json_object(
                    serde_json::json!({
                        "tool_id": tool_id,
                        "denied": true,
                    }),
                    &denied.result,
                ))
            } else {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: None,
                    })
                    .await?;
                Err(err)
            }
        }
    }
}

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "example",
];
