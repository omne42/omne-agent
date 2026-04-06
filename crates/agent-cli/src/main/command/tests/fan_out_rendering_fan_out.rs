use super::*;

#[test]
fn render_fan_out_approval_blocked_markdown_contains_approve_command() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "file_write".to_string(),
        summary: None,
    };
    let text = render_fan_out_approval_blocked_markdown(3, &[], &issue, test_scheduling());
    assert!(text.contains("Status: blocked (need approval)"));
    assert!(text.contains("Progress: 0/3"));
    assert!(text.contains(&issue.approval_id.to_string()));
    assert!(text.contains(&issue.thread_id.to_string()));
    assert!(text.contains("omne approval decide"));
    assert!(text.contains(&format!(
        "omne approval decide {} {} --deny",
        issue.thread_id, issue.approval_id
    )));
    assert!(text.contains("## Scheduling"));
    assert!(text.contains("env_max_concurrent_subagents: `4`"));
    assert!(text.contains("effective_concurrency_limit: `3`"));
    assert!(text.contains("priority_aging_rounds: `5`"));
}

#[test]
fn render_fan_out_approval_blocked_markdown_includes_failed_task_quick_read() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "file_write".to_string(),
        summary: None,
    };
    let failed_thread_id = ThreadId::new();
    let failed_artifact_id = ArtifactId::new();
    let finished = vec![WorkflowTaskResult {
        task_id: "t-failed".to_string(),
        title: "failed task".to_string(),
        thread_id: Some(failed_thread_id),
        turn_id: Some(TurnId::new()),
        result_artifact_id: Some(failed_artifact_id),
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Failed,
        reason: Some("boom".to_string()),
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: None,
    }];

    let text = render_fan_out_approval_blocked_markdown(3, &finished, &issue, test_scheduling());
    assert!(text.contains("Failed task quick reads:"));
    assert!(text.contains("t-failed"));
    assert!(text.contains(&format!(
        "omne artifact read {} {}",
        failed_thread_id, failed_artifact_id
    )));
}

#[test]
fn render_fan_out_result_markdown_includes_status_and_output() {
    let turn_id = TurnId::new();
    let text = render_fan_out_result_markdown(
        "t-review",
        "Review API",
        turn_id,
        TurnStatus::Completed,
        Some("all checks passed"),
        Some("result body"),
    );
    assert!(text.contains("# Fan-out Result"));
    assert!(text.contains("task_id: `t-review`"));
    assert!(text.contains(&turn_id.to_string()));
    assert!(text.contains("status: `Completed`"));
    assert!(text.contains("all checks passed"));
    assert!(text.contains("result body"));
}

#[test]
fn render_fan_out_progress_markdown_includes_scheduling_section() {
    let finished = vec![WorkflowTaskResult {
        task_id: "t-done".to_string(),
        title: "done task".to_string(),
        thread_id: Some(ThreadId::new()),
        turn_id: Some(TurnId::new()),
        result_artifact_id: Some(ArtifactId::new()),
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Completed,
        reason: None,
        dependency_blocked: false,
        assistant_text: Some("ok".to_string()),
        pending_approval: None,
    }];
    let active = vec!["t-active"];
    let text = render_fan_out_progress_markdown(
        2,
        &finished,
        &active,
        std::time::Duration::from_secs(12),
        test_scheduling(),
    );
    assert!(text.contains("# Fan-out Progress"));
    assert!(text.contains("Progress: 1/2"));
    assert!(text.contains("## Scheduling"));
    assert!(text.contains("env_max_concurrent_subagents: `4`"));
    assert!(text.contains("effective_concurrency_limit: `3`"));
    assert!(text.contains("priority_aging_rounds: `5`"));
}

#[test]
fn render_fan_out_result_error_markdown_includes_context_fields() {
    let child_thread_id = ThreadId::new();
    let turn_id = TurnId::new();
    let text = render_fan_out_result_error_markdown(
        "t-review",
        "Review API",
        child_thread_id,
        turn_id,
        TurnStatus::Failed,
        Some("subagent failed"),
        "artifact/write rpc failed: timeout",
    );
    assert!(text.contains("# Fan-out Result Artifact Error"));
    assert!(text.contains("task_id: `t-review`"));
    assert!(text.contains("child_thread_id"));
    assert!(text.contains(&child_thread_id.to_string()));
    assert!(text.contains(&turn_id.to_string()));
    assert!(text.contains("status: `Failed`"));
    assert!(text.contains("subagent failed"));
    assert!(text.contains("artifact/write rpc failed"));
}

#[test]
fn render_fan_out_approval_blocked_markdown_includes_artifact_error_column() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "file_write".to_string(),
        summary: Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
    };
    let finished = vec![WorkflowTaskResult {
        task_id: "t-failed".to_string(),
        title: "failed task".to_string(),
        thread_id: Some(ThreadId::new()),
        turn_id: Some(TurnId::new()),
        result_artifact_id: None,
        result_artifact_error: Some("artifact/write rejected: denied".to_string()),
        result_artifact_structured_error: None,
        result_artifact_error_id: Some(ArtifactId::new()),
        status: TurnStatus::Failed,
        reason: Some("boom".to_string()),
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: None,
    }];

    let text = render_fan_out_approval_blocked_markdown(3, &finished, &issue, test_scheduling());
    assert!(text.contains("artifact_error="));
    assert!(text.contains("artifact/write rejected: denied"));
    assert!(text.contains("summary"));
    assert!(text.contains("child_thread_id=abc"));
}
