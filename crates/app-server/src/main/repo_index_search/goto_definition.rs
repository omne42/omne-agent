fn symbol_tail_name(symbol: &str) -> &str {
    symbol.rsplit("::").next().unwrap_or(symbol)
}

fn definition_match_score(
    query: &str,
    query_tail: &str,
    candidate: &omne_repo_symbols_runtime::RepoSymbol,
    path_hint: Option<&str>,
) -> i32 {
    let mut score = 0i32;
    let candidate_name = candidate.name.trim();
    let candidate_tail = symbol_tail_name(candidate_name);

    if candidate_name == query {
        score += 120;
    }
    if candidate_name.ends_with(&format!("::{query}")) {
        score += 100;
    }
    if candidate_tail == query_tail {
        score += 80;
    }
    if candidate_name.contains(query) {
        score += 30;
    }

    if let Some(hint) = path_hint {
        if candidate.path == hint {
            score += 40;
        } else if candidate.path.ends_with(hint) {
            score += 30;
        } else if candidate.path.contains(hint) {
            score += 20;
        }
    }

    score
}

fn select_repo_definition_candidates<'a>(
    symbols: &'a [omne_repo_symbols_runtime::RepoSymbol],
    symbol: &str,
    path_hint: Option<&str>,
    max_results: usize,
) -> Vec<&'a omne_repo_symbols_runtime::RepoSymbol> {
    let query = symbol.trim().trim_end_matches("()").trim();
    let query_tail = symbol_tail_name(query);

    let mut ranked = symbols
        .iter()
        .filter_map(|candidate| {
            let score = definition_match_score(query, query_tail, candidate, path_hint);
            (score > 0).then_some((score, candidate))
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|(score_a, a), (score_b, b)| {
        score_b
            .cmp(score_a)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.name.cmp(&b.name))
    });

    ranked
        .into_iter()
        .take(max_results)
        .map(|(_, candidate)| candidate)
        .collect()
}

async fn handle_repo_goto_definition(
    server: &Server,
    params: RepoGotoDefinitionParams,
) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, role_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.role.clone(),
            state.allowed_tools.clone(),
        )
    };

    let symbol = params.symbol.trim().to_string();
    if symbol.is_empty() {
        anyhow::bail!("symbol must not be empty");
    }

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_results = params.max_results.unwrap_or(20).clamp(1, 200);
    let max_files = params.max_files.unwrap_or(20_000).min(200_000);
    let max_bytes_per_file = params
        .max_bytes_per_file
        .unwrap_or(1024 * 1024)
        .min(16 * 1024 * 1024);
    let max_symbols = params.max_symbols.unwrap_or(10_000).min(100_000);
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
        "max_results": max_results,
        "max_files": max_files,
        "max_bytes_per_file": max_bytes_per_file,
        "max_symbols": max_symbols,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "repo/goto_definition",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return repo_allowed_tools_denied_response(tool_id, "repo/goto_definition", &allowed_tools);
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
            role_name: &role_name,
            action: "repo/goto_definition",
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
            tool: "repo/goto_definition".to_string(),
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
        .context("join repo/goto_definition task")??;

        let definition_candidates = select_repo_definition_candidates(
            &symbols_outcome.symbols,
            &symbol,
            path_hint.as_deref(),
            max_results,
        );
        let include_glob_str = include_glob.trim();
        let summary = format!("repo/goto_definition: {}", symbol_tail_name(&symbol));
        let artifact_text = format_repo_goto_definition_artifact(
            file_root.as_str(),
            &symbol,
            path_hint.as_deref(),
            include_glob_str,
            max_results,
            &symbols_outcome,
            &definition_candidates,
        );

        let (mut artifact_response, _artifact_completed) = write_user_artifact(
            server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id: params.thread_id,
                turn_id: params.turn_id,
                artifact_id: None,
                artifact_type: "repo_goto_definition".to_string(),
                summary,
                text: artifact_text,
            },
        )
        .await?;

        let artifact_id = artifact_response
            .get("artifact_id")
            .cloned()
            .unwrap_or(Value::Null);
        let resolved = !definition_candidates.is_empty();

        let completed = serde_json::json!({
            "artifact_id": artifact_id,
            "symbol": symbol,
            "definitions": definition_candidates.len(),
            "resolved": resolved,
            "files_scanned": symbols_outcome.files_scanned,
            "files_parsed": symbols_outcome.files_parsed,
            "truncated_files": symbols_outcome.truncated_files,
            "truncated_symbols": symbols_outcome.truncated_symbols,
            "files_skipped_too_large": symbols_outcome.files_skipped_too_large,
            "files_skipped_binary": symbols_outcome.files_skipped_binary,
            "files_failed_parse": symbols_outcome.files_failed_parse,
        });

        if let Some(obj) = artifact_response.as_object_mut() {
            obj.insert("root".to_string(), serde_json::json!(file_root.as_str()));
            obj.insert("symbol".to_string(), serde_json::json!(symbol));
            obj.insert(
                "definitions".to_string(),
                serde_json::json!(definition_candidates.len()),
            );
            obj.insert("resolved".to_string(), serde_json::json!(resolved));
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

        let response: omne_app_server_protocol::RepoGotoDefinitionResponse =
            serde_json::from_value(artifact_response)
                .context("parse repo/goto_definition response")?;
        let response_value =
            serde_json::to_value(response).context("serialize repo/goto_definition response")?;
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

#[cfg(test)]
mod goto_definition_tests {
    use super::{select_repo_definition_candidates, symbol_tail_name};

    fn repo_symbol(
        path: &str,
        kind: &str,
        name: &str,
        start_line: usize,
    ) -> omne_repo_symbols_runtime::RepoSymbol {
        omne_repo_symbols_runtime::RepoSymbol {
            path: path.to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            start_line,
            end_line: start_line,
        }
    }

    #[test]
    fn symbol_tail_name_handles_qualified_symbols() {
        assert_eq!(symbol_tail_name("foo::bar::Baz"), "Baz");
        assert_eq!(symbol_tail_name("Widget"), "Widget");
    }

    #[test]
    fn goto_definition_prefers_exact_qualified_name_over_tail_match() {
        let symbols = vec![
            repo_symbol("src/bar.rs", "struct", "bar::Thing", 8),
            repo_symbol("src/foo.rs", "struct", "foo::Thing", 4),
        ];

        let ranked = select_repo_definition_candidates(&symbols, "foo::Thing", None, 10);
        assert_eq!(
            ranked.first().map(|symbol| symbol.name.as_str()),
            Some("foo::Thing")
        );
    }

    #[test]
    fn goto_definition_uses_path_hint_to_break_tail_ties() {
        let symbols = vec![
            repo_symbol("src/bar.rs", "struct", "bar::Thing", 8),
            repo_symbol("src/foo.rs", "struct", "foo::Thing", 4),
        ];

        let ranked = select_repo_definition_candidates(&symbols, "Thing", Some("src/foo.rs"), 10);
        assert_eq!(
            ranked.first().map(|symbol| symbol.path.as_str()),
            Some("src/foo.rs")
        );
    }
}
