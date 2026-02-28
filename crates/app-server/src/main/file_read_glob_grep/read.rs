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
    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "path": params.path.clone(),
        "max_bytes": max_bytes,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/read",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/read", &allowed_tools);
    }

    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "file/read".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Failed,
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

    let rel_path = omne_core::modes::relative_path_under_root(&root, Path::new(&params.path));
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/read".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("refusing to read secrets file (.env)".to_string()),
                result: Some(serde_json::json!({
                    "reason": "secrets file is always denied",
                })),
            })
            .await?;
        return file_denied_response(tool_id, None);
    }
    let rel_path_for_mode = rel_path.as_ref().ok().cloned();
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
            action: "file/read",
            tool_id,
            approval_params: &approval_params,
        },
        move |mode| match rel_path_for_mode.as_ref() {
            Some(rel) if mode.permissions.edit.is_denied(rel) => omne_core::modes::Decision::Deny,
            Some(_) => mode.permissions.read,
            None => omne_core::modes::Decision::Deny,
        },
    )
    .await?
    {
        FileModeApprovalGate::Allowed(mode) => mode,
        FileModeApprovalGate::Denied(result) => return Ok(result),
    };

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/read".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let outcome: anyhow::Result<(PathBuf, String, bool, usize)> = async {
        let path = match file_root {
            FileRoot::Workspace => {
                omne_core::resolve_file_for_sandbox(
                    &thread_root,
                    sandbox_policy,
                    &sandbox_writable_roots,
                    Path::new(&params.path),
                    omne_core::PathAccess::Read,
                    false,
                )
                .await?
            }
            FileRoot::Reference => {
                omne_core::resolve_file(
                    &root,
                    Path::new(&params.path),
                    omne_core::PathAccess::Read,
                    false,
                )
                .await?
            }
        };

        let resolved_rel = omne_core::modes::relative_path_under_root(&root, &path)?;
        if rel_path_is_secret(&resolved_rel) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to read secrets file (.env)".to_string(),
                result,
            ));
        }
        let base_decision = if mode.permissions.edit.is_denied(&resolved_rel) {
            omne_core::modes::Decision::Deny
        } else {
            mode.permissions.read
        };
        let mode_decision = resolve_mode_decision_audit(&mode, "file/read", base_decision);
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies file/read".to_string(),
                result,
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
        Ok((path, text, truncated, bytes))
    }
    .await;

    match outcome {
        Ok((path, text, truncated, bytes)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "bytes": bytes,
                        "truncated": truncated,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "root": file_root.as_str(),
                "text": text,
                "truncated": truncated,
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
