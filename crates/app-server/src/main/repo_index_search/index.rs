#[derive(Debug)]
struct RepoIndexOutcome {
    paths: Vec<String>,
    truncated: bool,
    files_scanned: usize,
    size_bytes: u64,
}

#[derive(Clone, Debug)]
struct RepoSymbol {
    path: String,
    kind: &'static str,
    name: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Debug)]
struct RepoSymbolsOutcome {
    symbols: Vec<RepoSymbol>,
    truncated_files: bool,
    truncated_symbols: bool,
    files_scanned: usize,
    files_parsed: usize,
    files_skipped_too_large: usize,
    files_skipped_binary: usize,
    files_failed_parse: usize,
}

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
    let tool_id = omne_agent_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "include_glob": params.include_glob.clone(),
        "max_files": max_files,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/index",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let catalog = omne_agent_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = omne_agent_core::modes::Decision::Deny;

            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "repo/index".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.read.combine(mode.permissions.artifact);
    let effective_decision = match mode.tool_overrides.get("repo/index").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == omne_agent_core::modes::Decision::Deny {
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "repo/index".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_agent_protocol::ToolStatus::Denied,
                error: Some("mode denies repo/index".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == omne_agent_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "repo/index",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "repo/index".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
                        error: Some(approval_denied_error(remembered).to_string()),
                        result: Some(serde_json::json!({
                            "approval_policy": approval_policy,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
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
        let index_outcome =
            tokio::task::spawn_blocking(move || -> anyhow::Result<RepoIndexOutcome> {
                let include_matcher = match include_glob.as_deref() {
                    Some(glob) => Some(
                        Glob::new(glob)
                            .with_context(|| format!("invalid glob pattern: {glob}"))?
                            .compile_matcher(),
                    ),
                    None => None,
                };

                let mut paths = Vec::<String>::new();
                let mut truncated = false;
                let mut files_scanned = 0usize;
                let mut size_bytes = 0u64;

                const MAX_LISTED_PATHS: usize = 2000;

                for entry in WalkDir::new(&root_for_task)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(should_walk_entry)
                {
                    let entry = entry?;
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    if files_scanned >= max_files {
                        truncated = true;
                        break;
                    }

                    let rel = entry
                        .path()
                        .strip_prefix(&root_for_task)
                        .unwrap_or(entry.path());
                    if rel_path_is_secret(rel) {
                        continue;
                    }
                    if let Some(ref matcher) = include_matcher {
                        if !matcher.is_match(rel) {
                            continue;
                        }
                    }

                    files_scanned += 1;

                    let meta = entry.metadata()?;
                    size_bytes = size_bytes.saturating_add(meta.len());

                    if paths.len() < MAX_LISTED_PATHS {
                        paths.push(rel.to_string_lossy().to_string());
                    }
                }

                paths.sort();

                Ok(RepoIndexOutcome {
                    paths,
                    truncated,
                    files_scanned,
                    size_bytes,
                })
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

        let artifact_text =
            format_repo_index_artifact(file_root, include_glob_str, max_files, &index_outcome);

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

        Ok((artifact_response, completed))
    }
    .await;

    match outcome {
        Ok((artifact_response, completed)) => {
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(artifact_response)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

