async fn handle_repo_index(server: &Server, params: RepoIndexParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_files = params.max_files.unwrap_or(20_000).min(200_000);
    let tool_id = omne_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "include_glob": params.include_glob.clone(),
        "max_files": max_files,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/index",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return repo_allowed_tools_denied_response(tool_id, "repo/index", &allowed_tools);
    }

    if let Some(result) = enforce_repo_mode_and_approval(
        server,
        RepoModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            action: "repo/index",
            tool_id,
            approval_params: &approval_params,
        },
    )
    .await?
    {
        return Ok(result);
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "repo/index".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let outcome: anyhow::Result<(Value, Value)> = async {
        let root = match file_root {
            FileRoot::Workspace => thread_root.clone(),
            FileRoot::Reference => resolve_reference_repo_root(&thread_root).await?,
        };

        let include_glob = params
            .include_glob
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        let root_for_task = root.clone();
        let index_outcome = tokio::task::spawn_blocking(move || {
            omne_repo_scan_runtime::scan_repo_index(root_for_task, include_glob, max_files)
        })
        .await
        .context("join repo/index task")??;

        let include_glob_str = params
            .include_glob
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let summary = match include_glob_str {
            Some(glob) => format!("repo/index ({glob})"),
            None => "repo/index".to_string(),
        };

        let artifact_text = format_repo_index_artifact(
            file_root.as_str(),
            include_glob_str,
            max_files,
            &index_outcome,
        );

        let (mut artifact_response, _artifact_completed) = write_user_artifact(
            server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                artifact_id: None,
                artifact_type: "repo_index".to_string(),
                summary,
                text: artifact_text,
            },
        )
        .await?;

        let artifact_id = artifact_response
            .get("artifact_id")
            .cloned()
            .unwrap_or(Value::Null);

        let completed = serde_json::json!({
            "artifact_id": artifact_id,
            "files_scanned": index_outcome.files_scanned,
            "truncated": index_outcome.truncated,
            "size_bytes": index_outcome.size_bytes,
            "paths_listed": index_outcome.paths.len(),
        });

        if let Some(obj) = artifact_response.as_object_mut() {
            obj.insert("root".to_string(), serde_json::json!(file_root.as_str()));
            obj.insert(
                "paths_listed".to_string(),
                serde_json::json!(index_outcome.paths.len()),
            );
            obj.insert("truncated".to_string(), serde_json::json!(index_outcome.truncated));
            obj.insert(
                "files_scanned".to_string(),
                serde_json::json!(index_outcome.files_scanned),
            );
            obj.insert("size_bytes".to_string(), serde_json::json!(index_outcome.size_bytes));
        }

        let response: omne_app_server_protocol::RepoIndexResponse =
            serde_json::from_value(artifact_response).context("parse repo/index response")?;
        let response_value =
            serde_json::to_value(response).context("serialize repo/index response")?;
        Ok((response_value, completed))
    }
    .await;

    match outcome {
        Ok((artifact_response, completed)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(artifact_response)
        }
        Err(err) => {
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
