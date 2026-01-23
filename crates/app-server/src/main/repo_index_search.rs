#[derive(Debug)]
struct RepoGrepOutcome {
    matches: Vec<GrepMatch>,
    truncated: bool,
    files_scanned: usize,
    files_skipped_too_large: usize,
    files_skipped_binary: usize,
}

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
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "query": query.clone(),
        "is_regex": params.is_regex,
        "include_glob": params.include_glob.clone(),
        "max_matches": max_matches,
        "max_bytes_per_file": max_bytes_per_file,
        "max_files": max_files,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/search",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "repo/search".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
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
    let effective_decision = match mode.tool_overrides.get("repo/search").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "repo/search".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies repo/search".to_string()),
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

    if effective_decision == pm_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "repo/search",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "repo/search".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
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
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
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

        let pattern = if params.is_regex {
            query.clone()
        } else {
            regex::escape(&query)
        };
        let re = Regex::new(&pattern).with_context(|| format!("invalid regex: {query}"))?;

        let include_matcher = match params.include_glob.as_deref() {
            Some(glob) => Some(
                Glob::new(glob)
                    .with_context(|| format!("invalid glob pattern: {glob}"))?
                    .compile_matcher(),
            ),
            None => None,
        };

        let root_for_task = root.clone();
        let grep_outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<RepoGrepOutcome> {
            let mut matches = Vec::new();
            let mut truncated = false;
            let mut files_scanned = 0usize;
            let mut files_skipped_too_large = 0usize;
            let mut files_skipped_binary = 0usize;

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
                if meta.len() > max_bytes_per_file {
                    files_skipped_too_large += 1;
                    continue;
                }

                let bytes = match std::fs::read(entry.path()) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                if bytes.contains(&0) {
                    files_skipped_binary += 1;
                    continue;
                }

                let text = String::from_utf8_lossy(&bytes);
                for (idx, line) in text.lines().enumerate() {
                    if !re.is_match(line) {
                        continue;
                    }

                    matches.push(GrepMatch {
                        path: rel.to_string_lossy().to_string(),
                        line_number: (idx + 1) as u64,
                        line: truncate_line(line, 4000),
                    });
                    if matches.len() >= max_matches {
                        truncated = true;
                        break;
                    }
                }

                if truncated {
                    break;
                }
            }

            Ok(RepoGrepOutcome {
                matches,
                truncated,
                files_scanned,
                files_skipped_too_large,
                files_skipped_binary,
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
            file_root,
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

        Ok((artifact_response, completed))
    }
    .await;

    match outcome {
        Ok((artifact_response, completed)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(artifact_response)
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

#[derive(Debug)]
struct RepoIndexOutcome {
    paths: Vec<String>,
    truncated: bool,
    files_scanned: usize,
    size_bytes: u64,
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
    let tool_id = pm_protocol::ToolId::new();

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

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "repo/index".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
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
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "repo/index".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
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

    if effective_decision == pm_core::modes::Decision::Prompt {
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
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "repo/index".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Denied,
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
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
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
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(artifact_response)
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

fn format_repo_search_artifact(
    root: FileRoot,
    query: &str,
    is_regex: bool,
    include_glob: Option<&str>,
    outcome: &RepoGrepOutcome,
) -> String {
    let mut out = String::new();
    out.push_str("# Repo Search\n\n");
    out.push_str("## Query\n");
    out.push_str(&format!("- root: `{}`\n", root.as_str()));
    out.push_str(&format!("- query: `{}`\n", query.trim()));
    out.push_str(&format!("- is_regex: `{}`\n", is_regex));
    if let Some(glob) = include_glob {
        out.push_str(&format!("- include_glob: `{glob}`\n"));
    } else {
        out.push_str("- include_glob: (none)\n");
    }

    out.push_str("\n## Stats\n");
    let stats = serde_json::json!({
        "matches": outcome.matches.len(),
        "truncated": outcome.truncated,
        "files_scanned": outcome.files_scanned,
        "files_skipped_too_large": outcome.files_skipped_too_large,
        "files_skipped_binary": outcome.files_skipped_binary,
    });
    match serde_json::to_string_pretty(&stats) {
        Ok(json) => out.push_str(&format!("```json\n{json}\n```\n")),
        Err(_) => out.push_str(&format!("```json\n{}\n```\n", stats)),
    }

    out.push_str("\n## Results\n");
    out.push_str("```text\n");
    for m in &outcome.matches {
        out.push_str(&format!(
            "{}:{}: {}\n",
            m.path,
            m.line_number,
            m.line.replace('\n', " ")
        ));
    }
    if outcome.truncated {
        out.push_str("… (truncated)\n");
    }
    out.push_str("```\n");
    out
}

fn format_repo_index_artifact(
    root: FileRoot,
    include_glob: Option<&str>,
    max_files: usize,
    outcome: &RepoIndexOutcome,
) -> String {
    let mut out = String::new();
    out.push_str("# Repo Index\n\n");

    out.push_str("## Config\n");
    out.push_str(&format!("- root: `{}`\n", root.as_str()));
    if let Some(glob) = include_glob {
        out.push_str(&format!("- include_glob: `{glob}`\n"));
    } else {
        out.push_str("- include_glob: (none)\n");
    }
    out.push_str(&format!("- max_files: `{max_files}`\n"));

    out.push_str("\n## Stats\n");
    let stats = serde_json::json!({
        "files_scanned": outcome.files_scanned,
        "truncated": outcome.truncated,
        "size_bytes": outcome.size_bytes,
        "paths_listed": outcome.paths.len(),
    });
    match serde_json::to_string_pretty(&stats) {
        Ok(json) => out.push_str(&format!("```json\n{json}\n```\n")),
        Err(_) => out.push_str(&format!("```json\n{}\n```\n", stats)),
    }

    out.push_str("\n## Sample Paths\n");
    out.push_str("```text\n");
    for path in &outcome.paths {
        out.push_str(path);
        out.push('\n');
    }
    if outcome.truncated {
        out.push_str("… (truncated)\n");
    }
    out.push_str("```\n");
    out
}
