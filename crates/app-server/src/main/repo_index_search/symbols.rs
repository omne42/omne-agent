async fn handle_repo_symbols(server: &Server, params: RepoSymbolsParams) -> anyhow::Result<Value> {
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
    let max_bytes_per_file = params
        .max_bytes_per_file
        .unwrap_or(1024 * 1024)
        .min(16 * 1024 * 1024);
    let max_symbols = params.max_symbols.unwrap_or(5000).min(50_000);
    let tool_id = omne_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "include_glob": params.include_glob.clone(),
        "max_files": max_files,
        "max_bytes_per_file": max_bytes_per_file,
        "max_symbols": max_symbols,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/symbols",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return repo_allowed_tools_denied_response(tool_id, "repo/symbols", &allowed_tools);
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
            action: "repo/symbols",
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
            tool: "repo/symbols".to_string(),
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
            .map(ToString::to_string)
            .unwrap_or_else(|| "**/*.rs".to_string());

        let include_glob_for_task = include_glob.clone();
        let symbols_outcome = tokio::task::spawn_blocking(move || {
            omne_repo_symbols_runtime::collect_repo_symbols(
                omne_repo_symbols_runtime::RepoSymbolsRequest {
                    root,
                    include_glob: include_glob_for_task,
                    max_files,
                    max_bytes_per_file,
                    max_symbols,
                },
            )
        })
            .await
            .context("join repo/symbols task")??;

        let include_glob_str = include_glob.trim();
        let summary = format!("repo/symbols ({include_glob_str})");

        let artifact_text = format_repo_symbols_artifact(
            file_root.as_str(),
            include_glob_str,
            max_files,
            max_bytes_per_file,
            max_symbols,
            &symbols_outcome,
        );

        let (mut artifact_response, _artifact_completed) = write_user_artifact(
            server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                artifact_id: None,
                artifact_type: "repo_symbols".to_string(),
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
            "files_scanned": symbols_outcome.files_scanned,
            "files_parsed": symbols_outcome.files_parsed,
            "symbols": symbols_outcome.symbols.len(),
            "truncated_files": symbols_outcome.truncated_files,
            "truncated_symbols": symbols_outcome.truncated_symbols,
            "files_skipped_too_large": symbols_outcome.files_skipped_too_large,
            "files_skipped_binary": symbols_outcome.files_skipped_binary,
            "files_failed_parse": symbols_outcome.files_failed_parse,
        });

        if let Some(obj) = artifact_response.as_object_mut() {
            obj.insert("root".to_string(), serde_json::json!(file_root.as_str()));
            obj.insert("symbols".to_string(), serde_json::json!(symbols_outcome.symbols.len()));
            obj.insert(
                "files_scanned".to_string(),
                serde_json::json!(symbols_outcome.files_scanned),
            );
            obj.insert(
                "files_parsed".to_string(),
                serde_json::json!(symbols_outcome.files_parsed),
            );
            obj.insert(
                "truncated_files".to_string(),
                serde_json::json!(symbols_outcome.truncated_files),
            );
            obj.insert(
                "truncated_symbols".to_string(),
                serde_json::json!(symbols_outcome.truncated_symbols),
            );
            obj.insert(
                "files_skipped_too_large".to_string(),
                serde_json::json!(symbols_outcome.files_skipped_too_large),
            );
            obj.insert(
                "files_skipped_binary".to_string(),
                serde_json::json!(symbols_outcome.files_skipped_binary),
            );
            obj.insert(
                "files_failed_parse".to_string(),
                serde_json::json!(symbols_outcome.files_failed_parse),
            );
        }

        let response: omne_app_server_protocol::RepoSymbolsResponse =
            serde_json::from_value(artifact_response).context("parse repo/symbols response")?;
        let response_value =
            serde_json::to_value(response).context("serialize repo/symbols response")?;
        Ok((response_value, completed))
    }
    .await;

    match outcome {
        Ok((artifact_response, completed)) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
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
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}
