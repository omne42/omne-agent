use super::*;

#[test]
fn fan_out_approval_error_contains_actionable_handles() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "process/start".to_string(),
        summary: None,
    };
    let artifact_id = omne_protocol::ArtifactId::new();
    let message = fan_out_approval_error(&issue, artifact_id);
    assert!(message.contains("fan-out task needs approval"));
    assert!(message.contains("approval_id="));
    assert!(message.contains("thread_id="));
    assert!(message.contains("turn_id="));
    assert!(message.contains("omne approval decide"));
    assert!(message.contains(&format!(
        "omne approval decide {} {} --approve",
        issue.thread_id, issue.approval_id
    )));
    assert!(message.contains(&format!(
        "omne approval decide {} {} --deny",
        issue.thread_id, issue.approval_id
    )));
    assert!(!message.contains("--thread-id"));
    assert!(message.contains(&artifact_id.to_string()));
}

#[test]
fn fan_out_approval_error_includes_summary_when_present() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "subagent/proxy_approval".to_string(),
        summary: Some(
            "child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string(),
        ),
    };
    let artifact_id = omne_protocol::ArtifactId::new();
    let message = fan_out_approval_error(&issue, artifact_id);
    assert!(message.contains("summary=child_thread_id=abc"));
    assert!(message.contains("path=/tmp/ws/main.rs"));
}

#[test]
fn find_pending_approval_task_from_fan_in_summary_prefers_approval_id_match() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "process/start".to_string(),
        summary: None,
    };
    let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
        schema_version: "fan_in_summary.v1".to_string(),
        thread_id: ThreadId::new().to_string(),
        task_count: 2,
        scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
            env_max_concurrent_subagents: 4,
            effective_concurrency_limit: 2,
            priority_aging_rounds: 3,
        },
        tasks: vec![
            omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t-review".to_string(),
                title: "title".to_string(),
                thread_id: Some("child-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                status: "NeedUserInput".to_string(),
                reason: None,
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_structured_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: Some(
                    omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                        approval_id: "not-this-one".to_string(),
                        action: "process/start".to_string(),
                        summary: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    },
                ),
            },
            omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "other-task".to_string(),
                title: "title".to_string(),
                thread_id: Some("child-2".to_string()),
                turn_id: Some("turn-2".to_string()),
                status: "NeedUserInput".to_string(),
                reason: None,
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_structured_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: Some(
                    omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                        approval_id: issue.approval_id.to_string(),
                        action: "process/start".to_string(),
                        summary: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    },
                ),
            },
        ],
    };

    let task = find_pending_approval_task_from_fan_in_summary(&payload, &issue)
        .expect("pending approval task");
    assert_eq!(task.task_id, "other-task");
}

#[test]
fn fan_out_approval_error_from_structured_task_prefers_structured_handles() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "process/start".to_string(),
        summary: Some("from_issue".to_string()),
    };
    let artifact_id = ArtifactId::new();
    let task = omne_app_server_protocol::ArtifactFanInSummaryTask {
        task_id: "t-approval".to_string(),
        title: "approval task".to_string(),
        thread_id: Some("child-thread-1".to_string()),
        turn_id: Some("child-turn-1".to_string()),
        status: "NeedUserInput".to_string(),
        reason: Some("blocked".to_string()),
        dependency_blocked: false,
        dependency_blocker_task_id: None,
        dependency_blocker_status: None,
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        result_artifact_diagnostics: None,
        pending_approval: Some(omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
            approval_id: "approval-1".to_string(),
            action: "subagent/proxy_approval".to_string(),
            summary: Some("child_thread_id=abc".to_string()),
            approve_cmd: Some("omne approval decide child-thread-1 approval-1 --approve".to_string()),
            deny_cmd: Some("omne approval decide child-thread-1 approval-1 --deny".to_string()),
        }),
    };

    let message = fan_out_approval_error_from_structured_task(&issue, artifact_id, &task);
    assert!(message.contains("task_id=t-approval"));
    assert!(message.contains("thread_id=child-thread-1"));
    assert!(message.contains("turn_id=child-turn-1"));
    assert!(message.contains("approval_id=approval-1"));
    assert!(message.contains("action=subagent/proxy_approval"));
    assert!(message.contains("summary=child_thread_id=abc"));
    assert!(
        message.contains("approve_cmd=`omne approval decide child-thread-1 approval-1 --approve`")
    );
    assert!(message.contains("deny_cmd=`omne approval decide child-thread-1 approval-1 --deny`"));
}

#[test]
fn fan_out_approval_error_from_structured_task_generates_missing_commands() {
    let issue = FanOutApprovalIssue {
        task_id: "t-review".to_string(),
        thread_id: ThreadId::new(),
        turn_id: TurnId::new(),
        approval_id: ApprovalId::new(),
        action: "process/start".to_string(),
        summary: None,
    };
    let artifact_id = ArtifactId::new();
    let task = omne_app_server_protocol::ArtifactFanInSummaryTask {
        task_id: "t-review".to_string(),
        title: "approval task".to_string(),
        thread_id: Some("child-thread-2".to_string()),
        turn_id: Some("child-turn-2".to_string()),
        status: "NeedUserInput".to_string(),
        reason: None,
        dependency_blocked: false,
        dependency_blocker_task_id: None,
        dependency_blocker_status: None,
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_structured_error: None,
        result_artifact_error_id: None,
        result_artifact_diagnostics: None,
        pending_approval: Some(omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
            approval_id: "approval-2".to_string(),
            action: "process/start".to_string(),
            summary: None,
            approve_cmd: None,
            deny_cmd: None,
        }),
    };

    let message = fan_out_approval_error_from_structured_task(&issue, artifact_id, &task);
    assert!(message.contains("approve_cmd=`omne approval decide child-thread-2 approval-2 --approve`"));
    assert!(message.contains("deny_cmd=`omne approval decide child-thread-2 approval-2 --deny`"));
}

#[test]
fn pending_approval_task_result_captures_structured_summary() {
    let thread_id = ThreadId::new();
    let turn_id = TurnId::new();
    let approval_id = ApprovalId::new();
    let result = pending_approval_task_result(
        "t-review".to_string(),
        "review task".to_string(),
        thread_id,
        turn_id,
        "subagent/proxy_approval".to_string(),
        approval_id,
        Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
    );

    assert_eq!(result.status, TurnStatus::Interrupted);
    assert_eq!(result.thread_id, Some(thread_id));
    assert_eq!(result.turn_id, Some(turn_id));
    assert!(result.result_artifact_id.is_none());
    assert!(result
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("blocked on approval")));
    let pending = result.pending_approval.expect("pending approval");
    assert_eq!(pending.approval_id, approval_id);
    assert_eq!(pending.action, "subagent/proxy_approval");
    assert!(pending
        .summary
        .as_deref()
        .is_some_and(|summary| summary.contains("child_thread_id=abc")));
    let expected_approve = format!("omne approval decide {} {} --approve", thread_id, approval_id);
    let expected_deny = format!("omne approval decide {} {} --deny", thread_id, approval_id);
    assert_eq!(pending.approve_cmd.as_deref(), Some(expected_approve.as_str()));
    assert_eq!(pending.deny_cmd.as_deref(), Some(expected_deny.as_str()));
}
