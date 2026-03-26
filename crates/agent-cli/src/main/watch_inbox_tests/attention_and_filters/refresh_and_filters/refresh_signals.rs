use super::*;

#[test]
fn should_refresh_watch_detail_summary_true_for_tool_completed() {
    let events = vec![event_with_kind(
        omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id: omne_protocol::ToolId::new(),
            status: omne_protocol::ToolStatus::Completed,
            structured_error: None,
            error: None,
            result: None,
        },
    )];
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
    let events = vec![event_with_kind(
        omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "ok".to_string(),
            model: None,
            response_id: None,
            token_usage: None,
        },
    )];
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
    assert!(!should_refresh_watch_fan_in_dependency_blocker_summary(
        &events
    ));
}

#[test]
fn should_refresh_watch_fan_in_summary_true_for_turn_completed() {
    let events = vec![event_with_kind(
        omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: TurnId::new(),
            status: TurnStatus::Completed,
            reason: None,
        },
    )];
    assert!(should_refresh_watch_fan_in_dependency_blocker_summary(
        &events
    ));
}

#[test]
fn should_refresh_watch_subagent_pending_summary_true_for_approval_requested() {
    let events = vec![event_with_kind(
        omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: ApprovalId::new(),
            turn_id: None,
            action: "subagent/proxy_approval".to_string(),
            params: serde_json::json!({}),
        },
    )];
    assert!(should_refresh_watch_subagent_pending_summary(&events));
    assert!(should_refresh_watch_detail_summary(&events));
}

#[test]
fn should_refresh_watch_subagent_pending_summary_true_for_turn_completed() {
    let events = vec![event_with_kind(
        omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: TurnId::new(),
            status: TurnStatus::Completed,
            reason: None,
        },
    )];
    assert!(should_refresh_watch_subagent_pending_summary(&events));
}
