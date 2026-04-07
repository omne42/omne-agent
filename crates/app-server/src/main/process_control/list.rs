use super::*;

#[cfg(unix)]
fn arg0_matches(actual: &str, expected: &str) -> bool {
    let actual_name = Path::new(actual)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(actual);
    let expected_name = Path::new(expected)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(expected);
    actual_name == expected_name
}

#[cfg(unix)]
pub(super) fn os_process_exists(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(Errno::ESRCH) => false,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub(super) fn os_process_exists(_pid: u32) -> bool {
    false
}

#[cfg(all(unix, target_os = "linux"))]
pub(super) fn os_process_matches_argv(pid: u32, argv: &[String]) -> bool {
    if !os_process_exists(pid) {
        return false;
    }
    if argv.is_empty() {
        return true;
    }

    let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return true;
    };
    let actual = cmdline
        .split(|byte| *byte == b'\0')
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect::<Vec<_>>();
    if actual.is_empty() {
        return true;
    }
    if actual.len() != argv.len() {
        return false;
    }
    if !arg0_matches(&actual[0], &argv[0]) {
        return false;
    }
    actual.iter().skip(1).eq(argv.iter().skip(1))
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn os_process_matches_argv(pid: u32, _argv: &[String]) -> bool {
    os_process_exists(pid)
}

#[cfg(not(unix))]
pub(super) fn os_process_matches_argv(_pid: u32, _argv: &[String]) -> bool {
    false
}

pub(super) async fn handle_process_list(
    server: &Server,
    params: ProcessListParams,
) -> anyhow::Result<Vec<ProcessInfo>> {
    let thread_ids = if let Some(thread_id) = params.thread_id {
        vec![thread_id]
    } else {
        server.thread_store.list_threads().await?
    };

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
                    os_pid,
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
                            os_pid,
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
            info.status = match info.os_pid {
                Some(os_pid) if os_process_matches_argv(os_pid, &info.argv) => {
                    ProcessStatus::Running
                }
                _ => ProcessStatus::Abandoned,
            };
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
        os_pid: info.os_pid,
        argv: omne_core::redact_command_argv(&info.argv),
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
            os_pid: None,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn process_list_keeps_running_status_for_live_pid_without_registry_entry()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let child = std::process::Command::new("sleep").arg("30").spawn()?;
        let os_pid = child.id();
        let thread_id = ThreadId::new();
        let process_id = ProcessId::new();
        let log_path = tmp
            .path()
            .join(".omne_data")
            .join("threads")
            .join(thread_id.to_string())
            .join("events.jsonl");
        let mut writer = omne_eventlog::EventLogWriter::open(thread_id, log_path).await?;
        writer
            .append(omne_protocol::ThreadEventKind::ThreadCreated {
                cwd: repo_dir.display().to_string(),
            })
            .await?;
        writer
            .append(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                os_pid: Some(os_pid),
                argv: vec!["sleep".to_string(), "30".to_string()],
                cwd: repo_dir.display().to_string(),
                stdout_path: repo_dir.join("stdout.log").display().to_string(),
                stderr_path: repo_dir.join("stderr.log").display().to_string(),
            })
            .await?;
        drop(writer);

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let listed = handle_process_list(
            &server,
            ProcessListParams {
                thread_id: Some(thread_id),
            },
        )
        .await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].process_id, process_id);
        assert_eq!(listed[0].os_pid, Some(os_pid));
        assert!(matches!(listed[0].status, ProcessStatus::Running));

        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(os_pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(os_pid as i32), None);
        Ok(())
    }

    #[tokio::test]
    async fn process_list_does_not_resume_cold_thread_or_append_recovery_events()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let seed_server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let mut handle = seed_server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        let turn_id = TurnId::new();
        handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "still running".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(handle);

        let before_events = seed_server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread should exist"))?;
        assert_eq!(before_events.len(), 2);
        assert!(matches!(
            before_events.last().map(|event| &event.kind),
            Some(omne_protocol::ThreadEventKind::TurnStarted { .. })
        ));

        let cold_server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let listed = handle_process_list(
            &cold_server,
            ProcessListParams {
                thread_id: Some(thread_id),
            },
        )
        .await?;
        assert!(listed.is_empty());

        let after_events = cold_server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread should exist"))?;
        assert_eq!(after_events.len(), before_events.len());
        assert!(matches!(
            after_events.last().map(|event| &event.kind),
            Some(omne_protocol::ThreadEventKind::TurnStarted { .. })
        ));
        Ok(())
    }

    #[test]
    fn into_protocol_process_info_redacts_sensitive_argv() {
        let info = ProcessInfo {
            process_id: ProcessId::new(),
            thread_id: ThreadId::new(),
            turn_id: None,
            os_pid: Some(42),
            argv: vec![
                "tool".to_string(),
                "--api-key".to_string(),
                "super-secret".to_string(),
                "--token=value".to_string(),
            ],
            cwd: "/tmp/repo".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: "/tmp/stdout.log".to_string(),
            stderr_path: "/tmp/stderr.log".to_string(),
            last_update_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let protocol = into_protocol_process_info(info);

        assert_eq!(
            protocol.argv,
            vec![
                "tool".to_string(),
                "--api-key".to_string(),
                "<REDACTED>".to_string(),
                "--token=<REDACTED>".to_string(),
            ]
        );
    }

}
