async fn handle_thread_delete(
    server: &Server,
    params: ThreadDeleteParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadDeleteResponse> {
    let thread_dir = server.thread_store.thread_dir(params.thread_id);
    let thread_cwd = match server.thread_store.read_state(params.thread_id).await {
        Ok(state) => state.and_then(|value| value.cwd),
        Err(err) => {
            tracing::warn!(
                thread_id = %params.thread_id,
                error = %err,
                "failed to read thread state before delete"
            );
            None
        }
    };

    let mut running = Vec::<ProcessId>::new();
    let mut to_kill = Vec::<ProcessEntry>::new();
    let mut to_remove = Vec::<ProcessId>::new();
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
            to_remove.push(process_id);
            if matches!(info.status, ProcessStatus::Running) {
                running.push(process_id);
                to_kill.push(entry.clone());
            }
        }
    }

    if !running.is_empty() && !params.force {
        anyhow::bail!(
            "refusing to delete thread with running processes (use force=true): {:?}",
            running
        );
    }

    if params.force {
        for entry in to_kill {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("thread deleted".to_string()),
                })
                .await;
        }
    }

    server.threads.lock().await.remove(&params.thread_id);
    {
        let mut entries = server.processes.lock().await;
        for process_id in to_remove {
            entries.remove(&process_id);
        }
    }
    cleanup_managed_subagent_worktree(
        server,
        params.thread_id,
        thread_cwd.as_deref(),
        "thread/delete",
    )
    .await;

    let deleted = match tokio::fs::remove_dir_all(&thread_dir).await {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(err).with_context(|| format!("remove {}", thread_dir.display())),
    };

    Ok(omne_app_server_protocol::ThreadDeleteResponse {
        thread_id: params.thread_id,
        deleted,
        thread_dir: thread_dir.display().to_string(),
    })
}
