async fn handle_file_write(server: &Server, params: FileWriteParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let create_parent_dirs = params.create_parent_dirs.unwrap_or(true);
    let (
        approval_policy,
        sandbox_policy,
        sandbox_writable_roots,
        mode_name,
        role_name,
        allowed_tools,
    ) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.mode.clone(),
            state.role.clone(),
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
    if sandbox_policy == policy_meta::WriteScope::ReadOnly {
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

    let requested_target = preview_sandbox_write_target(
        &thread_root,
        sandbox_policy,
        &sandbox_writable_roots,
        Path::new(&params.path),
    )
    .await?;
    let rel_path = Ok::<PathBuf, anyhow::Error>(requested_target.rel_path.clone());
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
            "refusing to write .env-style secrets file".to_string(),
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
            role_name: &role_name,
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

    let text_for_runtime = params.text.clone();
    let outcome: anyhow::Result<(PathBuf, u64)> = async {
        let path = omne_core::resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            &sandbox_writable_roots,
            Path::new(&params.path),
            omne_core::PathAccess::Write,
            create_parent_dirs,
        )
        .await?;

        let target = resolve_sandbox_write_target(&thread_root, &sandbox_writable_roots, &path).await?;
        if rel_path_is_secret(&target.rel_path) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to write .env-style secrets file".to_string(),
                result,
            ));
        }
        let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
        let mode_decision = resolve_mode_and_role_decision_audit(
            &catalog,
            &mode,
            Some(&role_name),
            "file/write",
            |mode| mode.permissions.edit.decision_for_path(&target.rel_path),
        );
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/write".to_string(),
                result,
            ));
        }

        let root_for_runtime = target.root.clone();
        let path_for_runtime = target.rel_path;
        let write_result = tokio::task::spawn_blocking(move || {
            omne_fs_runtime::write_text_workspace(
                "workspace".to_string(),
                root_for_runtime,
                path_for_runtime,
                text_for_runtime,
                create_parent_dirs,
            )
        })
        .await
        .context("join file/write task")??;

        Ok((path, write_result.bytes_written))
    }
    .await;

    match outcome {
        Ok((path, bytes_written)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(serde_json::json!({ "bytes": bytes_written })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
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

async fn handle_file_patch(server: &Server, params: FilePatchParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let max_bytes = params
        .max_bytes
        .unwrap_or(4 * 1024 * 1024)
        .min(16 * 1024 * 1024);
    let patch_bytes = params.patch.len();

    let (
        approval_policy,
        sandbox_policy,
        sandbox_writable_roots,
        mode_name,
        role_name,
        allowed_tools,
    ) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.mode.clone(),
            state.role.clone(),
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
    if sandbox_policy == policy_meta::WriteScope::ReadOnly {
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

    let requested_target = preview_sandbox_write_target(
        &thread_root,
        sandbox_policy,
        &sandbox_writable_roots,
        Path::new(&params.path),
    )
    .await?;
    let rel_path = Ok::<PathBuf, anyhow::Error>(requested_target.rel_path.clone());
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
            "refusing to patch .env-style secrets file".to_string(),
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
            role_name: &role_name,
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

    let patch_for_runtime = params.patch.clone();
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

        let target = resolve_sandbox_write_target(&thread_root, &sandbox_writable_roots, &path).await?;
        if rel_path_is_secret(&target.rel_path) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to patch .env-style secrets file".to_string(),
                result,
            ));
        }
        let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
        let mode_decision = resolve_mode_and_role_decision_audit(
            &catalog,
            &mode,
            Some(&role_name),
            "file/patch",
            |mode| mode.permissions.edit.decision_for_path(&target.rel_path),
        );
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/patch".to_string(),
                result,
            ));
        }

        let root_for_runtime = target.root.clone();
        let path_for_runtime = target.rel_path;
        let patch_result = tokio::task::spawn_blocking(move || {
            omne_fs_runtime::patch_text_workspace(
                "workspace".to_string(),
                root_for_runtime,
                path_for_runtime,
                patch_for_runtime,
                max_bytes,
            )
        })
        .await
        .context("join file/patch task")??;

        Ok((path, patch_result.changed, patch_bytes, patch_result.bytes_written))
    }
    .await;

    match outcome {
        Ok((path, changed, patch_bytes, bytes_written)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
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
