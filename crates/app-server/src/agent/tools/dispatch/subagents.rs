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


include!("subagents_agent_spawn_guard_tests.rs");
include!("subagents_reference_repo_file_tools_tests.rs");
