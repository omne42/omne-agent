async fn handle_thread_clear_artifacts(
    server: &Server,
    params: ThreadClearArtifactsParams,
) -> anyhow::Result<Value> {
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;

    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    {
        let entries = {
            let entries = server.processes.lock().await;
            entries
                .iter()
                .map(|(process_id, entry)| (*process_id, entry.clone()))
                .collect::<Vec<_>>()
        };
        for (process_id, entry) in entries {
            let info = entry.info.lock().await;
            if info.thread_id != params.thread_id {
                continue;
            }
            if matches!(info.status, ProcessStatus::Running) {
                running.push(process_id);
                to_kill.push(entry.clone());
            }
        }
    }

    if !running.is_empty() && !params.force {
        anyhow::bail!(
            "refusing to clear artifacts with running processes (use force=true): {:?}",
            running
        );
    }

    if params.force {
        for entry in to_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("artifacts cleared".to_string()),
                })
                .await;
        }
    }

    let tool_id = omne_agent_protocol::ToolId::new();
    thread_rt
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
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
        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_agent_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "removed": removed,
                "artifacts_dir": artifacts_dir.display().to_string(),
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "removed": removed,
        "artifacts_dir": artifacts_dir.display().to_string(),
    }))
}
