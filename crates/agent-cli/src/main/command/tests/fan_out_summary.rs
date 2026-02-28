use super::*;

#[test]
fn first_non_completed_task_from_fan_in_summary_finds_first_non_completed() {
    let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
        schema_version: "fan_in_summary.v1".to_string(),
        thread_id: "thread-1".to_string(),
        task_count: 3,
        scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
            env_max_concurrent_subagents: 4,
            effective_concurrency_limit: 2,
            priority_aging_rounds: 3,
        },
        tasks: vec![
            omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t1".to_string(),
                title: "done".to_string(),
                thread_id: Some("child-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                status: "Completed".to_string(),
                reason: None,
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
                task_id: "t2".to_string(),
                title: "blocked".to_string(),
                thread_id: Some("child-2".to_string()),
                turn_id: Some("turn-2".to_string()),
                status: "NeedUserInput".to_string(),
                reason: Some("approval needed".to_string()),
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: Some("approval required".to_string()),
                result_artifact_error_id: Some("artifact-error-2".to_string()),
                result_artifact_diagnostics: None,
                pending_approval: None,
            },
            omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t3".to_string(),
                title: "failed".to_string(),
                thread_id: Some("child-3".to_string()),
                turn_id: Some("turn-3".to_string()),
                status: "Failed".to_string(),
                reason: None,
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: None,
            },
        ],
    };

    let task = first_non_completed_task_from_fan_in_summary(&payload).expect("task");
    assert_eq!(task.task_id, "t2");
}

#[test]
fn format_non_completed_fan_out_issue_from_structured_task_includes_pending_handles() {
    let parent_thread_id = ThreadId::new().to_string();
    let artifact_id = ArtifactId::new();
    let task = omne_app_server_protocol::ArtifactFanInSummaryTask {
        task_id: "t-approval".to_string(),
        title: "approval task".to_string(),
        thread_id: Some("child-thread-1".to_string()),
        turn_id: Some("child-turn-1".to_string()),
        status: "NeedUserInput".to_string(),
        reason: Some("blocked on approval".to_string()),
        dependency_blocked: true,
        dependency_blocker_task_id: Some("t-upstream".to_string()),
        dependency_blocker_status: Some("Failed".to_string()),
        result_artifact_id: None,
        result_artifact_error: Some("approval required".to_string()),
        result_artifact_error_id: Some("artifact-error-1".to_string()),
        result_artifact_diagnostics: None,
        pending_approval: Some(omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
            approval_id: "approval-1".to_string(),
            action: "subagent/proxy_approval".to_string(),
            summary: Some("child_thread_id=abc".to_string()),
            approve_cmd: Some("omne approval decide child-thread-1 approval-1 --approve".to_string()),
            deny_cmd: Some("omne approval decide child-thread-1 approval-1 --deny".to_string()),
        }),
    };

    let text = format_non_completed_fan_out_issue_from_structured_task(
        "fan-out task is not completed",
        &parent_thread_id,
        &task,
        artifact_id,
    );
    assert!(text.contains("fan-out task is not completed"));
    assert!(text.contains("task_id=t-approval"));
    assert!(text.contains("status=NeedUserInput"));
    assert!(text.contains("thread_id=child-thread-1"));
    assert!(text.contains("turn_id=child-turn-1"));
    assert!(text.contains("artifact_error=approval required"));
    assert!(text.contains(&format!(
        "artifact_error_read_cmd=omne artifact read {} artifact-error-1",
        parent_thread_id
    )));
    assert!(text.contains("pending_approval_action=subagent/proxy_approval"));
    assert!(text.contains("pending_approval_id=approval-1"));
    assert!(text.contains("pending_approval_summary=child_thread_id=abc"));
    assert!(text.contains("approve_cmd=omne approval decide child-thread-1 approval-1 --approve"));
    assert!(text.contains("deny_cmd=omne approval decide child-thread-1 approval-1 --deny"));
    assert!(text.contains("dependency_blocked=true"));
    assert!(text.contains("dependency_blocker_task_id=t-upstream"));
    assert!(text.contains("dependency_blocker_status=Failed"));
    assert!(text.contains(&format!("fan_in_summary artifact_id={}", artifact_id)));
}
