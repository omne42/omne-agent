use super::*;

#[test]
fn watch_detail_summary_lines_with_delta_emits_cleared_marker() {
    let previous = WatchDetailSummarySnapshot {
        auto_apply: Some(FanOutAutoApplyInboxSummary {
            task_id: "t-auto".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: None,
            recovery_commands: None,
            recovery_1: None,
            error: Some("git apply failed".to_string()),
        }),
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], "summary: fan_out_auto_apply: cleared");
}

#[test]
fn watch_detail_summary_lines_with_delta_emits_only_changed_summary() {
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

    let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("fan_out_auto_apply: task_id=t-auto status=error"));
}

#[test]
fn watch_detail_summary_lines_with_delta_emits_subagent_pending_summary() {
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

    let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("subagent_pending: total=2"));
    assert!(lines[0].contains("done:1"));
}

#[test]
fn watch_detail_summary_lines_with_delta_emits_fan_in_result_diagnostics_summary() {
    let previous = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
            task_count: 2,
            diagnostics_tasks: 2,
            diagnostics_matched_completion_total: 5,
            diagnostics_pending_matching_tool_ids_total: 1,
            diagnostics_scan_last_seq_max: 50,
        }),
        subagent_pending: None,
    };

    let lines = watch_detail_summary_lines_with_delta(Some(&previous), &current);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("fan_in_result_diagnostics: tasks=2 diagnostics_tasks=2"));
    assert!(lines[0].contains("matched_completion_total=5"));
}
