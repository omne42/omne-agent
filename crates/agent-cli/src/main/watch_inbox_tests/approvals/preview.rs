use super::*;

#[test]
fn pending_approval_preview_prefers_action_id_and_summary_hint() {
    let pending = omne_app_server_protocol::ThreadAttentionPendingApproval {
        approval_id: ApprovalId::new(),
        turn_id: None,
        action: Some("legacy/action".to_string()),
        action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ArtifactWrite),
        params: None,
        summary: Some(
            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
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
            },
        ),
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
        summary: Some(
            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
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
            },
        ),
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
        summary: Some(
            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: Some("prompt".to_string()),
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
            },
        ),
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
        summary: Some(
            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
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
            },
        ),
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
    let preview = format_pending_approval_preview(&pending);
    assert!(preview.contains("approve_cmd="));
    assert!(preview.contains("--approve"));
    assert!(preview.contains("deny_cmd="));
    assert!(preview.contains("--deny"));
}
