use super::*;

#[test]
fn watch_detail_summary_lines_include_auto_apply_and_fan_in_blocker() {
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
    let lines = watch_detail_summary_lines(Some(&auto_apply), Some(&fan_in_blocker), None, None);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("summary: fan_out_auto_apply: task_id=t-auto status=error"));
    assert!(lines[1].contains(
        "summary: fan_in_dependency_blocker: task_id=t-dependent status=Cancelled blocked=1/2"
    ));
}

#[test]
fn watch_detail_summary_lines_is_empty_when_no_summaries() {
    let lines = watch_detail_summary_lines(None, None, None, None);
    assert!(lines.is_empty());
}

#[test]
fn format_inbox_summary_cache_stats_includes_all_counters() {
    let stats = InboxSummaryCacheStats {
        fan_out_meta: 1,
        fan_out_cache_some: 2,
        fan_out_cache_none: 3,
        fan_out_attention: 4,
        fan_out_fetch_some: 5,
        fan_out_fetch_none: 6,
        fan_in_meta: 7,
        fan_in_cache_some: 8,
        fan_in_cache_none: 9,
        fan_in_attention: 10,
        fan_in_fetch_some: 11,
        fan_in_fetch_none: 12,
        fan_in_skip_unblocked: 13,
        fan_in_diag_meta: 14,
        fan_in_diag_cache_some: 15,
        fan_in_diag_cache_none: 16,
        fan_in_diag_attention: 17,
        fan_in_diag_fetch_some: 18,
        fan_in_diag_fetch_none: 19,
        fan_in_diag_skip_absent: 20,
        subagent_meta: 21,
        subagent_cache_some: 22,
        subagent_cache_none: 23,
        subagent_attention_some: 24,
        subagent_attention_none: 25,
        subagent_fetch_some: 26,
        subagent_fetch_none: 27,
        subagent_skip_no_pending: 28,
    };
    let line = format_inbox_summary_cache_stats(3, 20, 21, &stats);
    assert!(line.contains("iter=3 prev=20 cur=21"));
    assert!(line.contains(
        "fan_out(meta=1,cache_some=2,cache_none=3,attention=4,fetch_some=5,fetch_none=6)"
    ));
    assert!(line.contains(
        "fan_in(meta=7,cache_some=8,cache_none=9,attention=10,fetch_some=11,fetch_none=12,skip_unblocked=13)"
    ));
    assert!(line.contains(
        "fan_in_diag(meta=14,cache_some=15,cache_none=16,attention=17,fetch_some=18,fetch_none=19,skip_absent=20)"
    ));
    assert!(line.contains(
        "subagent(meta=21,cache_some=22,cache_none=23,attention_some=24,attention_none=25,fetch_some=26,fetch_none=27,skip_no_pending=28)"
    ));
}

#[test]
fn format_watch_summary_refresh_debug_renders_sources() {
    let line = format_watch_summary_refresh_debug(
        7,
        4,
        true,
        false,
        true,
        true,
        SummarySource::Attention,
        SummarySource::Previous,
        SummarySource::Artifact,
        SummarySource::None,
    );
    assert!(line.contains("iter=7 events=4"));
    assert!(line.contains("auto_apply(refresh=true,source=attention)"));
    assert!(line.contains("fan_in(refresh=false,source=previous)"));
    assert!(line.contains("fan_in_diag(refresh=true,source=artifact)"));
    assert!(line.contains("subagent(refresh=true,source=none)"));
}

#[test]
fn build_watch_summary_refresh_debug_json_row_renders_sources() {
    let row = build_watch_summary_refresh_debug_json_row(
        7,
        4,
        true,
        false,
        true,
        true,
        SummarySource::Attention,
        SummarySource::Previous,
        SummarySource::Artifact,
        SummarySource::None,
    );
    assert_eq!(row["kind"].as_str(), Some("watch_summary_refresh_debug"));
    assert_eq!(row["iteration"].as_u64(), Some(7));
    assert_eq!(row["event_count"].as_u64(), Some(4));
    assert_eq!(row["auto_apply"]["refresh"].as_bool(), Some(true));
    assert_eq!(row["auto_apply"]["source"].as_str(), Some("attention"));
    assert_eq!(row["fan_in"]["refresh"].as_bool(), Some(false));
    assert_eq!(row["fan_in"]["source"].as_str(), Some("previous"));
    assert_eq!(row["fan_in_diagnostics"]["refresh"].as_bool(), Some(true));
    assert_eq!(
        row["fan_in_diagnostics"]["source"].as_str(),
        Some("artifact")
    );
    assert_eq!(row["subagent"]["refresh"].as_bool(), Some(true));
    assert_eq!(row["subagent"]["source"].as_str(), Some("none"));
}
