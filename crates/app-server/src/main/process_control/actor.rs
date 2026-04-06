use super::*;

pub(super) struct ProcessActorArgs {
    pub(super) server: Server,
    pub(super) thread_rt: Arc<ThreadRuntime>,
    pub(super) process_id: ProcessId,
    pub(super) child: tokio::process::Child,
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

    fn try_send_interrupt(child: &tokio::process::Child) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            let Some(pid) = child.id() else {
                anyhow::bail!("process has no pid");
            };
            kill(Pid::from_raw(pid as i32), Signal::SIGINT)
                .with_context(|| format!("send SIGINT to pid {pid}"))?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            anyhow::bail!("process interrupt is not supported on this platform")
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
                    if let Err(err) = child.start_kill() {
                        tracing::warn!(process_id = %process_id, error = %err, "failed to kill process after process control dropped");
                    }
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
                        if try_send_interrupt(&child).is_err() {
                            if let Err(err) = child.start_kill() {
                                tracing::warn!(process_id = %process_id, error = %err, "failed to kill process after interrupt failure");
                            }
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
                        if let Err(err) = child.start_kill() {
                            tracing::warn!(process_id = %process_id, error = %err, "failed to kill process");
                        }
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
}
