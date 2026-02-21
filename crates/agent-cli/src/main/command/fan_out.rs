impl FanOutScheduler {
    async fn start(
        app: &mut App,
        parent_thread_id: ThreadId,
        tasks: Vec<WorkflowTask>,
        fan_in_artifact_id: omne_protocol::ArtifactId,
        subagent_fork: bool,
    ) -> anyhow::Result<Self> {
        let max_concurrent_subagents = parse_env_usize("OMNE_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
        let concurrency_limit = if max_concurrent_subagents == 0 {
            tasks.len().max(1)
        } else {
            max_concurrent_subagents
        };

        let parent_cwd = if subagent_fork {
            None
        } else {
            Some(resolve_thread_cwd(app, parent_thread_id).await?)
        };

        let started_at = Instant::now();
        let scheduler = Self {
            tasks,
            fan_in_artifact_id,
            concurrency_limit,
            subagent_fork,
            parent_cwd,
            pending_idx: 0,
            active: Vec::new(),
            finished: Vec::new(),
            final_summary_written: false,
            started_at,
            last_progress_print: Instant::now(),
            last_progress_artifact_write: Instant::now(),
        };

        write_fan_out_progress_artifact(
            app,
            parent_thread_id,
            scheduler.fan_in_artifact_id,
            scheduler.tasks.len(),
            &scheduler.finished,
            &scheduler.active,
            scheduler.started_at,
        )
        .await?;

        Ok(scheduler)
    }

    fn is_done(&self) -> bool {
        self.finished.len() >= self.tasks.len()
    }

    fn results_ordered(&self) -> Vec<WorkflowTaskResult> {
        let mut by_id = std::collections::HashMap::<String, WorkflowTaskResult>::new();
        for result in &self.finished {
            by_id.insert(result.task_id.clone(), result.clone());
        }
        let mut ordered = Vec::<WorkflowTaskResult>::new();
        for task in &self.tasks {
            if let Some(result) = by_id.remove(&task.id) {
                ordered.push(result);
            }
        }
        ordered
    }

    async fn tick(&mut self, app: &mut App, parent_thread_id: ThreadId) -> anyhow::Result<()> {
        #[derive(Debug, Deserialize)]
        struct ForkResult {
            thread_id: ThreadId,
            last_seq: u64,
        }

        if self.tasks.is_empty() {
            return Ok(());
        }
        if self.final_summary_written {
            return Ok(());
        }

        while self.active.len() < self.concurrency_limit && self.pending_idx < self.tasks.len() {
            let task = &self.tasks[self.pending_idx];
            self.pending_idx += 1;

            let spawned = if self.subagent_fork {
                app.thread_fork(parent_thread_id).await?
            } else {
                let cwd = self
                    .parent_cwd
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("fan-out parent cwd missing"))?;
                app.thread_start(Some(cwd.to_string())).await?
            };
            let forked: ForkResult = serde_json::from_value(spawned).with_context(|| {
                if self.subagent_fork {
                    "parse thread/fork".to_string()
                } else {
                    "parse thread/start".to_string()
                }
            })?;

            let _ = app
                .rpc(
                    "thread/configure",
                    serde_json::json!({
                        "thread_id": forked.thread_id,
                        "approval_policy": null,
                        "sandbox_policy": omne_protocol::SandboxPolicy::ReadOnly,
                        "sandbox_writable_roots": null,
                        "sandbox_network_access": null,
                        "mode": "reviewer",
                        "model": null,
                        "openai_base_url": null,
                        "allowed_tools": null,
                    }),
                )
                .await?;

            let mut input = format!(
                "You are a read-only subagent.\nTask: {}{}\n\n",
                task.id,
                if task.title.is_empty() {
                    "".to_string()
                } else {
                    format!(" ({})", task.title)
                }
            );
            let body = task.body.trim();
            if body.is_empty() {
                input.push_str("(no task body)\n");
            } else {
                input.push_str(body);
                input.push('\n');
            }
            input.push_str("\nReturn a concise result.\n");

            let turn_id = app
                .turn_start(
                    forked.thread_id,
                    input,
                    Some(omne_protocol::TurnPriority::Background),
                )
                .await?;
            self.active.push(FanOutActiveTask {
                task_id: task.id.clone(),
                title: task.title.clone(),
                thread_id: forked.thread_id,
                turn_id,
                since_seq: forked.last_seq,
                assistant_text: None,
            });
        }

        let mut idx = 0usize;
        while idx < self.active.len() {
            let mut done: Option<(TurnStatus, Option<String>)> = None;
            let thread_id = self.active[idx].thread_id;
            let turn_id = self.active[idx].turn_id;

            loop {
                let resp = app
                    .thread_subscribe(thread_id, self.active[idx].since_seq, Some(1_000), Some(0))
                    .await?;
                self.active[idx].since_seq = resp.last_seq;

                for event in resp.events {
                    match event.kind {
                        omne_protocol::ThreadEventKind::AssistantMessage {
                            turn_id: Some(msg_turn_id),
                            text,
                            ..
                        } if msg_turn_id == turn_id => {
                            self.active[idx].assistant_text = Some(text);
                        }
                        omne_protocol::ThreadEventKind::ApprovalRequested { .. } => {
                            anyhow::bail!(
                                "fan-out task needs approval (thread_id={thread_id}); use `omne inbox`"
                            );
                        }
                        omne_protocol::ThreadEventKind::TurnCompleted {
                            turn_id: completed_turn_id,
                            status,
                            reason,
                        } if completed_turn_id == turn_id => {
                            done = Some((status, reason));
                        }
                        _ => {}
                    }
                }

                if !resp.has_more {
                    break;
                }
            }

            if let Some((status, reason)) = done {
                let task = self.active.remove(idx);
                self.finished.push(WorkflowTaskResult {
                    task_id: task.task_id,
                    title: task.title,
                    thread_id: task.thread_id,
                    turn_id: task.turn_id,
                    status,
                    reason,
                    assistant_text: task.assistant_text,
                });
                continue;
            }

            idx += 1;
        }

        if !self.is_done() {
            if self.last_progress_print.elapsed() >= Duration::from_secs(1) {
                eprintln!(
                    "[fan-out] completed {}/{} (active={}, max={})",
                    self.finished.len(),
                    self.tasks.len(),
                    self.active.len(),
                    self.concurrency_limit
                );
                self.last_progress_print = Instant::now();
            }

            if self.last_progress_artifact_write.elapsed() >= Duration::from_secs(2) {
                let outcome = write_fan_out_progress_artifact(
                    app,
                    parent_thread_id,
                    self.fan_in_artifact_id,
                    self.tasks.len(),
                    &self.finished,
                    &self.active,
                    self.started_at,
                )
                .await;
                if let Err(err) = outcome {
                    eprintln!("[fan-out] progress artifact update failed: {err}");
                } else {
                    self.last_progress_artifact_write = Instant::now();
                }
            }
        } else {
            let outcome = write_fan_out_progress_artifact(
                app,
                parent_thread_id,
                self.fan_in_artifact_id,
                self.tasks.len(),
                &self.finished,
                &self.active,
                self.started_at,
            )
            .await;
            if let Err(err) = outcome {
                eprintln!("[fan-out] final progress artifact update failed: {err}");
            }
        }

        Ok(())
    }
}
