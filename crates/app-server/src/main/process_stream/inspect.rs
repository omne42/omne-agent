async fn handle_process_inspect(
    server: &Server,
    params: ProcessInspectParams,
) -> anyhow::Result<Value> {
    let max_lines = params.max_lines.unwrap_or(200).min(2000);

    let info = resolve_process_info(server, params.process_id).await?;
    let (thread_rt, thread_root) = load_thread_root(server, info.thread_id).await?;
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

    if let Some(result) = enforce_process_mode_and_approval(
        server,
        ProcessModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: info.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            role_name: &role_name,
            action: "process/inspect",
            tool_id,
            approval_params: &approval_params,
        },
        |mode| mode.permissions.process.inspect,
    )
    .await?
    {
        return Ok(result);
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
            structured_error: None,
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
