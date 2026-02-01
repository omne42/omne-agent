#[derive(Debug, Serialize)]
struct GrepMatch {
    path: String,
    line_number: u64,
    line: String,
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in line.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
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
    let tool_id = omne_agent_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "path_prefix": params.path_prefix.clone(),
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
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/grep",
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
                    tool: "file/grep".to_string(),
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
    let base_decision = mode.permissions.read;
    let effective_decision = match mode.tool_overrides.get("file/grep").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == omne_agent_core::modes::Decision::Deny {
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/grep".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_agent_protocol::ToolStatus::Denied,
                error: Some("mode denies file/grep".to_string()),
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
                action: "file/grep",
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
                        tool: "file/grep".to_string(),
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
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/grep".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    if matches!(file_root, FileRoot::Workspace)
        && let Some(db_vfs) = server.db_vfs.clone()
    {
        let path_prefix = params.path_prefix.clone().map(|p| p.replace('\\', "/"));
        let resp = db_vfs
            .grep(
                params.thread_id.to_string(),
                params.query.clone(),
                params.is_regex,
                params.include_glob.clone(),
                path_prefix,
            )
            .await;

        match resp {
            Ok(resp) => {
                let mut matches = resp
                    .matches
                    .into_iter()
                    .map(|m| GrepMatch {
                        path: m.path,
                        line_number: m.line,
                        line: truncate_line(&m.text, 4000),
                    })
                    .collect::<Vec<_>>();

                let mut truncated = resp.truncated;
                if matches.len() > max_matches {
                    matches.truncate(max_matches);
                    truncated = true;
                }

                let files_scanned = resp.scanned_files as usize;
                let files_skipped_too_large = resp.skipped_too_large_files as usize;
                let files_skipped_binary = 0usize;

                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Completed,
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
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "root": file_root.as_str(),
                    "matches": matches,
                    "truncated": truncated,
                    "files_scanned": files_scanned,
                    "files_skipped_too_large": files_skipped_too_large,
                    "files_skipped_binary": files_skipped_binary,
                }));
            }
            Err(err) if err.is_denied() => {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Denied,
                        error: Some(err.to_string()),
                        result: Some(serde_json::json!({
                            "db_vfs_code": err.code,
                            "db_vfs_status": err.status.map(|status| status.as_u16()),
                        })),
                    })
                    .await?;
                let mut result = serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "db_vfs_code": err.code,
                    "db_vfs_status": err.status.map(|status| status.as_u16()),
                });
                if err.code.as_deref() == Some("not_permitted") {
                    result["reason"] = serde_json::Value::String(err.message.clone());
                }
                return Ok(result);
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
                return Err(anyhow::anyhow!(err));
            }
        }
    }

    let pattern = if params.is_regex {
        params.query.clone()
    } else {
        regex::escape(&params.query)
    };
    let re = Regex::new(&pattern).with_context(|| format!("invalid regex: {}", params.query))?;
    let include_matcher = match params.include_glob.as_deref() {
        Some(glob) => Some(
            Glob::new(glob)
                .with_context(|| format!("invalid glob pattern: {glob}"))?
                .compile_matcher(),
        ),
        None => None,
    };

    let path_prefix = params.path_prefix.clone();
    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
                thread_rt
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_agent_protocol::ToolStatus::Failed,
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
    let outcome = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(Vec<GrepMatch>, bool, usize, usize, usize)> {
            let path_prefix = match path_prefix.as_deref() {
                Some(prefix) => Some(normalize_path_prefix_for_filter(prefix)?),
                None => None,
            };
            let mut matches = Vec::new();
            let mut truncated = false;
            let mut files_scanned = 0usize;
            let mut files_skipped_too_large = 0usize;
            let mut files_skipped_binary = 0usize;

            for entry in WalkDir::new(&root)
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
                let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                if rel_path_is_secret(rel) {
                    continue;
                }
                let rel_string = rel.to_string_lossy().replace('\\', "/");
                if let Some(prefix) = path_prefix.as_deref() {
                    if !rel_string.starts_with(prefix) {
                        continue;
                    }
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
                        path: rel_string.clone(),
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

            Ok((
                matches,
                truncated,
                files_scanned,
                files_skipped_too_large,
                files_skipped_binary,
            ))
        },
    )
    .await
    .context("join grep task")?;

    match outcome {
        Ok((matches, truncated, files_scanned, files_skipped_too_large, files_skipped_binary)) => {
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Completed,
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
