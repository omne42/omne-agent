use super::*;

#[test]
fn format_fan_in_dependency_blocked_summary_includes_diagnostics_fields() {
    let summary = FanInDependencyBlockedInboxSummary {
        task_id: "t-dependent".to_string(),
        status: "Cancelled".to_string(),
        dependency_blocked_count: 1,
        task_count: 2,
        dependency_blocked_ratio: 0.5,
        diagnostics_tasks: Some(2),
        diagnostics_matched_completion_total: Some(5),
        diagnostics_pending_matching_tool_ids_total: Some(1),
        diagnostics_scan_last_seq_max: Some(50),
        blocker_task_id: Some("t-upstream".to_string()),
        blocker_status: Some("Failed".to_string()),
        reason: Some("blocked by dependency".to_string()),
    };
    let text = format_fan_in_dependency_blocked_summary(&summary);
    assert!(text.contains("diagnostics_tasks=2"));
    assert!(text.contains("diagnostics_matched_completion_total=5"));
    assert!(text.contains("diagnostics_pending_matching_tool_ids_total=1"));
    assert!(text.contains("diagnostics_scan_last_seq_max=50"));
}

#[test]
fn fan_in_result_diagnostics_summary_reports_without_dependency_blocker() {
    let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
        schema_version: omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
        thread_id: "thread-1".to_string(),
        task_count: 1,
        scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
            env_max_concurrent_subagents: 4,
            effective_concurrency_limit: 2,
            priority_aging_rounds: 3,
        },
        tasks: vec![omne_app_server_protocol::ArtifactFanInSummaryTask {
            task_id: "t1".to_string(),
            title: "first".to_string(),
            thread_id: Some("thread-subagent".to_string()),
            turn_id: Some("turn-subagent".to_string()),
            status: "Completed".to_string(),
            reason: Some("done".to_string()),
            dependency_blocked: false,
            dependency_blocker_task_id: None,
            dependency_blocker_status: None,
            result_artifact_id: Some("artifact-1".to_string()),
            result_artifact_error: None,
            result_artifact_structured_error: None,
            result_artifact_error_id: None,
            result_artifact_diagnostics: Some(
                omne_app_server_protocol::ArtifactFanInSummaryResultArtifactDiagnostics {
                    scan_last_seq: 42,
                    matched_completion_count: 2,
                    pending_matching_tool_ids: 1,
                },
            ),
            pending_approval: None,
        }],
    };

    let diagnostics = fan_in_result_diagnostics_summary_from_payload(&payload)
        .expect("fan-in diagnostics summary");
    assert_eq!(diagnostics.task_count, 1);
    assert_eq!(diagnostics.diagnostics_tasks, 1);
    assert_eq!(diagnostics.diagnostics_matched_completion_total, 2);
    assert_eq!(diagnostics.diagnostics_pending_matching_tool_ids_total, 1);
    assert_eq!(diagnostics.diagnostics_scan_last_seq_max, 42);

    let text = format_fan_in_result_diagnostics_summary(&diagnostics);
    assert!(text.contains("fan_in_result_diagnostics: tasks=1 diagnostics_tasks=1"));
    assert!(text.contains("matched_completion_total=2"));
    assert!(text.contains("pending_matching_tool_ids_total=1"));
}

#[test]
fn watch_detail_summary_json_rows_with_delta_includes_subagent_pending_changes() {
    let thread_id = ThreadId::new();
    let previous = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: Some(SubagentPendingApprovalsSummary {
            total: 1,
            states: std::collections::BTreeMap::from([("running".to_string(), 1)]),
        }),
    };
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: Some(SubagentPendingApprovalsSummary {
            total: 2,
            states: std::collections::BTreeMap::from([
                ("done".to_string(), 1),
                ("running".to_string(), 1),
            ]),
        }),
    };

    let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["summary_type"].as_str(), Some("subagent_pending"));
    assert_eq!(rows[0]["payload"]["total"].as_u64(), Some(2));
    let changed_fields = rows[0]["changed_fields"]
        .as_array()
        .expect("changed_fields array");
    assert!(
        changed_fields
            .iter()
            .any(|value| value.as_str() == Some("total"))
    );
    assert!(
        changed_fields
            .iter()
            .any(|value| value.as_str() == Some("states"))
    );
}

#[test]
fn watch_detail_summary_json_rows_with_delta_includes_fan_in_result_diagnostics_changes() {
    let thread_id = ThreadId::new();
    let previous = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
            task_count: 2,
            diagnostics_tasks: 1,
            diagnostics_matched_completion_total: 2,
            diagnostics_pending_matching_tool_ids_total: 1,
            diagnostics_scan_last_seq_max: 42,
        }),
        subagent_pending: None,
    };
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
            task_count: 2,
            diagnostics_tasks: 2,
            diagnostics_matched_completion_total: 5,
            diagnostics_pending_matching_tool_ids_total: 0,
            diagnostics_scan_last_seq_max: 50,
        }),
        subagent_pending: None,
    };

    let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["summary_type"].as_str(),
        Some("fan_in_result_diagnostics")
    );
    let changed_fields = rows[0]["changed_fields"]
        .as_array()
        .expect("changed_fields array");
    assert!(
        changed_fields
            .iter()
            .any(|value| value.as_str() == Some("diagnostics_tasks"))
    );
    assert!(
        changed_fields
            .iter()
            .any(|value| { value.as_str() == Some("diagnostics_matched_completion_total") })
    );
}
