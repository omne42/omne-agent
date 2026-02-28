use super::*;

#[test]
fn render_inbox_json_threads_attaches_fan_in_result_diagnostics_when_enabled() -> anyhow::Result<()>
{
    let mut t1 = test_thread_meta(false, false, false);
    t1.has_fan_in_result_diagnostics = true;
    t1.fan_in_result_diagnostics = Some(FanInResultDiagnosticsInboxSummary {
        task_count: 2,
        diagnostics_tasks: 2,
        diagnostics_matched_completion_total: 5,
        diagnostics_pending_matching_tool_ids_total: 1,
        diagnostics_scan_last_seq_max: 50,
    });
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let fan_in_blockers = std::collections::BTreeMap::new();
    let fan_in_diagnostics = std::collections::BTreeMap::new();
    let subagent_pending = std::collections::BTreeMap::new();

    let rows_with_details = render_inbox_json_threads(
        [&t1],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(
        rows_with_details[0]["fan_in_result_diagnostics"]["diagnostics_tasks"].as_u64(),
        Some(2)
    );

    let rows_without_details = render_inbox_json_threads(
        [&t1],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        false,
    )?;
    assert!(rows_without_details[0]["fan_in_result_diagnostics"].is_null());
    Ok(())
}

#[test]
fn render_inbox_json_threads_attaches_fan_in_result_diagnostics_from_collected_summaries()
-> anyhow::Result<()> {
    let mut t1 = test_thread_meta(false, false, false);
    t1.has_fan_in_result_diagnostics = true;
    t1.fan_in_result_diagnostics = None;
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let fan_in_blockers = std::collections::BTreeMap::new();
    let mut fan_in_diagnostics = std::collections::BTreeMap::new();
    fan_in_diagnostics.insert(
        t1.thread_id,
        FanInResultDiagnosticsInboxSummary {
            task_count: 3,
            diagnostics_tasks: 2,
            diagnostics_matched_completion_total: 6,
            diagnostics_pending_matching_tool_ids_total: 1,
            diagnostics_scan_last_seq_max: 77,
        },
    );
    let subagent_pending = std::collections::BTreeMap::new();

    let rows = render_inbox_json_threads(
        [&t1],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(
        rows[0]["fan_in_result_diagnostics"]["diagnostics_scan_last_seq_max"].as_u64(),
        Some(77)
    );
    Ok(())
}

#[test]
fn render_inbox_json_threads_attaches_subagent_pending_when_present() -> anyhow::Result<()> {
    let t1 = test_thread_meta(false, false, false);
    let t2 = test_thread_meta(false, false, false);
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let fan_in_blockers = std::collections::BTreeMap::new();
    let fan_in_diagnostics = std::collections::BTreeMap::new();
    let mut subagent_pending = std::collections::BTreeMap::new();
    subagent_pending.insert(
        t1.thread_id,
        SubagentPendingApprovalsSummary {
            total: 3,
            states: std::collections::BTreeMap::from([
                ("running".to_string(), 2),
                ("done".to_string(), 1),
            ]),
        },
    );
    let rows = render_inbox_json_threads(
        [&t1, &t2],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["subagent_pending"]["total"].as_u64(), Some(3));
    assert_eq!(
        rows[0]["subagent_pending"]["states"]["running"].as_u64(),
        Some(2)
    );
    assert_eq!(rows[1]["subagent_pending"].as_object(), None);
    Ok(())
}
