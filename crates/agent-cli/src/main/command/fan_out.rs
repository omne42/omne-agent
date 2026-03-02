use super::*;

impl FanOutScheduler {
    async fn write_progress_artifact(
        &self,
        app: &mut App,
        parent_thread_id: ThreadId,
        parent_turn_id: Option<TurnId>,
    ) -> anyhow::Result<()> {
        write_fan_out_progress_artifact(
            app,
            parent_thread_id,
            parent_turn_id,
            self.fan_in_artifact_id,
            self.tasks.len(),
            &self.finished,
            &self.active,
            self.started_at,
            self.scheduling,
        )
        .await
    }

    pub(super) async fn start(
        app: &mut App,
        parent_thread_id: ThreadId,
        tasks: Vec<WorkflowTask>,
        fan_in_artifact_id: omne_protocol::ArtifactId,
        subagent_fork: bool,
    ) -> anyhow::Result<Self> {
        let scheduling = fan_out_scheduling_params(tasks.len());

        let parent_cwd = if subagent_fork {
            None
        } else {
            Some(resolve_thread_cwd(app, parent_thread_id).await?)
        };

        let started_at = Instant::now();
        let scheduler = Self {
            tasks,
            fan_in_artifact_id,
            scheduling,
            subagent_fork,
            parent_cwd,
            started_ids: BTreeSet::new(),
            ready_wait_rounds: BTreeMap::new(),
            task_statuses: BTreeMap::new(),
            active: Vec::new(),
            finished: Vec::new(),
            final_summary_written: false,
            started_at,
            last_progress_print: Instant::now(),
            last_progress_artifact_write: Instant::now(),
        };

        scheduler
            .write_progress_artifact(app, parent_thread_id, None)
            .await?;

        Ok(scheduler)
    }

    pub(super) fn is_done(&self) -> bool {
        self.finished.len() >= self.tasks.len()
    }

    pub(super) fn results_ordered(&self) -> Vec<WorkflowTaskResult> {
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

    pub(super) fn first_non_completed_result(&self) -> Option<&WorkflowTaskResult> {
        self.finished
            .iter()
            .find(|result| !matches!(result.status, TurnStatus::Completed))
    }

    fn has_ready_pending_task(&self) -> bool {
        pick_next_runnable_task(&self.tasks, &self.started_ids, &self.task_statuses).is_some()
    }

    pub(super) async fn tick(
        &mut self,
        app: &mut App,
        parent_thread_id: ThreadId,
        parent_turn_id: Option<TurnId>,
    ) -> anyhow::Result<()> {
        if self.tasks.is_empty() {
            return Ok(());
        }
        if self.final_summary_written {
            return Ok(());
        }

        update_ready_wait_rounds(
            &self.tasks,
            &self.started_ids,
            &self.task_statuses,
            &mut self.ready_wait_rounds,
        );
        while self.active.len() < self.scheduling.effective_concurrency_limit {
            let Some(task) = pick_next_runnable_task_fair(
                &self.tasks,
                &self.started_ids,
                &self.task_statuses,
                &self.ready_wait_rounds,
                self.scheduling.priority_aging_rounds,
            ) else {
                break;
            };
            self.started_ids.insert(task.id.clone());
            self.ready_wait_rounds.remove(&task.id);

            let forked = if self.subagent_fork {
                let forked = app.thread_fork(parent_thread_id).await?;
                (forked.thread_id, forked.last_seq)
            } else {
                let cwd = self
                    .parent_cwd
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("fan-out parent cwd missing"))?;
                let started = app.thread_start(Some(cwd.to_string())).await?;
                ensure_thread_start_auto_hook_ready("command/fan_out", &started)?;
                (started.thread_id, started.last_seq)
            };
            let (forked_thread_id, forked_last_seq) = forked;

            app.thread_configure_rpc(omne_app_server_protocol::ThreadConfigureParams {
                thread_id: forked_thread_id,
                approval_policy: None,
                sandbox_policy: Some(omne_protocol::SandboxPolicy::ReadOnly),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("reviewer".to_string()),
                role: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
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
                    forked_thread_id,
                    input,
                    Some(omne_protocol::TurnPriority::Background),
                )
                .await?;
            self.active.push(FanOutActiveTask {
                task_id: task.id.clone(),
                title: task.title.clone(),
                thread_id: forked_thread_id,
                turn_id,
                since_seq: forked_last_seq,
                assistant_text: None,
            });
        }

        let mut idx = 0usize;
        while idx < self.active.len() {
            let mut done: Option<(TurnStatus, Option<String>)> = None;
            let mut approval_issue: Option<FanOutApprovalIssue> = None;
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
                        omne_protocol::ThreadEventKind::ApprovalRequested {
                            approval_id,
                            action,
                            params,
                            ..
                        } => {
                            let summary = approval_summary_from_params_with_context(
                                Some(thread_id),
                                Some(approval_id),
                                Some(action.as_str()),
                                &params,
                            )
                            .and_then(|summary| approval_summary_display_from_summary(&summary));
                            approval_issue = Some(FanOutApprovalIssue {
                                task_id: self.active[idx].task_id.clone(),
                                thread_id,
                                turn_id,
                                approval_id,
                                action,
                                summary,
                            });
                            break;
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

                if approval_issue.is_some() || !resp.has_more {
                    break;
                }
            }

            if let Some(issue) = approval_issue {
                let task = self.active.remove(idx);
                self.task_statuses
                    .insert(task.task_id.clone(), TurnStatus::Interrupted);
                self.finished.push(pending_approval_task_result(
                    task.task_id.clone(),
                    task.title.clone(),
                    task.thread_id,
                    task.turn_id,
                    issue.action.clone(),
                    issue.approval_id,
                    issue.summary.clone(),
                ));
                let _ = write_fan_out_approval_blocked_artifact(
                    app,
                    parent_thread_id,
                    parent_turn_id,
                    self.fan_in_artifact_id,
                    self.tasks.len(),
                    &self.finished,
                    &issue,
                    self.scheduling,
                )
                .await;
                let issue_text = fan_out_approval_error_with_artifact_fallback(
                    app,
                    parent_thread_id,
                    &issue,
                    self.fan_in_artifact_id,
                )
                .await;
                anyhow::bail!("{issue_text}");
            }

            if let Some((status, reason)) = done {
                let task = self.active.remove(idx);
                self.task_statuses.insert(task.task_id.clone(), status);
                let artifact_write = write_fan_out_result_artifacts(
                    app,
                    parent_thread_id,
                    parent_turn_id,
                    task.thread_id,
                    &task.task_id,
                    &task.title,
                    task.turn_id,
                    status,
                    reason.as_deref(),
                    task.assistant_text.as_deref(),
                )
                .await;
                self.finished.push(WorkflowTaskResult {
                    task_id: task.task_id,
                    title: task.title,
                    thread_id: Some(task.thread_id),
                    turn_id: Some(task.turn_id),
                    result_artifact_id: artifact_write.result_artifact_id,
                    result_artifact_error: artifact_write.result_artifact_error,
                    result_artifact_error_id: artifact_write.result_artifact_error_id,
                    status,
                    reason,
                    dependency_blocked: false,
                    assistant_text: task.assistant_text,
                    pending_approval: None,
                });
                continue;
            }

            idx += 1;
        }

        if !self.is_done() {
            let blocked = collect_dependency_blocked_task_ids(
                &self.tasks,
                &self.started_ids,
                &self.task_statuses,
            );
            if !blocked.is_empty() {
                for (task_id, dep_id, dep_status) in blocked {
                    self.started_ids.insert(task_id.clone());
                    self.task_statuses
                        .insert(task_id.clone(), TurnStatus::Cancelled);
                    if let Some(task) = self.tasks.iter().find(|task| task.id == task_id) {
                        self.finished
                            .push(blocked_task_result(task, &dep_id, dep_status));
                    }
                }
            }
            if self.active.is_empty() && !self.has_ready_pending_task() {
                anyhow::bail!(
                    "fan-out dependency deadlock: no runnable task (finished={}, total={})",
                    self.finished.len(),
                    self.tasks.len()
                );
            }
            if self.last_progress_print.elapsed() >= Duration::from_secs(1) {
                eprintln!(
                    "[fan-out] completed {}/{} (active={}, max={})",
                    self.finished.len(),
                    self.tasks.len(),
                    self.active.len(),
                    self.scheduling.effective_concurrency_limit
                );
                self.last_progress_print = Instant::now();
            }

            if self.last_progress_artifact_write.elapsed() >= Duration::from_secs(2) {
                let outcome = self
                    .write_progress_artifact(app, parent_thread_id, parent_turn_id)
                    .await;
                if let Err(err) = outcome {
                    eprintln!("[fan-out] progress artifact update failed: {err}");
                } else {
                    self.last_progress_artifact_write = Instant::now();
                }
            }
        } else {
            let outcome = self
                .write_progress_artifact(app, parent_thread_id, parent_turn_id)
                .await;
            if let Err(err) = outcome {
                eprintln!("[fan-out] final progress artifact update failed: {err}");
            }
        }

        Ok(())
    }

    pub(super) async fn run_to_completion(
        mut self,
        app: &mut App,
        parent_thread_id: ThreadId,
        parent_turn_id: Option<TurnId>,
    ) -> anyhow::Result<Vec<WorkflowTaskResult>> {
        while !self.is_done() {
            self.tick(app, parent_thread_id, parent_turn_id).await?;
            if !self.is_done() {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        Ok(self.results_ordered())
    }
}
