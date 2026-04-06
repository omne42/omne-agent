use super::*;

pub(super) async fn handle_process_list(
    server: &Server,
    params: ProcessListParams,
) -> anyhow::Result<Vec<ProcessInfo>> {
    let thread_ids = if let Some(thread_id) = params.thread_id {
        vec![thread_id]
    } else {
        server.thread_store.list_threads().await?
    };

    for thread_id in &thread_ids {
        server.get_or_load_thread(*thread_id).await?;
    }

    let mut derived = HashMap::<ProcessId, ProcessInfo>::new();
    for thread_id in &thread_ids {
        let events = server
            .thread_store
            .read_events_since(*thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

        for event in events {
            let ts = event.timestamp.format(&Rfc3339)?;
            match event.kind {
                omne_protocol::ThreadEventKind::ProcessStarted {
                    process_id,
                    turn_id,
                    argv,
                    cwd,
                    stdout_path,
                    stderr_path,
                } => {
                    derived.insert(
                        process_id,
                        ProcessInfo {
                            process_id,
                            thread_id: event.thread_id,
                            turn_id,
                            argv,
                            cwd,
                            started_at: ts.clone(),
                            status: ProcessStatus::Running,
                            exit_code: None,
                            stdout_path,
                            stderr_path,
                            last_update_at: ts,
                        },
                    );
                }
                omne_protocol::ThreadEventKind::ProcessInterruptRequested { process_id, .. } => {
                    if let Some(info) = derived.get_mut(&process_id) {
                        info.last_update_at = ts;
                    }
                }
                omne_protocol::ThreadEventKind::ProcessKillRequested { process_id, .. } => {
                    if let Some(info) = derived.get_mut(&process_id) {
                        info.last_update_at = ts;
                    }
                }
                omne_protocol::ThreadEventKind::ProcessExited {
                    process_id,
                    exit_code,
                    ..
                } => {
                    if let Some(info) = derived.get_mut(&process_id) {
                        info.status = ProcessStatus::Exited;
                        info.exit_code = exit_code;
                        info.last_update_at = ts;
                    }
                }
                _ => {}
            }
        }
    }

    let entries = {
        let entries = server.processes.lock().await;
        entries.values().cloned().collect::<Vec<_>>()
    };

    let mut in_mem_running = HashSet::<ProcessId>::new();
    for entry in entries {
        let info = entry.info.lock().await;
        if params.thread_id.is_some_and(|id| id != info.thread_id) {
            continue;
        }
        if matches!(info.status, ProcessStatus::Running) {
            in_mem_running.insert(info.process_id);
        }
        derived.insert(info.process_id, info.clone());
    }

    for info in derived.values_mut() {
        if matches!(info.status, ProcessStatus::Running)
            && !in_mem_running.contains(&info.process_id)
        {
            info.status = ProcessStatus::Abandoned;
        }
    }

    let mut out = derived.into_values().collect::<Vec<_>>();
    out.sort_by(|a, b| {
        a.thread_id
            .cmp(&b.thread_id)
            .then_with(|| a.process_id.cmp(&b.process_id))
    });
    Ok(out)
}

fn into_protocol_process_status(status: ProcessStatus) -> omne_app_server_protocol::ProcessStatus {
    match status {
        ProcessStatus::Running => omne_app_server_protocol::ProcessStatus::Running,
        ProcessStatus::Exited => omne_app_server_protocol::ProcessStatus::Exited,
        ProcessStatus::Abandoned => omne_app_server_protocol::ProcessStatus::Abandoned,
    }
}

pub(super) fn into_protocol_process_info(info: ProcessInfo) -> omne_app_server_protocol::ProcessInfo {
    omne_app_server_protocol::ProcessInfo {
        process_id: info.process_id,
        thread_id: info.thread_id,
        turn_id: info.turn_id,
        argv: info.argv,
        cwd: info.cwd,
        started_at: info.started_at,
        status: into_protocol_process_status(info.status),
        exit_code: info.exit_code,
        stdout_path: info.stdout_path,
        stderr_path: info.stderr_path,
        last_update_at: info.last_update_at,
    }
}

#[cfg(test)]
mod process_list_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn process_list_releases_process_registry_lock_before_waiting_on_entry_info(
    ) -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;
        let process_id = ProcessId::new();
        let (cmd_tx, _cmd_rx) = mpsc::channel(1);
        let now = "2026-01-01T00:00:00Z".to_string();
        let info = Arc::new(tokio::sync::Mutex::new(ProcessInfo {
            process_id,
            thread_id,
            turn_id: None,
            argv: vec!["sleep".to_string(), "999".to_string()],
            cwd: "/tmp".to_string(),
            started_at: now.clone(),
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: "/tmp/omne-test.stdout.log".to_string(),
            stderr_path: "/tmp/omne-test.stderr.log".to_string(),
            last_update_at: now,
        }));
        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id,
                info: info.clone(),
                cmd_tx,
                completion: ProcessCompletion::new(),
            },
        );

        let held_info_guard = info.lock().await;
        let server_for_list = server.clone();
        let list_task = tokio::spawn(async move {
            handle_process_list(
                &server_for_list,
                ProcessListParams {
                    thread_id: Some(thread_id),
                },
            )
            .await
        });

        tokio::task::yield_now().await;

        let registry_lock_result =
            tokio::time::timeout(Duration::from_millis(100), server.processes.lock()).await;
        assert!(
            registry_lock_result.is_ok(),
            "process registry lock remained held while waiting on entry info"
        );
        drop(registry_lock_result);

        drop(held_info_guard);
        let listed = list_task.await??;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].process_id, process_id);
        assert!(matches!(listed[0].status, ProcessStatus::Running));
        Ok(())
    }
}
