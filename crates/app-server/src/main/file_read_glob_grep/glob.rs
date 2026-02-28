async fn handle_file_glob(server: &Server, params: FileGlobParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_results = params.max_results.unwrap_or(2000).min(20_000);
    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "pattern": params.pattern.clone(),
        "max_results": max_results,
    });

    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/glob",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/glob", &allowed_tools);
    }
    if let FileModeApprovalGate::Denied(result) = enforce_file_mode_and_approval(
        server,
        FileModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            action: "file/glob",
            tool_id,
            approval_params: &approval_params,
        },
        |mode| mode.permissions.read,
    )
    .await?
    {
        return Ok(result);
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/glob".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let pattern = params.pattern.clone();
    let root_id = file_root.as_str().to_string();
    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
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
    let outcome = tokio::task::spawn_blocking(move || {
        omne_fs_runtime::glob_read_only_paths(root_id, root, pattern, max_results)
    })
    .await
    .context("join glob task")?;

    match outcome {
        Ok(omne_fs_runtime::GlobOutcome { paths, truncated }) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "matches": paths.len(),
                        "truncated": truncated,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "root": file_root.as_str(),
                "paths": paths,
                "truncated": truncated,
            }))
        }
        Err(err) => {
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
