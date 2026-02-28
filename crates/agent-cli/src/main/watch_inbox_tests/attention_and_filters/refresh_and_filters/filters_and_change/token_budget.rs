use super::*;

#[test]
fn apply_inbox_filters_only_token_budget_exceeded_keeps_marked_threads() {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_exceeded = Some(true);
    let t2 = test_thread_meta(false, false, false);
    let mut t3 = test_thread_meta(false, false, false);
    t3.token_budget_exceeded = Some(true);
    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);
    threads.insert(t3.thread_id, t3.clone());

    let filtered =
        apply_inbox_filters(threads, false, false, false, false, true, false, 0.9, false);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.get(&t1.thread_id).is_some());
    assert!(filtered.get(&t3.thread_id).is_some());
}

#[test]
fn apply_inbox_filters_only_token_budget_warning_keeps_warning_threads() {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_limit = Some(200);
    t1.token_budget_utilization = Some(0.95);
    t1.token_budget_exceeded = Some(false);

    let mut t2 = test_thread_meta(false, false, false);
    t2.token_budget_limit = Some(200);
    t2.token_budget_utilization = Some(0.89);
    t2.token_budget_exceeded = Some(false);

    let mut t3 = test_thread_meta(false, false, false);
    t3.token_budget_limit = Some(200);
    t3.token_budget_utilization = Some(0.97);
    t3.token_budget_exceeded = Some(true);

    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);
    threads.insert(t3.thread_id, t3);

    let filtered =
        apply_inbox_filters(threads, false, false, false, false, false, true, 0.9, false);
    assert_eq!(filtered.len(), 1);
    assert!(filtered.get(&t1.thread_id).is_some());
}

#[test]
fn apply_inbox_filters_only_token_budget_warning_prefers_server_flag() {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_limit = Some(200);
    t1.token_budget_utilization = Some(0.10);
    t1.token_budget_exceeded = Some(false);
    t1.token_budget_warning_active = Some(true);

    let mut t2 = test_thread_meta(false, false, false);
    t2.token_budget_limit = Some(200);
    t2.token_budget_utilization = Some(0.95);
    t2.token_budget_exceeded = Some(false);
    t2.token_budget_warning_active = Some(false);

    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1.clone());
    threads.insert(t2.thread_id, t2);

    let filtered =
        apply_inbox_filters(threads, false, false, false, false, false, true, 0.9, false);
    assert_eq!(filtered.len(), 1);
    assert!(filtered.get(&t1.thread_id).is_some());
}

#[test]
fn apply_inbox_filters_token_budget_exceeded_and_warning_intersection_is_empty() {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_limit = Some(200);
    t1.token_budget_utilization = Some(0.95);
    t1.token_budget_exceeded = Some(false);

    let mut t2 = test_thread_meta(false, false, false);
    t2.token_budget_limit = Some(200);
    t2.token_budget_utilization = Some(0.95);
    t2.token_budget_exceeded = Some(true);

    let mut threads = std::collections::BTreeMap::new();
    threads.insert(t1.thread_id, t1);
    threads.insert(t2.thread_id, t2);

    let filtered = apply_inbox_filters(threads, false, false, false, false, true, true, 0.9, false);
    assert!(filtered.is_empty());
}
