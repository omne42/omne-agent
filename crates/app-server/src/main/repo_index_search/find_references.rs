fn pick_repo_reference_matches(
    outcome: &omne_repo_scan_runtime::RepoGrepOutcome,
    path_hint: Option<&str>,
) -> Vec<omne_repo_scan_runtime::RepoGrepMatch> {
    let Some(path_hint) = path_hint else {
        return outcome.matches.clone();
    };

    let mut preferred = outcome
        .matches
        .iter()
        .filter(|m| m.path == path_hint || m.path.ends_with(path_hint) || m.path.contains(path_hint))
        .cloned()
        .collect::<Vec<_>>();

    if preferred.is_empty() {
        outcome.matches.clone()
    } else {
        preferred.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| a.line_number.cmp(&b.line_number))
        });
        preferred
    }
}

async fn handle_repo_find_references(
    server: &Server,
    params: RepoFindReferencesParams,
) -> anyhow::Result<Value> {
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

    let symbol = params.symbol.trim().to_string();
    if symbol.is_empty() {
        anyhow::bail!("symbol must not be empty");
    }

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_matches = params.max_matches.unwrap_or(300).clamp(1, 5000);
    let max_bytes_per_file = params
        .max_bytes_per_file
        .unwrap_or(1024 * 1024)
        .min(16 * 1024 * 1024);
    let max_files = params.max_files.unwrap_or(20_000).min(200_000);
    let tool_id = omne_protocol::ToolId::new();

    let path_hint = params
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "symbol": symbol.clone(),
        "path": path_hint.clone(),
        "include_glob": params.include_glob.clone(),
        "max_matches": max_matches,
        "max_bytes_per_file": max_bytes_per_file,
        "max_files": max_files,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/find_references",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return repo_allowed_tools_denied_response(tool_id, "repo/find_references", &allowed_tools);
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
            action: "repo/find_references",
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
            tool: "repo/find_references".to_string(),
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

        let search_symbol = symbol_tail_name(&symbol);
        let regex_query = format!(r"\b{}\b", regex::escape(search_symbol));
        let include_glob_for_task = include_glob.clone();
        let grep_outcome = tokio::task::spawn_blocking(move || {
            omne_repo_scan_runtime::search_repo(omne_repo_scan_runtime::RepoGrepRequest {
                root,
                query: regex_query,
                is_regex: true,
                include_glob: Some(include_glob_for_task),
                max_matches,
                max_bytes_per_file,
                max_files,
            })
        })
        .await
        .context("join repo/find_references task")??;

        let references = pick_repo_reference_matches(&grep_outcome, path_hint.as_deref());
        let include_glob_str = include_glob.trim();
        let summary = format!("repo/find_references: {}", symbol_tail_name(&symbol));
        let artifact_text = format_repo_find_references_artifact(
            file_root.as_str(),
            &symbol,
            path_hint.as_deref(),
            include_glob_str,
            max_matches,
            &grep_outcome,
            &references,
        );

        let (mut artifact_response, _artifact_completed) = write_user_artifact(
            server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                artifact_id: None,
                artifact_type: "repo_find_references".to_string(),
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
            "symbol": symbol,
            "references": references.len(),
            "truncated": grep_outcome.truncated,
            "files_scanned": grep_outcome.files_scanned,
            "files_skipped_too_large": grep_outcome.files_skipped_too_large,
            "files_skipped_binary": grep_outcome.files_skipped_binary,
        });

        if let Some(obj) = artifact_response.as_object_mut() {
            obj.insert("root".to_string(), serde_json::json!(file_root.as_str()));
            obj.insert("symbol".to_string(), serde_json::json!(symbol));
            obj.insert("references".to_string(), serde_json::json!(references.len()));
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

        let response: omne_app_server_protocol::RepoFindReferencesResponse =
            serde_json::from_value(artifact_response)
                .context("parse repo/find_references response")?;
        let response_value =
            serde_json::to_value(response).context("serialize repo/find_references response")?;
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
