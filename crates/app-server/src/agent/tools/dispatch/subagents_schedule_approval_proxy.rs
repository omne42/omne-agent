impl SubagentSpawnSchedule {
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
}
