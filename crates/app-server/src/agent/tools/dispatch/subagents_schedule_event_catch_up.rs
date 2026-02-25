impl SubagentSpawnSchedule {
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
}
