use super::*;

#[test]
fn apply_inbox_filters_only_fan_out_linkage_issue_keeps_marked_threads() {
    let t1 = test_thread_meta(true, false, false);
    let t2 = test_thread_meta(false, false, false);
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);

    let filtered =
        apply_inbox_filters(threads, true, false, false, false, false, false, 0.9, false);
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains_key(&t1.thread_id));
}

#[test]
fn apply_inbox_filters_without_marker_filter_keeps_all_threads() {
    let t1 = test_thread_meta(true, false, false);
    let t2 = test_thread_meta(false, true, false);
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1);
    threads.insert(t2.thread_id, t2);

    let filtered = apply_inbox_filters(
        threads, false, false, false, false, false, false, 0.9, false,
    );
    assert_eq!(filtered.len(), 2);
}

#[test]
fn inbox_thread_changed_true_when_prev_missing() {
    let current = test_thread_meta(false, false, false);
    assert!(inbox_thread_changed(None, &current));
}

#[test]
fn inbox_thread_changed_false_when_seq_and_state_unchanged() {
    let current = test_thread_meta(false, false, false);
    let previous = current.clone();
    assert!(!inbox_thread_changed(Some(&previous), &current));
}

#[test]
fn inbox_thread_changed_true_when_seq_or_state_changes() {
    let current = test_thread_meta(false, false, false);

    let mut previous_seq = current.clone();
    previous_seq.last_seq = current.last_seq.saturating_add(1);
    assert!(inbox_thread_changed(Some(&previous_seq), &current));

    let mut previous_state = current.clone();
    previous_state.attention_state = "failed".to_string();
    assert!(inbox_thread_changed(Some(&previous_state), &current));
}

#[test]
fn apply_inbox_filters_only_fan_out_auto_apply_error_keeps_marked_threads() {
    let t1 = test_thread_meta(false, true, false);
    let t2 = test_thread_meta(false, false, false);
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);

    let filtered =
        apply_inbox_filters(threads, false, true, false, false, false, false, 0.9, false);
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains_key(&t1.thread_id));
}

#[test]
fn apply_inbox_filters_with_both_marker_filters_requires_both_markers() {
    let t1 = test_thread_meta(true, true, false);
    let t2 = test_thread_meta(true, false, false);
    let t3 = test_thread_meta(false, true, false);
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);
    threads.insert(t3.thread_id, t3);

    let filtered = apply_inbox_filters(threads, true, true, false, false, false, false, 0.9, false);
    assert_eq!(filtered.len(), 1);
    assert!(filtered.contains_key(&t1.thread_id));
}

#[test]
fn apply_inbox_filters_only_fan_in_dependency_blocked_keeps_marked_threads() {
    let t1 = test_thread_meta(false, false, true);
    let t2 = test_thread_meta(false, false, false);
    let t3 = test_thread_meta(false, false, true);
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);
    threads.insert(t3.thread_id, t3.clone());

    let filtered =
        apply_inbox_filters(threads, false, false, true, false, false, false, 0.9, false);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.contains_key(&t1.thread_id));
    assert!(filtered.contains_key(&t3.thread_id));
}

#[test]
fn apply_inbox_filters_only_subagent_proxy_approval_keeps_marked_threads() {
    let mut t1 = test_thread_meta(false, false, false);
    t1.pending_subagent_proxy_approvals = 1;
    let t2 = test_thread_meta(false, false, false);
    let mut t3 = test_thread_meta(false, false, false);
    t3.pending_subagent_proxy_approvals = 2;
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);
    threads.insert(t3.thread_id, t3.clone());

    let filtered =
        apply_inbox_filters(threads, false, false, false, false, false, false, 0.9, true);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.contains_key(&t1.thread_id));
    assert!(filtered.contains_key(&t3.thread_id));
}

#[test]
fn apply_inbox_filters_only_fan_in_result_diagnostics_keeps_marked_threads() {
    let mut t1 = test_thread_meta(false, false, false);
    t1.has_fan_in_result_diagnostics = true;
    let t2 = test_thread_meta(false, false, false);
    let mut t3 = test_thread_meta(false, false, false);
    t3.has_fan_in_result_diagnostics = true;
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);
    threads.insert(t3.thread_id, t3.clone());

    let filtered =
        apply_inbox_filters(threads, false, false, false, true, false, false, 0.9, false);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.contains_key(&t1.thread_id));
    assert!(filtered.contains_key(&t3.thread_id));
}
