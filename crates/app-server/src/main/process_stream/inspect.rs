async fn handle_process_inspect(
    server: &Server,
    params: ProcessInspectParams,
) -> anyhow::Result<Value> {
    let max_lines = params.max_lines.unwrap_or(200).min(2000);

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
        "max_lines": max_lines,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "process/inspect",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return process_allowed_tools_denied_response(tool_id, "process/inspect", &allowed_tools);
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
                "process/inspect",
                &approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(result);
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(mode, "process/inspect", mode.permissions.process.inspect);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result = process_mode_denied_response(tool_id, info.thread_id, &mode_name, mode_decision)?;
        emit_process_tool_denied(
            &thread_rt,
            tool_id,
            params.turn_id,
            "process/inspect",
            &approval_params,
            "mode denies process/inspect".to_string(),
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
                action: "process/inspect",
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
                    "process/inspect",
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
            tool: "process/inspect".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let stdout_tail = omne_core::redact_text(
        &tail_file_lines(PathBuf::from(&info.stdout_path), max_lines).await?,
    );
    let stderr_tail = omne_core::redact_text(
        &tail_file_lines(PathBuf::from(&info.stderr_path), max_lines).await?,
    );

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "process_id": params.process_id,
                "max_lines": max_lines,
            })),
        })
        .await?;

    let response = omne_app_server_protocol::ProcessInspectResponse {
        tool_id,
        process: into_protocol_process_info(info),
        stdout_tail,
        stderr_tail,
    };
    serde_json::to_value(response).context("serialize process/inspect response")
}
