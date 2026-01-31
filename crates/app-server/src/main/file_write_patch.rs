async fn handle_file_write(server: &Server, params: FileWriteParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let create_parent_dirs = params.create_parent_dirs.unwrap_or(true);
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
    let tool_id = pm_protocol::ToolId::new();
    let bytes = params.text.len();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "bytes": bytes,
        "create_parent_dirs": create_parent_dirs,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/write",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/write".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/write".to_string()),
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
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/write".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("refusing to write secrets file (.env)".to_string()),
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
                    tool: "file/write".to_string(),
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
    let effective_decision = match mode.tool_overrides.get("file/write").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/write".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/write".to_string()),
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
                action: "file/write",
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
                        tool: "file/write".to_string(),
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
            tool: "file/write".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let db_vfs = server.db_vfs.clone();
    let outcome: anyhow::Result<String> = async {
        if let Some(db_vfs) = db_vfs {
            let rel = pm_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path))?;
            if rel_path_is_secret(&rel) {
                return Err(tool_denied(
                    "refusing to write secrets file (.env)".to_string(),
                    serde_json::json!({
                        "reason": "secrets file is always denied",
                    }),
                ));
            }
            let base_decision = mode.permissions.edit.decision_for_path(&rel);
            let effective_decision = match mode.tool_overrides.get("file/write").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision == pm_core::modes::Decision::Deny {
                return Err(tool_denied(
                    "mode denies file/write".to_string(),
                    serde_json::json!({
                        "mode": mode_name,
                        "decision": effective_decision,
                    }),
                ));
            }

            let workspace_id = params.thread_id.to_string();
            let normalized = rel.to_string_lossy().replace('\\', "/");

            let mut attempts = 0u32;
            loop {
                attempts = attempts.saturating_add(1);
                let existing = db_vfs
                    .read(workspace_id.clone(), normalized.clone())
                    .await;

                let expected_version = match existing {
                    Ok(record) => Some(record.version),
                    Err(err) if err.code.as_deref() == Some("not_found") => None,
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

                match db_vfs
                    .write(
                        workspace_id.clone(),
                        normalized.clone(),
                        params.text.clone(),
                        expected_version,
                    )
                    .await
                {
                    Ok(_) => return Ok(normalized),
                    Err(err) if err.code.as_deref() == Some("conflict") && attempts < 3 => continue,
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
                }
            }
        } else {
            let path = resolve_file_for_sandbox(
                &thread_root,
                sandbox_policy,
                &sandbox_writable_roots,
                Path::new(&params.path),
                pm_core::PathAccess::Write,
                create_parent_dirs,
            )
            .await?;

            let resolved_rel = canonical_rel_path_for_write(&thread_root, &path).await?;
            if rel_path_is_secret(&resolved_rel) {
                return Err(tool_denied(
                    "refusing to write secrets file (.env)".to_string(),
                    serde_json::json!({
                        "reason": "secrets file is always denied",
                    }),
                ));
            }
            let base_decision = mode.permissions.edit.decision_for_path(&resolved_rel);
            let effective_decision = match mode.tool_overrides.get("file/write").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision == pm_core::modes::Decision::Deny {
                return Err(tool_denied(
                    "mode denies file/write".to_string(),
                    serde_json::json!({
                        "mode": mode_name,
                        "decision": effective_decision,
                    }),
                ));
            }

            tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .await
                .with_context(|| format!("open {}", path.display()))?
                .write_all(params.text.as_bytes())
                .await
                .with_context(|| format!("write {}", path.display()))?;

            Ok(path.to_string_lossy().to_string())
        }
    }
    .await;

    match outcome {
        Ok(resolved_path) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({ "bytes": bytes })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": resolved_path,
                "bytes_written": bytes,
            }))
        }
        Err(err) => {
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
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
}

async fn handle_file_patch(server: &Server, params: FilePatchParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_bytes = params
        .max_bytes
        .unwrap_or(4 * 1024 * 1024)
        .min(16 * 1024 * 1024);
    let patch_bytes = params.patch.len();

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
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "patch_bytes": patch_bytes,
        "max_bytes": max_bytes,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/patch",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/patch".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids file/patch".to_string()),
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
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/patch".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("refusing to patch secrets file (.env)".to_string()),
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
                    tool: "file/patch".to_string(),
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
    let effective_decision = match mode.tool_overrides.get("file/patch").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/patch".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/patch".to_string()),
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
                action: "file/patch",
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
                        tool: "file/patch".to_string(),
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
            tool: "file/patch".to_string(),
            params: Some(serde_json::json!({
                "path": params.path.clone(),
                "patch_bytes": patch_bytes,
                "max_bytes": max_bytes,
            })),
        })
        .await?;

    let db_vfs = server.db_vfs.clone();
    let outcome: anyhow::Result<(String, bool, usize, usize)> = async {
        if let Some(db_vfs) = db_vfs {
            let rel = pm_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path))?;
            if rel_path_is_secret(&rel) {
                return Err(tool_denied(
                    "refusing to patch secrets file (.env)".to_string(),
                    serde_json::json!({
                        "reason": "secrets file is always denied",
                    }),
                ));
            }
            let base_decision = mode.permissions.edit.decision_for_path(&rel);
            let effective_decision = match mode.tool_overrides.get("file/patch").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision == pm_core::modes::Decision::Deny {
                return Err(tool_denied(
                    "mode denies file/patch".to_string(),
                    serde_json::json!({
                        "mode": mode_name,
                        "decision": effective_decision,
                    }),
                ));
            }

            let workspace_id = params.thread_id.to_string();
            let normalized = rel.to_string_lossy().replace('\\', "/");
            let record = match db_vfs.read(workspace_id.clone(), normalized.clone()).await {
                Ok(record) => record,
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

            let original_bytes = record.content.len();
            if original_bytes > max_bytes as usize {
                anyhow::bail!(
                    "file too large for patch: {} ({} bytes)",
                    normalized,
                    original_bytes
                );
            }

            let patch = Patch::from_str(&params.patch).context("parse unified diff patch")?;
            let updated = apply(&record.content, &patch).context("apply patch")?;
            let changed = updated != record.content;
            let bytes_written = updated.len();
            if bytes_written > max_bytes as usize {
                anyhow::bail!(
                    "patched file too large: {} ({} bytes)",
                    normalized,
                    bytes_written
                );
            }

            match db_vfs
                .write(
                    workspace_id,
                    normalized.clone(),
                    updated,
                    Some(record.version),
                )
                .await
            {
                Ok(_) => Ok((normalized, changed, patch_bytes, bytes_written)),
                Err(err) if err.is_denied() => Err(tool_denied(
                    err.to_string(),
                    serde_json::json!({
                        "db_vfs_code": err.code,
                        "db_vfs_status": err.status.map(|status| status.as_u16()),
                    }),
                )),
                Err(err) => Err(anyhow::anyhow!(err)),
            }
        } else {
            let path = resolve_file_for_sandbox(
                &thread_root,
                sandbox_policy,
                &sandbox_writable_roots,
                Path::new(&params.path),
                pm_core::PathAccess::Write,
                false,
            )
            .await?;

            let resolved_rel = canonical_rel_path_for_write(&thread_root, &path).await?;
            if rel_path_is_secret(&resolved_rel) {
                return Err(tool_denied(
                    "refusing to patch secrets file (.env)".to_string(),
                    serde_json::json!({
                        "reason": "secrets file is always denied",
                    }),
                ));
            }
            let base_decision = mode.permissions.edit.decision_for_path(&resolved_rel);
            let effective_decision = match mode.tool_overrides.get("file/patch").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision == pm_core::modes::Decision::Deny {
                return Err(tool_denied(
                    "mode denies file/patch".to_string(),
                    serde_json::json!({
                        "mode": mode_name,
                        "decision": effective_decision,
                    }),
                ));
            }

            let bytes = tokio::fs::read(&path)
                .await
                .with_context(|| format!("read {}", path.display()))?;
            if bytes.len() > max_bytes as usize {
                anyhow::bail!(
                    "file too large for patch: {} ({} bytes)",
                    path.display(),
                    bytes.len()
                );
            }

            let original = String::from_utf8(bytes).context("file is not valid utf-8")?;
            let patch = Patch::from_str(&params.patch).context("parse unified diff patch")?;
            let updated = apply(&original, &patch).context("apply patch")?;
            let changed = updated != original;
            let bytes_written = updated.len();
            if bytes_written > max_bytes as usize {
                anyhow::bail!(
                    "patched file too large: {} ({} bytes)",
                    path.display(),
                    bytes_written
                );
            }

            tokio::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .await
                .with_context(|| format!("open {}", path.display()))?
                .write_all(updated.as_bytes())
                .await
                .with_context(|| format!("write {}", path.display()))?;

            Ok((path.to_string_lossy().to_string(), changed, patch_bytes, bytes_written))
        }
    }
    .await;

    match outcome {
        Ok((resolved_path, changed, patch_bytes, bytes_written)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "changed": changed,
                        "patch_bytes": patch_bytes,
                        "bytes": bytes_written,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": resolved_path,
                "changed": changed,
                "patch_bytes": patch_bytes,
                "bytes_written": bytes_written,
            }))
        }
        Err(err) => {
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
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
}

fn count_non_overlapping(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }

    let mut count = 0usize;
    let mut rest = haystack;
    while let Some(pos) = rest.find(needle) {
        count += 1;
        rest = &rest[(pos + needle.len())..];
    }
    count
}
