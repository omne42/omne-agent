#[derive(Debug, Serialize)]
struct GrepMatch {
    path: String,
    line_number: u64,
    line: String,
}

async fn handle_file_grep(server: &Server, params: FileGrepParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

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
        "query": params.query.clone(),
        "is_regex": params.is_regex,
        "include_glob": params.include_glob.clone(),
        "max_matches": max_matches,
        "max_bytes_per_file": max_bytes_per_file,
        "max_files": max_files,
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
        "file/grep",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return file_allowed_tools_denied_response(tool_id, "file/grep", &allowed_tools);
    }
    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = file_unknown_mode_denied_response(
                tool_id,
                &mode_name,
                available,
                catalog.load_error.clone(),
            )?;

            emit_file_tool_denied(
                &thread_rt,
                tool_id,
                params.turn_id,
                "file/grep",
                &approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(result);
        }
    };
    let mode_decision = resolve_mode_decision_audit(mode, "file/grep", mode.permissions.read);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = file_mode_denied_response(tool_id, &mode_name, mode_decision)?;
        emit_file_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "file/grep",
            &approval_params,
            "mode denies file/grep".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "file/grep",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = file_denied_response(tool_id, Some(remembered))?;
                emit_file_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "file/grep",
                    &approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return file_needs_approval_response(approval_id);
            }
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/grep".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

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
    let query_for_task = params.query.clone();
    let is_regex = params.is_regex;
    let include_glob_for_task = params.include_glob.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        omne_repo_scan_runtime::search_repo(omne_repo_scan_runtime::RepoGrepRequest {
            root,
            query: query_for_task,
            is_regex,
            include_glob: include_glob_for_task,
            max_matches,
            max_bytes_per_file,
            max_files,
        })
    })
    .await
    .context("join grep task")?;

    match outcome {
        Ok(omne_repo_scan_runtime::RepoGrepOutcome {
            matches,
            truncated,
            files_scanned,
            files_skipped_too_large,
            files_skipped_binary,
        }) => {
            let matches = matches
                .into_iter()
                .map(|m| GrepMatch {
                    path: m.path,
                    line_number: m.line_number,
                    line: m.line,
                })
                .collect::<Vec<_>>();
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "matches": matches.len(),
                        "truncated": truncated,
                        "files_scanned": files_scanned,
                        "files_skipped_too_large": files_skipped_too_large,
                        "files_skipped_binary": files_skipped_binary,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "root": file_root.as_str(),
                "matches": matches,
                "truncated": truncated,
                "files_scanned": files_scanned,
                "files_skipped_too_large": files_skipped_too_large,
                "files_skipped_binary": files_skipped_binary,
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
