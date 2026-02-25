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

    fn clear_proxy_mappings_for_thread(&mut self, thread_id: ThreadId) -> Vec<ApprovalId> {
        let keys = self
            .approval_proxy_by_child
            .keys()
            .filter(|key| key.thread_id == thread_id)
            .copied()
            .collect::<Vec<_>>();
        let mut removed_proxy_approval_ids = Vec::new();
        for key in keys {
            let Some(proxy_approval_id) = self.approval_proxy_by_child.remove(&key) else {
                continue;
            };
            self.approval_proxy_targets.remove(&proxy_approval_id);
            removed_proxy_approval_ids.push(proxy_approval_id);
        }
        self.clear_pending_approvals_for_thread(thread_id);
        removed_proxy_approval_ids
    }

    fn clear_proxy_mapping(&mut self, proxy_approval_id: ApprovalId) {
        let Some(child_key) = self.approval_proxy_targets.remove(&proxy_approval_id) else {
            self.clear_pending_approval_by_proxy(proxy_approval_id);
            return;
        };
        self.approval_proxy_by_child.remove(&child_key);
        self.clear_pending_approval_by_child(child_key);
    }

    fn set_pending_approval(
        &mut self,
        child_key: SubagentApprovalKey,
        child_turn_id: TurnId,
        child_action: &str,
        proxy_approval_id: ApprovalId,
    ) {
        self.pending_approvals_by_child.insert(
            child_key,
            SubagentPendingApproval {
                action: "subagent/proxy_approval".to_string(),
                approval_id: proxy_approval_id,
                summary: summarize_subagent_pending_approval(
                    child_key,
                    child_turn_id,
                    child_action,
                ),
                child_turn_id,
                child_approval_id: child_key.approval_id,
                child_action: child_action.to_string(),
            },
        );
    }

    fn clear_pending_approval_by_child(&mut self, child_key: SubagentApprovalKey) {
        self.pending_approvals_by_child.remove(&child_key);
    }

    fn clear_pending_approval_by_proxy(&mut self, proxy_approval_id: ApprovalId) {
        let Some(child_key) = self
            .pending_approvals_by_child
            .iter()
            .find_map(|(key, pending)| {
                if pending.approval_id == proxy_approval_id {
                    Some(*key)
                } else {
                    None
                }
            })
        else {
            return;
        };
        self.pending_approvals_by_child.remove(&child_key);
    }

    fn clear_pending_approvals_for_thread(&mut self, thread_id: ThreadId) {
        self.pending_approvals_by_child
            .retain(|key, _| key.thread_id != thread_id);
    }

    fn pending_approval_snapshot_for_task(&self, task: &SubagentSpawnTask) -> Option<Value> {
        let (child_key, pending) = self
            .pending_approvals_by_child
            .iter()
            .filter(|(key, _)| key.thread_id == task.thread_id)
            .min_by(|(left_key, _), (right_key, _)| {
                left_key
                    .approval_id
                    .to_string()
                    .cmp(&right_key.approval_id.to_string())
            })?;
        Some(serde_json::json!({
            "action": pending.action,
            "approval_id": pending.approval_id,
            "summary": pending.summary,
            "approve_cmd": format!(
                "omne approval decide {} {} --approve",
                self.parent_thread_id,
                pending.approval_id
            ),
            "child_thread_id": child_key.thread_id,
            "child_turn_id": pending.child_turn_id,
            "child_approval_id": pending.child_approval_id,
            "child_action": pending.child_action,
        }))
    }

    async fn find_existing_parent_proxy_approval(
        &self,
        server: &super::Server,
        child_key: SubagentApprovalKey,
    ) -> Option<ExistingParentProxyApproval> {
        let events = server
            .thread_store
            .read_events_since(self.parent_thread_id, EventSeq::ZERO)
            .await
            .ok()
            .flatten()?;
        let mut latest_matching_proxy_approval_id: Option<ApprovalId> = None;
        let mut decided = std::collections::HashMap::<ApprovalId, ExistingApprovalDecision>::new();
        for event in events {
            match event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    action,
                    params,
                    ..
                } => {
                    if action != "subagent/proxy_approval" {
                        continue;
                    }
                    let Some(requested_child_key) = parse_subagent_proxy_child_key(&params) else {
                        continue;
                    };
                    if requested_child_key == child_key {
                        latest_matching_proxy_approval_id = Some(approval_id);
                    }
                }
                omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    remember,
                    reason,
                } => {
                    decided.insert(
                        approval_id,
                        ExistingApprovalDecision {
                            decision,
                            remember,
                            reason,
                        },
                    );
                }
                _ => {}
            }
        }

        latest_matching_proxy_approval_id.map(|approval_id| ExistingParentProxyApproval {
            approval_id,
            decision: decided.get(&approval_id).cloned(),
        })
    }

    async fn approval_state(
        server: &super::Server,
        thread_id: ThreadId,
        approval_id: ApprovalId,
    ) -> Option<ApprovalState> {
        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await
            .ok()
            .flatten()?;

        let mut requested = false;
        let mut decided = false;
        for event in events {
            match event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id: got, ..
                } if got == approval_id => {
                    requested = true;
                }
                omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id: got, ..
                } if got == approval_id => {
                    decided = true;
                }
                _ => {}
            }
        }

        if !requested {
            return Some(ApprovalState::Missing);
        }
        if decided {
            return Some(ApprovalState::Decided);
        }
        Some(ApprovalState::Pending)
    }

    async fn reconcile_child_approval_with_parent_decision(
        &self,
        server: &super::Server,
        child_key: SubagentApprovalKey,
        decision: ExistingApprovalDecision,
    ) {
        let Some(child_state) =
            Self::approval_state(server, child_key.thread_id, child_key.approval_id).await
        else {
            return;
        };
        if child_state != ApprovalState::Pending {
            return;
        }

        let Ok(child_rt) = server.get_or_load_thread(child_key.thread_id).await else {
            return;
        };
        let _ = child_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: child_key.approval_id,
                decision: decision.decision,
                remember: decision.remember,
                reason: Some(decorate_subagent_proxy_forwarded_reason(
                    decision.reason.as_deref(),
                )),
            })
            .await;
    }

    async fn append_parent_proxy_decision_if_pending(
        &self,
        server: &super::Server,
        proxy_approval_id: ApprovalId,
        decision: omne_protocol::ApprovalDecision,
        remember: bool,
        reason: Option<String>,
    ) {
        let Some(parent_state) =
            Self::approval_state(server, self.parent_thread_id, proxy_approval_id).await
        else {
            return;
        };
        if parent_state != ApprovalState::Pending {
            return;
        }

        let Ok(parent_rt) = server.get_or_load_thread(self.parent_thread_id).await else {
            return;
        };
        let _ = parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: proxy_approval_id,
                decision,
                remember,
                reason,
            })
            .await;
    }

    fn update_task_last_seq(&mut self, thread_id: ThreadId, seq: EventSeq) {
        for task in &mut self.tasks {
            if task.thread_id == thread_id && seq.0 > task.last_seq {
                task.last_seq = seq.0;
                break;
            }
        }
    }

    fn handle_notify_line_for_settle(&mut self, line: &str) -> bool {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        let method = val.get("method").and_then(Value::as_str);
        if !matches!(
            method,
            Some("turn/completed") | Some("item/started") | Some("item/completed")
        ) {
            return false;
        }
        let Some(params) = val.get("params") else {
            return false;
        };
        let Ok(event) = serde_json::from_value::<omne_protocol::ThreadEvent>(params.clone()) else {
            return false;
        };
        let is_child_thread = self
            .tasks
            .iter()
            .any(|task| task.thread_id == event.thread_id);
        if !is_child_thread {
            return false;
        }
        self.update_task_last_seq(event.thread_id, event.seq);
        matches!(
            event.kind,
            omne_protocol::ThreadEventKind::ToolStarted { .. }
                | omne_protocol::ThreadEventKind::ToolCompleted { .. }
                | omne_protocol::ThreadEventKind::TurnCompleted { .. }
        )
    }

    async fn settle_late_result_artifacts_before_exit(
        &mut self,
        server: &super::Server,
        notify_rx: &mut tokio::sync::broadcast::Receiver<String>,
    ) {
        let mut touched = false;
        if let Ok(recv_result) =
            tokio::time::timeout(std::time::Duration::from_millis(200), notify_rx.recv()).await
        {
            match recv_result {
                Ok(line) => {
                    touched = self.handle_notify_line_for_settle(line.as_str()) || touched;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    touched = true;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
            }
        }

        loop {
            match notify_rx.try_recv() {
                Ok(line) => {
                    touched = self.handle_notify_line_for_settle(line.as_str()) || touched;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    touched = true;
                    continue;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            }
        }

        if touched {
            tracing::debug!(
                thread_id = %self.parent_thread_id,
                "subagent scheduler settled late result artifact notifications before exit"
            );
        }
        self.write_fan_in_summary_artifact_best_effort(server).await;
    }

    async fn handle_approval_requested(
        &mut self,
        server: &super::Server,
        child_thread_id: ThreadId,
        child_approval_id: ApprovalId,
        turn_id: Option<TurnId>,
        action: String,
        params: Value,
    ) {
        let Some((task_id, running_turn_id)) =
            self.running_by_thread.get(&child_thread_id).cloned()
        else {
            return;
        };
        if turn_id != Some(running_turn_id) {
            return;
        }

        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        if self.approval_proxy_by_child.contains_key(&child_key) {
            if let Some(proxy_approval_id) = self.approval_proxy_by_child.get(&child_key).copied() {
                self.set_pending_approval(
                    child_key,
                    running_turn_id,
                    action.as_str(),
                    proxy_approval_id,
                );
            }
            return;
        }
        if let Some(existing_proxy_approval) = self
            .find_existing_parent_proxy_approval(server, child_key)
            .await
        {
            if let Some(existing_decision) = existing_proxy_approval.decision {
                self.reconcile_child_approval_with_parent_decision(
                    server,
                    child_key,
                    existing_decision,
                )
                .await;
                self.clear_pending_approval_by_child(child_key);
            } else {
                self.approval_proxy_by_child
                    .insert(child_key, existing_proxy_approval.approval_id);
                self.approval_proxy_targets
                    .insert(existing_proxy_approval.approval_id, child_key);
                self.set_pending_approval(
                    child_key,
                    running_turn_id,
                    action.as_str(),
                    existing_proxy_approval.approval_id,
                );
            }
            return;
        }

        let Some(task_idx) = self.by_id.get(&task_id).copied() else {
            return;
        };
        let task = &self.tasks[task_idx];
        let proxy_approval_id = ApprovalId::new();
        let child_action = action.clone();

        let Ok(parent_rt) = server.get_or_load_thread(self.parent_thread_id).await else {
            return;
        };

        let proxy_params = serde_json::json!({
            "subagent_proxy": {
                "kind": "approval",
                "task_id": task.id.clone(),
                "task_title": task.title.clone(),
                "child_thread_id": child_thread_id,
                "child_turn_id": running_turn_id,
                "child_approval_id": child_approval_id,
            },
            "child_request": {
                "action": action,
                "params": params,
            }
        });

        if parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: proxy_params,
            })
            .await
            .is_err()
        {
            return;
        }

        self.approval_proxy_by_child
            .insert(child_key, proxy_approval_id);
        self.approval_proxy_targets
            .insert(proxy_approval_id, child_key);
        self.set_pending_approval(
            child_key,
            running_turn_id,
            child_action.as_str(),
            proxy_approval_id,
        );
    }

    async fn handle_approval_decided(
        &mut self,
        server: &super::Server,
        thread_id: ThreadId,
        approval_id: ApprovalId,
        decision: omne_protocol::ApprovalDecision,
        remember: bool,
        reason: Option<String>,
    ) {
        if thread_id == self.parent_thread_id {
            self.clear_proxy_mapping(approval_id);
            return;
        }

        if reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with(crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX))
        {
            let child_key = SubagentApprovalKey {
                thread_id,
                approval_id,
            };
            if let Some(proxy_approval_id) = self.approval_proxy_by_child.remove(&child_key) {
                self.approval_proxy_targets.remove(&proxy_approval_id);
            }
            self.clear_pending_approval_by_child(child_key);
            return;
        }

        let child_key = SubagentApprovalKey {
            thread_id,
            approval_id,
        };
        self.clear_pending_approval_by_child(child_key);
        let proxy_approval_id =
            if let Some(proxy_approval_id) = self.approval_proxy_by_child.remove(&child_key) {
                self.approval_proxy_targets.remove(&proxy_approval_id);
                Some(proxy_approval_id)
            } else {
                self.find_existing_parent_proxy_approval(server, child_key)
                    .await
                    .and_then(|existing| {
                        if existing.decision.is_some() {
                            None
                        } else {
                            Some(existing.approval_id)
                        }
                    })
            };
        let Some(proxy_approval_id) = proxy_approval_id else {
            return;
        };
        self.append_parent_proxy_decision_if_pending(
            server,
            proxy_approval_id,
            decision,
            remember,
            reason,
        )
        .await;
    }

    async fn auto_deny_stale_parent_proxy_approvals(
        &self,
        server: &super::Server,
        child_thread_id: ThreadId,
        child_turn_id: TurnId,
        child_status: omne_protocol::TurnStatus,
        stale_proxy_approval_ids: Vec<ApprovalId>,
    ) {
        if stale_proxy_approval_ids.is_empty() {
            return;
        }

        let Ok(Some(events)) = server
            .thread_store
            .read_events_since(self.parent_thread_id, EventSeq::ZERO)
            .await
        else {
            return;
        };
        let decided = events
            .into_iter()
            .filter_map(|event| match event.kind {
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                    Some(approval_id)
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        let Ok(parent_rt) = server.get_or_load_thread(self.parent_thread_id).await else {
            return;
        };
        for approval_id in stale_proxy_approval_ids {
            if decided.contains(&approval_id) {
                continue;
            }
            let reason = format!(
                "{} unresolved at turn completion \
                 (child_thread_id={child_thread_id} child_turn_id={child_turn_id} status={child_status:?})",
                crate::SUBAGENT_PROXY_AUTO_DENIED_REASON_PREFIX
            );
            let _ = parent_rt
                .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision: omne_protocol::ApprovalDecision::Denied,
                    remember: false,
                    reason: Some(reason),
                })
                .await;
        }
    }

    async fn catch_up_running_events(&mut self, server: &super::Server) {
        let running = self
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(idx, task)| {
                let turn_id = task.turn_id?;
                if task.status != SubagentTaskStatus::Running {
                    return None;
                }
                Some((idx, task.thread_id, turn_id, task.last_seq))
            })
            .collect::<Vec<_>>();

        for (idx, thread_id, running_turn_id, since_seq) in running {
            let Ok(Some(events)) = server
                .thread_store
                .read_events_since(thread_id, EventSeq(since_seq))
                .await
            else {
                continue;
            };

            let mut max_seq = since_seq;
            for event in events {
                max_seq = max_seq.max(event.seq.0);
                match event.kind {
                    omne_protocol::ThreadEventKind::TurnCompleted {
                        turn_id,
                        status,
                        reason,
                    } => {
                        if turn_id == running_turn_id {
                            let completed_task_id =
                                self.running_task_id_for_turn(thread_id, turn_id);
                            let hook_reason = reason.clone();
                            let stale_proxy_approval_ids =
                                self.handle_turn_completed(thread_id, turn_id, status, reason);
                            if let Some(task_id) = completed_task_id {
                                let _ = super::run_subagent_stop_hooks(
                                    server,
                                    self.parent_thread_id,
                                    task_id.as_str(),
                                    thread_id,
                                    turn_id,
                                    status,
                                    hook_reason.as_deref(),
                                )
                                .await;
                                self.write_fan_in_summary_artifact_best_effort(server).await;
                            }
                            self.auto_deny_stale_parent_proxy_approvals(
                                server,
                                thread_id,
                                turn_id,
                                status,
                                stale_proxy_approval_ids,
                            )
                            .await;
                        }
                    }
                    omne_protocol::ThreadEventKind::ApprovalRequested {
                        approval_id,
                        turn_id,
                        action,
                        params,
                    } => {
                        self.handle_approval_requested(
                            server,
                            thread_id,
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
                        self.handle_approval_decided(
                            server,
                            thread_id,
                            approval_id,
                            decision,
                            remember,
                            reason,
                        )
                        .await;
                    }
                    _ => {}
                }
            }

            if max_seq > since_seq && idx < self.tasks.len() {
                self.tasks[idx].last_seq = max_seq;
            }
        }
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
                    error,
                    result,
                } if state.matching_tool_ids.contains(&tool_id) => {
                    state.matched_completion_count = state.matched_completion_count.saturating_add(1);
                    state.matching_tool_ids.remove(&tool_id);
                    if status == omne_protocol::ToolStatus::Completed {
                        if let Some(artifact_id) = result
                            .as_ref()
                            .and_then(|value| value.get("artifact_id"))
                            .and_then(Value::as_str)
                        {
                            state.summary.result_artifact_id = Some(artifact_id.to_string());
                            state.summary.result_artifact_error = None;
                        }
                    } else {
                        state.summary.result_artifact_error = Some(error.unwrap_or_else(|| {
                            format!(
                                "{} write finished with status={status:?}",
                                task.expected_artifact_type
                            )
                        }));
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

fn spawn_mode_label(mode: AgentSpawnMode) -> &'static str {
    match mode {
        AgentSpawnMode::Fork => "fork",
        AgentSpawnMode::New => "new",
    }
}

fn workspace_mode_label(mode: AgentSpawnWorkspaceMode) -> &'static str {
    match mode {
        AgentSpawnWorkspaceMode::ReadOnly => "read_only",
        AgentSpawnWorkspaceMode::IsolatedWrite => "isolated_write",
    }
}

fn priority_label(priority: AgentSpawnTaskPriority) -> &'static str {
    match priority {
        AgentSpawnTaskPriority::High => "high",
        AgentSpawnTaskPriority::Normal => "normal",
        AgentSpawnTaskPriority::Low => "low",
    }
}

fn task_status_label(status: SubagentTaskStatus) -> &'static str {
    match status {
        SubagentTaskStatus::Pending => "pending",
        SubagentTaskStatus::Running => "running",
        SubagentTaskStatus::Completed => "completed",
        SubagentTaskStatus::Failed => "failed",
    }
}

fn render_fan_in_summary_markdown(
    payload: &omne_workflow_spec::FanInSummaryStructuredData,
) -> String {
    let structured_json = serde_json::to_string_pretty(payload)
        .or_else(|_| serde_json::to_string(payload))
        .unwrap_or_else(|_| "{}".to_string());
    format!("# Fan-in Summary\n\n## Structured Data\n\n```json\n{structured_json}\n```\n")
}

fn render_fan_out_result_error_markdown(
    task_id: &str,
    title: &str,
    child_thread_id: ThreadId,
    turn_id: Option<TurnId>,
    status: omne_protocol::TurnStatus,
    reason: Option<&str>,
    write_error: &str,
) -> String {
    let mut text = String::new();
    text.push_str("# Fan-out Result Artifact Error\n\n");
    text.push_str(&format!("- task_id: `{task_id}`\n"));
    if !title.trim().is_empty() {
        text.push_str(&format!("- title: {title}\n"));
    }
    text.push_str(&format!("- child_thread_id: `{child_thread_id}`\n"));
    text.push_str(&format!(
        "- turn_id: `{}`\n",
        turn_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<none>".to_string())
    ));
    text.push_str(&format!("- status: `{:?}`\n", status));
    if let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) {
        text.push_str(&format!("- reason: {}\n", reason));
    }
    text.push_str(&format!("- error: {}\n", write_error));
    text
}

fn dependency_blocker_details_from_error(error: Option<&str>) -> Option<(String, String)> {
    let error = error?.trim();
    let rest = error.strip_prefix("blocked by dependency: ")?;
    let (dependency_task_id, dependency_status) = rest.split_once(" status=")?;
    let dependency_task_id = dependency_task_id.trim();
    let dependency_status = dependency_status.trim();
    if dependency_task_id.is_empty() || dependency_status.is_empty() {
        return None;
    }
    Some((
        dependency_task_id.to_string(),
        dependency_status.to_string(),
    ))
}

fn parse_subagent_proxy_child_key(params: &Value) -> Option<SubagentApprovalKey> {
    let proxy = params.get("subagent_proxy")?.as_object()?;
    if proxy.get("kind").and_then(Value::as_str) != Some("approval") {
        return None;
    }
    let thread_id = proxy
        .get("child_thread_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    let approval_id = proxy
        .get("child_approval_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    Some(SubagentApprovalKey {
        thread_id,
        approval_id,
    })
}

fn decorate_subagent_proxy_forwarded_reason(reason: Option<&str>) -> String {
    let suffix = reason.unwrap_or_default().trim();
    if suffix.is_empty() {
        crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX.to_string()
    } else {
        format!("{} {suffix}", crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX)
    }
}

fn summarize_subagent_pending_approval(
    child_key: SubagentApprovalKey,
    child_turn_id: TurnId,
    child_action: &str,
) -> String {
    format!(
        "child_thread_id={} child_turn_id={} child_approval_id={} child_action={}",
        child_key.thread_id, child_turn_id, child_key.approval_id, child_action
    )
}

async fn start_subagent_turn(
    server: &super::Server,
    parent_thread_id: ThreadId,
    task: &SubagentSpawnTask,
) -> anyhow::Result<TurnId> {
    let rt = server.get_or_load_thread(task.thread_id).await?;
    let server_arc = Arc::new(server.clone());
    let turn_id = rt
        .start_turn(
            server_arc,
            task.input.clone(),
            None,
            None,
            None,
            omne_protocol::TurnPriority::Background,
        )
        .await?;

    let parent_workspace_cwd =
        if matches!(task.workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite) {
            match server.get_or_load_thread(parent_thread_id).await {
                Ok(parent_rt) => {
                    let handle = parent_rt.handle.lock().await;
                    handle.state().cwd.clone()
                }
                Err(_) => None,
            }
        } else {
            None
        };

    let notify_rx = server.notify_tx.subscribe();
    spawn_fan_out_result_writer_with_target_workspace(
        server.clone(),
        notify_rx,
        task.thread_id,
        turn_id,
        task.id.clone(),
        task.expected_artifact_type.clone(),
        task.workspace_mode,
        task.workspace_cwd.clone(),
        parent_workspace_cwd,
        isolated_auto_apply_patch_enabled_from_env(),
    );

    Ok(turn_id)
}

async fn create_new_thread(server: &super::Server, cwd: &str) -> anyhow::Result<SpawnedThread> {
    let handle = server
        .thread_store
        .create_thread(PathBuf::from(cwd))
        .await?;
    let thread_id = handle.thread_id();
    let log_path = handle.log_path().display().to_string();
    let last_seq = handle.last_seq().0;

    let rt = Arc::new(crate::ThreadRuntime::new(handle, server.notify_tx.clone()));
    server.threads.lock().await.insert(thread_id, rt);

    Ok(SpawnedThread {
        thread_id,
        log_path,
        last_seq,
    })
}

const DEFAULT_ISOLATED_MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_ISOLATED_MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;

fn sanitize_isolated_workspace_component(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    let out = if out.is_empty() { "task" } else { out };
    out.chars().take(80).collect::<String>()
}

fn is_isolated_runtime_rel_path(rel: &std::path::Path) -> bool {
    let mut components = rel.components();
    let Some(first) = components.next() else {
        return false;
    };
    let std::path::Component::Normal(first) = first else {
        return false;
    };
    if first != std::ffi::OsStr::new(".omne_data") && first != std::ffi::OsStr::new("omne_data") {
        return false;
    }
    let Some(std::path::Component::Normal(second)) = components.next() else {
        return false;
    };
    matches!(
        second.to_str().unwrap_or_default(),
        "tmp" | "threads" | "locks" | "logs" | "data" | "repos" | "reference"
    )
}

fn should_walk_isolated_workspace_entry(
    source_root: &std::path::Path,
    entry: &walkdir::DirEntry,
) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }
    let rel = entry
        .path()
        .strip_prefix(source_root)
        .unwrap_or(entry.path());
    !is_isolated_runtime_rel_path(rel)
}

#[cfg(unix)]
fn create_isolated_symlink(
    target: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
fn create_isolated_symlink(
    target: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    let metadata = std::fs::metadata(target);
    if metadata.as_ref().is_ok_and(|meta| meta.is_dir()) {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}

async fn prepare_isolated_workspace(
    server: &super::Server,
    parent_thread_id: ThreadId,
    task_id: &str,
    source_root: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    let max_file_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_MAX_FILE_BYTES",
        DEFAULT_ISOLATED_MAX_FILE_BYTES,
        1,
        8 * 1024 * 1024 * 1024,
    );
    let max_total_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_MAX_TOTAL_BYTES",
        DEFAULT_ISOLATED_MAX_TOTAL_BYTES,
        max_file_bytes,
        64 * 1024 * 1024 * 1024,
    );

    let source_root = source_root.to_path_buf();
    let label = sanitize_isolated_workspace_component(task_id);
    let nonce = omne_protocol::ToolId::new().to_string();
    let isolated_root = server
        .cwd
        .join(".omne_data")
        .join("tmp")
        .join("subagents")
        .join(parent_thread_id.to_string())
        .join(format!("{label}-{nonce}"))
        .join("repo");
    let isolated_root_for_task = isolated_root.clone();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        std::fs::create_dir_all(&isolated_root_for_task).with_context(|| {
            format!(
                "create isolated workspace {}",
                isolated_root_for_task.display()
            )
        })?;

        let mut total_bytes = 0u64;
        for entry in walkdir::WalkDir::new(&source_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| should_walk_isolated_workspace_entry(&source_root, entry))
        {
            let entry = entry?;
            let rel = entry
                .path()
                .strip_prefix(&source_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() || is_isolated_runtime_rel_path(rel) {
                continue;
            }
            let destination = isolated_root_for_task.join(rel);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&destination)
                    .with_context(|| format!("create {}", destination.display()))?;
                continue;
            }
            if entry.file_type().is_symlink() {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                let target = std::fs::read_link(entry.path())
                    .with_context(|| format!("read symlink {}", entry.path().display()))?;
                create_isolated_symlink(&target, &destination).with_context(|| {
                    format!("symlink {} -> {}", destination.display(), target.display())
                })?;
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > max_file_bytes {
                anyhow::bail!(
                    "isolated workspace copy skipped oversized file: {} ({} bytes > {} bytes)",
                    rel.display(),
                    meta.len(),
                    max_file_bytes
                );
            }
            total_bytes = total_bytes.saturating_add(meta.len());
            if total_bytes > max_total_bytes {
                anyhow::bail!(
                    "isolated workspace copy exceeds max_total_bytes={} (current={})",
                    max_total_bytes,
                    total_bytes
                );
            }
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(entry.path(), &destination).with_context(|| {
                format!(
                    "copy {} -> {}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        }

        Ok(())
    })
    .await
    .context("join isolated workspace copy task")??;

    Ok(isolated_root)
}

#[allow(dead_code)]
fn spawn_fan_out_result_writer(
    server: super::Server,
    notify_rx: tokio::sync::broadcast::Receiver<String>,
    thread_id: omne_protocol::ThreadId,
    turn_id: TurnId,
    task_id: String,
    expected_artifact_type: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    workspace_cwd: Option<String>,
) {
    spawn_fan_out_result_writer_with_target_workspace(
        server,
        notify_rx,
        thread_id,
        turn_id,
        task_id,
        expected_artifact_type,
        workspace_mode,
        workspace_cwd,
        None,
        false,
    );
}

fn spawn_fan_out_result_writer_with_target_workspace(
    server: super::Server,
    mut notify_rx: tokio::sync::broadcast::Receiver<String>,
    thread_id: omne_protocol::ThreadId,
    turn_id: TurnId,
    task_id: String,
    expected_artifact_type: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    workspace_cwd: Option<String>,
    target_workspace_cwd: Option<String>,
    isolated_write_auto_apply_enabled: bool,
) {
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(line) => {
                    let Ok(val) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if val.get("method").and_then(Value::as_str) != Some("turn/completed") {
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
                    if event.thread_id != thread_id {
                        continue;
                    }
                    let omne_protocol::ThreadEventKind::TurnCompleted {
                        turn_id: completed_turn_id,
                        status,
                        reason,
                    } = event.kind
                    else {
                        continue;
                    };
                    if completed_turn_id != turn_id {
                        continue;
                    }

                    let isolated_write_patch = if expected_artifact_type == "fan_out_result"
                        && matches!(workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite)
                    {
                        if let Some(cwd) = workspace_cwd.as_deref() {
                            try_write_isolated_workspace_patch_artifact(
                                &server,
                                thread_id,
                                turn_id,
                                task_id.as_str(),
                                cwd,
                            )
                            .await
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let isolated_write_handoff = if matches!(
                        workspace_mode,
                        AgentSpawnWorkspaceMode::IsolatedWrite
                    ) {
                        workspace_cwd.as_ref().map(|cwd| {
                                let mut handoff = serde_json::json!({
                                    "workspace_cwd": cwd,
                                    "status_argv": ["git", "-C", cwd, "status", "--short", "--"],
                                    "diff_argv": ["git", "-C", cwd, "diff", "--binary", "--"],
                                    "apply_patch_hint": "capture diff output and apply in target workspace with git apply"
                                });
                                if let Some(patch) = isolated_write_patch.as_ref() {
                                    handoff["patch"] = patch.clone();
                                }
                                handoff
                            })
                    } else {
                        None
                    };
                    let mut isolated_write_auto_apply = if expected_artifact_type
                        == "fan_out_result"
                        && matches!(workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite)
                    {
                        if let Some(cwd) = workspace_cwd.as_deref() {
                            try_auto_apply_isolated_workspace_patch(
                                cwd,
                                target_workspace_cwd.as_deref(),
                                &status,
                                isolated_write_auto_apply_enabled,
                            )
                            .await
                        } else if isolated_write_auto_apply_enabled {
                            let failure_stage =
                                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Precondition;
                            Some(serde_json::json!({
                                "enabled": true,
                                "attempted": false,
                                "applied": false,
                                "failure_stage": failure_stage.as_str(),
                                "recovery_hint": "ensure isolated workspace cwd is available before enabling auto-apply",
                                "error": "isolated workspace cwd is missing",
                            }))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let (Some(auto_apply), Some(patch)) = (
                        isolated_write_auto_apply.as_mut(),
                        isolated_write_patch.as_ref(),
                    ) {
                        if let Some(auto_apply_obj) = auto_apply.as_object_mut() {
                            if let Some(patch_artifact_id) = patch
                                .get("artifact_id")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                auto_apply_obj.insert(
                                    "patch_artifact_id".to_string(),
                                    serde_json::json!(patch_artifact_id),
                                );
                            }
                            if let Some(patch_read_cmd) = patch
                                .get("read_cmd")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                auto_apply_obj.insert(
                                    "patch_read_cmd".to_string(),
                                    serde_json::json!(patch_read_cmd),
                                );
                            }
                        }
                    }
                    if let Some(auto_apply) = isolated_write_auto_apply.as_mut() {
                        if let Some(auto_apply_obj) = auto_apply.as_object_mut() {
                            let has_error = auto_apply_obj
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .is_some_and(|value| !value.is_empty());
                            if has_error {
                                let mut recovery_commands = Vec::<Value>::new();

                                if let Some(patch_artifact_id) = auto_apply_obj
                                    .get("patch_artifact_id")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "read_patch_artifact",
                                        "argv": ["omne", "artifact", "read", thread_id.to_string(), patch_artifact_id],
                                    }));
                                } else if let Some(patch_read_cmd) = auto_apply_obj
                                    .get("patch_read_cmd")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "read_patch_artifact",
                                        "argv": patch_read_cmd.split_whitespace().collect::<Vec<_>>(),
                                    }));
                                }

                                if let Some(target_workspace_cwd) = auto_apply_obj
                                    .get("target_workspace_cwd")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "show_target_status",
                                        "argv": ["git", "-C", target_workspace_cwd, "status", "--short", "--"],
                                    }));
                                }

                                if let Some(check_argv) = auto_apply_obj
                                    .get("check_argv")
                                    .and_then(json_value_string_array)
                                    .filter(|argv| !argv.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "check_apply_with_patch_stdin",
                                        "argv": check_argv,
                                    }));
                                }

                                if let Some(apply_argv) = auto_apply_obj
                                    .get("apply_argv")
                                    .and_then(json_value_string_array)
                                    .filter(|argv| !argv.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "apply_with_patch_stdin",
                                        "argv": apply_argv,
                                    }));
                                }

                                if !recovery_commands.is_empty() {
                                    auto_apply_obj.insert(
                                        "recovery_commands".to_string(),
                                        Value::Array(recovery_commands),
                                    );
                                }
                            }
                        }
                    }
                    let mut payload = serde_json::json!({
                        "task_id": task_id,
                        "thread_id": thread_id,
                        "turn_id": turn_id,
                        "workspace_mode": workspace_mode_label(workspace_mode),
                        "workspace_cwd": workspace_cwd,
                        "isolated_write_patch": isolated_write_patch,
                        "isolated_write_handoff": isolated_write_handoff,
                        "isolated_write_auto_apply": isolated_write_auto_apply,
                        "status": status,
                        "reason": reason,
                    });
                    let is_fan_out_result = expected_artifact_type == "fan_out_result";
                    if is_fan_out_result {
                        payload["schema_version"] = serde_json::Value::String(
                            omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
                        );
                    }
                    let text = match serde_json::to_string_pretty(&payload) {
                        Ok(json) => format!("```json\n{json}\n```\n"),
                        Err(_) => payload.to_string(),
                    };

                    let write_result = super::handle_artifact_write(
                        &server,
                        super::ArtifactWriteParams {
                            thread_id,
                            turn_id: Some(turn_id),
                            approval_id: None,
                            artifact_id: None,
                            artifact_type: expected_artifact_type.clone(),
                            summary: "fan-out result".to_string(),
                            text,
                        },
                    )
                    .await;
                    if is_fan_out_result
                        && let Ok(write) = &write_result
                        && let Some(raw_id) = write.get("artifact_id")
                        && let Ok(artifact_id) =
                            serde_json::from_value::<omne_protocol::ArtifactId>(raw_id.clone())
                    {
                        let auto_apply_error_present = payload
                            .get("isolated_write_auto_apply")
                            .and_then(serde_json::Value::as_object)
                            .and_then(|value| value.get("error"))
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|value| !value.trim().is_empty());

                        let marker_event = if auto_apply_error_present {
                            omne_protocol::ThreadEventKind::AttentionMarkerSet {
                                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                                turn_id: Some(turn_id),
                                artifact_id: Some(artifact_id),
                                artifact_type: Some("fan_out_result".to_string()),
                                process_id: None,
                                exit_code: None,
                                command: None,
                            }
                        } else {
                            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                                turn_id: Some(turn_id),
                                reason: Some(
                                    "fan-out auto-apply completed without error".to_string(),
                                ),
                            }
                        };
                        if let Ok(thread_rt) = server.get_or_load_thread(thread_id).await {
                            let _ = thread_rt.append_event(marker_event).await;
                        }
                    }
                    return;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

const DEFAULT_ISOLATED_PATCH_MAX_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_ISOLATED_PATCH_TIMEOUT_MS: u64 = 5_000;
const ISOLATED_AUTO_APPLY_PATCH_ENV: &str = "OMNE_SUBAGENT_ISOLATED_AUTO_APPLY_PATCH";

fn parse_subagent_env_bool(raw: Option<&str>, default: bool) -> bool {
    let Some(raw) = raw else {
        return default;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn json_value_string_array(value: &Value) -> Option<Vec<String>> {
    let values = value.as_array()?;
    let mut out = Vec::with_capacity(values.len());
    for item in values {
        let text = item.as_str()?.trim();
        if text.is_empty() {
            continue;
        }
        out.push(text.to_string());
    }
    Some(out)
}

fn isolated_auto_apply_patch_enabled_from_env() -> bool {
    parse_subagent_env_bool(
        std::env::var(ISOLATED_AUTO_APPLY_PATCH_ENV).ok().as_deref(),
        false,
    )
}

async fn capture_isolated_workspace_patch(cwd: &str) -> anyhow::Result<Option<(String, bool)>> {
    let max_patch_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_PATCH_MAX_BYTES",
        DEFAULT_ISOLATED_PATCH_MAX_BYTES,
        1_024,
        64 * 1024 * 1024,
    ) as usize;
    let timeout_ms = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_PATCH_TIMEOUT_MS",
        DEFAULT_ISOLATED_PATCH_TIMEOUT_MS,
        100,
        120_000,
    );

    // Best-effort: include untracked files in the generated patch without staging content.
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tokio::process::Command::new("git")
            .args(["add", "--intent-to-add", "--", "."])
            .current_dir(cwd)
            .output(),
    )
    .await;

    let output = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tokio::process::Command::new("git")
            .args([
                "--no-pager",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--binary",
                "--patch",
            ])
            .current_dir(cwd)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("git diff timed out after {}ms", timeout_ms))?
    .with_context(|| format!("spawn git diff in {}", cwd))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git diff --binary --patch failed in {} (exit {:?}): {}",
            cwd,
            output.status.code(),
            stderr
        );
    }

    if output.stdout.is_empty() {
        return Ok(None);
    }

    let mut bytes = output.stdout;
    let truncated = bytes.len() > max_patch_bytes;
    if truncated {
        bytes.truncate(max_patch_bytes);
    }
    let mut text = String::from_utf8_lossy(&bytes).to_string();
    if truncated {
        text.push_str("\n# <...truncated...>\n");
    }
    Ok(Some((text, truncated)))
}

async fn run_git_apply_with_patch_stdin(
    cwd: &str,
    args: &[&str],
    patch_text: &str,
) -> anyhow::Result<()> {
    let mut child = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn git {} in {}", args.join(" "), cwd))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(patch_text.as_bytes())
            .await
            .with_context(|| format!("write patch stdin for git {} in {}", args.join(" "), cwd))?;
    }

    let output = child
        .wait_with_output()
        .await
        .with_context(|| format!("wait git {} in {}", args.join(" "), cwd))?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "git {} failed in {} (exit {:?}): stdout={}, stderr={}",
        args.join(" "),
        cwd,
        output.status.code(),
        stdout,
        stderr
    );
}

async fn try_auto_apply_isolated_workspace_patch(
    workspace_cwd: &str,
    target_workspace_cwd: Option<&str>,
    status: &omne_protocol::TurnStatus,
    enabled: bool,
) -> Option<Value> {
    if !enabled {
        return None;
    }

    let mut payload = serde_json::json!({
        "enabled": true,
        "attempted": false,
        "applied": false,
        "workspace_cwd": workspace_cwd,
        "target_workspace_cwd": target_workspace_cwd,
    });

    let set_failure =
        |payload: &mut Value,
         stage: omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage,
         hint: &str,
         error: String| {
            payload["failure_stage"] = serde_json::json!(stage.as_str());
            payload["recovery_hint"] = serde_json::json!(hint);
            payload["error"] = serde_json::json!(error);
        };

    if !matches!(status, omne_protocol::TurnStatus::Completed) {
        set_failure(
            &mut payload,
            omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Precondition,
            "retry auto-apply after the child turn reaches completed status",
            format!("turn status is not completed: {status:?}"),
        );
        return Some(payload);
    }

    let Some(target_workspace_cwd) = target_workspace_cwd
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        set_failure(
            &mut payload,
            omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Precondition,
            "ensure parent workspace cwd is available for auto-apply",
            "target workspace cwd is missing".to_string(),
        );
        return Some(payload);
    };

    let patch = match capture_isolated_workspace_patch(workspace_cwd).await {
        Ok(Some(patch)) => patch,
        Ok(None) => {
            set_failure(
                &mut payload,
                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CapturePatch,
                "collect patch manually from isolated workspace and apply it in parent workspace",
                "isolated workspace has no patch to apply".to_string(),
            );
            return Some(payload);
        }
        Err(err) => {
            set_failure(
                &mut payload,
                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CapturePatch,
                "collect patch manually from isolated workspace and apply it in parent workspace",
                format!("capture isolated patch for auto-apply failed: {err}"),
            );
            return Some(payload);
        }
    };

    if patch.1 {
        set_failure(
            &mut payload,
            omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CapturePatch,
            "patch is truncated; use the patch artifact or manual git diff/apply workflow",
            "isolated patch is truncated; refusing to auto-apply".to_string(),
        );
        return Some(payload);
    }

    payload["attempted"] = serde_json::json!(true);
    payload["check_argv"] = serde_json::json!([
        "git",
        "-C",
        target_workspace_cwd,
        "apply",
        "--check",
        "--whitespace=nowarn",
        "-",
    ]);
    payload["apply_argv"] = serde_json::json!([
        "git",
        "-C",
        target_workspace_cwd,
        "apply",
        "--whitespace=nowarn",
        "-",
    ]);

    if let Err(err) = run_git_apply_with_patch_stdin(
        target_workspace_cwd,
        &["apply", "--check", "--whitespace=nowarn", "-"],
        &patch.0,
    )
    .await
    {
        set_failure(
            &mut payload,
            omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
            "resolve apply-check conflicts in parent workspace, then apply patch manually",
            format!("git apply --check failed: {err}"),
        );
        return Some(payload);
    }

    if let Err(err) = run_git_apply_with_patch_stdin(
        target_workspace_cwd,
        &["apply", "--whitespace=nowarn", "-"],
        &patch.0,
    )
    .await
    {
        set_failure(
            &mut payload,
            omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::ApplyPatch,
            "inspect git apply output and apply patch manually if needed",
            format!("git apply failed: {err}"),
        );
        return Some(payload);
    }

    payload["applied"] = serde_json::json!(true);
    Some(payload)
}

async fn try_write_isolated_workspace_patch_artifact(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    task_id: &str,
    workspace_cwd: &str,
) -> Option<Value> {
    let patch = match capture_isolated_workspace_patch(workspace_cwd).await {
        Ok(Some(patch)) => patch,
        Ok(None) => return None,
        Err(err) => {
            return Some(serde_json::json!({
                "workspace_cwd": workspace_cwd,
                "error": err.to_string(),
            }));
        }
    };

    let summary = format!("fan-out isolated patch ({task_id})");
    let write = match super::handle_artifact_write(
        server,
        super::ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "patch".to_string(),
            summary,
            text: patch.0,
        },
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            return Some(serde_json::json!({
                "workspace_cwd": workspace_cwd,
                "error": format!("patch artifact write failed: {err}"),
            }));
        }
    };

    let Some(artifact_id) = write.get("artifact_id").and_then(Value::as_str) else {
        return Some(serde_json::json!({
            "workspace_cwd": workspace_cwd,
            "error": "patch artifact write response missing artifact_id",
        }));
    };

    Some(serde_json::json!({
        "artifact_type": "patch",
        "artifact_id": artifact_id,
        "truncated": patch.1,
        "read_cmd": format!("omne artifact read {} {}", thread_id, artifact_id),
    }))
}

#[cfg(test)]
mod agent_spawn_guard_tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tokio::time::{Duration, Instant};

    fn build_test_server(omne_root: PathBuf) -> super::super::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        super::super::Server {
            cwd: omne_root.clone(),
            notify_tx,
            thread_store: super::super::ThreadStore::new(super::super::PmPaths::new(omne_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(super::super::McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_execpolicy::Policy::empty(),
        }
    }

    #[tokio::test]
    async fn isolated_workspace_copy_skips_runtime_dirs() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let omne_root = tmp.path().join(".omne_data");
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/threads")).await?;
        tokio::fs::write(repo_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "pub fn demo() {}\n").await?;
        tokio::fs::write(repo_dir.join(".omne_data/threads/skip.txt"), "skip\n").await?;

        let server = build_test_server(omne_root.clone());
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = handle.thread_id();
        drop(handle);

        let isolated_root =
            prepare_isolated_workspace(&server, parent_thread_id, "t1", &repo_dir).await?;
        let isolated_root = Path::new(&isolated_root);
        assert!(isolated_root.join("hello.txt").exists());
        assert!(isolated_root.join("src/lib.rs").exists());
        assert!(!isolated_root.join(".omne_data/threads/skip.txt").exists());
        assert!(
            isolated_root.starts_with(
                omne_root
                    .join(".omne_data")
                    .join("tmp")
                    .join("subagents")
                    .join(parent_thread_id.to_string())
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_denies_disallowed_child_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "coder",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("mode_permission"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(false));
        assert!(result["allowed_modes"].as_array().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with explicit spawn deny override"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
    tool_overrides:
      - tool: "subagent/spawn"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_enforces_default_max_concurrent_subagents() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(omne_protocol::ThreadEventKind::TurnStarted {
                    turn_id: omne_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    directives: None,
                    priority: omne_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = omne_protocol::ToolId::new();
            parent
                .append(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["priority_aging_rounds"].as_u64(), Some(3));
        assert_eq!(result["limit_source"].as_str(), Some("env"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_enforces_mode_max_concurrent_subagents() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with strict subagent limit"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 1
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
        let child_id = child.thread_id();
        child
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: omne_protocol::TurnId::new(),
                input: "child".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(child);

        let tool_id = omne_protocol::ToolId::new();
        parent
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: None,
                tool: "subagent/spawn".to_string(),
                params: None,
            })
            .await?;
        parent
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "thread_id": child_id,
                })),
            })
            .await?;
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["limit_source"].as_str(), Some("combined"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(
            result["mode_max_concurrent_subagents"]
                .as_u64()
                .unwrap_or(0),
            1
        );
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 1);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 1);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_mode_max_threads_zero_falls_back_to_env_limit() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with unlimited mode max"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 0
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(omne_protocol::ThreadEventKind::TurnStarted {
                    turn_id: omne_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    directives: None,
                    priority: omne_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = omne_protocol::ToolId::new();
            parent
                .append(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["limit_source"].as_str(), Some("env"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(result["env_max_concurrent_subagents"].as_u64(), Some(4));
        assert_eq!(result["mode_max_concurrent_subagents"].as_u64(), Some(0));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_uses_stricter_env_limit_when_mode_limit_is_higher() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with loose mode max"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 10
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(omne_protocol::ThreadEventKind::TurnStarted {
                    turn_id: omne_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    directives: None,
                    priority: omne_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = omne_protocol::ToolId::new();
            parent
                .append(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["limit_source"].as_str(), Some("combined"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(result["env_max_concurrent_subagents"].as_u64(), Some(4));
        assert_eq!(result["mode_max_concurrent_subagents"].as_u64(), Some(10));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_priority_defaults_and_overrides_are_reflected_in_preview()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with explicit spawn deny override"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
    tool_overrides:
      - tool: "subagent/spawn"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let _result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "priority": "low",
                "tasks": [
                    {
                        "id": "t-default",
                        "input": "x",
                    },
                    {
                        "id": "t-high",
                        "input": "y",
                        "priority": "high",
                    }
                ],
            }),
            None,
        )
        .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        let params = events
            .into_iter()
            .find_map(|event| match event.kind {
                omne_protocol::ThreadEventKind::ToolStarted { tool, params, .. }
                    if tool == "subagent/spawn" =>
                {
                    params
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing subagent/spawn ToolStarted params"))?;

        assert_eq!(params["default_priority"].as_str(), Some("low"));
        assert_eq!(params["priority_aging_rounds"].as_u64(), Some(3));
        let tasks = params["tasks"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing task previews"))?;
        let mut by_id = std::collections::HashMap::<String, String>::new();
        for task in tasks {
            let id = task.get("id").and_then(Value::as_str).unwrap_or_default();
            let priority = task
                .get("priority")
                .and_then(Value::as_str)
                .unwrap_or_default();
            by_id.insert(id.to_string(), priority.to_string());
        }

        assert_eq!(by_id.get("t-default").map(String::as_str), Some("low"));
        assert_eq!(by_id.get("t-high").map(String::as_str), Some("high"));
        Ok(())
    }

    #[test]
    fn subagent_schedule_blocks_dependents_when_dependency_fails() {
        let parent_thread = ThreadId::new();
        let t1_thread = ThreadId::new();
        let t1_turn = TurnId::new();
        let t2_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t1".to_string(),
                title: "first".to_string(),
                input: "run first".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: t1_thread,
                log_path: "t1.log".to_string(),
                last_seq: 0,
                turn_id: Some(t1_turn),
                status: SubagentTaskStatus::Running,
                error: None,
            },
            SubagentSpawnTask {
                id: "t2".to_string(),
                title: "second".to_string(),
                input: "run second".to_string(),
                depends_on: vec!["t1".to_string()],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: t2_thread,
                log_path: "t2.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        schedule.handle_turn_completed(
            t1_thread,
            t1_turn,
            omne_protocol::TurnStatus::Failed,
            Some("boom".to_string()),
        );

        let t1 = &schedule.tasks[0];
        let t2 = &schedule.tasks[1];
        assert!(matches!(t1.status, SubagentTaskStatus::Failed));
        assert!(matches!(t2.status, SubagentTaskStatus::Failed));
        assert!(
            t2.error
                .as_deref()
                .unwrap_or("")
                .contains("blocked by dependency")
        );
        assert_eq!(
            schedule.task_statuses.get("t2").copied(),
            Some(omne_protocol::TurnStatus::Cancelled)
        );

        let snapshot = schedule.snapshot();
        assert_eq!(snapshot[0]["dependency_blocked"].as_bool(), Some(false));
        assert!(snapshot[0]["dependency_blocker_task_id"].is_null());
        assert!(snapshot[0]["dependency_blocker_status"].is_null());
        assert_eq!(snapshot[1]["dependency_blocked"].as_bool(), Some(true));
        assert_eq!(
            snapshot[1]["dependency_blocker_task_id"].as_str(),
            Some("t1")
        );
        assert_eq!(
            snapshot[1]["dependency_blocker_status"].as_str(),
            Some("Failed")
        );
    }

    #[test]
    fn subagent_schedule_snapshot_marks_non_dependency_failures_as_not_dependency_blocked() {
        let parent_thread = ThreadId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "first".to_string(),
            input: "run first".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: ThreadId::new(),
            log_path: "t1.log".to_string(),
            last_seq: 0,
            turn_id: None,
            status: SubagentTaskStatus::Failed,
            error: Some("turn finished with status=Failed".to_string()),
        }];
        let schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        let snapshot = schedule.snapshot();
        assert_eq!(snapshot[0]["dependency_blocked"].as_bool(), Some(false));
        assert!(snapshot[0]["dependency_blocker_task_id"].is_null());
        assert!(snapshot[0]["dependency_blocker_status"].is_null());
    }

    #[test]
    fn subagent_schedule_snapshot_includes_pending_approval_handles() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        let proxy_approval_id = ApprovalId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        schedule
            .approval_proxy_by_child
            .insert(child_key, proxy_approval_id);
        schedule
            .approval_proxy_targets
            .insert(proxy_approval_id, child_key);
        schedule.set_pending_approval(child_key, child_turn_id, "process/start", proxy_approval_id);

        let snapshot = schedule.snapshot();
        let pending = snapshot[0]["pending_approval"]
            .as_object()
            .expect("missing pending_approval snapshot");
        let approve_cmd = format!(
            "omne approval decide {} {} --approve",
            parent_thread_id, proxy_approval_id
        );
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let child_approval_id_text = child_approval_id.to_string();
        let proxy_approval_id_text = proxy_approval_id.to_string();

        assert_eq!(
            pending.get("action").and_then(Value::as_str),
            Some("subagent/proxy_approval")
        );
        assert_eq!(
            pending.get("approval_id").and_then(Value::as_str),
            Some(proxy_approval_id_text.as_str())
        );
        assert_eq!(
            pending.get("approve_cmd").and_then(Value::as_str),
            Some(approve_cmd.as_str())
        );
        assert_eq!(
            pending.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            pending.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert_eq!(
            pending.get("child_approval_id").and_then(Value::as_str),
            Some(child_approval_id_text.as_str())
        );
        assert_eq!(
            pending.get("child_action").and_then(Value::as_str),
            Some("process/start")
        );
        let summary = pending.get("summary").and_then(Value::as_str).unwrap_or("");
        assert!(summary.contains("child_thread_id="));
        assert!(summary.contains("child_turn_id="));
        assert!(summary.contains("child_approval_id="));
        assert!(summary.contains("child_action=process/start"));
    }

    #[test]
    fn subagent_schedule_snapshot_clears_pending_approval_after_proxy_resolution() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        let proxy_approval_id = ApprovalId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        schedule
            .approval_proxy_by_child
            .insert(child_key, proxy_approval_id);
        schedule
            .approval_proxy_targets
            .insert(proxy_approval_id, child_key);
        schedule.set_pending_approval(child_key, child_turn_id, "process/start", proxy_approval_id);

        schedule.clear_proxy_mapping(proxy_approval_id);
        assert!(schedule.pending_approvals_by_child.is_empty());
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());
        let snapshot = schedule.snapshot();
        assert!(snapshot[0]["pending_approval"].is_null());
    }

    #[test]
    fn subagent_schedule_prefers_high_priority_task_when_multiple_ready() {
        let parent_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-normal".to_string(),
                title: "normal".to_string(),
                input: "run normal".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "normal.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        let idx = schedule.pick_next_ready_task_index();
        assert_eq!(idx.map(|v| schedule.tasks[v].id.as_str()), Some("t-high"));
    }

    #[test]
    fn subagent_schedule_available_slots_saturates_with_running_and_external_threads() {
        let parent_thread = ThreadId::new();
        let running_thread = ThreadId::new();
        let running_turn = TurnId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t-running".to_string(),
            title: "running".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: running_thread,
            log_path: "running.log".to_string(),
            last_seq: 0,
            turn_id: Some(running_turn),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let external = HashSet::from([external_thread]);
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, external, 1, 3);

        // max=1 with one running task + one external reservation should saturate to zero slots.
        assert_eq!(schedule.available_slots(), 0);

        // Completing external work only releases one reservation.
        schedule.handle_turn_completed(
            external_thread,
            TurnId::new(),
            omne_protocol::TurnStatus::Completed,
            None,
        );
        assert_eq!(schedule.available_slots(), 0);

        schedule.handle_turn_completed(
            running_thread,
            running_turn,
            omne_protocol::TurnStatus::Completed,
            None,
        );
        assert_eq!(schedule.available_slots(), 1);
    }

    #[test]
    fn subagent_schedule_external_completion_releases_slot_without_mutating_pending_tasks() {
        let parent_thread = ThreadId::new();
        let pending_thread = ThreadId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t-pending".to_string(),
            title: "pending".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: pending_thread,
            log_path: "pending.log".to_string(),
            last_seq: 0,
            turn_id: None,
            status: SubagentTaskStatus::Pending,
            error: None,
        }];
        let external = HashSet::from([external_thread]);
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, external, 1, 3);

        assert_eq!(schedule.available_slots(), 0);
        schedule.handle_turn_completed(
            external_thread,
            TurnId::new(),
            omne_protocol::TurnStatus::Failed,
            Some("external-failed".to_string()),
        );

        assert_eq!(schedule.available_slots(), 1);
        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Pending
        ));
        assert!(schedule.task_statuses.is_empty());
    }

    #[test]
    fn subagent_schedule_priority_aging_can_promote_low_priority_task() {
        let parent_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        for _ in 0..6 {
            schedule.update_ready_wait_rounds();
        }
        let idx = schedule.pick_next_ready_task_index();
        assert_eq!(idx.map(|v| schedule.tasks[v].id.as_str()), Some("t-low"));
    }

    #[tokio::test]
    async fn subagent_schedule_no_slots_still_accumulates_ready_wait_rounds() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_thread = ThreadId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(
            parent_thread,
            tasks,
            HashSet::from([external_thread]),
            1,
            3,
        );

        assert_eq!(schedule.available_slots(), 0);
        schedule.start_ready_tasks(&server).await;
        assert_eq!(schedule.ready_wait_rounds.get("t-low").copied(), Some(1));
        assert_eq!(schedule.ready_wait_rounds.get("t-high").copied(), Some(1));
        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Pending
        ));
        assert!(matches!(
            schedule.tasks[1].status,
            SubagentTaskStatus::Pending
        ));

        schedule.start_ready_tasks(&server).await;
        assert_eq!(schedule.ready_wait_rounds.get("t-low").copied(), Some(2));
        assert_eq!(schedule.ready_wait_rounds.get("t-high").copied(), Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_contention_rounds_can_shift_pick_order() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_thread = ThreadId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(
            parent_thread,
            tasks,
            HashSet::from([external_thread]),
            1,
            3,
        );

        assert_eq!(schedule.available_slots(), 0);
        for _ in 0..6 {
            schedule.start_ready_tasks(&server).await;
        }
        assert_eq!(
            schedule
                .pick_next_ready_task_index()
                .map(|idx| schedule.tasks[idx].id.as_str()),
            Some("t-low")
        );

        schedule.handle_turn_completed(
            external_thread,
            TurnId::new(),
            omne_protocol::TurnStatus::Completed,
            None,
        );
        assert_eq!(schedule.available_slots(), 1);
        Ok(())
    }

    #[test]
    fn fan_out_priority_aging_rounds_parser_defaults_for_missing_or_invalid_values() {
        assert_eq!(parse_fan_out_priority_aging_rounds_value(None), 3);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("")), 3);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("abc")), 3);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("-1")), 3);
    }

    #[test]
    fn fan_out_priority_aging_rounds_parser_clamps_to_expected_bounds() {
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("0")), 1);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("1")), 1);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("7")), 7);
        assert_eq!(
            parse_fan_out_priority_aging_rounds_value(Some("10000")),
            10_000
        );
        assert_eq!(
            parse_fan_out_priority_aging_rounds_value(Some("10001")),
            10_000
        );
    }

    #[test]
    fn combine_subagent_spawn_limits_covers_all_sources() {
        let unlimited = combine_subagent_spawn_limits(0, None);
        assert_eq!(unlimited.effective, 0);
        assert_eq!(unlimited.source.as_str(), "unlimited");

        let env_only = combine_subagent_spawn_limits(4, None);
        assert_eq!(env_only.effective, 4);
        assert_eq!(env_only.source.as_str(), "env");

        let mode_only = combine_subagent_spawn_limits(0, Some(2));
        assert_eq!(mode_only.effective, 2);
        assert_eq!(mode_only.source.as_str(), "mode");

        let combined = combine_subagent_spawn_limits(4, Some(2));
        assert_eq!(combined.effective, 2);
        assert_eq!(combined.source.as_str(), "combined");
    }

    async fn wait_for_artifact_write_completion_result(
        server: &super::super::Server,
        thread_id: ThreadId,
    ) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let mut artifact_write_tool_ids = std::collections::HashSet::new();
            for event in events {
                match event.kind {
                    omne_protocol::ThreadEventKind::ToolStarted { tool_id, tool, .. } => {
                        if tool == "artifact/write" {
                            artifact_write_tool_ids.insert(tool_id);
                        }
                    }
                    omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status,
                        result,
                        ..
                    } => {
                        if !artifact_write_tool_ids.contains(&tool_id) {
                            continue;
                        }
                        if status == omne_protocol::ToolStatus::Completed {
                            return result.ok_or_else(|| {
                                anyhow::anyhow!("artifact/write ToolCompleted missing result")
                            });
                        }
                        anyhow::bail!(
                            "artifact/write finished with non-completed status={status:?}"
                        );
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for artifact/write ToolCompleted");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_hook_input_by_point(
        server: &super::super::Server,
        thread_id: ThreadId,
        hook_point: &str,
    ) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let mut matching_hook_tool_ids =
                std::collections::HashSet::<omne_protocol::ToolId>::new();
            for event in events {
                match event.kind {
                    omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        tool,
                        params,
                        ..
                    } if tool == "hook/run" => {
                        if params
                            .as_ref()
                            .and_then(|params| params.get("hook_point"))
                            .and_then(Value::as_str)
                            == Some(hook_point)
                        {
                            matching_hook_tool_ids.insert(tool_id);
                        }
                    }
                    omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Completed,
                        result: Some(result),
                        ..
                    } if matching_hook_tool_ids.contains(&tool_id) => {
                        let input_path = result
                            .get("input_path")
                            .and_then(Value::as_str)
                            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;
                        let input_bytes = tokio::fs::read(input_path).await?;
                        return serde_json::from_slice::<Value>(&input_bytes)
                            .context("parse hook input json");
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for hook input for point={hook_point}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn assert_no_artifact_write_started_for_duration(
        server: &super::super::Server,
        thread_id: ThreadId,
        duration: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + duration;
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            if events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::ToolStarted { ref tool, .. } if tool == "artifact/write"
                )
            }) {
                anyhow::bail!("unexpected artifact/write ToolStarted event");
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_linkage_marker_set(
        server: &super::super::Server,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerSet {
                        marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                        artifact_id: Some(got_artifact_id),
                        ..
                    } if got_artifact_id == artifact_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerSet(FanOutLinkageIssue) with artifact_id={artifact_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_linkage_marker_cleared(
        server: &super::super::Server,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                        marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                        turn_id: Some(got_turn_id),
                        ..
                    } if got_turn_id == turn_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerCleared(FanOutLinkageIssue) turn_id={turn_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_auto_apply_marker_set(
        server: &super::super::Server,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerSet {
                        marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                        artifact_id: Some(got_artifact_id),
                        ..
                    } if got_artifact_id == artifact_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerSet(FanOutAutoApplyError) with artifact_id={artifact_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_auto_apply_marker_cleared(
        server: &super::super::Server,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                        marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                        turn_id: Some(got_turn_id),
                        ..
                    } if got_turn_id == turn_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerCleared(FanOutAutoApplyError) turn_id={turn_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn run_git(repo_dir: &std::path::Path, args: &[&str]) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(repo_dir)
            .output()
            .await
            .with_context(|| format!("spawn git {}", args.join(" ")))?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "git {} failed (exit {:?}): stdout={}, stderr={}",
                args.join(" "),
                output.status.code(),
                stdout,
                stderr
            );
        }
        Ok(())
    }

    async fn wait_for_artifact_id_by_type(
        server: &super::super::Server,
        thread_id: ThreadId,
        artifact_type: &str,
    ) -> anyhow::Result<omne_protocol::ArtifactId> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

            let mut seen = std::collections::HashSet::<omne_protocol::ArtifactId>::new();
            for event in events {
                let omne_protocol::ThreadEventKind::ToolCompleted {
                    status: omne_protocol::ToolStatus::Completed,
                    result: Some(result),
                    ..
                } = event.kind
                else {
                    continue;
                };
                let Some(raw_id) = result.get("artifact_id").and_then(Value::as_str) else {
                    continue;
                };
                let Ok(artifact_id) = raw_id.parse::<omne_protocol::ArtifactId>() else {
                    continue;
                };
                if !seen.insert(artifact_id) {
                    continue;
                }
                let read = crate::handle_artifact_read(
                    server,
                    crate::ArtifactReadParams {
                        thread_id,
                        turn_id: None,
                        approval_id: None,
                        artifact_id,
                        version: None,
                        max_bytes: None,
                    },
                )
                .await;
                let Ok(read) = read else {
                    continue;
                };
                let Ok(read) =
                    serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read)
                else {
                    continue;
                };
                if read.metadata.artifact_type == artifact_type {
                    return Ok(read.metadata.artifact_id);
                }
            }

            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for artifact_type={artifact_type}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_fan_in_summary_with_task_result_artifact_id(
        server: &super::super::Server,
        thread_id: ThreadId,
        task_id: &str,
    ) -> anyhow::Result<omne_app_server_protocol::ArtifactReadResponse> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

            let mut seen = std::collections::HashSet::<omne_protocol::ArtifactId>::new();
            for event in events {
                let omne_protocol::ThreadEventKind::ToolCompleted {
                    status: omne_protocol::ToolStatus::Completed,
                    result: Some(result),
                    ..
                } = event.kind
                else {
                    continue;
                };
                let Some(raw_id) = result.get("artifact_id").and_then(Value::as_str) else {
                    continue;
                };
                let Ok(artifact_id) = raw_id.parse::<omne_protocol::ArtifactId>() else {
                    continue;
                };
                if !seen.insert(artifact_id) {
                    continue;
                }
                let read = crate::handle_artifact_read(
                    server,
                    crate::ArtifactReadParams {
                        thread_id,
                        turn_id: None,
                        approval_id: None,
                        artifact_id,
                        version: None,
                        max_bytes: None,
                    },
                )
                .await;
                let Ok(read) = read else {
                    continue;
                };
                let Ok(read) =
                    serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read)
                else {
                    continue;
                };
                if read.metadata.artifact_type != "fan_in_summary" {
                    continue;
                }
                let has_result_artifact_id = read
                    .fan_in_summary
                    .as_ref()
                    .map(|payload| {
                        payload.tasks.iter().any(|task| {
                            task.task_id == task_id && task.result_artifact_id.is_some()
                        })
                    })
                    .unwrap_or(false);
                if has_result_artifact_id {
                    return Ok(read);
                }
            }

            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for fan_in_summary with task result_artifact_id task_id={task_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn parse_fan_in_summary_structured_data_from_text(
        text: &str,
    ) -> anyhow::Result<omne_workflow_spec::FanInSummaryStructuredData> {
        let marker = "## Structured Data";
        let marker_idx = text
            .find(marker)
            .ok_or_else(|| anyhow::anyhow!("missing Structured Data section"))?;
        let after_marker = &text[(marker_idx + marker.len())..];
        let fence_start_rel = after_marker
            .find("```json")
            .ok_or_else(|| anyhow::anyhow!("missing Structured Data json fence start"))?;
        let json_start = marker_idx + marker.len() + fence_start_rel + "```json".len();
        let remainder = &text[json_start..];
        let fence_end_rel = remainder
            .find("```")
            .ok_or_else(|| anyhow::anyhow!("missing Structured Data json fence end"))?;
        let json_block = remainder[..fence_end_rel].trim();
        serde_json::from_str::<omne_workflow_spec::FanInSummaryStructuredData>(json_block)
            .context("parse fan_in_summary structured data json")
    }

    #[tokio::test]
    async fn fan_out_result_writer_writes_artifact_with_expected_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t1".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("boom".to_string()),
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        let artifact_id_raw = completion
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("artifact/write completion missing artifact_id"))?;
        let artifact_id = artifact_id_raw
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse artifact_id {artifact_id_raw}: {err}"))?;

        let read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read).context("parse artifact/read response")?;

        assert_eq!(read.metadata.artifact_type, "fan_out_result");
        assert_eq!(read.metadata.summary, "fan-out result");
        assert!(read.text.contains("\"task_id\": \"t1\""));
        assert!(read.text.contains("\"workspace_mode\": \"read_only\""));
        assert!(read.text.contains("\"workspace_cwd\": null"));
        assert!(read.text.contains("\"isolated_write_patch\": null"));
        assert!(read.text.contains("\"isolated_write_handoff\": null"));
        assert!(read.text.contains("\"status\": \"failed\""));
        assert!(read.text.contains("\"reason\": \"boom\""));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_includes_isolated_write_handoff_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let workspace_cwd = "/tmp/isolated/subagent/repo".to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-isolated".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd.clone()),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        let artifact_id_raw = completion
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("artifact/write completion missing artifact_id"))?;
        let artifact_id = artifact_id_raw
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse artifact_id {artifact_id_raw}: {err}"))?;

        let read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read).context("parse artifact/read response")?;

        assert!(read.text.contains("\"workspace_mode\": \"isolated_write\""));
        assert!(
            read.text
                .contains(&format!("\"workspace_cwd\": \"{workspace_cwd}\""))
        );
        assert!(read.text.contains("\"isolated_write_patch\""));
        assert!(read.text.contains("\"isolated_write_handoff\""));
        assert!(read.text.contains("\"status_argv\""));
        assert!(read.text.contains("\"diff_argv\""));
        assert!(read.text.contains("\"apply_patch_hint\""));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_writes_patch_artifact_for_isolated_workspace()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        run_git(&repo_dir, &["init"]).await?;
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(&repo_dir, &["config", "user.name", "Test User"]).await?;
        let file_path = repo_dir.join("hello.txt");
        tokio::fs::write(&file_path, "hello\n").await?;
        run_git(&repo_dir, &["add", "hello.txt"]).await?;
        run_git(&repo_dir, &["commit", "-m", "init"]).await?;
        tokio::fs::write(&file_path, "hello\nworld\n").await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let workspace_cwd = repo_dir.display().to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-isolated-patch".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let patch_artifact_id = wait_for_artifact_id_by_type(&server, thread_id, "patch").await?;
        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;

        let read_patch = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: patch_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_patch: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_patch).context("parse patch artifact/read response")?;
        assert_eq!(read_patch.metadata.artifact_type, "patch");
        assert!(
            read_patch
                .text
                .contains("diff --git a/hello.txt b/hello.txt")
        );
        assert!(read_patch.text.contains("+world"));

        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        assert_eq!(read_result.metadata.artifact_type, "fan_out_result");
        assert!(read_result.text.contains("\"isolated_write_patch\""));
        assert!(read_result.text.contains("\"artifact_type\": \"patch\""));
        assert!(
            read_result
                .text
                .contains("\"read_cmd\": \"omne artifact read")
        );
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_auto_applies_patch_to_parent_workspace_when_enabled()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;

        run_git(&parent_repo_dir, &["init"]).await?;
        run_git(
            &parent_repo_dir,
            &["config", "user.email", "test@example.com"],
        )
        .await?;
        run_git(&parent_repo_dir, &["config", "user.name", "Test User"]).await?;
        let parent_file_path = parent_repo_dir.join("hello.txt");
        tokio::fs::write(&parent_file_path, "hello\n").await?;
        run_git(&parent_repo_dir, &["add", "hello.txt"]).await?;
        run_git(&parent_repo_dir, &["commit", "-m", "init"]).await?;

        let isolated_repo_dir = tmp.path().join("isolated-repo");
        let parent_repo_dir_text = parent_repo_dir.display().to_string();
        let isolated_repo_dir_text = isolated_repo_dir.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_dir_text.as_str(),
                isolated_repo_dir_text.as_str(),
            ],
        )
        .await?;
        let isolated_file_path = isolated_repo_dir.join("hello.txt");
        tokio::fs::write(&isolated_file_path, "hello\nworld\n").await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server
            .thread_store
            .create_thread(isolated_repo_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer_with_target_workspace(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-auto-apply".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(isolated_repo_dir.display().to_string()),
            Some(parent_repo_dir.display().to_string()),
            true,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        wait_for_fan_out_auto_apply_marker_cleared(&server, thread_id, turn_id).await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        assert!(read_result.text.contains("\"isolated_write_auto_apply\""));
        assert!(read_result.text.contains("\"applied\": true"));

        let parent_file = tokio::fs::read_to_string(&parent_file_path).await?;
        assert!(parent_file.contains("world"));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_auto_applies_untracked_file_to_parent_workspace_when_enabled()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;

        run_git(&parent_repo_dir, &["init"]).await?;
        run_git(
            &parent_repo_dir,
            &["config", "user.email", "test@example.com"],
        )
        .await?;
        run_git(&parent_repo_dir, &["config", "user.name", "Test User"]).await?;
        let parent_file_path = parent_repo_dir.join("hello.txt");
        tokio::fs::write(&parent_file_path, "hello\n").await?;
        run_git(&parent_repo_dir, &["add", "hello.txt"]).await?;
        run_git(&parent_repo_dir, &["commit", "-m", "init"]).await?;

        let isolated_repo_dir = tmp.path().join("isolated-repo");
        let parent_repo_dir_text = parent_repo_dir.display().to_string();
        let isolated_repo_dir_text = isolated_repo_dir.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_dir_text.as_str(),
                isolated_repo_dir_text.as_str(),
            ],
        )
        .await?;
        let new_file_path = isolated_repo_dir.join("new_file.txt");
        tokio::fs::write(&new_file_path, "brand new\n").await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server
            .thread_store
            .create_thread(isolated_repo_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer_with_target_workspace(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-auto-apply-untracked".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(isolated_repo_dir.display().to_string()),
            Some(parent_repo_dir.display().to_string()),
            true,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        wait_for_fan_out_auto_apply_marker_cleared(&server, thread_id, turn_id).await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let auto_apply = payload.isolated_write_auto_apply.ok_or_else(|| {
            anyhow::anyhow!("missing isolated_write_auto_apply structured payload")
        })?;
        assert!(auto_apply.applied);

        let parent_new_file =
            tokio::fs::read_to_string(parent_repo_dir.join("new_file.txt")).await?;
        assert_eq!(parent_new_file, "brand new\n");
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_auto_apply_records_check_patch_failure_stage()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;

        run_git(&parent_repo_dir, &["init"]).await?;
        run_git(
            &parent_repo_dir,
            &["config", "user.email", "test@example.com"],
        )
        .await?;
        run_git(&parent_repo_dir, &["config", "user.name", "Test User"]).await?;
        let parent_file_path = parent_repo_dir.join("hello.txt");
        tokio::fs::write(&parent_file_path, "hello\nbase\n").await?;
        run_git(&parent_repo_dir, &["add", "hello.txt"]).await?;
        run_git(&parent_repo_dir, &["commit", "-m", "init"]).await?;

        let isolated_repo_dir = tmp.path().join("isolated-repo");
        let parent_repo_dir_text = parent_repo_dir.display().to_string();
        let isolated_repo_dir_text = isolated_repo_dir.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_dir_text.as_str(),
                isolated_repo_dir_text.as_str(),
            ],
        )
        .await?;

        let isolated_file_path = isolated_repo_dir.join("hello.txt");
        tokio::fs::write(&isolated_file_path, "hello\nchild-change\n").await?;
        tokio::fs::write(&parent_file_path, "hello\nparent-change\n").await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server
            .thread_store
            .create_thread(isolated_repo_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer_with_target_workspace(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-auto-apply-conflict".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(isolated_repo_dir.display().to_string()),
            Some(parent_repo_dir.display().to_string()),
            true,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        wait_for_fan_out_auto_apply_marker_set(&server, thread_id, result_artifact_id).await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let auto_apply = payload.isolated_write_auto_apply.ok_or_else(|| {
            anyhow::anyhow!("missing isolated_write_auto_apply structured payload")
        })?;
        assert!(auto_apply.enabled);
        assert!(auto_apply.attempted);
        assert!(!auto_apply.applied);
        assert_eq!(
            auto_apply.failure_stage,
            Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
            )
        );
        assert!(
            auto_apply
                .recovery_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("resolve apply-check conflicts"))
        );
        assert!(
            auto_apply
                .error
                .as_deref()
                .is_some_and(|err| err.contains("git apply --check failed"))
        );
        assert!(
            auto_apply
                .patch_artifact_id
                .as_deref()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(
            auto_apply
                .patch_read_cmd
                .as_deref()
                .is_some_and(|cmd| cmd.contains("omne artifact read"))
        );
        assert!(!auto_apply.recovery_commands.is_empty());
        assert_eq!(auto_apply.recovery_commands[0].label, "read_patch_artifact");
        assert!(
            auto_apply
                .recovery_commands
                .iter()
                .any(|cmd| cmd.label == "apply_with_patch_stdin")
        );

        let parent_file = tokio::fs::read_to_string(&parent_file_path).await?;
        assert_eq!(parent_file, "hello\nparent-change\n");
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_skips_patch_artifact_when_workspace_is_clean()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        run_git(&repo_dir, &["init"]).await?;
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(&repo_dir, &["config", "user.name", "Test User"]).await?;
        tokio::fs::write(repo_dir.join("hello.txt"), "hello\n").await?;
        run_git(&repo_dir, &["add", "hello.txt"]).await?;
        run_git(&repo_dir, &["commit", "-m", "init"]).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let workspace_cwd = repo_dir.display().to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-clean".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        assert!(payload.isolated_write_patch.is_none());

        let list = crate::handle_artifact_list(
            &server,
            crate::ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let list: omne_app_server_protocol::ArtifactListResponse =
            serde_json::from_value(list).context("parse artifact/list response")?;
        assert!(
            list.artifacts
                .iter()
                .all(|meta| meta.artifact_type.as_str() != "patch")
        );
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_records_patch_error_for_invalid_workspace() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let invalid_workspace = tmp.path().join("missing-workspace");
        let workspace_cwd = invalid_workspace.display().to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-missing".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd.clone()),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let patch = payload
            .isolated_write_patch
            .ok_or_else(|| anyhow::anyhow!("missing isolated_write_patch payload"))?;
        assert_eq!(patch.workspace_cwd.as_deref(), Some(workspace_cwd.as_str()));
        assert!(
            patch
                .error
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(patch.artifact_id.is_none());
        assert!(patch.read_cmd.is_none());
        let handoff_patch_error = payload
            .isolated_write_handoff
            .as_ref()
            .and_then(|handoff| handoff.patch.as_ref())
            .and_then(|patch| patch.error.as_deref());
        assert!(handoff_patch_error.is_some_and(|value| !value.is_empty()));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_marks_patch_as_truncated_for_large_diff() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        run_git(&repo_dir, &["init"]).await?;
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(&repo_dir, &["config", "user.name", "Test User"]).await?;
        let file_path = repo_dir.join("large.txt");
        // Keep this deterministic without env overrides by ensuring diff > 64MiB clamp.
        let original = format!("{}\n", "a".repeat(256)).repeat(170_000);
        let modified = format!("{}\n", "b".repeat(256)).repeat(170_000);
        tokio::fs::write(&file_path, original).await?;
        run_git(&repo_dir, &["add", "large.txt"]).await?;
        run_git(&repo_dir, &["commit", "-m", "init"]).await?;
        tokio::fs::write(&file_path, modified).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-large-diff".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(repo_dir.display().to_string()),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let patch_artifact_id = wait_for_artifact_id_by_type(&server, thread_id, "patch").await?;
        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;

        let read_patch = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: patch_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_patch: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_patch).context("parse patch artifact/read response")?;
        let full_patch = tokio::fs::read_to_string(&read_patch.metadata.content_path)
            .await
            .with_context(|| format!("read {}", read_patch.metadata.content_path))?;
        assert!(full_patch.contains("# <...truncated...>"));

        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let patch = payload
            .isolated_write_patch
            .ok_or_else(|| anyhow::anyhow!("missing isolated_write_patch payload"))?;
        let patch_artifact_id_text = patch_artifact_id.to_string();
        assert_eq!(patch.artifact_type.as_deref(), Some("patch"));
        assert_eq!(
            patch.artifact_id.as_deref(),
            Some(patch_artifact_id_text.as_str())
        );
        assert_eq!(patch.truncated, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_ignores_non_matching_completion_events() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-ignore".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        // Mode/approval setup for the eventual matching write path.
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let other_thread_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id: ThreadId::new(),
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": other_thread_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);
        assert_no_artifact_write_started_for_duration(
            &server,
            thread_id,
            Duration::from_millis(150),
        )
        .await?;

        let other_turn_event = omne_protocol::ThreadEvent {
            seq: EventSeq(2),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: TurnId::new(),
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": other_turn_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);
        assert_no_artifact_write_started_for_duration(
            &server,
            thread_id,
            Duration::from_millis(150),
        )
        .await?;

        let matching_event = omne_protocol::ThreadEvent {
            seq: EventSeq(3),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": matching_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        assert_eq!(completion["created"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_skips_artifact_write_when_type_is_invalid() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-invalid".to_string(),
            String::new(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        assert_no_artifact_write_started_for_duration(
            &server,
            thread_id,
            Duration::from_millis(300),
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_sets_linkage_attention_marker_for_linkage_issue_type()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-linkage".to_string(),
            "fan_out_linkage_issue".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("fan-out linkage issue".to_string()),
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        let artifact_id_raw = completion
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("artifact/write completion missing artifact_id"))?;
        let artifact_id = artifact_id_raw
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse artifact_id {artifact_id_raw}: {err}"))?;

        wait_for_fan_out_linkage_marker_set(&server, thread_id, artifact_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_clears_linkage_attention_marker_for_clear_type()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-linkage-clear".to_string(),
            "fan_out_linkage_issue_clear".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let _ = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        wait_for_fan_out_linkage_marker_cleared(&server, thread_id, turn_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_bridges_child_approval_to_parent_thread() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let schedule = SubagentSpawnSchedule::new(
            parent_thread_id,
            vec![SubagentSpawnTask {
                id: "t1".to_string(),
                title: "child task".to_string(),
                input: "run".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: child_thread_id,
                log_path: "child.log".to_string(),
                last_seq: 0,
                turn_id: Some(child_turn_id),
                status: SubagentTaskStatus::Running,
                error: None,
            }],
            HashSet::new(),
            4,
            3,
        );
        spawn_subagent_scheduler(server.clone(), schedule);

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;

        let decision = crate::handle_approval_decide(
            &server,
            crate::ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved in parent".to_string()),
            },
        )
        .await?;
        let decision: omne_app_server_protocol::ApprovalDecideResponse =
            serde_json::from_value(decision).context("parse approval/decide test response")?;
        assert!(decision.forwarded);
        assert_eq!(decision.child_thread_id, Some(child_thread_id));
        assert_eq!(decision.child_approval_id, Some(child_approval_id));

        wait_for_approval_decided(&server, child_thread_id, child_approval_id).await?;
        wait_for_approval_decided(&server, parent_thread_id, proxy_approval_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catches_up_completed_turn_before_scheduler_start()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.set_env_max_concurrent_subagents(9);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Completed
        ));
        assert_eq!(
            schedule.task_statuses.get("t1").copied(),
            Some(omne_protocol::TurnStatus::Completed)
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_start_ready_tasks_triggers_subagent_start_hook() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  subagent_start:
    - argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: None,
            status: SubagentTaskStatus::Pending,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                schedule.start_ready_tasks(&server).await;
            })
            .await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Running
        ));
        let child_turn_id = schedule.tasks[0]
            .turn_id
            .ok_or_else(|| anyhow::anyhow!("expected child turn_id after schedule start"))?;
        let parent_thread_id_text = parent_thread_id.to_string();
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();

        let hook_input =
            wait_for_hook_input_by_point(&server, parent_thread_id, "subagent_start").await?;
        assert_eq!(
            hook_input.get("thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        let subagent = hook_input
            .get("subagent")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing subagent context"))?;
        assert_eq!(subagent.get("task_id").and_then(Value::as_str), Some("t1"));
        assert_eq!(
            subagent.get("parent_thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert!(subagent.get("status").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_running_events_triggers_subagent_stop_hook()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  subagent_stop:
    - when_turn_status: ["failed"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("Bearer super-secret-token-abcdefghijklmnopqrstuvwxyz".to_string()),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.set_env_max_concurrent_subagents(9);
        schedule.catch_up_running_events(&server).await;
        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Failed
        ));

        let parent_thread_id_text = parent_thread_id.to_string();
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let hook_input =
            wait_for_hook_input_by_point(&server, parent_thread_id, "subagent_stop").await?;
        assert_eq!(
            hook_input.get("thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            hook_input.get("turn_status").and_then(Value::as_str),
            Some("failed")
        );
        let subagent = hook_input
            .get("subagent")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing subagent context"))?;
        assert_eq!(subagent.get("task_id").and_then(Value::as_str), Some("t1"));
        assert_eq!(
            subagent.get("parent_thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert_eq!(
            subagent.get("status").and_then(Value::as_str),
            Some("failed")
        );
        let reason = subagent
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing subagent stop reason"))?;
        assert!(reason.contains("Bearer <REDACTED>"));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_running_events_writes_fan_in_summary_artifact()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("child failed".to_string()),
            })
            .await?;
        let patch_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: patch_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "patch",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: patch_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": omne_protocol::ArtifactId::new().to_string(),
                })),
            })
            .await?;
        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        let fan_out_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.set_env_max_concurrent_subagents(9);
        schedule.catch_up_running_events(&server).await;

        let artifact_id =
            wait_for_artifact_id_by_type(&server, parent_thread_id, "fan_in_summary").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read_result)
                .context("parse fan_in_summary artifact/read response")?;
        assert_eq!(read_result.metadata.artifact_type, "fan_in_summary");

        let payload = read_result
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary structured payload"))?;
        assert_eq!(
            payload.schema_version,
            omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1
        );
        assert_eq!(payload.thread_id, parent_thread_id.to_string());
        assert_eq!(payload.task_count, 1);
        assert_eq!(payload.scheduling.env_max_concurrent_subagents, 9);
        assert_eq!(payload.scheduling.effective_concurrency_limit, 4);
        assert_eq!(payload.scheduling.priority_aging_rounds, 3);

        let task = payload
            .tasks
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary task"))?;
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(task.task_id, "t1");
        assert_eq!(task.status, "Failed");
        assert_eq!(task.reason.as_deref(), Some("child failed"));
        assert_eq!(
            task.thread_id.as_deref(),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(task.turn_id.as_deref(), Some(child_turn_id_text.as_str()));
        assert!(!task.dependency_blocked);
        assert_eq!(
            task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(task.result_artifact_error.is_none());
        assert!(task.result_artifact_error_id.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_running_events_records_result_error_artifact_id()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("child failed".to_string()),
            })
            .await?;
        let fan_out_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("artifact/write denied: approval required".to_string()),
                result: None,
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        let artifact_id =
            wait_for_artifact_id_by_type(&server, parent_thread_id, "fan_in_summary").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read_result)
                .context("parse fan_in_summary artifact/read response")?;
        let payload = read_result
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary structured payload"))?;
        let task = payload
            .tasks
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary task"))?;
        assert!(task.result_artifact_id.is_none());
        let result_error = task
            .result_artifact_error
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_error"))?;
        assert!(result_error.contains("artifact/write denied: approval required"));
        let result_error_id = task
            .result_artifact_error_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_error_id"))?;
        let result_error_artifact_id = result_error_id
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse result_error_artifact_id: {err}"))?;

        let error_read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_error_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let error_read =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(error_read)
                .context("parse fan_out_result_error artifact/read response")?;
        assert_eq!(error_read.metadata.artifact_type, "fan_out_result_error");
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_result_artifact_scan_state_handles_incremental_tool_events()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        let fan_out_tool_id = omne_protocol::ToolId::new();
        let tool_started_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let first = schedule.fan_in_summary_structured_data(&server).await;
        let first_task = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in first fan_in_summary"))?;
        assert!(first_task.result_artifact_id.is_none());
        let first_scan_state = schedule
            .result_artifact_scan_state_by_task
            .get("t1")
            .ok_or_else(|| anyhow::anyhow!("missing scan state after first summary"))?;
        assert_eq!(first_scan_state.matching_tool_ids.len(), 1);
        assert!(first_scan_state.matching_tool_ids.contains(&fan_out_tool_id));
        assert!(first_scan_state.last_scanned_seq >= tool_started_event.seq.0);
        let first_scanned_seq = first_scan_state.last_scanned_seq;

        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        let tool_completed_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let second = schedule.fan_in_summary_structured_data(&server).await;
        let second_task = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in second fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            second_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        let second_scan_state = schedule
            .result_artifact_scan_state_by_task
            .get("t1")
            .ok_or_else(|| anyhow::anyhow!("missing scan state after second summary"))?;
        assert!(second_scan_state.matching_tool_ids.is_empty());
        assert_eq!(
            second_scan_state.summary.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(second_scan_state.last_scanned_seq >= tool_completed_event.seq.0);
        assert!(second_scan_state.last_scanned_seq >= first_scanned_seq);
        let second_scanned_seq = second_scan_state.last_scanned_seq;

        let third = schedule.fan_in_summary_structured_data(&server).await;
        let third_task = third
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in third fan_in_summary"))?;
        assert_eq!(
            third_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        let third_scan_state = schedule
            .result_artifact_scan_state_by_task
            .get("t1")
            .ok_or_else(|| anyhow::anyhow!("missing scan state after third summary"))?;
        assert_eq!(third_scan_state.last_scanned_seq, second_scanned_seq);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_result_artifact_success_clears_error_artifact_tracking()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let first_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: first_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: first_tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("artifact/write denied: approval required".to_string()),
                result: None,
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let first = schedule.fan_in_summary_structured_data(&server).await;
        let first_task = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in first fan_in_summary"))?;
        assert!(first_task.result_artifact_id.is_none());
        let first_error_id = first_task
            .result_artifact_error_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_error_id after failed write"))?;
        assert!(
            schedule
                .result_artifact_error_ids_by_task
                .contains_key("t1")
        );

        let second_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: second_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: second_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let second = schedule.fan_in_summary_structured_data(&server).await;
        let second_task = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in second fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            second_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(second_task.result_artifact_error.is_none());
        assert!(second_task.result_artifact_error_id.is_none());
        assert!(!schedule
            .result_artifact_error_ids_by_task
            .contains_key("t1"));

        let first_error_artifact_id = first_error_id
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse first result_artifact_error_id: {err}"))?;
        let first_error_read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: first_error_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let first_error_read =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(
                first_error_read,
            )
            .context("parse first fan_out_result_error artifact/read response")?;
        assert_eq!(first_error_read.metadata.artifact_type, "fan_out_result_error");
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_result_artifact_scan_state_isolated_per_task_under_interleaved_events()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child1_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let child1_thread_id = child1_handle.thread_id();
        let child1_turn_id = TurnId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child1_turn_id,
                input: "child1 task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let mut child2_handle = server.thread_store.create_thread(repo_dir).await?;
        let child2_thread_id = child2_handle.thread_id();
        let child2_turn_id = TurnId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child2_turn_id,
                input: "child2 task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let tasks = vec![
            SubagentSpawnTask {
                id: "t1".to_string(),
                title: "child1 task".to_string(),
                input: "run child1".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: child1_thread_id,
                log_path: "child1.log".to_string(),
                last_seq: 0,
                turn_id: Some(child1_turn_id),
                status: SubagentTaskStatus::Completed,
                error: None,
            },
            SubagentSpawnTask {
                id: "t2".to_string(),
                title: "child2 task".to_string(),
                input: "run child2".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: child2_thread_id,
                log_path: "child2.log".to_string(),
                last_seq: 0,
                turn_id: Some(child2_turn_id),
                status: SubagentTaskStatus::Completed,
                error: None,
            },
        ];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let child1_wave1_tool_id = omne_protocol::ToolId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child1_wave1_tool_id,
                turn_id: Some(child1_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let child2_wave1_tool_id = omne_protocol::ToolId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child2_wave1_tool_id,
                turn_id: Some(child2_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;

        let child1_wave1_artifact_id = omne_protocol::ArtifactId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child1_wave1_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child1_wave1_artifact_id.to_string(),
                })),
            })
            .await?;
        let child2_wave1_artifact_id = omne_protocol::ArtifactId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child2_wave1_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child2_wave1_artifact_id.to_string(),
                })),
            })
            .await?;

        let first = schedule.fan_in_summary_structured_data(&server).await;
        let first_t1 = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in first fan_in_summary"))?;
        let first_t2 = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t2")
            .ok_or_else(|| anyhow::anyhow!("missing t2 in first fan_in_summary"))?;
        let child1_wave1_artifact_id_text = child1_wave1_artifact_id.to_string();
        let child2_wave1_artifact_id_text = child2_wave1_artifact_id.to_string();
        assert_eq!(
            first_t1.result_artifact_id.as_deref(),
            Some(child1_wave1_artifact_id_text.as_str())
        );
        assert_eq!(
            first_t2.result_artifact_id.as_deref(),
            Some(child2_wave1_artifact_id_text.as_str())
        );
        let first_t1_diag = first_t1
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t1 diagnostics after first wave"))?;
        let first_t2_diag = first_t2
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t2 diagnostics after first wave"))?;
        assert_eq!(first_t1_diag.matched_completion_count, 1);
        assert_eq!(first_t2_diag.matched_completion_count, 1);
        assert_eq!(first_t1_diag.pending_matching_tool_ids, 0);
        assert_eq!(first_t2_diag.pending_matching_tool_ids, 0);
        let first_t1_scan_last_seq = first_t1_diag.scan_last_seq;
        let first_t2_scan_last_seq = first_t2_diag.scan_last_seq;

        let child2_wave2_tool_id = omne_protocol::ToolId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child2_wave2_tool_id,
                turn_id: Some(child2_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let child1_wave2_tool_id = omne_protocol::ToolId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child1_wave2_tool_id,
                turn_id: Some(child1_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;

        let child2_wave2_artifact_id = omne_protocol::ArtifactId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child2_wave2_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child2_wave2_artifact_id.to_string(),
                })),
            })
            .await?;
        let child1_wave2_artifact_id = omne_protocol::ArtifactId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child1_wave2_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child1_wave2_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child1_handle);
        drop(child2_handle);

        let second = schedule.fan_in_summary_structured_data(&server).await;
        let second_t1 = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in second fan_in_summary"))?;
        let second_t2 = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t2")
            .ok_or_else(|| anyhow::anyhow!("missing t2 in second fan_in_summary"))?;
        let child1_wave2_artifact_id_text = child1_wave2_artifact_id.to_string();
        let child2_wave2_artifact_id_text = child2_wave2_artifact_id.to_string();
        assert_eq!(
            second_t1.result_artifact_id.as_deref(),
            Some(child1_wave2_artifact_id_text.as_str())
        );
        assert_eq!(
            second_t2.result_artifact_id.as_deref(),
            Some(child2_wave2_artifact_id_text.as_str())
        );
        let second_t1_diag = second_t1
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t1 diagnostics after second wave"))?;
        let second_t2_diag = second_t2
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t2 diagnostics after second wave"))?;
        assert_eq!(second_t1_diag.matched_completion_count, 2);
        assert_eq!(second_t2_diag.matched_completion_count, 2);
        assert_eq!(second_t1_diag.pending_matching_tool_ids, 0);
        assert_eq!(second_t2_diag.pending_matching_tool_ids, 0);
        assert!(second_t1_diag.scan_last_seq >= first_t1_scan_last_seq);
        assert!(second_t2_diag.scan_last_seq >= first_t2_scan_last_seq);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_fan_in_artifact_read_clears_result_error_fields_after_success()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let first_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: first_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: first_tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("artifact/write denied: approval required".to_string()),
                result: None,
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.write_fan_in_summary_artifact_best_effort(&server).await;
        let first_fan_in_artifact_id =
            wait_for_artifact_id_by_type(&server, parent_thread_id, "fan_in_summary").await?;
        let first_read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: first_fan_in_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let first_read =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(first_read)
                .context("parse first fan_in_summary artifact/read response")?;
        let first_payload = first_read
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing first fan_in_summary payload"))?;
        let first_task = first_payload
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in first fan_in_summary"))?;
        assert!(first_task.result_artifact_id.is_none());
        assert!(first_task.result_artifact_error.is_some());
        assert!(first_task.result_artifact_error_id.is_some());
        let first_structured =
            parse_fan_in_summary_structured_data_from_text(first_read.text.as_str())?;
        let first_structured_task = first_structured
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in first structured fan_in_summary"))?;
        let first_diagnostics = first_structured_task
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_diagnostics in first summary"))?;
        assert_eq!(first_diagnostics.matched_completion_count, 1);
        assert_eq!(first_diagnostics.pending_matching_tool_ids, 0);
        assert!(first_diagnostics.scan_last_seq > 0);

        let second_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: second_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: second_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        schedule.write_fan_in_summary_artifact_best_effort(&server).await;
        let second_read =
            wait_for_fan_in_summary_with_task_result_artifact_id(&server, parent_thread_id, "t1")
                .await?;
        let second_payload = second_read
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing second fan_in_summary payload"))?;
        let second_task = second_payload
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in second fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            second_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(second_task.result_artifact_error.is_none());
        assert!(second_task.result_artifact_error_id.is_none());
        let second_structured =
            parse_fan_in_summary_structured_data_from_text(second_read.text.as_str())?;
        let second_structured_task = second_structured
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in second structured fan_in_summary"))?;
        let second_diagnostics = second_structured_task
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_diagnostics in second summary"))?;
        assert_eq!(second_diagnostics.matched_completion_count, 2);
        assert_eq!(second_diagnostics.pending_matching_tool_ids, 0);
        assert!(second_diagnostics.scan_last_seq >= first_diagnostics.scan_last_seq);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_scheduler_settles_late_result_artifact_notifications_before_exit()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        let mut notify_rx = server.notify_tx.subscribe();

        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        let fan_out_tool_id = omne_protocol::ToolId::new();
        let tool_started_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let tool_completed_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let tool_started_line = serde_json::json!({
            "method": "item/started",
            "params": tool_started_event,
        })
        .to_string();
        let _ = server.notify_tx.send(tool_started_line);
        let tool_completed_line = serde_json::json!({
            "method": "item/completed",
            "params": tool_completed_event,
        })
        .to_string();
        let _ = server.notify_tx.send(tool_completed_line);
        schedule
            .settle_late_result_artifacts_before_exit(&server, &mut notify_rx)
            .await;

        let read =
            wait_for_fan_in_summary_with_task_result_artifact_id(&server, parent_thread_id, "t1")
                .await?;
        let payload = read
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary payload"))?;
        let task = payload
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_ignores_late_approval_after_completion_in_batch()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: ApprovalId::new(),
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Completed
        ));
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        assert!(!parent_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalRequested { action, .. }
                    if action == "subagent/proxy_approval"
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_advances_last_seq_for_running_task() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Running
        ));
        assert!(schedule.tasks[0].last_seq >= 2);
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_is_idempotent_for_approval_forwarding() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        let first_last_seq = schedule.tasks[0].last_seq;
        let first_proxy_count = count_proxy_approval_requests(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;
        assert_eq!(first_proxy_count, 1);

        schedule.catch_up_running_events(&server).await;
        let second_last_seq = schedule.tasks[0].last_seq;
        let second_proxy_count = count_proxy_approval_requests(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;
        assert_eq!(second_proxy_count, 1);
        assert_eq!(second_last_seq, first_last_seq);
        assert_eq!(schedule.approval_proxy_by_child.len(), 1);
        assert_eq!(schedule.approval_proxy_targets.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_auto_denies_stale_parent_proxy_approval_on_turn_completion()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("child failed".to_string()),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Failed
        ));
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;
        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let decided = parent_events
            .iter()
            .filter_map(|event| {
                let omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    reason,
                    ..
                } = &event.kind
                else {
                    return None;
                };
                if *approval_id != proxy_approval_id {
                    return None;
                }
                Some((*decision, reason.clone()))
            })
            .collect::<Vec<_>>();
        assert_eq!(decided.len(), 1);
        assert_eq!(decided[0].0, omne_protocol::ApprovalDecision::Denied);
        assert!(
            decided[0]
                .1
                .as_deref()
                .unwrap_or_default()
                .starts_with(crate::SUBAGENT_PROXY_AUTO_DENIED_REASON_PREFIX)
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_does_not_override_existing_parent_proxy_decision()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;

        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            })
            .await?;
        drop(child_handle);

        // Simulate parent decision already persisted before scheduler replays completion.
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved from parent".to_string()),
            })
            .await?;

        schedule.catch_up_running_events(&server).await;

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let decided = parent_events
            .iter()
            .filter_map(|event| {
                let omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    ..
                } = &event.kind
                else {
                    return None;
                };
                if *approval_id != proxy_approval_id {
                    return None;
                }
                Some(*decision)
            })
            .collect::<Vec<_>>();
        assert_eq!(decided, vec![omne_protocol::ApprovalDecision::Approved]);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_auto_deny_stale_proxy_is_idempotent_for_duplicate_completion_handling()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;

        let stale_proxy_approval_ids = schedule.handle_turn_completed(
            child_thread_id,
            child_turn_id,
            omne_protocol::TurnStatus::Failed,
            Some("boom".to_string()),
        );
        schedule
            .auto_deny_stale_parent_proxy_approvals(
                &server,
                child_thread_id,
                child_turn_id,
                omne_protocol::TurnStatus::Failed,
                stale_proxy_approval_ids,
            )
            .await;

        // Duplicate completion handling should be a no-op for proxy auto-deny.
        let stale_proxy_approval_ids = schedule.handle_turn_completed(
            child_thread_id,
            child_turn_id,
            omne_protocol::TurnStatus::Failed,
            Some("boom".to_string()),
        );
        schedule
            .auto_deny_stale_parent_proxy_approvals(
                &server,
                child_thread_id,
                child_turn_id,
                omne_protocol::TurnStatus::Failed,
                stale_proxy_approval_ids,
            )
            .await;

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let denied_count = parent_events
            .into_iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_protocol::ApprovalDecision::Denied,
                        ..
                    } if approval_id == proxy_approval_id
                )
            })
            .count();
        assert_eq!(denied_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_reuses_existing_pending_parent_proxy_request()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let existing_proxy_approval_id = ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: existing_proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "task_title": "child task",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": { "argv": ["echo", "hi"] },
                    }
                }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        assert_eq!(
            schedule.approval_proxy_by_child.get(&child_key),
            Some(&existing_proxy_approval_id)
        );
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );
        let snapshot = schedule.snapshot();
        let pending = snapshot[0]["pending_approval"]
            .as_object()
            .expect("missing pending_approval after catch-up");
        let existing_proxy_approval_id_text = existing_proxy_approval_id.to_string();
        assert_eq!(
            pending.get("approval_id").and_then(Value::as_str),
            Some(existing_proxy_approval_id_text.as_str())
        );
        assert_eq!(
            pending.get("action").and_then(Value::as_str),
            Some("subagent/proxy_approval")
        );
        assert_eq!(
            pending.get("child_action").and_then(Value::as_str),
            Some("process/start")
        );

        schedule.catch_up_running_events(&server).await;
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_reconciles_child_with_existing_decided_parent_proxy()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let existing_proxy_approval_id = ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: existing_proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "task_title": "child task",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": { "argv": ["echo", "hi"] },
                    }
                }),
            })
            .await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: existing_proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: true,
                reason: Some("approved from parent".to_string()),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        wait_for_approval_decided(&server, child_thread_id, child_approval_id).await?;
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());
        assert!(schedule.pending_approvals_by_child.is_empty());
        let snapshot = schedule.snapshot();
        assert!(snapshot[0]["pending_approval"].is_null());
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );

        schedule.catch_up_running_events(&server).await;

        let child_events = server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {child_thread_id}"))?;
        let child_decisions = child_events
            .iter()
            .filter_map(|event| {
                let omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    remember,
                    reason,
                } = &event.kind
                else {
                    return None;
                };
                if *approval_id != child_approval_id {
                    return None;
                }
                Some((*decision, *remember, reason.clone()))
            })
            .collect::<Vec<_>>();
        assert_eq!(child_decisions.len(), 1);
        assert_eq!(
            child_decisions[0].0,
            omne_protocol::ApprovalDecision::Approved
        );
        assert!(child_decisions[0].1);
        assert!(
            child_decisions[0]
                .2
                .as_deref()
                .unwrap_or_default()
                .starts_with(crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX)
        );

        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_forwards_child_decision_when_request_event_was_already_seen()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: child_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("child approved".to_string()),
            })
            .await?;
        drop(child_handle);

        let child_events = server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {child_thread_id}"))?;
        let request_seq = child_events
            .into_iter()
            .find_map(|event| match event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested { approval_id, .. }
                    if approval_id == child_approval_id =>
                {
                    Some(event.seq.0)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing child approval requested event"))?;

        let existing_proxy_approval_id = ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: existing_proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "task_title": "child task",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": { "argv": ["echo", "hi"] },
                    }
                }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: request_seq,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        assert_eq!(
            count_approval_decisions(&server, parent_thread_id, existing_proxy_approval_id).await?,
            1
        );

        schedule.catch_up_running_events(&server).await;
        assert_eq!(
            count_approval_decisions(&server, parent_thread_id, existing_proxy_approval_id).await?,
            1
        );
        Ok(())
    }

    async fn count_proxy_approval_requests(
        server: &super::super::Server,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        child_approval_id: ApprovalId,
    ) -> anyhow::Result<usize> {
        let events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let child_thread = child_thread_id.to_string();
        let child_approval = child_approval_id.to_string();
        let count = events
            .into_iter()
            .filter(|event| {
                let omne_protocol::ThreadEventKind::ApprovalRequested { action, params, .. } =
                    &event.kind
                else {
                    return false;
                };
                if action != "subagent/proxy_approval" {
                    return false;
                }
                let Some(proxy) = params.get("subagent_proxy") else {
                    return false;
                };
                let proxy_thread = proxy
                    .get("child_thread_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let proxy_approval = proxy
                    .get("child_approval_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                proxy_thread == child_thread && proxy_approval == child_approval
            })
            .count();
        Ok(count)
    }

    async fn count_approval_decisions(
        server: &super::super::Server,
        thread_id: ThreadId,
        approval_id: ApprovalId,
    ) -> anyhow::Result<usize> {
        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        Ok(events
            .into_iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id: got, ..
                    } if got == approval_id
                )
            })
            .count())
    }

    async fn wait_for_proxy_request(
        server: &super::super::Server,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        child_approval_id: ApprovalId,
    ) -> anyhow::Result<ApprovalId> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let child_thread = child_thread_id.to_string();
        let child_approval = child_approval_id.to_string();
        loop {
            let events = server
                .thread_store
                .read_events_since(parent_thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
            for event in events {
                let omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    action,
                    params,
                    ..
                } = event.kind
                else {
                    continue;
                };
                if action != "subagent/proxy_approval" {
                    continue;
                }
                let Some(proxy) = params.get("subagent_proxy") else {
                    continue;
                };
                let proxy_thread = proxy
                    .get("child_thread_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let proxy_approval = proxy
                    .get("child_approval_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if proxy_thread == child_thread && proxy_approval == child_approval {
                    return Ok(approval_id);
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for parent proxy approval request");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_approval_decided(
        server: &super::super::Server,
        thread_id: ThreadId,
        approval_id: ApprovalId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.iter().any(|event| {
                matches!(
                    &event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id: got, ..
                    } if *got == approval_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let mut seen = Vec::new();
                for event in &events {
                    match &event.kind {
                        omne_protocol::ThreadEventKind::ApprovalRequested {
                            approval_id,
                            action,
                            ..
                        } => {
                            seen.push(format!("requested id={approval_id} action={action}"));
                        }
                        omne_protocol::ThreadEventKind::ApprovalDecided {
                            approval_id,
                            decision,
                            ..
                        } => {
                            seen.push(format!("decided id={approval_id} decision={decision:?}"));
                        }
                        _ => {}
                    }
                }
                anyhow::bail!(
                    "timed out waiting for approval decided: thread_id={thread_id} approval_id={approval_id} seen={}",
                    seen.join("; ")
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[cfg(test)]
mod reference_repo_file_tools_tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn build_test_server(omne_root: PathBuf) -> super::super::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        super::super::Server {
            cwd: omne_root.clone(),
            notify_tx,
            thread_store: super::super::ThreadStore::new(super::super::PmPaths::new(omne_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(super::super::McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_execpolicy::Policy::empty(),
        }
    }

    async fn append_plan_turn_started(
        server: &super::super::Server,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "/plan".to_string(),
                context_refs: None,
                attachments: None,
                directives: Some(vec![omne_protocol::TurnDirective::Plan]),
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        Ok(())
    }

    async fn append_thread_approval_policy(
        server: &super::super::Server,
        thread_id: ThreadId,
        approval_policy: omne_protocol::ApprovalPolicy,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;
        Ok(())
    }

    async fn append_thread_mode(
        server: &super::super::Server,
        thread_id: ThreadId,
        mode: &str,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::Manual,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some(mode.to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;
        Ok(())
    }

    async fn append_process_started(
        server: &super::super::Server,
        thread_id: ThreadId,
        process_id: omne_protocol::ProcessId,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                argv: vec!["echo".to_string(), "cross-thread".to_string()],
                cwd: "/tmp".to_string(),
                stdout_path: "/tmp/omne-test.stdout.log".to_string(),
                stderr_path: "/tmp/omne-test.stderr.log".to_string(),
            })
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_excludes_omne_reference_dir_for_workspace_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".omne_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".omne_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "pattern": "**/*.txt" }),
            None,
        )
        .await?;

        let paths = result["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("hello.txt")));
        assert!(
            !paths
                .iter()
                .any(|p| p.as_str().unwrap_or("").contains(".omne_data/reference/"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_and_read_can_use_reference_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".omne_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".omne_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let glob = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "root": "reference", "pattern": "**/*.txt" }),
            None,
        )
        .await?;
        let paths = glob["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("ref.txt")));
        assert!(!paths.iter().any(|p| p.as_str() == Some("hello.txt")));

        let read = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await?;
        assert_eq!(read["text"].as_str(), Some("ref\n"));
        assert_eq!(read["root"].as_str(), Some("reference"));
        Ok(())
    }

    #[tokio::test]
    async fn reference_root_fails_closed_when_not_configured() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let err = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await
        .expect_err("expected root=reference to fail when not configured");
        assert!(
            err.to_string().contains("reference repo root")
                || err.to_string().contains(".omne_data/reference/repo")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_blocks_side_effect_tool_calls() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server
            .thread_store
            .create_thread(project_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_write",
            serde_json::json!({ "path": "blocked.txt", "text": "blocked\n" }),
            None,
        )
        .await
        .expect_err("expected file_write to be blocked by /plan directive");
        assert!(err.to_string().contains("tool blocked by /plan directive"));
        assert!(
            !tokio::fs::try_exists(project_dir.join("blocked.txt")).await?,
            "blocked file_write should not create files"
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_allows_read_only_tool_calls() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(project_dir.join("note.txt"), "hello-plan\n").await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "note.txt" }),
            None,
        )
        .await?;

        assert_eq!(result["text"].as_str(), Some("hello-plan\n"));
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_uses_architect_mode_gate_for_read_only_tools() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("note.txt"), "hello-plan\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect override"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/read"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "note.txt" }),
            None,
        )
        .await
        .expect_err("expected file_read to be denied by architect mode under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_prompt_returns_needs_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("note.txt"), "hello-plan\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect prompt override"
    permissions:
      read: { decision: prompt }
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_approval_policy(&server, thread_id, omne_protocol::ApprovalPolicy::Manual)
            .await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "note.txt" }),
            None,
        )
        .await?;

        assert_eq!(result["needs_approval"].as_bool(), Some(true));
        assert!(result["approval_id"].as_str().is_some());
        assert!(result.get("text").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_read_honors_deny_globs() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("blocked.txt"), "blocked\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect read deny globs"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked.txt"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "blocked.txt" }),
            None,
        )
        .await
        .expect_err("expected file_read to be denied by architect deny_globs under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_glob_honors_deny_globs_for_explicit_path()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("blocked.txt"), "blocked\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect glob deny globs"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked.txt"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_glob",
            serde_json::json!({ "pattern": "blocked.txt" }),
            None,
        )
        .await
        .expect_err("expected file_glob to be denied for explicit denied path under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_grep_honors_deny_globs_for_explicit_include()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("blocked.txt"), "blocked content\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect grep deny globs"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked.txt"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_grep",
            serde_json::json!({ "query": "blocked", "include_glob": "blocked.txt" }),
            None,
        )
        .await
        .expect_err("expected file_grep to be denied for explicit denied include_glob under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_glob_honors_deny_globs_for_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "a\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect glob prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_glob",
            serde_json::json!({ "pattern": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected file_glob to be denied for denied glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_grep_honors_deny_globs_for_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "blocked text\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect grep prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_grep",
            serde_json::json!({ "query": "blocked", "include_glob": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected file_grep to be denied for denied include_glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_repo_search_honors_deny_globs_for_include_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "blocked text\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect repo search prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "repo_search",
            serde_json::json!({ "query": "blocked", "include_glob": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected repo_search to be denied for denied include_glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_repo_index_honors_deny_globs_for_include_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "hello\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect repo index prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "repo_index",
            serde_json::json!({ "include_glob": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected repo_index to be denied for denied include_glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_repo_symbols_honors_deny_globs_for_include_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.rs"), "fn blocked() {}\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect repo symbols prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "repo_symbols",
            serde_json::json!({ "include_glob": "blocked/**/*.rs" }),
            None,
        )
        .await
        .expect_err(
            "expected repo_symbols to be denied for denied include_glob prefix under /plan",
        );

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_state_blocks_cross_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "thread_state",
            serde_json::json!({ "thread_id": thread_b.to_string() }),
            None,
        )
        .await
        .expect_err("expected thread_state to be denied for cross-thread target under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_state_allows_same_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "thread_state",
            serde_json::json!({ "thread_id": thread_id.to_string() }),
            None,
        )
        .await?;

        let expected = thread_id.to_string();
        assert_eq!(result["thread_id"].as_str(), Some(expected.as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_usage_blocks_cross_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "thread_usage",
            serde_json::json!({ "thread_id": thread_b.to_string() }),
            None,
        )
        .await
        .expect_err("expected thread_usage to be denied for cross-thread target under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_usage_allows_same_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "thread_usage",
            serde_json::json!({ "thread_id": thread_id.to_string() }),
            None,
        )
        .await?;

        let expected = thread_id.to_string();
        assert_eq!(result["thread_id"].as_str(), Some(expected.as_str()));
        assert!(result["total_tokens_used"].as_u64().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_events_blocks_cross_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "thread_events",
            serde_json::json!({
                "thread_id": thread_b.to_string(),
                "since_seq": 0
            }),
            None,
        )
        .await
        .expect_err("expected thread_events to be denied for cross-thread target under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_events_allows_same_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "thread_events",
            serde_json::json!({
                "thread_id": thread_id.to_string(),
                "since_seq": 0
            }),
            None,
        )
        .await?;

        assert!(result["events"].as_array().is_some());
        assert!(result["last_seq"].as_u64().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_inspect_blocks_cross_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_b, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "process_inspect",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "max_lines": 10
            }),
            None,
        )
        .await
        .expect_err("expected process_inspect to be denied for cross-thread process under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_inspect_allows_same_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "process_inspect",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "max_lines": 10
            }),
            None,
        )
        .await?;

        let expected = process_id.to_string();
        assert_eq!(
            result["process"]["process_id"].as_str(),
            Some(expected.as_str())
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_tail_blocks_cross_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_b, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "process_tail",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "max_lines": 10
            }),
            None,
        )
        .await
        .expect_err("expected process_tail to be denied for cross-thread process under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_tail_allows_same_thread_process() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "process_tail",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "max_lines": 10
            }),
            None,
        )
        .await?;

        assert!(result.get("text").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_follow_blocks_cross_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_b, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "process_follow",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "since_offset": 0,
                "max_bytes": 128
            }),
            None,
        )
        .await
        .expect_err("expected process_follow to be denied for cross-thread process under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_follow_allows_same_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "process_follow",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "since_offset": 0,
                "max_bytes": 128
            }),
            None,
        )
        .await?;

        assert_eq!(result["next_offset"].as_u64(), Some(0));
        assert_eq!(result["eof"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_inspect_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny inspect"
    permissions:
      process:
        inspect: { decision: allow }
    tool_overrides:
      - tool: "process/inspect"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "process_inspect",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "max_lines": 10
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_tail_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny tail"
    permissions:
      process:
        inspect: { decision: allow }
    tool_overrides:
      - tool: "process/tail"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "process_tail",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "max_lines": 10
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_follow_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny follow"
    permissions:
      process:
        inspect: { decision: allow }
    tool_overrides:
      - tool: "process/follow"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "process_follow",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "since_offset": 0,
                "max_bytes": 128
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_read_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(project.join("note.txt"), "hello\n").await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny file read"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/read"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({
                "path": "note.txt"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(project.join("note.txt"), "hello\n").await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny file glob"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/glob"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({
                "pattern": "*.txt"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_grep_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(project.join("note.txt"), "hello\n").await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny file grep"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/grep"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_grep",
            serde_json::json!({
                "query": "hello",
                "include_glob": "*.txt"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_write_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact write"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/write"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_write",
            serde_json::json!({
                "artifact_type": "test",
                "summary": "s",
                "text": "hello"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_list_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact list"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/list"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_list",
            serde_json::json!({}),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact read"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/read"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let artifact_id = omne_protocol::ArtifactId::new();
        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_read",
            serde_json::json!({
                "artifact_id": artifact_id.to_string()
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_delete_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact delete"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/delete"
        decision: deny
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let artifact_id = omne_protocol::ArtifactId::new();
        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_delete",
            serde_json::json!({
                "artifact_id": artifact_id.to_string()
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }
}
