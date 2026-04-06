async fn handle_fs_mkdir(server: &Server, params: FsMkdirParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

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
        "recursive": params.recursive,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "fs/mkdir",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "fs/mkdir", &allowed_tools);
    }
    if sandbox_policy == policy_meta::WriteScope::ReadOnly {
        let result = file_sandbox_policy_denied_response(tool_id, sandbox_policy)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "fs/mkdir",
            &approval_params,
            "sandbox_policy=read_only forbids fs/mkdir".to_string(),
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
            action: "fs/mkdir",
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
            tool: "fs/mkdir".to_string(),
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
            params.recursive,
        )
        .await?;

        if path == thread_root {
            anyhow::bail!("refusing to create thread root: {}", path.display());
        }

        let target = resolve_sandbox_write_target(&thread_root, &sandbox_writable_roots, &path).await?;
        if rel_path_is_secret(&target.rel_path) {
            let result = file_denied_response(tool_id, None)?;
            return Err(tool_denied(
                "refusing to mkdir .env-style secrets path".to_string(),
                result,
            ));
        }
        let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
        let mode_decision = resolve_mode_and_role_decision_audit(
            &catalog,
            &mode,
            Some(&role_name),
            "fs/mkdir",
            |mode| mode.permissions.edit.decision_for_path(&target.rel_path),
        );
        if mode_decision.decision == omne_core::modes::Decision::Deny {
            let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
            return Err(tool_denied(
                "mode denies fs/mkdir".to_string(),
                result,
            ));
        }

        let root_for_runtime = target.root.clone();
        let path_for_runtime = target.rel_path;
        let mkdir_result = tokio::task::spawn_blocking(move || {
            omne_fs_runtime::mkdir_workspace(
                "workspace".to_string(),
                root_for_runtime,
                path_for_runtime,
                params.recursive,
                true,
            )
        })
        .await
        .context("join fs/mkdir task")??;

        Ok((mkdir_result.created, path))
    }
    .await;

    match outcome {
        Ok((created, path)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(serde_json::json!({
                        "created": created,
                        "path": path.display().to_string(),
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "created": created,
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

fn user_artifacts_dir_for_thread(server: &Server, thread_id: ThreadId) -> PathBuf {
    omne_artifact_store::user_artifacts_dir_for_thread(&server.thread_store.thread_dir(thread_id))
}

fn user_artifact_paths(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
) -> (PathBuf, PathBuf) {
    omne_artifact_store::user_artifact_paths(&server.thread_store.thread_dir(thread_id), artifact_id)
}

fn user_artifact_history_dir_for_thread(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
) -> PathBuf {
    omne_artifact_store::user_artifact_history_dir_for_thread(
        &server.thread_store.thread_dir(thread_id),
        artifact_id,
    )
}

fn user_artifact_history_path(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
    version: u32,
) -> PathBuf {
    omne_artifact_store::user_artifact_history_path(
        &server.thread_store.thread_dir(thread_id),
        artifact_id,
        version,
    )
}

fn user_artifact_history_metadata_path(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
    version: u32,
) -> PathBuf {
    omne_artifact_store::user_artifact_history_metadata_path(
        &server.thread_store.thread_dir(thread_id),
        artifact_id,
        version,
    )
}

async fn read_artifact_metadata(path: &Path) -> anyhow::Result<ArtifactMetadata> {
    omne_artifact_store::read_artifact_metadata(path).await
}

async fn write_file_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    omne_artifact_store::write_file_atomic(path, bytes).await
}
