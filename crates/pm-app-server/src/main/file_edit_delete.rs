async fn handle_file_edit(server: &Server, params: FileEditParams) -> anyhow::Result<Value> {
    if params.edits.is_empty() {
        anyhow::bail!("edits must not be empty");
    }
    if params.edits.iter().any(|e| e.old.is_empty()) {
        anyhow::bail!("edit.old must not be empty");
    }

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_bytes = params
        .max_bytes
        .unwrap_or(4 * 1024 * 1024)
        .min(16 * 1024 * 1024);

    let (approval_policy, sandbox_policy, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.mode.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "edits": params.edits.len(),
        "max_bytes": max_bytes,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/edit".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/edit".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let rel_path = pm_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path));
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
                    tool: "file/edit".to_string(),
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

    let base_decision = match rel_path.as_ref() {
        Ok(rel) => mode.permissions.edit.decision_for_path(rel),
        Err(_) => pm_core::modes::Decision::Deny,
    };
    let effective_decision = match mode.tool_overrides.get("file/edit").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/edit".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/edit".to_string()),
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
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "file/edit",
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
                        tool: "file/edit".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
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
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/edit".to_string(),
            params: Some(serde_json::json!({
                "path": params.path.clone(),
                "edits": params.edits.len(),
                "max_bytes": max_bytes,
            })),
        })
        .await?;

    let outcome: anyhow::Result<(PathBuf, bool, usize, usize)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            false,
        )
        .await?;

        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        if bytes.len() > max_bytes as usize {
            anyhow::bail!(
                "file too large for edit: {} ({} bytes)",
                path.display(),
                bytes.len()
            );
        }
        let mut text = String::from_utf8(bytes).context("file is not valid utf-8")?;

        let mut total_replacements = 0usize;
        let mut changed = false;
        for edit in &params.edits {
            let expected = edit.expected_replacements.unwrap_or(1);
            let found = count_non_overlapping(&text, &edit.old);
            if found != expected {
                anyhow::bail!(
                    "edit mismatch for {}: expected {} replacements, found {}",
                    path.display(),
                    expected,
                    found
                );
            }
            if edit.old != edit.new {
                changed = true;
            }
            total_replacements += expected;
            text = text.replacen(&edit.old, &edit.new, expected);
        }

        let bytes_written = text.len();
        tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?
            .write_all(text.as_bytes())
            .await
            .with_context(|| format!("write {}", path.display()))?;

        Ok((path, changed, total_replacements, bytes_written))
    }
    .await;

    match outcome {
        Ok((path, changed, replacements, bytes_written)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "changed": changed,
                        "replacements": replacements,
                        "bytes": bytes_written,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "changed": changed,
                "replacements": replacements,
                "bytes_written": bytes_written,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_file_delete(server: &Server, params: FileDeleteParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let (approval_policy, sandbox_policy, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.mode.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/delete".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/delete".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let rel_path = pm_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path));
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
                    tool: "file/delete".to_string(),
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

    let base_decision = match rel_path.as_ref() {
        Ok(rel) => mode.permissions.edit.decision_for_path(rel),
        Err(_) => pm_core::modes::Decision::Deny,
    };
    let effective_decision = match mode.tool_overrides.get("file/delete").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/delete".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/delete".to_string()),
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
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "file/delete",
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
                        tool: "file/delete".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
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
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/delete".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let thread_root = thread_root.clone();
    let outcome: anyhow::Result<(bool, PathBuf)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            false,
        )
        .await?;

        if path == thread_root {
            anyhow::bail!("refusing to delete thread root: {}", path.display());
        }

        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((false, path)),
            Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
        };

        if meta.is_dir() {
            if params.recursive {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .with_context(|| format!("remove dir {}", path.display()))?;
            } else {
                tokio::fs::remove_dir(&path)
                    .await
                    .with_context(|| format!("remove dir {}", path.display()))?;
            }
        } else {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("remove file {}", path.display()))?;
        }

        Ok((true, path))
    }
    .await;

    match outcome {
        Ok((deleted, path)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "deleted": deleted,
                        "path": path.display().to_string(),
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "deleted": deleted,
                "resolved_path": path.display().to_string(),
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}
