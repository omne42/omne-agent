struct SubagentSpawnTaskPlan {
    id: String,
    title: String,
    input: String,
    depends_on: Vec<String>,
    priority: AgentSpawnTaskPriority,
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    model: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
}

#[derive(Debug, Deserialize)]
struct SpawnedThread {
    thread_id: ThreadId,
    log_path: String,
    last_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubagentTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
struct SubagentSpawnTask {
    id: String,
    title: String,
    input: String,
    depends_on: Vec<String>,
    priority: AgentSpawnTaskPriority,
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    model: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
    workspace_cwd: Option<String>,
    thread_id: ThreadId,
    log_path: String,
    last_seq: u64,
    turn_id: Option<TurnId>,
    status: SubagentTaskStatus,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SubagentApprovalKey {
    thread_id: ThreadId,
    approval_id: ApprovalId,
}

#[derive(Debug, Clone)]
struct ExistingApprovalDecision {
    decision: omne_protocol::ApprovalDecision,
    remember: bool,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct ExistingParentProxyApproval {
    approval_id: ApprovalId,
    decision: Option<ExistingApprovalDecision>,
}

#[derive(Debug, Clone)]
struct SubagentPendingApproval {
    action: String,
    approval_id: ApprovalId,
    summary: String,
    child_turn_id: TurnId,
    child_approval_id: ApprovalId,
    child_action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalState {
    Missing,
    Pending,
    Decided,
}

struct SubagentSpawnSchedule {
    parent_thread_id: ThreadId,
    tasks: Vec<SubagentSpawnTask>,
    by_id: std::collections::HashMap<String, usize>,
    task_statuses: std::collections::HashMap<String, omne_protocol::TurnStatus>,
    ready_wait_rounds: std::collections::HashMap<String, usize>,
    running_by_thread: std::collections::HashMap<ThreadId, (String, TurnId)>,
    approval_proxy_by_child: std::collections::HashMap<SubagentApprovalKey, ApprovalId>,
    approval_proxy_targets: std::collections::HashMap<ApprovalId, SubagentApprovalKey>,
    pending_approvals_by_child:
        std::collections::HashMap<SubagentApprovalKey, SubagentPendingApproval>,
    result_artifact_error_ids_by_task: std::collections::HashMap<String, String>,
    result_artifact_scan_state_by_task:
        std::collections::HashMap<String, TaskResultArtifactScanState>,
    env_max_concurrent_subagents: usize,
    external_active: std::collections::HashSet<ThreadId>,
    max_concurrent: usize,
    priority_aging_rounds: usize,
}

#[derive(Clone, Default)]
struct TaskResultArtifactSummary {
    result_artifact_id: Option<String>,
    result_artifact_error: Option<String>,
    result_artifact_error_id: Option<String>,
    result_artifact_diagnostics: Option<omne_workflow_spec::FanInResultArtifactDiagnosticsStructuredData>,
}

#[derive(Default)]
struct TaskResultArtifactScanState {
    last_scanned_seq: u64,
    matched_completion_count: u64,
    matching_tool_ids: std::collections::HashSet<omne_protocol::ToolId>,
    summary: TaskResultArtifactSummary,
}

impl SubagentSpawnSchedule {
    fn new(
        parent_thread_id: ThreadId,
        tasks: Vec<SubagentSpawnTask>,
        external_active: std::collections::HashSet<ThreadId>,
        max_concurrent: usize,
        priority_aging_rounds: usize,
    ) -> Self {
        let mut by_id = std::collections::HashMap::<String, usize>::new();
        let mut task_statuses =
            std::collections::HashMap::<String, omne_protocol::TurnStatus>::new();
        let mut running_by_thread = std::collections::HashMap::<ThreadId, (String, TurnId)>::new();

        for (idx, task) in tasks.iter().enumerate() {
            by_id.insert(task.id.clone(), idx);
            match task.status {
                SubagentTaskStatus::Completed => {
                    task_statuses.insert(task.id.clone(), omne_protocol::TurnStatus::Completed);
                }
                SubagentTaskStatus::Failed => {
                    task_statuses.insert(task.id.clone(), omne_protocol::TurnStatus::Failed);
                }
                SubagentTaskStatus::Running => {
                    if let Some(turn_id) = task.turn_id {
                        running_by_thread.insert(task.thread_id, (task.id.clone(), turn_id));
                    }
                }
                SubagentTaskStatus::Pending => {}
            }
        }

        Self {
            parent_thread_id,
            tasks,
            by_id,
            task_statuses,
            ready_wait_rounds: std::collections::HashMap::new(),
            running_by_thread,
            approval_proxy_by_child: std::collections::HashMap::new(),
            approval_proxy_targets: std::collections::HashMap::new(),
            pending_approvals_by_child: std::collections::HashMap::new(),
            result_artifact_error_ids_by_task: std::collections::HashMap::new(),
            result_artifact_scan_state_by_task: std::collections::HashMap::new(),
            env_max_concurrent_subagents: max_concurrent,
            external_active,
            max_concurrent,
            priority_aging_rounds: priority_aging_rounds.max(1),
        }
    }

    fn set_env_max_concurrent_subagents(&mut self, env_max_concurrent_subagents: usize) {
        self.env_max_concurrent_subagents = env_max_concurrent_subagents;
    }

    fn is_done(&self) -> bool {
        self.task_statuses.len() >= self.tasks.len()
    }

    fn available_slots(&self) -> usize {
        if self.max_concurrent == 0 {
            usize::MAX
        } else {
            self.max_concurrent
                .saturating_sub(self.running_by_thread.len() + self.external_active.len())
        }
    }

    async fn start_ready_tasks(&mut self, server: &super::Server) {
        self.mark_dependency_blocked_tasks();
        self.update_ready_wait_rounds();

        let mut available = self.available_slots();
        if available == 0 {
            return;
        }

        while let Some(task_idx) = self.pick_next_ready_task_index() {
            if available == 0 {
                break;
            }
            let task_id = self.tasks[task_idx].id.clone();
            self.ready_wait_rounds.remove(&task_id);
            let turn_start = {
                let task = &mut self.tasks[task_idx];
                start_subagent_turn(server, self.parent_thread_id, task).await
            };
            match turn_start {
                Ok(turn_id) => {
                    let child_thread_id = self.tasks[task_idx].thread_id;
                    self.tasks[task_idx].turn_id = Some(turn_id);
                    self.tasks[task_idx].status = SubagentTaskStatus::Running;
                    self.running_by_thread
                        .insert(child_thread_id, (task_id.clone(), turn_id));
                    let _ = super::run_subagent_start_hooks(
                        server,
                        self.parent_thread_id,
                        task_id.as_str(),
                        child_thread_id,
                        turn_id,
                    )
                    .await;
                    available = available.saturating_sub(1);
                }
                Err(err) => {
                    self.tasks[task_idx].status = SubagentTaskStatus::Failed;
                    self.tasks[task_idx].error = Some(err.to_string());
                    self.task_statuses
                        .insert(task_id, omne_protocol::TurnStatus::Failed);
                }
            }
        }

        self.mark_dependency_blocked_tasks();
    }

    fn running_task_id_for_turn(&self, thread_id: ThreadId, turn_id: TurnId) -> Option<String> {
        let (task_id, expected_turn_id) = self.running_by_thread.get(&thread_id)?;
        if *expected_turn_id != turn_id {
            return None;
        }
        Some(task_id.clone())
    }

    fn handle_turn_completed(
        &mut self,
        thread_id: ThreadId,
        turn_id: TurnId,
        status: omne_protocol::TurnStatus,
        reason: Option<String>,
    ) -> Vec<ApprovalId> {
        if self.external_active.remove(&thread_id) {
            return Vec::new();
        }
        let Some((task_id, expected_turn_id)) = self.running_by_thread.get(&thread_id).cloned()
        else {
            return Vec::new();
        };
        if expected_turn_id != turn_id {
            return Vec::new();
        }
        self.running_by_thread.remove(&thread_id);
        let stale_proxy_approval_ids = self.clear_proxy_mappings_for_thread(thread_id);
        if let Some(idx) = self.by_id.get(&task_id).copied() {
            if matches!(status, omne_protocol::TurnStatus::Completed) {
                self.tasks[idx].status = SubagentTaskStatus::Completed;
                self.tasks[idx].error = None;
            } else {
                self.tasks[idx].status = SubagentTaskStatus::Failed;
                self.tasks[idx].error =
                    reason.or_else(|| Some(format!("turn finished with status={status:?}")));
            }
            self.task_statuses.insert(task_id, status);
            self.mark_dependency_blocked_tasks();
        }
        stale_proxy_approval_ids
    }


    fn mark_dependency_blocked_tasks(&mut self) {
        loop {
            let mut changed = false;
            for idx in 0..self.tasks.len() {
                if self.tasks[idx].status != SubagentTaskStatus::Pending {
                    continue;
                }
                let mut blocker: Option<(String, omne_protocol::TurnStatus)> = None;
                for dep in &self.tasks[idx].depends_on {
                    let Some(status) = self.task_statuses.get(dep).copied() else {
                        continue;
                    };
                    if !matches!(status, omne_protocol::TurnStatus::Completed) {
                        blocker = Some((dep.clone(), status));
                        break;
                    }
                }
                let Some((dep_id, dep_status)) = blocker else {
                    continue;
                };
                let task_id = self.tasks[idx].id.clone();
                self.tasks[idx].status = SubagentTaskStatus::Failed;
                self.tasks[idx].error = Some(format!(
                    "blocked by dependency: {dep_id} status={dep_status:?}"
                ));
                self.task_statuses
                    .insert(task_id, omne_protocol::TurnStatus::Cancelled);
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }

    fn is_ready_task(&self, task: &SubagentSpawnTask) -> bool {
        task.status == SubagentTaskStatus::Pending
            && task.depends_on.iter().all(|id| {
                matches!(
                    self.task_statuses.get(id),
                    Some(omne_protocol::TurnStatus::Completed)
                )
            })
    }

    fn update_ready_wait_rounds(&mut self) {
        let mut ready_ids = std::collections::HashSet::<String>::new();
        for task in &self.tasks {
            if self.is_ready_task(task) {
                ready_ids.insert(task.id.clone());
            }
        }

        self.ready_wait_rounds
            .retain(|task_id, _| ready_ids.contains(task_id));
        for task_id in ready_ids {
            *self.ready_wait_rounds.entry(task_id).or_insert(0) += 1;
        }
    }

    fn aged_priority_rank(&self, task: &SubagentSpawnTask) -> usize {
        let base = task.priority.rank();
        let waited_rounds = self.ready_wait_rounds.get(&task.id).copied().unwrap_or(0);
        base.saturating_sub(waited_rounds / self.priority_aging_rounds)
    }

    fn pick_next_ready_task_index(&self) -> Option<usize> {
        self.tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| self.is_ready_task(task))
            .min_by_key(|(idx, task)| (self.aged_priority_rank(task), *idx))
            .map(|(idx, _)| idx)
    }

}


include!("subagents_schedule_approval_proxy.rs");
include!("subagents_schedule_event_catch_up.rs");
include!("subagents_schedule_summary_artifacts.rs");

fn spawn_subagent_scheduler(server: super::Server, mut schedule: SubagentSpawnSchedule) {
    tokio::spawn(async move {
        let mut notify_rx = server.notify_tx.subscribe();
        schedule.catch_up_running_events(&server).await;
        loop {
            schedule.start_ready_tasks(&server).await;
            schedule.catch_up_running_events(&server).await;
            if schedule.is_done() {
                schedule
                    .settle_late_result_artifacts_before_exit(&server, &mut notify_rx)
                    .await;
                return;
            }

            match notify_rx.recv().await {
                Ok(line) => {
                    let Ok(val) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let method = val.get("method").and_then(Value::as_str);
                    if !matches!(
                        method,
                        Some("turn/completed") | Some("item/started") | Some("item/completed")
                    ) {
                        continue;
                    }
                    let Some(params) = val.get("params") else {
                        continue;
                    };
                    let Ok(event) =
                        serde_json::from_value::<omne_protocol::ThreadEvent>(params.clone())
                    else {
                        continue;
                    };
                    match event.kind {
                        omne_protocol::ThreadEventKind::TurnCompleted {
                            turn_id,
                            status,
                            reason,
                        } => {
                            let completed_task_id =
                                schedule.running_task_id_for_turn(event.thread_id, turn_id);
                            let hook_reason = reason.clone();
                            let stale_proxy_approval_ids = schedule.handle_turn_completed(
                                event.thread_id,
                                turn_id,
                                status,
                                reason,
                            );
                            if let Some(task_id) = completed_task_id {
                                let _ = super::run_subagent_stop_hooks(
                                    &server,
                                    schedule.parent_thread_id,
                                    task_id.as_str(),
                                    event.thread_id,
                                    turn_id,
                                    status,
                                    hook_reason.as_deref(),
                                )
                                .await;
                                schedule
                                    .write_fan_in_summary_artifact_best_effort(&server)
                                    .await;
                            }
                            schedule
                                .auto_deny_stale_parent_proxy_approvals(
                                    &server,
                                    event.thread_id,
                                    turn_id,
                                    status,
                                    stale_proxy_approval_ids,
                                )
                                .await;
                        }
                        omne_protocol::ThreadEventKind::ApprovalRequested {
                            approval_id,
                            turn_id,
                            action,
                            params,
                        } => {
                            schedule
                                .handle_approval_requested(
                                    &server,
                                    event.thread_id,
                                    approval_id,
                                    turn_id,
                                    action,
                                    params,
                                )
                                .await;
                        }
                        omne_protocol::ThreadEventKind::ApprovalDecided {
                            approval_id,
                            decision,
                            remember,
                            reason,
                        } => {
                            schedule
                                .handle_approval_decided(
                                    &server,
                                    event.thread_id,
                                    approval_id,
                                    decision,
                                    remember,
                                    reason,
                                )
                                .await;
                        }
                        _ => {}
                    }
                    schedule.update_task_last_seq(event.thread_id, event.seq);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    schedule.catch_up_running_events(&server).await;
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}
