async fn run_process_actor(
    thread_rt: Arc<ThreadRuntime>,
    process_id: ProcessId,
    mut child: tokio::process::Child,
    mut cmd_rx: mpsc::Receiver<ProcessCommand>,
    stdout_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    stderr_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
) {
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

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { /* sender dropped */ return; };
                match cmd {
                    ProcessCommand::Interrupt { reason } => {
                        if interrupt_reason.is_none() {
                            interrupt_reason = reason;
                        }
                        if !interrupt_logged {
                            let _ = thread_rt
                                .append_event(pm_protocol::ThreadEventKind::ProcessInterruptRequested {
                                    process_id,
                                    reason: interrupt_reason.clone(),
                                })
                                .await;
                            interrupt_logged = true;
                        }
                        if try_send_interrupt(&child).is_err() {
                            let _ = child.start_kill();
                        }
                    }
                    ProcessCommand::Kill { reason } => {
                        if kill_reason.is_none() {
                            kill_reason = reason;
                        }
                        if !kill_logged {
                            let _ = thread_rt.append_event(pm_protocol::ThreadEventKind::ProcessKillRequested {
                                process_id,
                                reason: kill_reason.clone(),
                            }).await;
                            kill_logged = true;
                        }
                        let _ = child.start_kill();
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(task) = stdout_task {
                    let _ = task.await;
                }
                if let Some(task) = stderr_task {
                    let _ = task.await;
                }

                let exit_code = status.code();
                let exited = thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ProcessExited {
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
                return;
            }
            Ok(None) => {}
            Err(_) => return,
        }
    }
}
