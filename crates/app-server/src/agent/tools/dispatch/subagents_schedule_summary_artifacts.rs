impl SubagentSpawnSchedule {
    fn snapshot(&self) -> Vec<Value> {
        self.tasks
            .iter()
            .map(|task| {
                let dependency_blocker = dependency_blocker_details_from_error(task.error.as_deref());
                serde_json::json!({
                    "id": task.id.clone(),
                    "title": task.title.clone(),
                    "priority": priority_label(task.priority),
                    "spawn_mode": spawn_mode_label(task.spawn_mode),
                    "mode": task.mode.clone(),
                    "workspace_mode": workspace_mode_label(task.workspace_mode),
                    "thread_id": task.thread_id,
                    "turn_id": task.turn_id,
                    "log_path": task.log_path.clone(),
                    "last_seq": task.last_seq,
                    "depends_on": task.depends_on.clone(),
                    "expected_artifact_type": task.expected_artifact_type.clone(),
                    "workspace_cwd": task.workspace_cwd.clone(),
                    "model": task.model.clone(),
                    "openai_base_url": task.openai_base_url.clone(),
                    "status": task_status_label(task.status),
                    "turn_status": self.task_statuses.get(&task.id).copied(),
                    "dependency_blocked": dependency_blocker.is_some(),
                    "dependency_blocker_task_id": dependency_blocker.as_ref().map(|(task_id, _)| task_id),
                    "dependency_blocker_status": dependency_blocker.as_ref().map(|(_, status)| status),
                    "error": task.error.clone(),
                    "pending_approval": self.pending_approval_snapshot_for_task(task),
                })
            })
            .collect::<Vec<_>>()
    }

    async fn fan_in_summary_structured_data(
        &mut self,
        server: &super::Server,
    ) -> omne_workflow_spec::FanInSummaryStructuredData {
        let mut tasks = Vec::with_capacity(self.tasks.len());
        for idx in 0..self.tasks.len() {
            let task = self.tasks[idx].clone();
            let dependency_blocker = dependency_blocker_details_from_error(task.error.as_deref());
            let pending_approval =
                self.pending_approval_snapshot_for_task(&task)
                    .and_then(|pending| {
                        Some(omne_workflow_spec::FanInPendingApprovalStructuredData {
                            approval_id: pending.get("approval_id")?.as_str()?.to_string(),
                            action: pending.get("action")?.as_str()?.to_string(),
                            summary: pending
                                .get("summary")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            approve_cmd: pending
                                .get("approve_cmd")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            deny_cmd: pending
                                .get("deny_cmd")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        })
                    });

            let status = self
                .task_statuses
                .get(&task.id)
                .map(|status| format!("{status:?}"))
                .unwrap_or_else(|| {
                    match task.status {
                        SubagentTaskStatus::Pending => "Pending",
                        SubagentTaskStatus::Running => "Running",
                        SubagentTaskStatus::Completed => "Completed",
                        SubagentTaskStatus::Failed => "Failed",
                    }
                    .to_string()
                });

            let result_artifact = self
                .resolve_task_result_artifact_summary(server, &task)
                .await;
            tasks.push(omne_workflow_spec::FanInTaskStructuredData {
                task_id: task.id.clone(),
                title: task.title.clone(),
                thread_id: Some(task.thread_id.to_string()),
                turn_id: task.turn_id.as_ref().map(ToString::to_string),
                status,
                reason: task.error.clone(),
                dependency_blocked: dependency_blocker.is_some(),
                dependency_blocker_task_id: dependency_blocker
                    .as_ref()
                    .map(|(task_id, _)| task_id.clone()),
                dependency_blocker_status: dependency_blocker
                    .as_ref()
                    .map(|(_, status)| status.clone()),
                result_artifact_id: result_artifact.result_artifact_id,
                result_artifact_error: result_artifact.result_artifact_error,
                result_artifact_structured_error: result_artifact.result_artifact_structured_error,
                result_artifact_error_id: result_artifact.result_artifact_error_id,
                result_artifact_diagnostics: result_artifact.result_artifact_diagnostics,
                pending_approval,
            });
        }

        omne_workflow_spec::FanInSummaryStructuredData::new(
            self.parent_thread_id.to_string(),
            omne_workflow_spec::FanInSchedulingStructuredData {
                env_max_concurrent_subagents: self.env_max_concurrent_subagents,
                effective_concurrency_limit: self.max_concurrent,
                priority_aging_rounds: self.priority_aging_rounds,
            },
            tasks,
        )
    }

    async fn resolve_task_result_artifact_summary(
        &mut self,
        server: &super::Server,
        task: &SubagentSpawnTask,
    ) -> TaskResultArtifactSummary {
        let Some(task_turn_id) = task.turn_id else {
            return TaskResultArtifactSummary::default();
        };
        let state = self
            .result_artifact_scan_state_by_task
            .entry(task.id.clone())
            .or_default();
        let Ok(Some(events)) = server
            .thread_store
            .read_events_since(task.thread_id, EventSeq(state.last_scanned_seq))
            .await
        else {
            return state.summary.clone();
        };

        let mut max_seq = state.last_scanned_seq;

        for event in events {
            max_seq = max_seq.max(event.seq.0);
            match event.kind {
                omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: Some(turn_id),
                    tool,
                    params,
                } if turn_id == task_turn_id && tool == "artifact/write" => {
                    let artifact_type = params
                        .as_ref()
                        .and_then(|value| value.get("artifact_type"))
                        .and_then(Value::as_str);
                    if artifact_type == Some(task.expected_artifact_type.as_str()) {
                        state.matching_tool_ids.insert(tool_id);
                    }
                }
                omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status,
                    structured_error,
                    error,
                    result,
                } if state.matching_tool_ids.contains(&tool_id) => {
                    state.matched_completion_count =
                        state.matched_completion_count.saturating_add(1);
                    state.matching_tool_ids.remove(&tool_id);
                    if status == omne_protocol::ToolStatus::Completed {
                        if let Some(artifact_id) = result
                            .as_ref()
                            .and_then(|value| value.get("artifact_id"))
                            .and_then(Value::as_str)
                        {
                            state.summary.result_artifact_id = Some(artifact_id.to_string());
                            state.summary.result_artifact_error = None;
                            state.summary.result_artifact_structured_error = None;
                        }
                    } else {
                        state.summary.result_artifact_error = Some(error.unwrap_or_else(|| {
                            format!(
                                "{} write finished with status={status:?}",
                                task.expected_artifact_type
                            )
                        }));
                        state.summary.result_artifact_structured_error = structured_error;
                    }
                }
                _ => {}
            }
        }

        state.last_scanned_seq = state.last_scanned_seq.max(max_seq);
        let mut summary = state.summary.clone();
        summary.result_artifact_diagnostics = Some(Self::task_result_artifact_diagnostics(state));
        state.summary.result_artifact_diagnostics = summary.result_artifact_diagnostics.clone();

        if summary.result_artifact_id.is_some() {
            self.result_artifact_error_ids_by_task.remove(&task.id);
            summary.result_artifact_error_id = None;
            return summary;
        }
        let Some(write_error) = summary.result_artifact_error.clone() else {
            return summary;
        };
        if let Some(error_artifact_id) = self.result_artifact_error_ids_by_task.get(&task.id) {
            summary.result_artifact_error_id = Some(error_artifact_id.clone());
            let marker = format!("error_artifact_id={error_artifact_id}");
            if !write_error.contains(marker.as_str()) {
                summary.result_artifact_error = Some(format!("{write_error} ({marker})"));
            }
            if let Some(scan_state) = self.result_artifact_scan_state_by_task.get_mut(&task.id) {
                scan_state.summary.result_artifact_error = summary.result_artifact_error.clone();
            }
            return summary;
        }

        let turn_status = self
            .task_statuses
            .get(&task.id)
            .copied()
            .unwrap_or(omne_protocol::TurnStatus::Failed);
        if let Some(error_artifact_id) = self
            .write_fan_out_result_error_artifact_best_effort(
                server,
                task,
                turn_status,
                task.error.as_deref(),
                &write_error,
            )
            .await
        {
            self.result_artifact_error_ids_by_task
                .insert(task.id.clone(), error_artifact_id.clone());
            summary.result_artifact_error_id = Some(error_artifact_id.clone());
            summary.result_artifact_error = Some(format!(
                "{write_error} (error_artifact_id={error_artifact_id})"
            ));
            if let Some(scan_state) = self.result_artifact_scan_state_by_task.get_mut(&task.id) {
                scan_state.summary.result_artifact_error = summary.result_artifact_error.clone();
            }
        }
        summary
    }

    fn task_result_artifact_diagnostics(
        state: &TaskResultArtifactScanState,
    ) -> omne_workflow_spec::FanInResultArtifactDiagnosticsStructuredData {
        omne_workflow_spec::FanInResultArtifactDiagnosticsStructuredData {
            scan_last_seq: state.last_scanned_seq,
            matched_completion_count: state.matched_completion_count,
            pending_matching_tool_ids: state.matching_tool_ids.len(),
        }
    }

    async fn write_fan_out_result_error_artifact_best_effort(
        &self,
        server: &super::Server,
        task: &SubagentSpawnTask,
        status: omne_protocol::TurnStatus,
        reason: Option<&str>,
        write_error: &str,
    ) -> Option<String> {
        let summary = format!("fan-out result artifact write failed: {}", task.id);
        let text = render_fan_out_result_error_markdown(
            task.id.as_str(),
            task.title.as_str(),
            task.thread_id,
            task.turn_id,
            status,
            reason,
            write_error,
        );
        let write = match super::handle_artifact_write(
            server,
            super::ArtifactWriteParams {
                thread_id: self.parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_result_error".to_string(),
                summary,
                text,
            },
        )
        .await
        {
            Ok(write) => write,
            Err(err) => {
                tracing::warn!(
                    thread_id = %self.parent_thread_id,
                    task_id = %task.id,
                    error = %err,
                    original_error = %write_error,
                    "subagent scheduler failed to write fan_out_result_error artifact"
                );
                return None;
            }
        };
        write
            .get("artifact_id")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    async fn write_fan_in_summary_artifact_best_effort(&mut self, server: &super::Server) {
        let payload = self.fan_in_summary_structured_data(server).await;
        let text = render_fan_in_summary_markdown(&payload);
        if let Err(err) = super::handle_artifact_write(
            server,
            super::ArtifactWriteParams {
                thread_id: self.parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_in_summary".to_string(),
                summary: "fan-in summary".to_string(),
                text,
            },
        )
        .await
        {
            tracing::warn!(
                thread_id = %self.parent_thread_id,
                error = %err,
                "subagent scheduler failed to write fan_in_summary artifact"
            );
        }
    }
}
