async fn handle_process_tail(server: &Server, params: ProcessTailParams) -> anyhow::Result<Value> {
    let max_lines = params.max_lines.unwrap_or(200).min(2000);
    let stream = stream_label(params.stream);

    let info = resolve_process_info(server, params.process_id).await?;
    let (thread_rt, thread_root) = load_thread_root(server, info.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    let tool_id = omne_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "process_id": params.process_id,
        "stream": stream,
        "max_lines": max_lines,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "process/tail",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return process_allowed_tools_denied_response(tool_id, "process/tail", &allowed_tools);
    }

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = process_unknown_mode_denied_response(
                tool_id,
                info.thread_id,
                &mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_process_tool_denied(
                &thread_rt,
                tool_id,
                params.turn_id,
                "process/tail",
                &approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(result);
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(mode, "process/tail", mode.permissions.process.inspect);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = process_mode_denied_response(tool_id, info.thread_id, &mode_name, mode_decision)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/tail",
            &approval_params,
            "mode denies process/tail".to_string(),
            result.clone(),
        )
        .await?;
        return Ok(result);
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            info.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "process/tail",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = process_denied_response(tool_id, info.thread_id, Some(remembered))?;
                emit_process_tool_denied(
                    &thread_rt,
                    tool_id,
                    params.turn_id,
                    "process/tail",
                    &approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(result);
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return process_needs_approval_response(info.thread_id, approval_id);
            }
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "process/tail".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let path = match params.stream {
        ProcessStream::Stdout => info.stdout_path,
        ProcessStream::Stderr => info.stderr_path,
    };

    let outcome = tail_file_lines(PathBuf::from(&path), max_lines).await;
    match outcome {
        Ok(text) => {
            let text = omne_core::redact_text(&text);
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "path": path,
                        "max_lines": max_lines,
                    })),
                })
                .await?;
            let response = omne_app_server_protocol::ProcessTailResponse { tool_id, text };
            serde_json::to_value(response).context("serialize process/tail response")
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

async fn tail_file_lines(path: PathBuf, max_lines: usize) -> anyhow::Result<String> {
    let files = list_rotating_log_files(&path).await?;
    if files.is_empty() {
        return Ok(String::new());
    }

    let mut collected = Vec::<String>::new();
    for file in files.into_iter().rev() {
        let lines = tail_single_file_lines(&file).await?;
        for line in lines.into_iter().rev() {
            collected.push(line);
            if collected.len() >= max_lines {
                break;
            }
        }
        if collected.len() >= max_lines {
            break;
        }
    }

    collected.reverse();
    Ok(collected.join("\n"))
}

async fn tail_single_file_lines(path: &Path) -> anyhow::Result<Vec<String>> {
    let max_bytes: u64 = 256 * 1024;
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };
    let len = file
        .metadata()
        .await
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))
        .await
        .with_context(|| format!("seek {}", path.display()))?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .with_context(|| format!("read {}", path.display()))?;

    let mut text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            text = text[(pos + 1)..].to_string();
        }
    }

    Ok(text.lines().map(ToString::to_string).collect::<Vec<_>>())
}
