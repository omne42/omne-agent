use super::*;

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
        "approval": { "requirement": "prompt" },
        "target_path": "/tmp/ws/main.rs"
    });
    let summary = approval_summary_from_params(&params).expect("summary");
    assert_eq!(summary.requirement.as_deref(), Some("prompt"));
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
