use super::*;

#[test]
fn fan_in_dependency_blocked_summary_reports_blocker_details() {
    let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
        schema_version: omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
        thread_id: "thread-1".to_string(),
        task_count: 2,
        scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
            env_max_concurrent_subagents: 4,
            effective_concurrency_limit: 2,
            priority_aging_rounds: 3,
        },
        tasks: vec![
            omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t-upstream".to_string(),
                title: "upstream".to_string(),
                thread_id: Some("thread-upstream".to_string()),
                turn_id: Some("turn-upstream".to_string()),
                status: "Failed".to_string(),
                reason: Some("unit tests failed".to_string()),
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_structured_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: None,
            },
            omne_app_server_protocol::ArtifactFanInSummaryTask {
                task_id: "t-dependent".to_string(),
                title: "dependent".to_string(),
                thread_id: None,
                turn_id: None,
                status: "Cancelled".to_string(),
                reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
                dependency_blocked: true,
                dependency_blocker_task_id: Some("t-upstream".to_string()),
                dependency_blocker_status: Some("Failed".to_string()),
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_structured_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: None,
            },
        ],
    };

    let summary =
        fan_in_dependency_blocked_summary_from_payload(&payload).expect("blocked summary");
    assert_eq!(summary.task_id, "t-dependent");
    assert_eq!(summary.status, "Cancelled");
    assert_eq!(summary.dependency_blocked_count, 1);
    assert_eq!(summary.task_count, 2);
    assert_eq!(summary.dependency_blocked_ratio, 0.5);
    assert_eq!(summary.blocker_task_id.as_deref(), Some("t-upstream"));
    assert_eq!(summary.blocker_status.as_deref(), Some("Failed"));
    assert_eq!(
        summary.reason.as_deref(),
        Some("blocked by dependency: t-upstream status=Failed")
    );
    assert!(summary.diagnostics_tasks.is_none());

    let text = format_fan_in_dependency_blocked_summary(&summary);
    assert!(
        text.contains(
            "fan_in_dependency_blocker: task_id=t-dependent status=Cancelled blocked=1/2"
        )
    );
    assert!(text.contains("blocker_task_id=t-upstream"));
    assert!(text.contains("blocker_status=Failed"));
}

#[test]
fn render_inbox_json_threads_attaches_fan_in_dependency_blocker_when_present() -> anyhow::Result<()>
{
    let t1 = test_thread_meta(false, false, false);
    let t2 = test_thread_meta(false, false, false);
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let mut fan_in_blockers = std::collections::BTreeMap::new();
    fan_in_blockers.insert(
        t1.thread_id,
        FanInDependencyBlockedInboxSummary {
            task_id: "t-dependent".to_string(),
            status: "Cancelled".to_string(),
            dependency_blocked_count: 1,
            task_count: 2,
            dependency_blocked_ratio: 0.5,
            diagnostics_tasks: None,
            diagnostics_matched_completion_total: None,
            diagnostics_pending_matching_tool_ids_total: None,
            diagnostics_scan_last_seq_max: None,
            blocker_task_id: Some("t-upstream".to_string()),
            blocker_status: Some("Failed".to_string()),
            reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
        },
    );
    let fan_in_diagnostics = std::collections::BTreeMap::new();
    let subagent_pending = std::collections::BTreeMap::new();
    let rows = render_inbox_json_threads(
        [&t1, &t2],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0]["fan_in_dependency_blocker"]["task_id"].as_str(),
        Some("t-dependent")
    );
    assert!(rows[1]["fan_in_dependency_blocker"].is_null());
    Ok(())
}
