use super::*;

#[test]
fn pending_approval_commands_include_approve_and_deny_cmd() {
    let approval_id = ApprovalId::new();
    let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
        approval_id,
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
                approve_cmd: Some("omne approval decide thread-1 approval-1 --approve".to_string()),
                deny_cmd: None,
            },
        ),
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
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: None,
                    child_attention_state: Some("running".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                },
            ),
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
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: None,
                    child_attention_state: Some("FAILED".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                },
            ),
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
