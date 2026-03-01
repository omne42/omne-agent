async fn handle_thread_clear_artifacts(
    server: &Server,
    params: ThreadClearArtifactsParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadClearArtifactsResponse> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let tool_id = omne_protocol::ToolId::new();
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: None,
            tool: "thread/clear_artifacts".to_string(),
            params: Some(serde_json::json!({
                "force": params.force,
            })),
        })
        .await?;

    let artifacts_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts");
    let removed = match tokio::fs::remove_dir_all(&artifacts_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", artifacts_dir.display())),
    };

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "removed": removed,
                "artifacts_dir": artifacts_dir.display().to_string(),
            })),
        })
        .await?;

    Ok(omne_app_server_protocol::ThreadClearArtifactsResponse {
        tool_id,
        removed,
        artifacts_dir: artifacts_dir.display().to_string(),
    })
}
