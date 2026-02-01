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
    let tool_id = omne_agent_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "include_glob": params.include_glob.clone(),
        "max_files": max_files,
        "max_bytes_per_file": max_bytes_per_file,
        "max_symbols": max_symbols,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/symbols",
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
                    tool: "repo/symbols".to_string(),
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
    let effective_decision = match mode.tool_overrides.get("repo/symbols").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == omne_agent_core::modes::Decision::Deny {
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "repo/symbols".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_agent_protocol::ToolStatus::Denied,
                error: Some("mode denies repo/symbols".to_string()),
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
                action: "repo/symbols",
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
                        tool: "repo/symbols".to_string(),
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

        let root_for_task = root.clone();
        let include_glob_for_task = include_glob.clone();
        let symbols_outcome =
            tokio::task::spawn_blocking(move || -> anyhow::Result<RepoSymbolsOutcome> {
                let include_matcher = Glob::new(&include_glob_for_task)
                    .with_context(|| format!("invalid glob pattern: {include_glob_for_task}"))?
                .compile_matcher();

            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .context("set tree-sitter language (rust)")?;

            let mut symbols = Vec::<RepoSymbol>::new();
            let mut truncated_files = false;
            let mut truncated_symbols = false;
            let mut files_scanned = 0usize;
            let mut files_parsed = 0usize;
            let mut files_skipped_too_large = 0usize;
            let mut files_skipped_binary = 0usize;
            let mut files_failed_parse = 0usize;

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
                    truncated_files = true;
                    break;
                }

                let rel = entry
                    .path()
                    .strip_prefix(&root_for_task)
                    .unwrap_or(entry.path());
                if rel_path_is_secret(rel) {
                    continue;
                }
                if !include_matcher.is_match(rel) {
                    continue;
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

                let Ok(source) = std::str::from_utf8(&bytes) else {
                    files_failed_parse += 1;
                    continue;
                };

                let Some(tree) = parser.parse(source, None) else {
                    files_failed_parse += 1;
                    continue;
                };
                files_parsed += 1;

                let rel_str = rel.to_string_lossy().to_string();
                let remaining = max_symbols.saturating_sub(symbols.len());
                if remaining == 0 {
                    truncated_symbols = true;
                    break;
                }

                let mut module_stack = Vec::<String>::new();
                collect_rust_symbols(
                    tree.root_node(),
                    source,
                    &rel_str,
                    &mut module_stack,
                    &mut symbols,
                    max_symbols,
                );

                if symbols.len() >= max_symbols {
                    truncated_symbols = true;
                    break;
                }
            }

            Ok(RepoSymbolsOutcome {
                symbols,
                truncated_files,
                truncated_symbols,
                files_scanned,
                files_parsed,
                files_skipped_too_large,
                files_skipped_binary,
                files_failed_parse,
            })
        })
        .await
        .context("join repo/symbols task")??;

        let include_glob_str = include_glob.trim();
        let summary = format!("repo/symbols ({include_glob_str})");

        let artifact_text = format_repo_symbols_artifact(
            file_root,
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

fn collect_rust_symbols(
    node: tree_sitter::Node<'_>,
    source: &str,
    path: &str,
    module_stack: &mut Vec<String>,
    out: &mut Vec<RepoSymbol>,
    max_symbols: usize,
) {
    if out.len() >= max_symbols {
        return;
    }

    let kind = node.kind();

    let mut entered_module = false;
    if kind == "mod_item"
        && let Some(name_node) = node.child_by_field_name("name")
        && let Some(name) = source.get(name_node.byte_range())
    {
        let full = if module_stack.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", module_stack.join("::"))
        };
        let start_line = node.start_position().row.saturating_add(1);
        let end_line = node.end_position().row.saturating_add(1);
        out.push(RepoSymbol {
            path: path.to_string(),
            kind: "mod",
            name: full.clone(),
            start_line,
            end_line,
        });
        module_stack.push(name.to_string());
        entered_module = true;
    }

    if matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
    ) && let Some(name_node) = node.child_by_field_name("name")
        && let Some(name) = source.get(name_node.byte_range())
    {
        let prefix = if module_stack.is_empty() {
            String::new()
        } else {
            format!("{}::", module_stack.join("::"))
        };
        let symbol_kind = match kind {
            "function_item" => "fn",
            "struct_item" => "struct",
            "enum_item" => "enum",
            "trait_item" => "trait",
            "type_item" => "type",
            "const_item" => "const",
            "static_item" => "static",
            _ => kind,
        };
        let start_line = node.start_position().row.saturating_add(1);
        let end_line = node.end_position().row.saturating_add(1);
        out.push(RepoSymbol {
            path: path.to_string(),
            kind: symbol_kind,
            name: format!("{prefix}{name}"),
            start_line,
            end_line,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if out.len() >= max_symbols {
            break;
        }
        collect_rust_symbols(child, source, path, module_stack, out, max_symbols);
    }

    if entered_module {
        let _ = module_stack.pop();
    }
}

