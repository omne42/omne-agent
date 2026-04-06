struct ProcessActorArgs {
    server: Server,
    thread_rt: Arc<ThreadRuntime>,
    process_id: ProcessId,
    child: tokio::process::Child,
    cmd_rx: mpsc::Receiver<ProcessCommand>,
    stdout_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    stderr_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    execve_gate: Option<ExecveGateHandle>,
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
}

async fn run_process_actor(args: ProcessActorArgs) {
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
    } = args;
    async fn finalize_process(
        server: &Server,
        process_id: ProcessId,
        execve_gate: &mut Option<ExecveGateHandle>,
    ) {
        cleanup_execve_gate(execve_gate).await;
        let _ = remove_mcp_connections_for_process(server, process_id).await;
        server.processes.lock().await.remove(&process_id);
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

    async fn cleanup_execve_gate(execve_gate: &mut Option<ExecveGateHandle>) {
        if let Some(gate) = execve_gate.take() {
            shutdown_execve_gate(gate).await;
        }
    }

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { /* sender dropped */ finalize_process(&server, process_id, &mut execve_gate).await; return; };
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
                finalize_process(&server, process_id, &mut execve_gate).await;
                return;
            }
            Ok(None) => {}
            Err(_) => {
                finalize_process(&server, process_id, &mut execve_gate).await;
                return;
            }
        }
    }
}
