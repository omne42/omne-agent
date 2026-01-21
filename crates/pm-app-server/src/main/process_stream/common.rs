fn stream_label(stream: ProcessStream) -> &'static str {
    match stream {
        ProcessStream::Stdout => "stdout",
        ProcessStream::Stderr => "stderr",
    }
}

async fn resolve_process_info(server: &Server, process_id: ProcessId) -> anyhow::Result<ProcessInfo> {
    let entry = server.processes.lock().await.get(&process_id).cloned();

    if let Some(entry) = entry {
        let info = entry.info.lock().await;
        return Ok(info.clone());
    }

    let processes = handle_process_list(server, ProcessListParams { thread_id: None }).await?;
    processes
        .into_iter()
        .find(|p| p.process_id == process_id)
        .ok_or_else(|| anyhow::anyhow!("process not found: {}", process_id))
}
