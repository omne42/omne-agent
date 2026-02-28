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
        lines.push(format!(
            "summary: {}",
            format_fan_out_auto_apply_summary(summary)
        ));
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
        summary: Some(
            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
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
            },
        ),
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

#[path = "watch_inbox_tests/attention_and_filters.rs"]
mod attention_and_filters;

#[path = "watch_inbox_tests/approvals.rs"]
mod approvals;

#[path = "watch_inbox_tests/inbox_json.rs"]
mod inbox_json;

#[path = "watch_inbox_tests/watch_detail_summary.rs"]
mod watch_detail_summary;
