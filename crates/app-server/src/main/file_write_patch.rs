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
    let tool_id = omne_protocol::ToolId::new();
    let bytes = params.text.len();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "bytes": bytes,
        "create_parent_dirs": create_parent_dirs,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/write",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/write", &allowed_tools);
    }
    if sandbox_policy == omne_protocol::SandboxPolicy::ReadOnly {
        let result = file_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/write",
            &approval_params,
            "sandbox_policy=read_only forbids file/write".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    let rel_path = omne_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path));
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        let result = file_denied_response(tool_id, None)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/write",
            &approval_params,
            "refusing to write secrets file (.env)".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }
    let mode = match enforce_file_mode_and_approval(
        server,
        FileModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            action: "file/write",
            tool_id,
            approval_params: &approval_params,
        },
        |mode| match rel_path.as_ref() {
            Ok(rel) => mode.permissions.edit.decision_for_path(rel),
            Err(_) => omne_core::modes::Decision::Deny,
        },
    )
    .await?
    {
        FileModeApprovalGate::Allowed(mode) => mode,
        FileModeApprovalGate::Denied(result) => return Ok(*result),
    };

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/write".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let outcome: anyhow::Result<PathBuf> = async {
        let path = omne_core::resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            &sandbox_writable_roots,
            Path::new(&params.path),
            omne_core::PathAccess::Write,
            create_parent_dirs,
        )
        .await?;

        let resolved_rel = canonical_rel_path_for_write(&thread_root, &path).await?;
        if rel_path_is_secret(&resolved_rel) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to write secrets file (.env)".to_string(),
                result,
            ));
        }
        let base_decision = mode.permissions.edit.decision_for_path(&resolved_rel);
        let mode_decision = resolve_mode_decision_audit(&mode, "file/write", base_decision);
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/write".to_string(),
                result,
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

        Ok(path)
    }
    .await;

    match outcome {
        Ok(path) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({ "bytes": bytes })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "bytes_written": bytes,
            }))
        }
        Err(err) => {
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        error: Some(denied.error.clone()),
                        result: Some(denied.result.clone()),
                    })
                    .await?;
                Ok(denied.result.clone())
            } else {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Failed,
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
    let tool_id = omne_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "patch_bytes": patch_bytes,
        "max_bytes": max_bytes,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/patch",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/patch", &allowed_tools);
    }
    if sandbox_policy == omne_protocol::SandboxPolicy::ReadOnly {
        let result = file_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/patch",
            &approval_params,
            "sandbox_policy=read_only forbids file/patch".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    let rel_path = omne_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path));
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        let result = file_denied_response(tool_id, None)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/patch",
            &approval_params,
            "refusing to patch secrets file (.env)".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }
    let mode = match enforce_file_mode_and_approval(
        server,
        FileModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            action: "file/patch",
            tool_id,
            approval_params: &approval_params,
        },
        |mode| match rel_path.as_ref() {
            Ok(rel) => mode.permissions.edit.decision_for_path(rel),
            Err(_) => omne_core::modes::Decision::Deny,
        },
    )
    .await?
    {
        FileModeApprovalGate::Allowed(mode) => mode,
        FileModeApprovalGate::Denied(result) => return Ok(*result),
    };

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
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

    let outcome: anyhow::Result<(PathBuf, bool, usize, usize)> = async {
        let path = omne_core::resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            &sandbox_writable_roots,
            Path::new(&params.path),
            omne_core::PathAccess::Write,
            false,
        )
        .await?;

        let resolved_rel = canonical_rel_path_for_write(&thread_root, &path).await?;
        if rel_path_is_secret(&resolved_rel) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to patch secrets file (.env)".to_string(),
                result,
            ));
        }
        let base_decision = mode.permissions.edit.decision_for_path(&resolved_rel);
        let mode_decision = resolve_mode_decision_audit(&mode, "file/patch", base_decision);
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/patch".to_string(),
                result,
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

        Ok((path, changed, patch_bytes, bytes_written))
    }
    .await;

    match outcome {
        Ok((path, changed, patch_bytes, bytes_written)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
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
                "resolved_path": path.display().to_string(),
                "changed": changed,
                "patch_bytes": patch_bytes,
                "bytes_written": bytes_written,
            }))
        }
        Err(err) => {
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        error: Some(denied.error.clone()),
                        result: Some(denied.result.clone()),
                    })
                    .await?;
                Ok(denied.result.clone())
            } else {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Failed,
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
