use super::*;

#[test]
fn should_notify_presence_rising_edge_only_on_false_to_true() {
    assert!(!should_notify_presence_rising_edge(None, true));
    assert!(!should_notify_presence_rising_edge(None, false));
    assert!(!should_notify_presence_rising_edge(Some(true), true));
    assert!(!should_notify_presence_rising_edge(Some(true), false));
    assert!(!should_notify_presence_rising_edge(Some(false), false));
    assert!(should_notify_presence_rising_edge(Some(false), true));
}

#[test]
fn attention_state_severity_maps_expected_levels() {
    assert_eq!(
        attention_state_severity("failed"),
        notify_kit::Severity::Error
    );
    assert_eq!(
        attention_state_severity("fan_out_auto_apply_error"),
        notify_kit::Severity::Error
    );
    assert_eq!(
        attention_state_severity("fan_out_linkage_issue"),
        notify_kit::Severity::Warning
    );
    assert_eq!(
        attention_state_severity("fan_in_dependency_blocked"),
        notify_kit::Severity::Warning
    );
    assert_eq!(
        attention_state_severity("fan_in_result_diagnostics"),
        notify_kit::Severity::Warning
    );
    assert_eq!(
        attention_state_severity("token_budget_warning"),
        notify_kit::Severity::Warning
    );
    assert_eq!(
        attention_state_severity("running"),
        notify_kit::Severity::Info
    );
}

#[test]
fn attention_detail_markers_include_fan_in_statuses() {
    let markers =
        attention_detail_markers(false, false, false, false, true, true, false, false, false);
    assert_eq!(
        markers,
        vec!["fan_in_dependency_blocked", "fan_in_result_diagnostics"]
    );
}

#[test]
fn attention_detail_markers_include_token_budget_statuses() {
    let markers =
        attention_detail_markers(false, false, false, false, false, false, true, true, false);
    assert_eq!(
        markers,
        vec!["token_budget_exceeded", "token_budget_warning"]
    );
}

#[test]
fn attention_detail_markers_preserve_display_order() {
    let markers = attention_detail_markers(true, true, true, true, true, true, true, true, true);
    assert_eq!(
        markers,
        vec![
            "plan_ready",
            "diff_ready",
            "fan_out_linkage_issue",
            "fan_out_auto_apply_error",
            "fan_in_dependency_blocked",
            "fan_in_result_diagnostics",
            "token_budget_exceeded",
            "token_budget_warning",
            "test_failed",
        ]
    );
}

#[test]
fn should_emit_presence_bell_tracks_initial_value_without_notifying() {
    let mut last_present = None;
    let mut last_bell_at = None;
    assert!(!should_emit_presence_bell(
        true,
        1000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert_eq!(last_present, Some(true));
    assert!(last_bell_at.is_none());
}

#[test]
fn should_emit_presence_bell_notifies_on_rising_edge() {
    let mut last_present = Some(false);
    let mut last_bell_at = None;
    assert!(should_emit_presence_bell(
        true,
        1000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert_eq!(last_present, Some(true));
    assert!(last_bell_at.is_some());
}

#[test]
fn should_emit_presence_bell_respects_debounce_window() {
    let mut last_present = Some(false);
    let mut last_bell_at = Some(Instant::now());
    assert!(!should_emit_presence_bell(
        true,
        60_000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert_eq!(last_present, Some(true));
}

#[test]
fn should_emit_presence_bell_does_not_notify_on_falling_edge() {
    let mut last_present = Some(true);
    let mut last_bell_at = Some(Instant::now());
    assert!(!should_emit_presence_bell(
        false,
        1_000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert_eq!(last_present, Some(false));
}

#[test]
fn should_emit_presence_bell_notifies_on_next_rising_edge_after_fall() {
    let mut last_present = Some(true);
    let mut last_bell_at = None;
    assert!(!should_emit_presence_bell(
        false,
        1_000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert!(should_emit_presence_bell(
        true,
        1_000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert_eq!(last_present, Some(true));
    assert!(last_bell_at.is_some());
}

#[test]
fn should_emit_presence_bell_debounces_rising_edge_after_fall_if_recently_notified() {
    let mut last_present = Some(true);
    let mut last_bell_at = Some(Instant::now());
    assert!(!should_emit_presence_bell(
        false,
        60_000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert!(!should_emit_presence_bell(
        true,
        60_000,
        &mut last_present,
        &mut last_bell_at
    ));
    assert_eq!(last_present, Some(true));
}

#[test]
fn token_budget_warning_present_only_triggers_near_limit_without_exceeded() {
    assert!(!token_budget_warning_present(
        None,
        Some(0.95),
        Some(false),
        0.9
    ));
    assert!(!token_budget_warning_present(
        Some(200),
        Some(0.95),
        Some(true),
        0.9
    ));
    assert!(!token_budget_warning_present(
        Some(200),
        None,
        Some(false),
        0.9
    ));
    assert!(!token_budget_warning_present(
        Some(200),
        Some(0.89),
        Some(false),
        0.9
    ));
    assert!(token_budget_warning_present(
        Some(200),
        Some(0.90),
        Some(false),
        0.9
    ));
    assert!(token_budget_warning_present(
        Some(200),
        Some(0.95),
        Some(false),
        0.9
    ));
}

#[test]
fn format_token_budget_snapshot_omits_when_limit_absent() {
    let line = format_token_budget_snapshot(None, Some(0), Some(1.0), Some(true));
    assert!(line.is_none());
}

#[test]
fn format_token_budget_snapshot_formats_all_fields() {
    let line = format_token_budget_snapshot(Some(200), Some(0), Some(1.25), Some(true))
        .expect("token budget line");
    assert_eq!(
        line,
        "token_budget: remaining=0 limit=200 utilization=125.0% exceeded=true"
    );
}

#[test]
fn format_token_budget_snapshot_uses_defaults_for_missing_fields() {
    let line =
        format_token_budget_snapshot(Some(200), None, None, None).expect("token budget line");
    assert_eq!(
        line,
        "token_budget: remaining=- limit=200 utilization=- exceeded=false"
    );
}
