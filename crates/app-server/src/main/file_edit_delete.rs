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
        "edits": params.edits.len(),
        "max_bytes": max_bytes,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/edit",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/edit", &allowed_tools);
    }
    if sandbox_policy == policy_meta::WriteScope::ReadOnly {
        let result = file_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/edit",
            &approval_params,
            "sandbox_policy=read_only forbids file/edit".to_string(),
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
            "file/edit",
            &approval_params,
            "refusing to edit secrets file (.env)".to_string(),
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
            action: "file/edit",
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
            tool: "file/edit".to_string(),
            params: Some(serde_json::json!({
                "path": params.path.clone(),
                "edits": params.edits.len(),
                "max_bytes": max_bytes,
            })),
        })
        .await?;

    let edits_for_runtime = params
        .edits
        .iter()
        .map(|edit| omne_fs_runtime::EditReplaceOp {
            old: edit.old.clone(),
            new: edit.new.clone(),
            expected_replacements: edit.expected_replacements,
        })
        .collect::<Vec<_>>();
    let outcome: anyhow::Result<(PathBuf, bool, usize, u64)> = async {
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
                "refusing to edit secrets file (.env)".to_string(),
                result,
            ));
        }
        let base_decision = mode.permissions.edit.decision_for_path(&resolved_rel);
        let mode_decision = resolve_mode_decision_audit(&mode, "file/edit", base_decision);
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/edit".to_string(),
                result,
            ));
        }

        let root_for_runtime = thread_root.clone();
        let path_for_runtime = resolved_rel;
        let edit_result = tokio::task::spawn_blocking(move || {
            omne_fs_runtime::edit_replace_workspace(
                "workspace".to_string(),
                root_for_runtime,
                path_for_runtime,
                edits_for_runtime,
                max_bytes,
            )
        })
        .await
        .context("join file/edit task")??;

        Ok((
            path,
            edit_result.changed,
            edit_result.replacements,
            edit_result.bytes_written,
        ))
    }
    .await;

    match outcome {
        Ok((path, changed, replacements, bytes_written)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
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
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        structured_error: structured_error_from_result_value(&denied.result),
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
                        structured_error: None,
                        error: Some(err.to_string()),
                        result: None,
                    })
                    .await?;
                Err(err)
            }
        }
    }
}

async fn handle_file_delete(server: &Server, params: FileDeleteParams) -> anyhow::Result<Value> {
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
    let tool_id = omne_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/delete",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/delete", &allowed_tools);
    }
    if sandbox_policy == policy_meta::WriteScope::ReadOnly {
        let result = file_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/delete",
            &approval_params,
            "sandbox_policy=read_only forbids file/delete".to_string(),
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
            "file/delete",
            &approval_params,
            "refusing to delete secrets file (.env)".to_string(),
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
            action: "file/delete",
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
            tool: "file/delete".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let thread_root = thread_root.clone();
    let outcome: anyhow::Result<(bool, PathBuf)> = async {
        let path = omne_core::resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            &sandbox_writable_roots,
            Path::new(&params.path),
            omne_core::PathAccess::Write,
            false,
        )
        .await?;

        if path == thread_root {
            anyhow::bail!("refusing to delete thread root: {}", path.display());
        }

        let resolved_rel = canonical_rel_path_for_write(&thread_root, &path).await?;
        if rel_path_is_secret(&resolved_rel) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to delete secrets file (.env)".to_string(),
                result,
            ));
        }
        let base_decision = mode.permissions.edit.decision_for_path(&resolved_rel);
        let mode_decision = resolve_mode_decision_audit(&mode, "file/delete", base_decision);
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/delete".to_string(),
                result,
            ));
        }

        let root_for_runtime = thread_root.clone();
        let path_for_runtime = resolved_rel;
        let delete_result = tokio::task::spawn_blocking(move || {
            omne_fs_runtime::delete_path_workspace(
                "workspace".to_string(),
                root_for_runtime,
                path_for_runtime,
                params.recursive,
                true,
            )
        })
        .await
        .context("join file/delete task")??;

        Ok((delete_result.deleted, path))
    }
    .await;

    match outcome {
        Ok((deleted, path)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
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
            if let Some(denied) = err.downcast_ref::<ToolDenied>() {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        structured_error: structured_error_from_result_value(&denied.result),
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
                        structured_error: None,
                        error: Some(err.to_string()),
                        result: None,
                    })
                    .await?;
                Err(err)
            }
        }
    }
}
