async fn handle_process_follow(
    server: &Server,
    params: ProcessFollowParams,
) -> anyhow::Result<Value> {
    let max_bytes = params.max_bytes.unwrap_or(64 * 1024).min(1024 * 1024);
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
        "since_offset": params.since_offset,
        "max_bytes": max_bytes,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "process/follow",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return process_allowed_tools_denied_response(tool_id, "process/follow", &allowed_tools);
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
                "process/follow",
                &approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(result);
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(mode, "process/follow", mode.permissions.process.inspect);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = process_mode_denied_response(tool_id, info.thread_id, &mode_name, mode_decision)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/follow",
            &approval_params,
            "mode denies process/follow".to_string(),
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
                action: "process/follow",
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
                    "process/follow",
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
            tool: "process/follow".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let path = match params.stream {
        ProcessStream::Stdout => info.stdout_path,
        ProcessStream::Stderr => info.stderr_path,
    };

    let outcome = read_file_chunk(PathBuf::from(&path), params.since_offset, max_bytes).await;
    match outcome {
        Ok((text, next_offset, eof)) => {
            let text = omne_core::redact_text(&text);
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "path": path,
                        "max_bytes": max_bytes,
                        "next_offset": next_offset,
                        "eof": eof,
                    })),
                })
                .await?;

            let response = omne_app_server_protocol::ProcessFollowResponse {
                tool_id,
                text,
                next_offset,
                eof,
            };
            serde_json::to_value(response).context("serialize process/follow response")
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

async fn read_file_chunk(
    path: PathBuf,
    since_offset: u64,
    max_bytes: u64,
) -> anyhow::Result<(String, u64, bool)> {
    let files = list_rotating_log_files(&path).await?;
    if files.is_empty() {
        return Ok((String::new(), since_offset, true));
    }

    let max_bytes = max_bytes.min(1024 * 1024);
    let mut lengths = Vec::<u64>::new();
    let mut total = 0u64;
    for file in &files {
        let len = match tokio::fs::metadata(file).await {
            Ok(meta) => meta.len(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
            Err(err) => return Err(err).with_context(|| format!("stat {}", file.display())),
        };
        lengths.push(len);
        total = total.saturating_add(len);
    }

    let start = since_offset.min(total);
    let mut remaining_offset = start;
    let mut remaining_bytes = max_bytes;
    let mut out: Vec<u8> = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        let len = lengths.get(idx).copied().unwrap_or(0);
        if remaining_offset >= len {
            remaining_offset = remaining_offset.saturating_sub(len);
            continue;
        }

        if remaining_bytes == 0 {
            break;
        }

        let mut f = match tokio::fs::File::open(file).await {
            Ok(f) => f,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("open {}", file.display())),
        };
        f.seek(SeekFrom::Start(remaining_offset))
            .await
            .with_context(|| format!("seek {}", file.display()))?;

        let buf_len = usize::try_from(remaining_bytes).unwrap_or(1024 * 1024);
        let mut buf = vec![0u8; buf_len];
        let mut n_total = 0usize;
        while n_total < buf_len {
            let n = f
                .read(&mut buf[n_total..])
                .await
                .with_context(|| format!("read {}", file.display()))?;
            if n == 0 {
                break;
            }
            n_total = n_total.saturating_add(n);
        }
        let n = n_total;
        buf.truncate(n);
        remaining_offset = 0;
        remaining_bytes = remaining_bytes.saturating_sub(n as u64);
        out.extend_from_slice(&buf);

        if n == 0 {
            continue;
        }
        if remaining_bytes == 0 {
            break;
        }
    }

    let next_offset = start + out.len() as u64;
    let eof = next_offset >= total;
    let text = String::from_utf8_lossy(&out).to_string();
    Ok((text, next_offset, eof))
}
