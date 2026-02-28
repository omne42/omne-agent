use super::*;

#[test]
fn watch_detail_summary_json_rows_include_auto_apply_and_fan_in_blocker() {
    let thread_id = ThreadId::new();
    let thread_id_text = thread_id.to_string();
    let auto_apply = FanOutAutoApplyInboxSummary {
        task_id: "t-auto".to_string(),
        status: "error".to_string(),
        stage: Some("check_patch".to_string()),
        patch_artifact_id: None,
        recovery_commands: None,
        recovery_1: None,
        error: Some("git apply failed".to_string()),
    };
    let fan_in_blocker = FanInDependencyBlockedInboxSummary {
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
        reason: Some("blocked by dependency".to_string()),
    };
    let rows = watch_detail_summary_json_rows(
        thread_id,
        Some(&auto_apply),
        Some(&fan_in_blocker),
        None,
        None,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["kind"].as_str(), Some("watch_detail_summary"));
    assert_eq!(rows[0]["thread_id"].as_str(), Some(thread_id_text.as_str()));
    assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_out_auto_apply"));
    assert_eq!(rows[0]["payload"]["task_id"].as_str(), Some("t-auto"));
    assert_eq!(
        rows[1]["summary_type"].as_str(),
        Some("fan_in_dependency_blocker")
    );
    assert_eq!(rows[1]["payload"]["task_id"].as_str(), Some("t-dependent"));
    assert_eq!(
        rows[1]["payload"]["dependency_blocked_count"].as_u64(),
        Some(1)
    );
    assert_eq!(rows[1]["payload"]["task_count"].as_u64(), Some(2));
    assert_eq!(
        rows[1]["payload"]["dependency_blocked_ratio"].as_f64(),
        Some(0.5)
    );
}

#[test]
fn watch_detail_summary_json_rows_is_empty_when_no_summaries() {
    let rows = watch_detail_summary_json_rows(ThreadId::new(), None, None, None, None);
    assert!(rows.is_empty());
}

#[test]
fn watch_detail_summary_json_rows_with_delta_emits_cleared_marker() {
    let thread_id = ThreadId::new();
    let previous = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
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
            reason: Some("blocked by dependency".to_string()),
        }),
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["kind"].as_str(), Some("watch_detail_summary"));
    assert_eq!(
        rows[0]["summary_type"].as_str(),
        Some("fan_in_dependency_blocker")
    );
    assert_eq!(rows[0]["cleared"].as_bool(), Some(true));
    assert_eq!(rows[0]["changed_fields"][0].as_str(), Some("cleared"));
}

#[test]
fn watch_detail_summary_json_rows_with_delta_includes_changed_fields() {
    let thread_id = ThreadId::new();
    let previous = WatchDetailSummarySnapshot {
        auto_apply: Some(FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "attempted_not_applied".to_string(),
            stage: Some("apply_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: None,
        }),
        fan_in_blocker: Some(FanInDependencyBlockedInboxSummary {
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
            reason: Some("blocked by dependency".to_string()),
        }),
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    let current = WatchDetailSummarySnapshot {
        auto_apply: Some(FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: Some("git apply failed".to_string()),
        }),
        fan_in_blocker: previous.fan_in_blocker.clone(),
        fan_in_diagnostics: None,
        subagent_pending: None,
    };

    let rows = watch_detail_summary_json_rows_with_delta(thread_id, Some(&previous), &current);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["summary_type"].as_str(), Some("fan_out_auto_apply"));
    let changed_fields = rows[0]["changed_fields"]
        .as_array()
        .expect("changed_fields array");
    assert!(
        changed_fields
            .iter()
            .any(|value| value.as_str() == Some("status"))
    );
    assert!(
        changed_fields
            .iter()
            .any(|value| value.as_str() == Some("stage"))
    );
}
