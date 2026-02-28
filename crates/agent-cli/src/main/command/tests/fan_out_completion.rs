use super::*;

#[test]
fn validate_fan_out_results_blocks_non_completed_when_required() {
    let parent_thread_id = ThreadId::new();
    let error_artifact_id = ArtifactId::new();
    let results = vec![WorkflowTaskResult {
        task_id: "t1".to_string(),
        title: "task".to_string(),
        thread_id: None,
        turn_id: None,
        result_artifact_id: None,
        result_artifact_error: Some("result artifact write failed".to_string()),
        result_artifact_error_id: Some(error_artifact_id),
        status: TurnStatus::Failed,
        reason: Some("failed".to_string()),
        dependency_blocked: true,
        assistant_text: None,
        pending_approval: None,
    }];

    let err = validate_fan_out_results(
        &results,
        parent_thread_id,
        omne_protocol::ArtifactId::new(),
        true,
    )
    .unwrap_err();
    assert!(err.to_string().contains("fan-out task is not completed"));
    assert!(err.to_string().contains("thread_id=-"));
    assert!(err.to_string().contains("artifact_error=result artifact write failed"));
    assert!(err.to_string().contains(&format!(
        "artifact_error_read_cmd=omne artifact read {} {}",
        parent_thread_id, error_artifact_id
    )));
    assert!(
        validate_fan_out_results(
            &results,
            parent_thread_id,
            omne_protocol::ArtifactId::new(),
            false
        )
        .is_ok()
    );
}

#[test]
fn format_non_completed_fan_out_issue_includes_error_read_command() {
    let parent_thread_id = ThreadId::new();
    let artifact_id = ArtifactId::new();
    let error_artifact_id = ArtifactId::new();
    let result = WorkflowTaskResult {
        task_id: "t1".to_string(),
        title: "task".to_string(),
        thread_id: Some(ThreadId::new()),
        turn_id: Some(TurnId::new()),
        result_artifact_id: None,
        result_artifact_error: Some("write failed".to_string()),
        result_artifact_error_id: Some(error_artifact_id),
        status: TurnStatus::Failed,
        reason: None,
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: None,
    };

    let text =
        format_non_completed_fan_out_issue("fan-out linkage issue", &result, parent_thread_id, artifact_id);
    assert!(text.contains("fan-out linkage issue"));
    assert!(text.contains("artifact_error=write failed"));
    assert!(text.contains(&format!(
        "artifact_error_read_cmd=omne artifact read {} {}",
        parent_thread_id, error_artifact_id
    )));
    assert!(text.contains(&format!("fan_in_summary artifact_id={}", artifact_id)));
}

#[test]
fn format_non_completed_fan_out_issue_includes_pending_approval_handles() {
    let parent_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let turn_id = TurnId::new();
    let artifact_id = ArtifactId::new();
    let approval_id = ApprovalId::new();
    let result = WorkflowTaskResult {
        task_id: "t-approval".to_string(),
        title: "approval task".to_string(),
        thread_id: Some(child_thread_id),
        turn_id: Some(turn_id),
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Interrupted,
        reason: Some("blocked on approval".to_string()),
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: Some(WorkflowPendingApproval {
            approval_id,
            action: "subagent/proxy_approval".to_string(),
            summary: Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
            approve_cmd: Some(format!(
                "omne approval decide {} {} --approve",
                child_thread_id, approval_id
            )),
            deny_cmd: Some(format!(
                "omne approval decide {} {} --deny",
                child_thread_id, approval_id
            )),
        }),
    };

    let text =
        format_non_completed_fan_out_issue("fan-out linkage issue", &result, parent_thread_id, artifact_id);
    assert!(text.contains("pending_approval_action=subagent/proxy_approval"));
    assert!(text.contains(&format!("pending_approval_id={approval_id}")));
    assert!(text.contains("pending_approval_summary=child_thread_id=abc"));
    assert!(text.contains(&format!(
        "approve_cmd=omne approval decide {} {} --approve",
        child_thread_id, approval_id
    )));
    assert!(text.contains(&format!(
        "deny_cmd=omne approval decide {} {} --deny",
        child_thread_id, approval_id
    )));
}
