async fn handle_repo_search(server: &Server, params: RepoSearchParams) -> anyhow::Result<Value> {
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
    let query = params.query.trim().to_string();
    if query.is_empty() {
        anyhow::bail!("query must not be empty");
    }

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_matches = params.max_matches.unwrap_or(200).min(2000);
    let max_bytes_per_file = params
        .max_bytes_per_file
        .unwrap_or(1024 * 1024)
        .min(16 * 1024 * 1024);
    let max_files = params.max_files.unwrap_or(20_000).min(200_000);
    let tool_id = omne_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "query": query.clone(),
        "is_regex": params.is_regex,
        "include_glob": params.include_glob.clone(),
        "max_matches": max_matches,
        "max_bytes_per_file": max_bytes_per_file,
        "max_files": max_files,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/search",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return repo_allowed_tools_denied_response(tool_id, "repo/search", &allowed_tools);
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
            action: "repo/search",
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
            tool: "repo/search".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let outcome: anyhow::Result<(Value, Value)> = async {
        let root = match file_root {
            FileRoot::Workspace => thread_root.clone(),
            FileRoot::Reference => resolve_reference_repo_root(&thread_root).await?,
        };

        let root_for_task = root.clone();
        let query_for_task = query.clone();
        let is_regex = params.is_regex;
        let include_glob_for_task = params.include_glob.clone();
        let grep_outcome = tokio::task::spawn_blocking(move || {
            omne_repo_scan_runtime::search_repo(omne_repo_scan_runtime::RepoGrepRequest {
                root: root_for_task,
                query: query_for_task,
                is_regex,
                include_glob: include_glob_for_task,
                max_matches,
                max_bytes_per_file,
                max_files,
            })
        })
        .await
        .context("join repo/search task")??;

        let include_glob = params
            .include_glob
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let summary = match include_glob {
            Some(glob) => format!("rg: {query} ({glob})"),
            None => format!("rg: {query}"),
        };

        let artifact_text = format_repo_search_artifact(
            file_root.as_str(),
            &query,
            params.is_regex,
            include_glob,
            &grep_outcome,
        );

        let (mut artifact_response, _artifact_completed) = write_user_artifact(
            server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                artifact_id: None,
                artifact_type: "repo_search".to_string(),
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
            "matches": grep_outcome.matches.len(),
            "truncated": grep_outcome.truncated,
            "files_scanned": grep_outcome.files_scanned,
            "files_skipped_too_large": grep_outcome.files_skipped_too_large,
            "files_skipped_binary": grep_outcome.files_skipped_binary,
        });

        if let Some(obj) = artifact_response.as_object_mut() {
            obj.insert("root".to_string(), serde_json::json!(file_root.as_str()));
            obj.insert("matches".to_string(), serde_json::json!(grep_outcome.matches.len()));
            obj.insert("truncated".to_string(), serde_json::json!(grep_outcome.truncated));
            obj.insert(
                "files_scanned".to_string(),
                serde_json::json!(grep_outcome.files_scanned),
            );
            obj.insert(
                "files_skipped_too_large".to_string(),
                serde_json::json!(grep_outcome.files_skipped_too_large),
            );
            obj.insert(
                "files_skipped_binary".to_string(),
                serde_json::json!(grep_outcome.files_skipped_binary),
            );
        }

        let response: omne_app_server_protocol::RepoSearchResponse =
            serde_json::from_value(artifact_response).context("parse repo/search response")?;
        let response_value =
            serde_json::to_value(response).context("serialize repo/search response")?;
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
