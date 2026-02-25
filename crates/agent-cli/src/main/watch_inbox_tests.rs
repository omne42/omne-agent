use super::*;

#[cfg(test)]
fn watch_detail_summary_lines(
    auto_apply: Option<&FanOutAutoApplyInboxSummary>,
    fan_in_blocker: Option<&FanInDependencyBlockedInboxSummary>,
    fan_in_diagnostics: Option<&FanInResultDiagnosticsInboxSummary>,
    subagent_pending: Option<&SubagentPendingApprovalsSummary>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(summary) = auto_apply {
        lines.push(format!("summary: {}", format_fan_out_auto_apply_summary(summary)));
    }
    if let Some(summary) = fan_in_blocker {
        lines.push(format!(
            "summary: {}",
            format_fan_in_dependency_blocked_summary(summary)
        ));
    }
    if let Some(summary) = fan_in_diagnostics {
        lines.push(format!(
            "summary: {}",
            format_fan_in_result_diagnostics_summary(summary)
        ));
    }
    if let Some(summary) = subagent_pending {
        lines.push(format!(
            "summary: {}",
            format_subagent_pending_summary(summary)
        ));
    }
    lines
}

#[cfg(test)]
fn watch_detail_summary_json_rows(
    thread_id: ThreadId,
    auto_apply: Option<&FanOutAutoApplyInboxSummary>,
    fan_in_blocker: Option<&FanInDependencyBlockedInboxSummary>,
    fan_in_diagnostics: Option<&FanInResultDiagnosticsInboxSummary>,
    subagent_pending: Option<&SubagentPendingApprovalsSummary>,
) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    if let Some(summary) = auto_apply {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_out_auto_apply",
            "payload": summary,
        }));
    }
    if let Some(summary) = fan_in_blocker {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_in_dependency_blocker",
            "payload": summary,
        }));
    }
    if let Some(summary) = fan_in_diagnostics {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "fan_in_result_diagnostics",
            "payload": summary,
        }));
    }
    if let Some(summary) = subagent_pending {
        rows.push(serde_json::json!({
            "kind": "watch_detail_summary",
            "thread_id": thread_id,
            "summary_type": "subagent_pending",
            "payload": summary,
        }));
    }
    rows
}

#[cfg(test)]
fn is_fan_out_auto_apply_error(summary: &FanOutAutoApplyInboxSummary) -> bool {
    summary.status == "error"
}


#[cfg(test)]
mod watch_inbox_tests {
    use super::*;

    fn event_with_kind(kind: omne_protocol::ThreadEventKind) -> ThreadEvent {
        ThreadEvent {
            seq: omne_protocol::EventSeq::ZERO,
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id: ThreadId::new(),
            kind,
        }
    }

    fn test_thread_meta(
        has_fan_out_linkage_issue: bool,
        has_fan_out_auto_apply_error: bool,
        has_fan_in_dependency_blocked: bool,
    ) -> ThreadMeta {
        ThreadMeta {
            thread_id: ThreadId::new(),
            cwd: "/tmp".to_string(),
            archived: false,
            archived_at: None,
            archived_reason: None,
            approval_policy: ApprovalPolicy::OnRequest,
            sandbox_policy: SandboxPolicy::WorkspaceWrite,
            model: None,
            openai_base_url: None,
            last_seq: 0,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
            last_turn_id: None,
            last_turn_status: None,
            last_turn_reason: None,
            token_budget_limit: None,
            token_budget_remaining: None,
            token_budget_utilization: None,
            token_budget_exceeded: None,
            token_budget_warning_active: None,
            attention_state: "running".to_string(),
            has_fan_out_linkage_issue,
            has_fan_out_auto_apply_error,
            fan_out_auto_apply: None,
            has_fan_in_dependency_blocked,
            fan_in_dependency_blocker: None,
            has_fan_in_result_diagnostics: false,
            fan_in_result_diagnostics: None,
            pending_subagent_proxy_approvals: 0,
        }
    }

    fn test_thread_attention() -> ThreadAttention {
        ThreadAttention {
            thread_id: ThreadId::new(),
            cwd: Some("/tmp/ws".to_string()),
            archived: false,
            archived_at: None,
            archived_reason: None,
            paused: false,
            paused_at: None,
            paused_reason: None,
            failed_processes: Vec::new(),
            approval_policy: ApprovalPolicy::OnRequest,
            sandbox_policy: SandboxPolicy::WorkspaceWrite,
            model: Some("gpt-5".to_string()),
            openai_base_url: None,
            last_seq: 0,
            active_turn_id: None,
            active_turn_interrupt_requested: false,
            last_turn_id: None,
            last_turn_status: None,
            last_turn_reason: None,
            token_budget_limit: None,
            token_budget_remaining: None,
            token_budget_utilization: None,
            token_budget_exceeded: None,
            token_budget_warning_active: None,
            attention_state: "running".to_string(),
            pending_approvals: Vec::new(),
            running_processes: Vec::new(),
            stale_processes: Vec::new(),
            attention_markers: omne_app_server_protocol::ThreadAttentionMarkers {
                plan_ready: None,
                diff_ready: None,
                fan_out_linkage_issue: None,
                fan_out_auto_apply_error: None,
                test_failed: None,
                token_budget_warning: None,
                token_budget_exceeded: None,
            },
            has_plan_ready: false,
            has_diff_ready: false,
            has_fan_out_linkage_issue: false,
            has_fan_out_auto_apply_error: false,
            fan_out_auto_apply: None,
            has_fan_in_dependency_blocked: false,
            fan_in_dependency_blocker: None,
            has_fan_in_result_diagnostics: false,
            fan_in_result_diagnostics: None,
            has_test_failed: false,
        }
    }

    fn test_subagent_pending_approval(
        approve_cmd: &str,
    ) -> omne_app_server_protocol::ThreadAttentionPendingApproval {
        omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some(approve_cmd.to_string()),
                deny_cmd: None,
            }),
            requested_at: None,
        }
    }

    fn marker_set_event(marker: omne_protocol::AttentionMarkerKind) -> ThreadEvent {
        ThreadEvent {
            seq: omne_protocol::EventSeq::ZERO,
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id: ThreadId::new(),
            kind: omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker,
                turn_id: None,
                artifact_id: None,
                artifact_type: None,
                process_id: None,
                exit_code: None,
                command: None,
            },
        }
    }

    #[test]
    fn attention_state_update_marks_linkage_issue_marker_as_failed() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::FanOutLinkageIssue);
        assert_eq!(attention_state_update(&event), Some("failed"));
    }

    #[test]
    fn attention_state_update_marks_auto_apply_error_marker_as_failed() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::FanOutAutoApplyError);
        assert_eq!(attention_state_update(&event), Some("failed"));
    }

    #[test]
    fn attention_state_update_marks_token_budget_warning_marker() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::TokenBudgetWarning);
        assert_eq!(attention_state_update(&event), Some("token_budget_warning"));
    }

    #[test]
    fn attention_state_update_marks_token_budget_exceeded_marker() {
        let event = marker_set_event(omne_protocol::AttentionMarkerKind::TokenBudgetExceeded);
        assert_eq!(attention_state_update(&event), Some("token_budget_exceeded"));
    }

    #[test]
    fn format_thread_row_renders_expected_shape() {
        let turn_id = TurnId::new();
        let mut thread = test_thread_meta(false, false, false);
        thread.thread_id = ThreadId::new();
        thread.attention_state = "failed".to_string();
        thread.last_seq = 42;
        thread.last_turn_id = Some(turn_id);
        thread.model = Some("gpt-5".to_string());
        thread.cwd = "/tmp/repo".to_string();

        let row = format_thread_row(&thread);
        assert_eq!(
            row,
            format!(
                "{}  state=failed  seq=42  turn={}  model=gpt-5  cwd=/tmp/repo",
                thread.thread_id, turn_id
            )
        );
    }

    #[test]
    fn format_thread_row_shortens_long_cwd() {
        let mut thread = test_thread_meta(false, false, false);
        thread.thread_id = ThreadId::new();
        thread.cwd = format!("/tmp/{}", "a".repeat(120));

        let row = format_thread_row(&thread);
        let cwd = row
            .split("  cwd=")
            .nth(1)
            .expect("thread row should contain cwd");
        assert!(cwd.starts_with("..."));
        assert!(cwd.len() <= 60);
    }

    #[test]
    fn format_thread_detail_lines_renders_expected_shape() {
        let mut att = test_thread_attention();
        let running_process_id = ProcessId::new();
        let failed_process_id = ProcessId::new();
        let stale_process_id = ProcessId::new();
        att.has_plan_ready = true;
        att.has_test_failed = true;
        att.token_budget_limit = Some(200);
        att.token_budget_remaining = Some(10);
        att.token_budget_utilization = Some(0.95);
        att.token_budget_exceeded = Some(false);
        att.token_budget_warning_active = Some(true);
        att.running_processes = vec![omne_app_server_protocol::ThreadAttentionRunningProcess {
            process_id: running_process_id,
            argv: vec!["cargo".to_string(), "test".to_string()],
            status: Some("running".to_string()),
        }];
        att.failed_processes = vec![failed_process_id];
        att.stale_processes = vec![omne_app_server_protocol::ThreadAttentionStaleProcess {
            process_id: stale_process_id,
            idle_seconds: 120,
            last_update_at: "2026-01-01T00:00:00Z".to_string(),
            stdout_path: "/tmp/stdout.log".to_string(),
            stderr_path: "/tmp/stderr.log".to_string(),
        }];

        let lines = format_thread_detail_lines(&att, 0.9);
        assert_eq!(
            lines,
            vec![
                "markers: plan_ready, token_budget_warning, test_failed".to_string(),
                "token_budget: remaining=10 limit=200 utilization=95.0% exceeded=false".to_string(),
                format!("processes: 1 ({running_process_id})"),
                format!("failed_processes: 1 ({failed_process_id})"),
                format!("stale_processes: 1 ({stale_process_id})"),
            ]
        );
    }

    #[test]
    fn format_thread_detail_lines_truncates_approval_sections_after_three_items() {
        let mut att = test_thread_attention();
        let pending_approvals = vec![
            test_subagent_pending_approval("x --approve"),
            test_subagent_pending_approval("x --approve"),
            test_subagent_pending_approval("x --approve"),
            test_subagent_pending_approval("x --approve"),
        ];
        let ids = pending_approvals
            .iter()
            .take(3)
            .map(|pending| pending.approval_id.to_string())
            .collect::<Vec<_>>();
        att.pending_approvals = pending_approvals;

        let lines = format_thread_detail_lines(&att, 0.9);
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            format!("approvals: 4 ({}, {}, {}, ...)", ids[0], ids[1], ids[2])
        );
        assert_eq!(
            lines[1],
            format!(
                "approval_details: {}:subagent/proxy_approval (approve_cmd=x --approve) (deny_cmd=x --deny); {}:subagent/proxy_approval (approve_cmd=x --approve) (deny_cmd=x --deny); {}:subagent/proxy_approval (approve_cmd=x --approve) (deny_cmd=x --deny); ...",
                ids[0], ids[1], ids[2]
            )
        );
        assert_eq!(
            lines[2],
            format!(
                "approval_commands: {}: approve_cmd=x --approve deny_cmd=x --deny; {}: approve_cmd=x --approve deny_cmd=x --deny; {}: approve_cmd=x --approve deny_cmd=x --deny; ...",
                ids[0], ids[1], ids[2]
            )
        );
        assert_eq!(lines[3], "subagent_pending: total=4 states=unknown:4");
    }

    #[test]
    fn attention_state_update_ignores_marker_clear_events() {
        let event = event_with_kind(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: None,
                reason: Some("new turn started".to_string()),
            });
        assert_eq!(attention_state_update(&event), None);
    }

    #[test]
    fn should_refresh_watch_detail_summary_true_for_tool_completed() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id: omne_protocol::ToolId::new(),
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: None,
        })];
        assert!(should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_detail_summary_true_for_attention_marker_set() {
        let events = vec![marker_set_event(
            omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
        )];
        assert!(should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_detail_summary_false_for_assistant_only_batch() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "ok".to_string(),
            model: None,
            response_id: None,
            token_usage: None,
        })];
        assert!(!should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_auto_apply_summary_true_for_auto_apply_marker_clear() {
        let events = vec![event_with_kind(
            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                turn_id: None,
                reason: None,
            },
        )];
        assert!(should_refresh_watch_auto_apply_summary(&events));
    }

    #[test]
    fn should_refresh_watch_fan_in_summary_false_for_marker_only_batch() {
        let events = vec![marker_set_event(
            omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
        )];
        assert!(!should_refresh_watch_fan_in_dependency_blocker_summary(&events));
    }

    #[test]
    fn should_refresh_watch_fan_in_summary_true_for_turn_completed() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: TurnId::new(),
            status: TurnStatus::Completed,
            reason: None,
        })];
        assert!(should_refresh_watch_fan_in_dependency_blocker_summary(&events));
    }

    #[test]
    fn should_refresh_watch_subagent_pending_summary_true_for_approval_requested() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: "subagent/proxy_approval".to_string(),
            params: serde_json::json!({}),
        })];
        assert!(should_refresh_watch_subagent_pending_summary(&events));
        assert!(should_refresh_watch_detail_summary(&events));
    }

    #[test]
    fn should_refresh_watch_subagent_pending_summary_true_for_turn_completed() {
        let events = vec![event_with_kind(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: TurnId::new(),
            status: TurnStatus::Completed,
            reason: None,
        })];
        assert!(should_refresh_watch_subagent_pending_summary(&events));
    }

    #[test]
    fn apply_inbox_filters_only_fan_out_linkage_issue_keeps_marked_threads() {
        let t1 = test_thread_meta(true, false, false);
        let t2 = test_thread_meta(false, false, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, true, false, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_without_marker_filter_keeps_all_threads() {
        let t1 = test_thread_meta(true, false, false);
        let t2 = test_thread_meta(false, true, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1);
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn inbox_thread_changed_true_when_prev_missing() {
        let current = test_thread_meta(false, false, false);
        assert!(inbox_thread_changed(None, &current));
    }

    #[test]
    fn inbox_thread_changed_false_when_seq_and_state_unchanged() {
        let current = test_thread_meta(false, false, false);
        let previous = current.clone();
        assert!(!inbox_thread_changed(Some(&previous), &current));
    }

    #[test]
    fn inbox_thread_changed_true_when_seq_or_state_changes() {
        let current = test_thread_meta(false, false, false);

        let mut previous_seq = current.clone();
        previous_seq.last_seq = current.last_seq.saturating_add(1);
        assert!(inbox_thread_changed(Some(&previous_seq), &current));

        let mut previous_state = current.clone();
        previous_state.attention_state = "failed".to_string();
        assert!(inbox_thread_changed(Some(&previous_state), &current));
    }

    #[test]
    fn apply_inbox_filters_only_fan_out_auto_apply_error_keeps_marked_threads() {
        let t1 = test_thread_meta(false, true, false);
        let t2 = test_thread_meta(false, false, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, true, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_with_both_marker_filters_requires_both_markers() {
        let t1 = test_thread_meta(true, true, false);
        let t2 = test_thread_meta(true, false, false);
        let t3 = test_thread_meta(false, true, false);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3);

        let filtered = apply_inbox_filters(threads, true, true, false, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_fan_in_dependency_blocked_keeps_marked_threads() {
        let t1 = test_thread_meta(false, false, true);
        let t2 = test_thread_meta(false, false, false);
        let t3 = test_thread_meta(false, false, true);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, true, false, false, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_subagent_proxy_approval_keeps_marked_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.pending_subagent_proxy_approvals = 1;
        let t2 = test_thread_meta(false, false, false);
        let mut t3 = test_thread_meta(false, false, false);
        t3.pending_subagent_proxy_approvals = 2;
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, false, 0.9, true);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_fan_in_result_diagnostics_keeps_marked_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.has_fan_in_result_diagnostics = true;
        let t2 = test_thread_meta(false, false, false);
        let mut t3 = test_thread_meta(false, false, false);
        t3.has_fan_in_result_diagnostics = true;
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, false, true, false, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_token_budget_exceeded_keeps_marked_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_exceeded = Some(true);
        let t2 = test_thread_meta(false, false, false);
        let mut t3 = test_thread_meta(false, false, false);
        t3.token_budget_exceeded = Some(true);
        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3.clone());

        let filtered = apply_inbox_filters(threads, false, false, false, false, true, false, 0.9, false);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.get(&t1.thread_id).is_some());
        assert!(filtered.get(&t3.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_token_budget_warning_keeps_warning_threads() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_utilization = Some(0.95);
        t1.token_budget_exceeded = Some(false);

        let mut t2 = test_thread_meta(false, false, false);
        t2.token_budget_limit = Some(200);
        t2.token_budget_utilization = Some(0.89);
        t2.token_budget_exceeded = Some(false);

        let mut t3 = test_thread_meta(false, false, false);
        t3.token_budget_limit = Some(200);
        t3.token_budget_utilization = Some(0.97);
        t3.token_budget_exceeded = Some(true);

        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);
        threads.insert(t3.thread_id, t3);

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, true, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_only_token_budget_warning_prefers_server_flag() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_utilization = Some(0.10);
        t1.token_budget_exceeded = Some(false);
        t1.token_budget_warning_active = Some(true);

        let mut t2 = test_thread_meta(false, false, false);
        t2.token_budget_limit = Some(200);
        t2.token_budget_utilization = Some(0.95);
        t2.token_budget_exceeded = Some(false);
        t2.token_budget_warning_active = Some(false);

        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1.clone());
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, false, false, false, false, true, 0.9, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.get(&t1.thread_id).is_some());
    }

    #[test]
    fn apply_inbox_filters_token_budget_exceeded_and_warning_intersection_is_empty() {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_utilization = Some(0.95);
        t1.token_budget_exceeded = Some(false);

        let mut t2 = test_thread_meta(false, false, false);
        t2.token_budget_limit = Some(200);
        t2.token_budget_utilization = Some(0.95);
        t2.token_budget_exceeded = Some(true);

        let mut threads = std::collections::BTreeMap::new();
        threads.insert(t1.thread_id, t1);
        threads.insert(t2.thread_id, t2);

        let filtered = apply_inbox_filters(threads, false, false, false, false, true, true, 0.9, false);
        assert!(filtered.is_empty());
    }

    #[test]
    fn should_notify_presence_rising_edge_only_on_false_to_true() {
        assert!(!should_notify_presence_rising_edge(None, true));
        assert!(!should_notify_presence_rising_edge(None, false));
        assert!(!should_notify_presence_rising_edge(Some(true), true));
        assert!(!should_notify_presence_rising_edge(Some(true), false));
        assert!(!should_notify_presence_rising_edge(Some(false), false));
        assert!(should_notify_presence_rising_edge(Some(false), true));
    }

    #[test]
    fn attention_state_severity_maps_expected_levels() {
        assert_eq!(attention_state_severity("failed"), notify_kit::Severity::Error);
        assert_eq!(
            attention_state_severity("fan_out_auto_apply_error"),
            notify_kit::Severity::Error
        );
        assert_eq!(
            attention_state_severity("fan_out_linkage_issue"),
            notify_kit::Severity::Warning
        );
        assert_eq!(
            attention_state_severity("fan_in_dependency_blocked"),
            notify_kit::Severity::Warning
        );
        assert_eq!(
            attention_state_severity("fan_in_result_diagnostics"),
            notify_kit::Severity::Warning
        );
        assert_eq!(
            attention_state_severity("token_budget_warning"),
            notify_kit::Severity::Warning
        );
        assert_eq!(attention_state_severity("running"), notify_kit::Severity::Info);
    }

    #[test]
    fn attention_detail_markers_include_fan_in_statuses() {
        let markers =
            attention_detail_markers(false, false, false, false, true, true, false, false, false);
        assert_eq!(
            markers,
            vec!["fan_in_dependency_blocked", "fan_in_result_diagnostics"]
        );
    }

    #[test]
    fn attention_detail_markers_include_token_budget_statuses() {
        let markers =
            attention_detail_markers(false, false, false, false, false, false, true, true, false);
        assert_eq!(
            markers,
            vec!["token_budget_exceeded", "token_budget_warning"]
        );
    }

    #[test]
    fn attention_detail_markers_preserve_display_order() {
        let markers = attention_detail_markers(true, true, true, true, true, true, true, true, true);
        assert_eq!(
            markers,
            vec![
                "plan_ready",
                "diff_ready",
                "fan_out_linkage_issue",
                "fan_out_auto_apply_error",
                "fan_in_dependency_blocked",
                "fan_in_result_diagnostics",
                "token_budget_exceeded",
                "token_budget_warning",
                "test_failed",
            ]
        );
    }

    #[test]
    fn should_emit_presence_bell_tracks_initial_value_without_notifying() {
        let mut last_present = None;
        let mut last_bell_at = None;
        assert!(!should_emit_presence_bell(
            true,
            1000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
        assert!(last_bell_at.is_none());
    }

    #[test]
    fn should_emit_presence_bell_notifies_on_rising_edge() {
        let mut last_present = Some(false);
        let mut last_bell_at = None;
        assert!(should_emit_presence_bell(
            true,
            1000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
        assert!(last_bell_at.is_some());
    }

    #[test]
    fn should_emit_presence_bell_respects_debounce_window() {
        let mut last_present = Some(false);
        let mut last_bell_at = Some(Instant::now());
        assert!(!should_emit_presence_bell(
            true,
            60_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
    }

    #[test]
    fn should_emit_presence_bell_does_not_notify_on_falling_edge() {
        let mut last_present = Some(true);
        let mut last_bell_at = Some(Instant::now());
        assert!(!should_emit_presence_bell(
            false,
            1_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(false));
    }

    #[test]
    fn should_emit_presence_bell_notifies_on_next_rising_edge_after_fall() {
        let mut last_present = Some(true);
        let mut last_bell_at = None;
        assert!(!should_emit_presence_bell(
            false,
            1_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert!(should_emit_presence_bell(
            true,
            1_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
        assert!(last_bell_at.is_some());
    }

    #[test]
    fn should_emit_presence_bell_debounces_rising_edge_after_fall_if_recently_notified() {
        let mut last_present = Some(true);
        let mut last_bell_at = Some(Instant::now());
        assert!(!should_emit_presence_bell(
            false,
            60_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert!(!should_emit_presence_bell(
            true,
            60_000,
            &mut last_present,
            &mut last_bell_at
        ));
        assert_eq!(last_present, Some(true));
    }

    #[test]
    fn token_budget_warning_present_only_triggers_near_limit_without_exceeded() {
        assert!(!token_budget_warning_present(None, Some(0.95), Some(false), 0.9));
        assert!(!token_budget_warning_present(Some(200), Some(0.95), Some(true), 0.9));
        assert!(!token_budget_warning_present(Some(200), None, Some(false), 0.9));
        assert!(!token_budget_warning_present(Some(200), Some(0.89), Some(false), 0.9));
        assert!(token_budget_warning_present(
            Some(200),
            Some(0.90),
            Some(false),
            0.9
        ));
        assert!(token_budget_warning_present(
            Some(200),
            Some(0.95),
            Some(false),
            0.9
        ));
    }

    #[test]
    fn format_token_budget_snapshot_omits_when_limit_absent() {
        let line = format_token_budget_snapshot(None, Some(0), Some(1.0), Some(true));
        assert!(line.is_none());
    }

    #[test]
    fn format_token_budget_snapshot_formats_all_fields() {
        let line = format_token_budget_snapshot(Some(200), Some(0), Some(1.25), Some(true))
            .expect("token budget line");
        assert_eq!(
            line,
            "token_budget: remaining=0 limit=200 utilization=125.0% exceeded=true"
        );
    }

    #[test]
    fn format_token_budget_snapshot_uses_defaults_for_missing_fields() {
        let line = format_token_budget_snapshot(Some(200), None, None, None).expect("token budget line");
        assert_eq!(
            line,
            "token_budget: remaining=- limit=200 utilization=- exceeded=false"
        );
    }

    #[test]
    fn pending_approval_preview_prefers_action_id_and_summary_hint() {
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("legacy/action".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ArtifactWrite),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: Some("need_approval".to_string()),
                argv: Some(vec!["cargo".to_string(), "test".to_string()]),
                cwd: Some("/tmp/workspace".to_string()),
                process_id: None,
                artifact_type: None,
                path: Some("/tmp/workspace/README.md".to_string()),
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains(":artifact/write"));
        assert!(preview.contains("path=/tmp/workspace/README.md"));
        assert!(!preview.contains("legacy/action"));
    }

    #[test]
    fn pending_approval_preview_falls_back_to_legacy_action() {
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("process/start".to_string()),
            action_id: None,
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: Some(vec!["python".to_string(), "script.py".to_string()]),
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains(":process/start"));
        assert!(preview.contains("argv=python script.py"));
    }

    #[test]
    fn pending_approval_preview_surfaces_subagent_child_ids() {
        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: Some("on_request".to_string()),
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: Some("process/start".to_string()),
                hook: None,
                child_thread_id: Some(child_thread_id),
                child_turn_id: Some(child_turn_id),
                child_approval_id: Some(child_approval_id),
                child_attention_state: Some("running".to_string()),
                child_last_turn_status: Some(TurnStatus::Stuck),
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains(":subagent/proxy_approval"));
        assert!(preview.contains("child_thread_id="));
        assert!(preview.contains(&child_thread_id.to_string()));
        assert!(preview.contains("child_turn_id="));
        assert!(preview.contains(&child_turn_id.to_string()));
        assert!(preview.contains("child_approval_id="));
        assert!(preview.contains(&child_approval_id.to_string()));
        assert!(preview.contains("child_attention_state=running"));
        assert!(preview.contains("child_last_turn_status=stuck"));
        assert!(!preview.contains("subagent="));
    }

    #[test]
    fn pending_approval_preview_includes_path_with_subagent_child_ids() {
        let child_thread_id = ThreadId::new();
        let child_approval_id = ApprovalId::new();
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: Some("/tmp/ws/src/main.rs".to_string()),
                server: None,
                tool: Some("file/write".to_string()),
                hook: None,
                child_thread_id: Some(child_thread_id),
                child_turn_id: None,
                child_approval_id: Some(child_approval_id),
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains("child_thread_id="));
        assert!(preview.contains("child_approval_id="));
        assert!(preview.contains("path=/tmp/ws/src/main.rs"));
    }

    #[test]
    fn approval_summary_display_combines_subagent_ids_and_context_hint() {
        let child_thread_id = ThreadId::new();
        let child_approval_id = ApprovalId::new();
        let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
            requirement: None,
            argv: None,
            cwd: None,
            process_id: None,
            artifact_type: None,
            path: Some("/tmp/ws/src/main.rs".to_string()),
            server: None,
            tool: None,
            hook: None,
            child_thread_id: Some(child_thread_id),
            child_turn_id: None,
            child_approval_id: Some(child_approval_id),
            child_attention_state: None,
            child_last_turn_status: None,
            approve_cmd: None,
            deny_cmd: None,
        };
        let display = approval_summary_display_from_summary(&summary).expect("summary");
        assert!(display.contains("child_thread_id="));
        assert!(display.contains(&child_thread_id.to_string()));
        assert!(display.contains("child_approval_id="));
        assert!(display.contains(&child_approval_id.to_string()));
        assert!(display.contains("path=/tmp/ws/src/main.rs"));
        assert!(!display.contains("subagent="));
    }

    #[test]
    fn pending_approval_preview_includes_approve_cmd_when_present() {
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some(
                    "omne approval decide thread-1 approval-1 --approve".to_string(),
                ),
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let preview = format_pending_approval_preview(&pending);
        assert!(preview.contains("approve_cmd="));
        assert!(preview.contains("--approve"));
        assert!(preview.contains("deny_cmd="));
        assert!(preview.contains("--deny"));
    }

    #[test]
    fn pending_approval_commands_include_approve_and_deny_cmd() {
        let approval_id = ApprovalId::new();
        let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id,
            turn_id: None,
            action: Some("subagent/proxy_approval".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
            params: None,
            summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some(
                    "omne approval decide thread-1 approval-1 --approve".to_string(),
                ),
                deny_cmd: None,
            }),
            requested_at: None,
        };
        let commands = format_pending_approval_commands(&pending).expect("commands");
        assert!(commands.contains(&approval_id.to_string()));
        assert!(commands.contains("approve_cmd="));
        assert!(commands.contains("--approve"));
        assert!(commands.contains("deny_cmd="));
        assert!(commands.contains("--deny"));
    }

    #[test]
    fn format_subagent_pending_approvals_summary_reports_state_breakdown() {
        let approvals = vec![
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("subagent/proxy_approval".to_string()),
                action_id: Some(
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                ),
                params: None,
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    requirement: None,
                    argv: None,
                    cwd: None,
                    process_id: None,
                    artifact_type: None,
                    path: None,
                    server: None,
                    tool: None,
                    hook: None,
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: None,
                    child_attention_state: Some("running".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                }),
                requested_at: None,
            },
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("subagent/proxy_approval".to_string()),
                action_id: Some(
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                ),
                params: None,
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    requirement: None,
                    argv: None,
                    cwd: None,
                    process_id: None,
                    artifact_type: None,
                    path: None,
                    server: None,
                    tool: None,
                    hook: None,
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: None,
                    child_attention_state: Some("FAILED".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                }),
                requested_at: None,
            },
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("subagent/proxy_approval".to_string()),
                action_id: None,
                params: None,
                summary: None,
                requested_at: None,
            },
            omne_app_server_protocol::ThreadAttentionPendingApproval {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: Some("process/start".to_string()),
                action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                params: None,
                summary: None,
                requested_at: None,
            },
        ];

        let line = format_subagent_pending_approvals_summary(&approvals).expect("summary");
        assert!(line.starts_with("subagent_pending: total=3 states="));
        assert!(line.contains("running:1"));
        assert!(line.contains("failed:1"));
        assert!(line.contains("unknown:1"));
    }

    #[test]
    fn format_subagent_pending_approvals_summary_skips_non_subagent_items() {
        let approvals = vec![omne_app_server_protocol::ThreadAttentionPendingApproval {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: Some("process/start".to_string()),
            action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
            params: None,
            summary: None,
            requested_at: None,
        }];
        assert!(format_subagent_pending_approvals_summary(&approvals).is_none());
    }

    #[test]
    fn approval_action_label_unknown_id_falls_back_to_raw_action() {
        let label = approval_action_label_from_parts(
            Some(omne_app_server_protocol::ThreadApprovalActionId::Unknown),
            Some("custom/tool"),
        );
        assert_eq!(label, "custom/tool");
    }

    #[test]
    fn approval_summary_from_params_extracts_path_and_requirement() {
        let params = serde_json::json!({
            "approval": { "requirement": "on_request" },
            "target_path": "/tmp/ws/main.rs"
        });
        let summary = approval_summary_from_params(&params).expect("summary");
        assert_eq!(summary.requirement.as_deref(), Some("on_request"));
        assert_eq!(summary.path.as_deref(), Some("/tmp/ws/main.rs"));
    }

    #[test]
    fn approval_summary_from_params_extracts_subagent_proxy_child_request_fields() {
        let params = serde_json::json!({
            "subagent_proxy": {
                "kind": "approval",
                "task_id": "t1"
            },
            "child_request": {
                "action": "process/start",
                "params": {
                    "approval": { "requirement": "prompt_strict" },
                    "argv": ["cargo", "test"],
                    "cwd": "/tmp/ws"
                }
            }
        });
        let summary = approval_summary_from_params(&params).expect("summary");
        assert_eq!(summary.requirement.as_deref(), Some("prompt_strict"));
        assert_eq!(
            summary.argv,
            Some(vec!["cargo".to_string(), "test".to_string()])
        );
        assert_eq!(summary.cwd.as_deref(), Some("/tmp/ws"));
        assert_eq!(summary.tool.as_deref(), Some("process/start"));
    }

    #[test]
    fn approval_summary_from_params_with_context_includes_approve_cmd_for_proxy() {
        let thread_id = ThreadId::new();
        let approval_id = ApprovalId::new();
        let params = serde_json::json!({
            "subagent_proxy": {
                "kind": "approval",
                "task_id": "t1"
            },
            "child_request": {
                "action": "process/start",
                "params": {
                    "approval": { "requirement": "prompt_strict" },
                    "argv": ["cargo", "test"],
                    "cwd": "/tmp/ws"
                }
            }
        });
        let summary = approval_summary_from_params_with_context(
            Some(thread_id),
            Some(approval_id),
            Some("subagent/proxy_approval"),
            &params,
        )
        .expect("summary");
        let expected_cmd = format!("omne approval decide {thread_id} {approval_id} --approve");
        assert_eq!(summary.approve_cmd.as_deref(), Some(expected_cmd.as_str()));
        let expected_deny_cmd = format!("omne approval decide {thread_id} {approval_id} --deny");
        assert_eq!(
            summary.deny_cmd.as_deref(),
            Some(expected_deny_cmd.as_str())
        );
    }

    #[test]
    fn fan_out_auto_apply_summary_reports_error_and_recovery_command_preview() {
        let payload = omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t-auto-apply".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: None,
            isolated_write_patch: None,
            isolated_write_handoff: None,
            isolated_write_auto_apply: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                    enabled: true,
                    attempted: true,
                    applied: false,
                    workspace_cwd: None,
                    target_workspace_cwd: None,
                    check_argv: vec![],
                    apply_argv: vec![],
                    patch_artifact_id: Some("artifact-7".to_string()),
                    patch_read_cmd: None,
                    failure_stage: Some(
                        omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
                    ),
                    recovery_hint: None,
                    recovery_commands: vec![
                        omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData {
                            label: "read_patch_artifact".to_string(),
                            argv: vec![
                                "omne".to_string(),
                                "artifact".to_string(),
                                "read".to_string(),
                                "thread-1".to_string(),
                                "artifact-7".to_string(),
                            ],
                        },
                    ],
                    error: Some("git apply --check failed: patch does not apply".to_string()),
                },
            ),
            status: "completed".to_string(),
            reason: None,
        };

        let summary = fan_out_auto_apply_summary_from_payload(&payload).expect("summary");
        assert_eq!(summary.task_id, "t-auto-apply");
        assert_eq!(summary.status, "error");
        assert_eq!(summary.stage.as_deref(), Some("check_patch"));
        assert_eq!(summary.patch_artifact_id.as_deref(), Some("artifact-7"));
        assert_eq!(summary.recovery_commands, Some(1));
        assert_eq!(
            summary.recovery_1.as_deref(),
            Some("read_patch_artifact: omne artifact read thread-1 artifact-7")
        );

        let text = format_fan_out_auto_apply_summary(&summary);
        assert!(text.contains("task_id=t-auto-apply"));
        assert!(text.contains("status=error"));
        assert!(text.contains("stage=check_patch"));
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_out_auto_apply_when_present() -> anyhow::Result<()> {
        let t1 = test_thread_meta(false, false, false);
        let t2 = test_thread_meta(false, false, false);
        let mut auto_apply_summaries = std::collections::BTreeMap::new();
        auto_apply_summaries.insert(
            t1.thread_id,
            FanOutAutoApplyInboxSummary {
                task_id: "t-auto-apply".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: Some("artifact-7".to_string()),
                recovery_commands: Some(1),
                recovery_1: Some("read_patch_artifact: omne artifact read".to_string()),
                error: Some("git apply --check failed".to_string()),
            },
        );
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1, &t2],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0]["fan_out_auto_apply"]["task_id"].as_str(),
            Some("t-auto-apply")
        );
        assert!(rows[1]["fan_out_auto_apply"].is_null());
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_includes_token_budget_snapshot_fields() -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_remaining = Some(0);
        t1.token_budget_utilization = Some(1.25);
        t1.token_budget_exceeded = Some(true);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["token_budget_limit"].as_u64(), Some(200));
        assert_eq!(rows[0]["token_budget_remaining"].as_u64(), Some(0));
        assert_eq!(rows[0]["token_budget_exceeded"].as_bool(), Some(true));
        assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(false));
        let utilization = rows[0]["token_budget_utilization"]
            .as_f64()
            .expect("token_budget_utilization should be numeric");
        assert!((utilization - 1.25).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_includes_token_budget_warning_active_when_threshold_reached()
    -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_remaining = Some(10);
        t1.token_budget_utilization = Some(1.0);
        t1.token_budget_exceeded = Some(false);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(true));
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_prefers_server_token_budget_warning_active() -> anyhow::Result<()>
    {
        let mut t1 = test_thread_meta(false, false, false);
        t1.token_budget_limit = Some(200);
        t1.token_budget_remaining = Some(190);
        t1.token_budget_utilization = Some(0.05);
        t1.token_budget_exceeded = Some(false);
        t1.token_budget_warning_active = Some(true);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(true));
        Ok(())
    }

    #[test]
    fn fan_in_dependency_blocked_summary_reports_blocker_details() {
        let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
            thread_id: "thread-1".to_string(),
            task_count: 2,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            tasks: vec![
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t-upstream".to_string(),
                    title: "upstream".to_string(),
                    thread_id: Some("thread-upstream".to_string()),
                    turn_id: Some("turn-upstream".to_string()),
                    status: "Failed".to_string(),
                    reason: Some("unit tests failed".to_string()),
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t-dependent".to_string(),
                    title: "dependent".to_string(),
                    thread_id: None,
                    turn_id: None,
                    status: "Cancelled".to_string(),
                    reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
                    dependency_blocked: true,
                    dependency_blocker_task_id: Some("t-upstream".to_string()),
                    dependency_blocker_status: Some("Failed".to_string()),
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
            ],
        };

        let summary =
            fan_in_dependency_blocked_summary_from_payload(&payload).expect("blocked summary");
        assert_eq!(summary.task_id, "t-dependent");
        assert_eq!(summary.status, "Cancelled");
        assert_eq!(summary.dependency_blocked_count, 1);
        assert_eq!(summary.task_count, 2);
        assert_eq!(summary.dependency_blocked_ratio, 0.5);
        assert_eq!(summary.blocker_task_id.as_deref(), Some("t-upstream"));
        assert_eq!(summary.blocker_status.as_deref(), Some("Failed"));
        assert_eq!(
            summary.reason.as_deref(),
            Some("blocked by dependency: t-upstream status=Failed")
        );
        assert!(summary.diagnostics_tasks.is_none());

        let text = format_fan_in_dependency_blocked_summary(&summary);
        assert!(text.contains(
            "fan_in_dependency_blocker: task_id=t-dependent status=Cancelled blocked=1/2"
        ));
        assert!(text.contains("blocker_task_id=t-upstream"));
        assert!(text.contains("blocker_status=Failed"));
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_in_dependency_blocker_when_present()
    -> anyhow::Result<()> {
        let t1 = test_thread_meta(false, false, false);
        let t2 = test_thread_meta(false, false, false);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let mut fan_in_blockers = std::collections::BTreeMap::new();
        fan_in_blockers.insert(
            t1.thread_id,
            FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
            },
        );
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();
        let rows = render_inbox_json_threads(
            [&t1, &t2],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0]["fan_in_dependency_blocker"]["task_id"].as_str(),
            Some("t-dependent")
        );
        assert!(rows[1]["fan_in_dependency_blocker"].is_null());
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_in_result_diagnostics_when_enabled()
    -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.has_fan_in_result_diagnostics = true;
        t1.fan_in_result_diagnostics = Some(FanInResultDiagnosticsInboxSummary {
            task_count: 2,
            diagnostics_tasks: 2,
            diagnostics_matched_completion_total: 5,
            diagnostics_pending_matching_tool_ids_total: 1,
            diagnostics_scan_last_seq_max: 50,
        });
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let subagent_pending = std::collections::BTreeMap::new();

        let rows_with_details = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(
            rows_with_details[0]["fan_in_result_diagnostics"]["diagnostics_tasks"].as_u64(),
            Some(2)
        );

        let rows_without_details = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            false,
        )?;
        assert!(rows_without_details[0]["fan_in_result_diagnostics"].is_null());
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_attaches_fan_in_result_diagnostics_from_collected_summaries()
    -> anyhow::Result<()> {
        let mut t1 = test_thread_meta(false, false, false);
        t1.has_fan_in_result_diagnostics = true;
        t1.fan_in_result_diagnostics = None;
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let mut fan_in_diagnostics = std::collections::BTreeMap::new();
        fan_in_diagnostics.insert(
            t1.thread_id,
            FanInResultDiagnosticsInboxSummary {
                task_count: 3,
                diagnostics_tasks: 2,
                diagnostics_matched_completion_total: 6,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 77,
            },
        );
        let subagent_pending = std::collections::BTreeMap::new();

        let rows = render_inbox_json_threads(
            [&t1],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(
            rows[0]["fan_in_result_diagnostics"]["diagnostics_scan_last_seq_max"].as_u64(),
            Some(77)
        );
        Ok(())
    }

    #[test]
    fn render_inbox_json_threads_attaches_subagent_pending_when_present() -> anyhow::Result<()> {
        let t1 = test_thread_meta(false, false, false);
        let t2 = test_thread_meta(false, false, false);
        let auto_apply_summaries = std::collections::BTreeMap::new();
        let fan_in_blockers = std::collections::BTreeMap::new();
        let fan_in_diagnostics = std::collections::BTreeMap::new();
        let mut subagent_pending = std::collections::BTreeMap::new();
        subagent_pending.insert(
            t1.thread_id,
            SubagentPendingApprovalsSummary {
                total: 3,
                states: std::collections::BTreeMap::from([
                    ("running".to_string(), 2),
                    ("done".to_string(), 1),
                ]),
            },
        );
        let rows = render_inbox_json_threads(
            [&t1, &t2],
            &auto_apply_summaries,
            &fan_in_blockers,
            &fan_in_diagnostics,
            &subagent_pending,
            true,
        )?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["subagent_pending"]["total"].as_u64(), Some(3));
        assert_eq!(
            rows[0]["subagent_pending"]["states"]["running"].as_u64(),
            Some(2)
        );
        assert_eq!(rows[1]["subagent_pending"].as_object(), None);
        Ok(())
    }

    #[test]
    fn build_inbox_json_output_omits_summary_cache_stats_when_absent() -> anyhow::Result<()> {
        let output = build_inbox_json_output(1, 2, vec![], None)?;
        assert_eq!(output["prev_count"].as_u64(), Some(1));
        assert_eq!(output["cur_count"].as_u64(), Some(2));
        assert!(output["threads"].as_array().is_some_and(|rows| rows.is_empty()));
        assert!(output.get("summary_cache_stats").is_none());
        Ok(())
    }

    #[test]
    fn build_inbox_json_output_includes_summary_cache_stats_when_present() -> anyhow::Result<()> {
        let stats = InboxSummaryCacheStats {
            fan_out_meta: 1,
            fan_out_cache_some: 2,
            fan_out_cache_none: 3,
            fan_out_attention: 4,
            fan_out_fetch_some: 5,
            fan_out_fetch_none: 6,
            fan_in_meta: 7,
            fan_in_cache_some: 8,
            fan_in_cache_none: 9,
            fan_in_attention: 10,
            fan_in_fetch_some: 11,
            fan_in_fetch_none: 12,
            fan_in_skip_unblocked: 13,
            fan_in_diag_meta: 14,
            fan_in_diag_cache_some: 15,
            fan_in_diag_cache_none: 16,
            fan_in_diag_attention: 17,
            fan_in_diag_fetch_some: 18,
            fan_in_diag_fetch_none: 19,
            fan_in_diag_skip_absent: 20,
            subagent_meta: 21,
            subagent_cache_some: 22,
            subagent_cache_none: 23,
            subagent_attention_some: 24,
            subagent_attention_none: 25,
            subagent_fetch_some: 26,
            subagent_fetch_none: 27,
            subagent_skip_no_pending: 28,
        };
        let output = build_inbox_json_output(3, 4, vec![], Some(&stats))?;
        assert_eq!(output["summary_cache_stats"]["fan_out_meta"].as_u64(), Some(1));
        assert_eq!(output["summary_cache_stats"]["fan_in_meta"].as_u64(), Some(7));
        assert_eq!(output["summary_cache_stats"]["fan_in_diag_meta"].as_u64(), Some(14));
        assert_eq!(output["summary_cache_stats"]["subagent_meta"].as_u64(), Some(21));
        assert_eq!(
            output["summary_cache_stats"]["subagent_skip_no_pending"].as_u64(),
            Some(28)
        );
        Ok(())
    }

    #[test]
    fn watch_detail_summary_lines_include_auto_apply_and_fan_in_blocker() {
        let auto_apply = FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: Some("git apply failed".to_string()),
        };
        let fan_in_blocker = FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency".to_string()),
        };
        let lines =
            watch_detail_summary_lines(Some(&auto_apply), Some(&fan_in_blocker), None, None);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("summary: fan_out_auto_apply: task_id=t-auto status=error"));
        assert!(lines[1].contains(
            "summary: fan_in_dependency_blocker: task_id=t-dependent status=Cancelled blocked=1/2"
        ));
    }

    #[test]
    fn watch_detail_summary_lines_is_empty_when_no_summaries() {
        let lines = watch_detail_summary_lines(None, None, None, None);
        assert!(lines.is_empty());
    }

    #[test]
    fn format_inbox_summary_cache_stats_includes_all_counters() {
        let stats = InboxSummaryCacheStats {
            fan_out_meta: 1,
            fan_out_cache_some: 2,
            fan_out_cache_none: 3,
            fan_out_attention: 4,
            fan_out_fetch_some: 5,
            fan_out_fetch_none: 6,
            fan_in_meta: 7,
            fan_in_cache_some: 8,
            fan_in_cache_none: 9,
            fan_in_attention: 10,
            fan_in_fetch_some: 11,
            fan_in_fetch_none: 12,
            fan_in_skip_unblocked: 13,
            fan_in_diag_meta: 14,
            fan_in_diag_cache_some: 15,
            fan_in_diag_cache_none: 16,
            fan_in_diag_attention: 17,
            fan_in_diag_fetch_some: 18,
            fan_in_diag_fetch_none: 19,
            fan_in_diag_skip_absent: 20,
            subagent_meta: 21,
            subagent_cache_some: 22,
            subagent_cache_none: 23,
            subagent_attention_some: 24,
            subagent_attention_none: 25,
            subagent_fetch_some: 26,
            subagent_fetch_none: 27,
            subagent_skip_no_pending: 28,
        };
        let line = format_inbox_summary_cache_stats(3, 20, 21, &stats);
        assert!(line.contains("iter=3 prev=20 cur=21"));
        assert!(line.contains(
            "fan_out(meta=1,cache_some=2,cache_none=3,attention=4,fetch_some=5,fetch_none=6)"
        ));
        assert!(line.contains(
            "fan_in(meta=7,cache_some=8,cache_none=9,attention=10,fetch_some=11,fetch_none=12,skip_unblocked=13)"
        ));
        assert!(line.contains(
            "fan_in_diag(meta=14,cache_some=15,cache_none=16,attention=17,fetch_some=18,fetch_none=19,skip_absent=20)"
        ));
        assert!(line.contains(
            "subagent(meta=21,cache_some=22,cache_none=23,attention_some=24,attention_none=25,fetch_some=26,fetch_none=27,skip_no_pending=28)"
        ));
    }

    #[test]
    fn format_watch_summary_refresh_debug_renders_sources() {
        let line = format_watch_summary_refresh_debug(
            7,
            4,
            true,
            false,
            true,
            true,
            SummarySource::Attention,
            SummarySource::Previous,
            SummarySource::Artifact,
            SummarySource::None,
        );
        assert!(line.contains("iter=7 events=4"));
        assert!(line.contains("auto_apply(refresh=true,source=attention)"));
        assert!(line.contains("fan_in(refresh=false,source=previous)"));
        assert!(line.contains("fan_in_diag(refresh=true,source=artifact)"));
        assert!(line.contains("subagent(refresh=true,source=none)"));
    }

    #[test]
    fn build_watch_summary_refresh_debug_json_row_renders_sources() {
        let row = build_watch_summary_refresh_debug_json_row(
            7,
            4,
            true,
            false,
            true,
            true,
            SummarySource::Attention,
            SummarySource::Previous,
            SummarySource::Artifact,
            SummarySource::None,
        );
        assert_eq!(row["kind"].as_str(), Some("watch_summary_refresh_debug"));
        assert_eq!(row["iteration"].as_u64(), Some(7));
        assert_eq!(row["event_count"].as_u64(), Some(4));
        assert_eq!(row["auto_apply"]["refresh"].as_bool(), Some(true));
        assert_eq!(row["auto_apply"]["source"].as_str(), Some("attention"));
        assert_eq!(row["fan_in"]["refresh"].as_bool(), Some(false));
        assert_eq!(row["fan_in"]["source"].as_str(), Some("previous"));
        assert_eq!(row["fan_in_diagnostics"]["refresh"].as_bool(), Some(true));
        assert_eq!(
            row["fan_in_diagnostics"]["source"].as_str(),
            Some("artifact")
        );
        assert_eq!(row["subagent"]["refresh"].as_bool(), Some(true));
        assert_eq!(row["subagent"]["source"].as_str(), Some("none"));
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_cleared_marker() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "summary: fan_out_auto_apply: cleared");
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_only_changed_summary() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "attempted_not_applied".to_string(),
                stage: Some("apply_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: None,
            }),
            fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency".to_string()),
            }),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: previous.fan_in_blocker.clone(),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };

        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("fan_out_auto_apply: task_id=t-auto status=error"));
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_subagent_pending_summary() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 1,
                states: std::collections::BTreeMap::from([("running".to_string(), 1)]),
            }),
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 2,
                states: std::collections::BTreeMap::from([
                    ("done".to_string(), 1),
                    ("running".to_string(), 1),
                ]),
            }),
        };

        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("subagent_pending: total=2"));
        assert!(lines[0].contains("done:1"));
    }

    #[test]
    fn watch_detail_summary_lines_with_delta_emits_fan_in_result_diagnostics_summary() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 2,
                diagnostics_tasks: 2,
                diagnostics_matched_completion_total: 5,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 50,
            }),
            subagent_pending: None,
        };

        let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("fan_in_result_diagnostics: tasks=2 diagnostics_tasks=2"));
        assert!(lines[0].contains("matched_completion_total=5"));
    }

    #[test]
    fn watch_detail_summary_json_rows_include_auto_apply_and_fan_in_blocker() {
        let thread_id = ThreadId::new();
        let thread_id_text = thread_id.to_string();
        let auto_apply = FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: Some("git apply failed".to_string()),
        };
        let fan_in_blocker = FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency".to_string()),
        };
        let rows = watch_detail_summary_json_rows(
            thread_id,
            Some(&auto_apply),
            Some(&fan_in_blocker),
            None,
            None,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"].as_str(), Some("watch_detail_summary"));
        assert_eq!(rows[0]["thread_id"].as_str(), Some(thread_id_text.as_str()));
        assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_out_auto_apply"));
        assert_eq!(rows[0]["payload"]["task_id"].as_str(), Some("t-auto"));
        assert_eq!(rows[1]["summary_type"].as_str(), Some("fan_in_dependency_blocker"));
        assert_eq!(rows[1]["payload"]["task_id"].as_str(), Some("t-dependent"));
        assert_eq!(rows[1]["payload"]["dependency_blocked_count"].as_u64(), Some(1));
        assert_eq!(rows[1]["payload"]["task_count"].as_u64(), Some(2));
        assert_eq!(
            rows[1]["payload"]["dependency_blocked_ratio"].as_f64(),
            Some(0.5)
        );
    }

    #[test]
    fn watch_detail_summary_json_rows_is_empty_when_no_summaries() {
        let rows = watch_detail_summary_json_rows(ThreadId::new(), None, None, None, None);
        assert!(rows.is_empty());
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_emits_cleared_marker() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency".to_string()),
            }),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["kind"].as_str(), Some("watch_detail_summary"));
        assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_in_dependency_blocker"));
        assert_eq!(rows[0]["cleared"].as_bool(), Some(true));
        assert_eq!(rows[0]["changed_fields"][0].as_str(), Some("cleared"));
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_includes_changed_fields() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "attempted_not_applied".to_string(),
                stage: Some("apply_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: None,
            }),
            fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
                task_id: "t-dependent".to_string(),
                status: "Cancelled".to_string(),
                dependency_blocked_count: 1,
                task_count: 2,
                dependency_blocked_ratio: 0.5,
                diagnostics_tasks: None,
                diagnostics_matched_completion_total: None,
                diagnostics_pending_matching_tool_ids_total: None,
                diagnostics_scan_last_seq_max: None,
                blocker_task_id: Some("t-upstream".to_string()),
                blocker_status: Some("Failed".to_string()),
                reason: Some("blocked by dependency".to_string()),
            }),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: previous.fan_in_blocker.clone(),
            fan_in_diagnostics: None,
            subagent_pending: None,
        };

        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_out_auto_apply"));
        let changed_fields = rows[0]["changed_fields"]
            .as_array()
            .expect("changed_fields array");
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("status")));
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("stage")));
    }

    #[test]
    fn format_fan_in_dependency_blocked_summary_includes_diagnostics_fields() {
        let summary = FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
            diagnostics_tasks: Some(2),
            diagnostics_matched_completion_total: Some(5),
            diagnostics_pending_matching_tool_ids_total: Some(1),
            diagnostics_scan_last_seq_max: Some(50),
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency".to_string()),
        };
        let text = format_fan_in_dependency_blocked_summary(&summary);
        assert!(text.contains("diagnostics_tasks=2"));
        assert!(text.contains("diagnostics_matched_completion_total=5"));
        assert!(text.contains("diagnostics_pending_matching_tool_ids_total=1"));
        assert!(text.contains("diagnostics_scan_last_seq_max=50"));
    }

    #[test]
    fn fan_in_result_diagnostics_summary_reports_without_dependency_blocker() {
        let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
            thread_id: "thread-1".to_string(),
            task_count: 1,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            tasks: vec![omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t1".to_string(),
                title: "first".to_string(),
                thread_id: Some("thread-subagent".to_string()),
                turn_id: Some("turn-subagent".to_string()),
                status: "Completed".to_string(),
                reason: Some("done".to_string()),
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: Some("artifact-1".to_string()),
                result_artifact_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: Some(
                    omne_app_server_protocol::ArtifactFanInSummaryResultArtifactDiagnostics {
                        scan_last_seq: 42,
                        matched_completion_count: 2,
                        pending_matching_tool_ids: 1,
                    },
                ),
                pending_approval: None,
            }],
        };

        let diagnostics = fan_in_result_diagnostics_summary_from_payload(&payload)
            .expect("fan-in diagnostics summary");
        assert_eq!(diagnostics.task_count, 1);
        assert_eq!(diagnostics.diagnostics_tasks, 1);
        assert_eq!(diagnostics.diagnostics_matched_completion_total, 2);
        assert_eq!(diagnostics.diagnostics_pending_matching_tool_ids_total, 1);
        assert_eq!(diagnostics.diagnostics_scan_last_seq_max, 42);

        let text = format_fan_in_result_diagnostics_summary(&diagnostics);
        assert!(text.contains("fan_in_result_diagnostics: tasks=1 diagnostics_tasks=1"));
        assert!(text.contains("matched_completion_total=2"));
        assert!(text.contains("pending_matching_tool_ids_total=1"));
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_includes_subagent_pending_changes() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 1,
                states: std::collections::BTreeMap::from([("running".to_string(), 1)]),
            }),
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: Some(SubagentPendingApprovalsSummary {
                total: 2,
                states: std::collections::BTreeMap::from([
                    ("done".to_string(), 1),
                    ("running".to_string(), 1),
                ]),
            }),
        };

        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["summary_type"].as_str(), Some("subagent_pending"));
        assert_eq!(rows[0]["payload"]["total"].as_u64(), Some(2));
        let changed_fields = rows[0]["changed_fields"]
            .as_array()
            .expect("changed_fields array");
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("total")));
        assert!(changed_fields.iter().any(|value| value.as_str() == Some("states")));
    }

    #[test]
    fn watch_detail_summary_json_rows_with_delta_includes_fan_in_result_diagnostics_changes() {
        let thread_id = ThreadId::new();
        let previous = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 2,
                diagnostics_tasks: 1,
                diagnostics_matched_completion_total: 2,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 42,
            }),
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 2,
                diagnostics_tasks: 2,
                diagnostics_matched_completion_total: 5,
                diagnostics_pending_matching_tool_ids_total: 0,
                diagnostics_scan_last_seq_max: 50,
            }),
            subagent_pending: None,
        };

        let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]["summary_type"].as_str(),
            Some("fan_in_result_diagnostics")
        );
        let changed_fields = rows[0]["changed_fields"]
            .as_array()
            .expect("changed_fields array");
        assert!(
            changed_fields
                .iter()
                .any(|value| value.as_str() == Some("diagnostics_tasks"))
        );
        assert!(
            changed_fields.iter().any(|value| {
                value.as_str() == Some("diagnostics_matched_completion_total")
            })
        );
    }

    #[test]
    fn should_emit_watch_detail_summary_emits_for_fan_in_result_diagnostics_only() {
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
                task_count: 1,
                diagnostics_tasks: 1,
                diagnostics_matched_completion_total: 2,
                diagnostics_pending_matching_tool_ids_total: 1,
                diagnostics_scan_last_seq_max: 42,
            }),
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(None, &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_emits_first_non_empty_snapshot() {
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(None, &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_suppresses_unchanged_snapshot() {
        let snapshot = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(!should_emit_watch_detail_summary(Some(&snapshot), &snapshot));
    }

    #[test]
    fn should_emit_watch_detail_summary_emits_when_snapshot_changes() {
        let previous = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "attempted_not_applied".to_string(),
                stage: Some("apply_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: None,
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let current = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(Some(&previous), &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_suppresses_empty_snapshot() {
        let current = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(!should_emit_watch_detail_summary(None, &current));
    }

    #[test]
    fn should_emit_watch_detail_summary_re_emits_after_empty_gap() {
        let non_empty = WatchDetailSummarySnapshot {
            auto_apply: Some(FanOutAutoApplyInboxSummary {
                task_id: "t-auto".to_string(),
                status: "error".to_string(),
                stage: Some("check_patch".to_string()),
                patch_artifact_id: None,
                recovery_commands: None,
                recovery_1: None,
                error: Some("git apply failed".to_string()),
            }),
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        let empty = WatchDetailSummarySnapshot {
            auto_apply: None,
            fan_in_blocker: None,
            fan_in_diagnostics: None,
            subagent_pending: None,
        };
        assert!(should_emit_watch_detail_summary(Some(&non_empty), &empty));
        assert!(!should_emit_watch_detail_summary(Some(&empty), &empty));
        assert!(should_emit_watch_detail_summary(Some(&empty), &non_empty));
    }

    #[test]
    fn fan_out_auto_apply_summary_omits_applied_payload() {
        let payload = omne_app_server_protocol::ArtifactFanOutResultStructuredData {
            schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id: "t-auto-apply".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            workspace_mode: "isolated_write".to_string(),
            workspace_cwd: None,
            isolated_write_patch: None,
            isolated_write_handoff: None,
            isolated_write_auto_apply: Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                    enabled: true,
                    attempted: true,
                    applied: true,
                    workspace_cwd: None,
                    target_workspace_cwd: None,
                    check_argv: vec![],
                    apply_argv: vec![],
                    patch_artifact_id: None,
                    patch_read_cmd: None,
                    failure_stage: None,
                    recovery_hint: None,
                    recovery_commands: vec![],
                    error: None,
                },
            ),
            status: "completed".to_string(),
            reason: None,
        };

        assert!(fan_out_auto_apply_summary_from_payload(&payload).is_none());
    }

    #[test]
    fn is_fan_out_auto_apply_error_matches_status() {
        let error = FanOutAutoApplyInboxSummary {
            task_id: "t1".to_string(),
            status: "error".to_string(),
            stage: None,
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: None,
        };
        let non_error = FanOutAutoApplyInboxSummary {
            task_id: "t2".to_string(),
            status: "attempted_not_applied".to_string(),
            stage: None,
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: None,
        };
        assert!(is_fan_out_auto_apply_error(&error));
        assert!(!is_fan_out_auto_apply_error(&non_error));
    }
}
