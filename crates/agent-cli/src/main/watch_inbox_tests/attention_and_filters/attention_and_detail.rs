use super::*;

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
    assert_eq!(
        attention_state_update(&event),
        Some("token_budget_exceeded")
    );
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
