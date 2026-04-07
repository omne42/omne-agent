use super::*;

pub(super) struct ProcessActorArgs {
    pub(super) server: Server,
    pub(super) thread_rt: Arc<ThreadRuntime>,
    pub(super) process_id: ProcessId,
    pub(super) child: tokio::process::Child,
    pub(super) process_tree_cleanup: Option<omne_process_primitives::ProcessTreeCleanup>,
    pub(super) cmd_rx: mpsc::Receiver<ProcessCommand>,
    pub(super) stdout_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    pub(super) stderr_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    pub(super) execve_gate: Option<ExecveGateHandle>,
    pub(super) info: Arc<tokio::sync::Mutex<ProcessInfo>>,
    pub(super) completion: ProcessCompletion,
}

pub(super) async fn run_process_actor(args: ProcessActorArgs) {
    let ProcessActorArgs {
        server,
        thread_rt,
        process_id,
        mut child,
        mut process_tree_cleanup,
        mut cmd_rx,
        stdout_task,
        stderr_task,
        mut execve_gate,
        info,
        completion,
    } = args;
    async fn finalize_process(
        server: &Server,
        process_id: ProcessId,
        execve_gate: &mut Option<ExecveGateHandle>,
        completion: &ProcessCompletion,
    ) {
        cleanup_execve_gate(execve_gate).await;
        let _ = remove_mcp_connections_for_process(server, process_id).await;
        server.processes.lock().await.remove(&process_id);
        completion.mark_complete();
    }

    fn try_send_interrupt(
        child: &tokio::process::Child,
        process_tree_cleanup: Option<&omne_process_primitives::ProcessTreeCleanup>,
    ) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            let Some(pid) = child.id() else {
                anyhow::bail!("process has no pid");
            };
            if process_tree_cleanup.is_some() {
                kill(Pid::from_raw(-(pid as i32)), Signal::SIGINT)
                    .with_context(|| format!("send SIGINT to process group {pid}"))?;
            } else {
                kill(Pid::from_raw(pid as i32), Signal::SIGINT)
                    .with_context(|| format!("send SIGINT to pid {pid}"))?;
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = (child, process_tree_cleanup);
            anyhow::bail!("process interrupt is not supported on this platform")
        }
    }

    fn initiate_process_tree_termination(
        process_id: ProcessId,
        child: &mut tokio::process::Child,
        process_tree_cleanup: &mut Option<omne_process_primitives::ProcessTreeCleanup>,
    ) {
        let needs_direct_child_kill = match process_tree_cleanup.as_mut() {
            Some(cleanup) => matches!(
                cleanup.start_termination(),
                omne_process_primitives::CleanupDisposition::DirectChildKillRequired
            ),
            None => true,
        };
        if let Some(cleanup) = process_tree_cleanup.as_ref() {
            cleanup.kill_tree();
        }
        if needs_direct_child_kill && child.start_kill().is_err() {
            tracing::warn!(process_id = %process_id, "failed to kill process");
        }
    }

    let mut interrupt_reason: Option<String> = None;
    let mut interrupt_logged = false;
    let mut kill_reason: Option<String> = None;
    let mut kill_logged = false;
    let mut command_channel_closed = false;

    async fn cleanup_execve_gate(execve_gate: &mut Option<ExecveGateHandle>) {
        if let Some(gate) = execve_gate.take() {
            shutdown_execve_gate(gate).await;
        }
    }

    loop {
        tokio::select! {
            cmd = cmd_rx.recv(), if !command_channel_closed => {
                let Some(cmd) = cmd else {
                    command_channel_closed = true;
                    if kill_reason.is_none() {
                        kill_reason = Some("process control dropped".to_string());
                    }
                    initiate_process_tree_termination(
                        process_id,
                        &mut child,
                        &mut process_tree_cleanup,
                    );
                    continue;
                };
                match cmd {
                    ProcessCommand::Interrupt { reason } => {
                        if interrupt_reason.is_none() {
                            interrupt_reason = reason;
                        }
                        if !interrupt_logged {
                            if let Err(err) = thread_rt
                                .append_event(omne_protocol::ThreadEventKind::ProcessInterruptRequested {
                                    process_id,
                                    reason: interrupt_reason.clone(),
                                })
                                .await
                            {
                                tracing::warn!(process_id = %process_id, error = %err, "failed to append ProcessInterruptRequested event");
                            }
                            interrupt_logged = true;
                        }
                        if try_send_interrupt(&child, process_tree_cleanup.as_ref()).is_err() {
                            initiate_process_tree_termination(
                                process_id,
                                &mut child,
                                &mut process_tree_cleanup,
                            );
                        }
                    }
                    ProcessCommand::Kill { reason } => {
                        if kill_reason.is_none() {
                            kill_reason = reason;
                        }
                        if !kill_logged {
                            if let Err(err) = thread_rt.append_event(omne_protocol::ThreadEventKind::ProcessKillRequested {
                                process_id,
                                reason: kill_reason.clone(),
                            }).await {
                                tracing::warn!(process_id = %process_id, error = %err, "failed to append ProcessKillRequested event");
                            }
                            kill_logged = true;
                        }
                        initiate_process_tree_termination(
                            process_id,
                            &mut child,
                            &mut process_tree_cleanup,
                        );
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(task) = stdout_task {
                    match task.await {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => {
                            tracing::warn!(process_id = %process_id, error = %err, "stdout streaming task failed");
                        }
                        Err(err) => {
                            tracing::warn!(process_id = %process_id, error = %err, "stdout streaming task panicked");
                        }
                    }
                }
                if let Some(task) = stderr_task {
                    match task.await {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => {
                            tracing::warn!(process_id = %process_id, error = %err, "stderr streaming task failed");
                        }
                        Err(err) => {
                            tracing::warn!(process_id = %process_id, error = %err, "stderr streaming task panicked");
                        }
                    }
                }

                let exit_code = status.code();
                let exited = thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ProcessExited {
                        process_id,
                        exit_code,
                        reason: kill_reason.clone().or_else(|| interrupt_reason.clone()),
                    })
                    .await;

                if let Ok(event) = exited {
                    if let Ok(ts) = event.timestamp.format(&Rfc3339) {
                        let mut info = info.lock().await;
                        info.status = ProcessStatus::Exited;
                        info.exit_code = exit_code;
                        info.last_update_at = ts;
                    }
                }

                let marker_context = {
                    let info = info.lock().await;
                    if looks_like_test_command(&info.argv) {
                        Some((info.turn_id, process_command_label(&info.argv)))
                    } else {
                        None
                    }
                };
                if let Some((turn_id, command)) = marker_context {
                    if exit_code.unwrap_or_default() != 0 {
                        if let Err(err) = thread_rt
                            .append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                                marker: omne_protocol::AttentionMarkerKind::TestFailed,
                                turn_id,
                                artifact_id: None,
                                artifact_type: None,
                                process_id: Some(process_id),
                                exit_code,
                                command,
                            })
                            .await
                        {
                            tracing::warn!(process_id = %process_id, error = %err, "failed to append AttentionMarkerSet(test_failed) event");
                        }
                    } else if let Err(err) = thread_rt
                        .append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                            marker: omne_protocol::AttentionMarkerKind::TestFailed,
                            turn_id,
                            reason: Some("test command succeeded".to_string()),
                        })
                        .await
                    {
                        tracing::warn!(process_id = %process_id, error = %err, "failed to append AttentionMarkerCleared(test_failed) event");
                    }
                }
                finalize_process(&server, process_id, &mut execve_gate, &completion).await;
                return;
            }
            Ok(None) => {}
            Err(_) => {
                finalize_process(&server, process_id, &mut execve_gate, &completion).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod process_actor_tests {
    use super::*;

    #[tokio::test]
    async fn process_actor_kills_child_and_records_exit_when_control_channel_drops(
    ) -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.kill_on_drop(true);
        let child = cmd.spawn().context("spawn test child")?;

        let process_id = ProcessId::new();
        let started_at = time::OffsetDateTime::now_utc().format(&Rfc3339)?;
        let info = Arc::new(tokio::sync::Mutex::new(ProcessInfo {
            process_id,
            thread_id,
            turn_id: None,
            os_pid: child.id(),
            argv: vec!["sleep".to_string(), "30".to_string()],
            cwd: repo_dir.display().to_string(),
            started_at: started_at.clone(),
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: String::new(),
            stderr_path: String::new(),
            last_update_at: started_at,
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        let completion = ProcessCompletion::new();
        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id,
                info: info.clone(),
                cmd_tx: cmd_tx.clone(),
                completion: completion.clone(),
            },
        );

        let actor = tokio::spawn(run_process_actor(ProcessActorArgs {
            server: server.clone(),
            thread_rt: thread_rt.clone(),
            process_id,
            child,
            process_tree_cleanup: None,
            cmd_rx,
            stdout_task: None,
            stderr_task: None,
            execve_gate: None,
            info: info.clone(),
            completion,
        }));

        drop(server.processes.lock().await.remove(&process_id).expect("entry exists").cmd_tx);
        drop(cmd_tx);

        tokio::time::timeout(Duration::from_secs(5), actor)
        .await
        .context("wait for actor to exit after process control dropped")??;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ProcessExited {
                    process_id: exited_process_id,
                    reason: Some(reason),
                    ..
                } if *exited_process_id == process_id && reason == "process control dropped"
            )
        }));

        Ok(())
    }

    #[tokio::test]
    async fn process_actor_exit_clears_cached_mcp_connection() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let mut cmd = Command::new("true");
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.kill_on_drop(true);
        let child = cmd.spawn().context("spawn test child")?;

        let process_id = ProcessId::new();
        let started_at = time::OffsetDateTime::now_utc().format(&Rfc3339)?;
        let info = Arc::new(tokio::sync::Mutex::new(ProcessInfo {
            process_id,
            thread_id,
            turn_id: None,
            os_pid: child.id(),
            argv: vec!["true".to_string()],
            cwd: repo_dir.display().to_string(),
            started_at: started_at.clone(),
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: String::new(),
            stderr_path: String::new(),
            last_update_at: started_at,
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        let completion = ProcessCompletion::new();
        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id,
                info: info.clone(),
                cmd_tx,
                completion: completion.clone(),
            },
        );

        let (client_stream, peer_stream) = tokio::io::duplex(1024);
        drop(peer_stream);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let client = omne_jsonrpc::Client::connect_io(client_read, client_write).await?;
        let server_name = "local".to_string();
        server.mcp.lock().await.connections.insert(
            (thread_id, server_name.clone()),
            Arc::new(McpConnection {
                process_id,
                config_fingerprint: "test-config".to_string(),
                client: tokio::sync::Mutex::new(client),
            }),
        );

        let actor = tokio::spawn(run_process_actor(ProcessActorArgs {
            server: server.clone(),
            thread_rt: thread_rt.clone(),
            process_id,
            child,
            process_tree_cleanup: None,
            cmd_rx,
            stdout_task: None,
            stderr_task: None,
            execve_gate: None,
            info,
            completion: completion.clone(),
        }));

        tokio::time::timeout(Duration::from_secs(5), actor)
            .await
            .context("wait for actor to exit")??;
        tokio::time::timeout(Duration::from_secs(5), completion.wait())
            .await
            .context("wait for completion")?;

        assert!(
            !server
                .mcp
                .lock()
                .await
                .connections
                .contains_key(&(thread_id, server_name)),
            "process exit should invalidate cached mcp connections"
        );
        assert!(
            server.processes.lock().await.get(&process_id).is_none(),
            "process exit should evict the managed process entry"
        );

        Ok(())
    }

    #[cfg(unix)]
    async fn wait_for_pid_file(path: &Path) -> anyhow::Result<u32> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(raw) = tokio::fs::read_to_string(path).await
                && let Ok(pid) = raw.trim().parse::<u32>()
            {
                return Ok(pid);
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for pid file: {}", path.display());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[cfg(unix)]
    async fn wait_for_process_to_exit(pid: u32) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if !super::list::os_process_exists(pid) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for pid {pid} to exit");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    #[cfg(unix)]
    async fn spawn_process_actor_with_background_child(
        server: &Server,
        thread_rt: &Arc<ThreadRuntime>,
        repo_dir: &Path,
        process_id: ProcessId,
        pid_file: &Path,
    ) -> anyhow::Result<(
        tokio::task::JoinHandle<()>,
        mpsc::Sender<ProcessCommand>,
        ProcessCompletion,
        u32,
    )> {
        let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        omne_process_primitives::configure_command_for_process_tree(&mut cmd);
        let child = cmd.spawn().context("spawn shell with background child")?;
        let process_tree_cleanup = Some(omne_process_primitives::ProcessTreeCleanup::new(&child)?);

        let started_at = time::OffsetDateTime::now_utc().format(&Rfc3339)?;
        let info = Arc::new(tokio::sync::Mutex::new(ProcessInfo {
            process_id,
            thread_id: thread_rt.handle.lock().await.thread_id(),
            turn_id: None,
            os_pid: child.id(),
            argv: vec!["sh".to_string(), "-c".to_string(), "sleep 30 & wait".to_string()],
            cwd: repo_dir.display().to_string(),
            started_at: started_at.clone(),
            status: ProcessStatus::Running,
            exit_code: None,
            stdout_path: String::new(),
            stderr_path: String::new(),
            last_update_at: started_at,
        }));
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        let completion = ProcessCompletion::new();
        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id: thread_rt.handle.lock().await.thread_id(),
                info: info.clone(),
                cmd_tx: cmd_tx.clone(),
                completion: completion.clone(),
            },
        );

        let actor = tokio::spawn(run_process_actor(ProcessActorArgs {
            server: server.clone(),
            thread_rt: thread_rt.clone(),
            process_id,
            child,
            process_tree_cleanup,
            cmd_rx,
            stdout_task: None,
            stderr_task: None,
            execve_gate: None,
            info,
            completion: completion.clone(),
        }));

        let background_pid = wait_for_pid_file(pid_file).await?;
        Ok((actor, cmd_tx, completion, background_pid))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_actor_kill_terminates_background_process_group() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let pid_file = repo_dir.join("background.pid");
        let process_id = ProcessId::new();

        let (actor, cmd_tx, completion, background_pid) = spawn_process_actor_with_background_child(
            &server,
            &thread_rt,
            &repo_dir,
            process_id,
            &pid_file,
        )
        .await?;
        cmd_tx
            .send(ProcessCommand::Kill {
                reason: Some("test kill".to_string()),
            })
            .await?;

        tokio::time::timeout(Duration::from_secs(5), actor)
            .await
            .context("wait for actor to exit after kill")??;
        tokio::time::timeout(Duration::from_secs(5), completion.wait())
            .await
            .context("wait for completion after kill")?;
        wait_for_process_to_exit(background_pid).await?;
        Ok(())
    }

}
