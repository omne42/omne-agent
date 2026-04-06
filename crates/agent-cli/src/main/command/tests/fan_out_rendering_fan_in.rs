use super::*;

#[test]
fn render_fan_in_summary_markdown_includes_scheduling_section() {
    let thread_id = ThreadId::new();
    let results = vec![WorkflowTaskResult {
        task_id: "t-summary".to_string(),
        title: "summary task".to_string(),
        thread_id: Some(thread_id),
        turn_id: Some(TurnId::new()),
        result_artifact_id: Some(ArtifactId::new()),
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Completed,
        reason: None,
        dependency_blocked: false,
        assistant_text: Some("all good".to_string()),
        pending_approval: None,
    }];
    let text = render_fan_in_summary_markdown(thread_id, &results, test_scheduling(), None);
    assert!(text.contains("# Fan-in Summary"));
    assert!(text.contains("Tasks: 1"));
    assert!(text.contains("## Scheduling"));
    assert!(text.contains("env_max_concurrent_subagents: `4`"));
    assert!(text.contains("effective_concurrency_limit: `3`"));
    assert!(text.contains("priority_aging_rounds: `5`"));
}

#[test]
fn render_fan_in_summary_markdown_structured_data_includes_dependency_blocker_fields() {
    let thread_id = ThreadId::new();
    let results = vec![WorkflowTaskResult {
        task_id: "t-blocked".to_string(),
        title: "blocked task".to_string(),
        thread_id: None,
        turn_id: None,
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Cancelled,
        reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
        dependency_blocked: true,
        assistant_text: None,
        pending_approval: None,
    }];
    let text = render_fan_in_summary_markdown(thread_id, &results, test_scheduling(), None);
    assert!(text.contains("- dependency_blocked: true"));
    assert!(text.contains("- dependency_blocker_task_id: t-upstream"));
    assert!(text.contains("- dependency_blocker_status: Failed"));
    assert!(text.contains("\"dependency_blocked\": true"));
    assert!(text.contains("\"dependency_blocker_task_id\": \"t-upstream\""));
    assert!(text.contains("\"dependency_blocker_status\": \"Failed\""));
}

#[test]
fn render_fan_in_summary_markdown_includes_pending_approval_summary() {
    let parent_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let turn_id = TurnId::new();
    let approval_id = ApprovalId::new();
    let summary = "child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs";
    let results = vec![WorkflowTaskResult {
        task_id: "t-blocked".to_string(),
        title: "blocked task".to_string(),
        thread_id: Some(child_thread_id),
        turn_id: Some(turn_id),
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Interrupted,
        reason: Some("blocked on approval".to_string()),
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: Some(WorkflowPendingApproval {
            approval_id,
            action: "subagent/proxy_approval".to_string(),
            summary: Some(summary.to_string()),
            approve_cmd: Some(format!(
                "omne approval decide {} {} --approve",
                child_thread_id, approval_id
            )),
            deny_cmd: Some(format!(
                "omne approval decide {} {} --deny",
                child_thread_id, approval_id
            )),
        }),
    }];
    let text = render_fan_in_summary_markdown(parent_thread_id, &results, test_scheduling(), None);
    assert!(text.contains("pending_approval: action=subagent/proxy_approval"));
    assert!(text.contains(&format!("approval_id={approval_id}")));
    assert!(text.contains(summary));
    assert!(text.contains(&format!(
        "approve_cmd: `omne approval decide {} {} --approve`",
        child_thread_id, approval_id
    )));
    assert!(text.contains(&format!(
        "deny_cmd: `omne approval decide {} {} --deny`",
        child_thread_id, approval_id
    )));
    assert!(text.contains("## Structured Data"));
    assert!(text.contains("\"schema_version\": \"fan_in_summary.v1\""));
    assert!(text.contains(&format!("\"thread_id\": \"{}\"", parent_thread_id)));
    assert!(text.contains("\"task_count\": 1"));
    assert!(text.contains("\"scheduling\""));
    assert!(text.contains("\"env_max_concurrent_subagents\": 4"));
    assert!(text.contains("\"effective_concurrency_limit\": 3"));
    assert!(text.contains("\"priority_aging_rounds\": 5"));
    assert!(text.contains("\"pending_approval\""));
    assert!(text.contains(&format!(
        "\"approve_cmd\": \"omne approval decide {} {} --approve\"",
        child_thread_id, approval_id
    )));
    assert!(text.contains(&format!(
        "\"deny_cmd\": \"omne approval decide {} {} --deny\"",
        child_thread_id, approval_id
    )));
}

#[test]
fn render_fan_in_summary_markdown_structured_data_falls_back_to_generated_approval_commands() {
    let parent_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let approval_id = ApprovalId::new();
    let results = vec![WorkflowTaskResult {
        task_id: "t-blocked".to_string(),
        title: "blocked task".to_string(),
        thread_id: Some(child_thread_id),
        turn_id: Some(TurnId::new()),
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Interrupted,
        reason: Some("blocked on approval".to_string()),
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: Some(WorkflowPendingApproval {
            approval_id,
            action: "subagent/proxy_approval".to_string(),
            summary: None,
            approve_cmd: None,
            deny_cmd: None,
        }),
    }];
    let text = render_fan_in_summary_markdown(parent_thread_id, &results, test_scheduling(), None);
    assert!(text.contains("## Structured Data"));
    assert!(text.contains("\"schema_version\": \"fan_in_summary.v1\""));
    assert!(text.contains(&format!("\"thread_id\": \"{}\"", parent_thread_id)));
    assert!(text.contains(&format!(
        "\"approve_cmd\": \"omne approval decide {} {} --approve\"",
        child_thread_id, approval_id
    )));
    assert!(text.contains(&format!(
        "\"deny_cmd\": \"omne approval decide {} {} --deny\"",
        child_thread_id, approval_id
    )));
}
