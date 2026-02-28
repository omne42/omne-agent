use super::*;

#[test]
fn should_emit_watch_detail_summary_emits_for_fan_in_result_diagnostics_only() {
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: Some(FanInResultDiagnosticsInboxSummary {
            task_count: 1,
            diagnostics_tasks: 1,
            diagnostics_matched_completion_total: 2,
            diagnostics_pending_matching_tool_ids_total: 1,
            diagnostics_scan_last_seq_max: 42,
        }),
        subagent_pending: None,
    };
    assert!(should_emit_watch_detail_summary(None, &current));
}

#[test]
fn should_emit_watch_detail_summary_emits_first_non_empty_snapshot() {
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
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    assert!(should_emit_watch_detail_summary(None, &current));
}

#[test]
fn should_emit_watch_detail_summary_suppresses_unchanged_snapshot() {
    let snapshot = WatchDetailSummarySnapshot {
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
    assert!(!should_emit_watch_detail_summary(
        Some(&snapshot),
        &snapshot
    ));
}

#[test]
fn should_emit_watch_detail_summary_emits_when_snapshot_changes() {
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
        fan_in_blocker: None,
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
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    assert!(should_emit_watch_detail_summary(Some(&previous), &current));
}

#[test]
fn should_emit_watch_detail_summary_suppresses_empty_snapshot() {
    let current = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    assert!(!should_emit_watch_detail_summary(None, &current));
}

#[test]
fn should_emit_watch_detail_summary_re_emits_after_empty_gap() {
    let non_empty = WatchDetailSummarySnapshot {
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
    let empty = WatchDetailSummarySnapshot {
        auto_apply: None,
        fan_in_blocker: None,
        fan_in_diagnostics: None,
        subagent_pending: None,
    };
    assert!(should_emit_watch_detail_summary(Some(&non_empty), &empty));
    assert!(!should_emit_watch_detail_summary(Some(&empty), &empty));
    assert!(should_emit_watch_detail_summary(Some(&empty), &non_empty));
}
