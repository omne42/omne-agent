use super::*;

#[test]
fn fan_out_auto_apply_summary_reports_error_and_recovery_command_preview() {
    let payload = omne_app_server_protocol::ArtifactFanOutResultStructuredData {
        schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
        task_id: "t-auto-apply".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        workspace_mode: "isolated_write".to_string(),
        workspace_cwd: None,
        isolated_write_patch: None,
        isolated_write_handoff: None,
        isolated_write_auto_apply: Some(
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                enabled: true,
                attempted: true,
                applied: false,
                workspace_cwd: None,
                target_workspace_cwd: None,
                check_argv: vec![],
                apply_argv: vec![],
                patch_artifact_id: Some("artifact-7".to_string()),
                patch_read_cmd: None,
                failure_stage: Some(
                    omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
                ),
                recovery_hint: None,
                recovery_commands: vec![
                    omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData {
                        label: "read_patch_artifact".to_string(),
                        argv: vec![
                            "omne".to_string(),
                            "artifact".to_string(),
                            "read".to_string(),
                            "thread-1".to_string(),
                            "artifact-7".to_string(),
                        ],
                    },
                ],
                error: Some("git apply --check failed: patch does not apply".to_string()),
            },
        ),
        status: "completed".to_string(),
        reason: None,
    };

    let summary = fan_out_auto_apply_summary_from_payload(&payload).expect("summary");
    assert_eq!(summary.task_id, "t-auto-apply");
    assert_eq!(summary.status, "error");
    assert_eq!(summary.stage.as_deref(), Some("check_patch"));
    assert_eq!(summary.patch_artifact_id.as_deref(), Some("artifact-7"));
    assert_eq!(summary.recovery_commands, Some(1));
    assert_eq!(
        summary.recovery_1.as_deref(),
        Some("read_patch_artifact: omne artifact read thread-1 artifact-7")
    );

    let text = format_fan_out_auto_apply_summary(&summary);
    assert!(text.contains("task_id=t-auto-apply"));
    assert!(text.contains("status=error"));
    assert!(text.contains("stage=check_patch"));
}

#[test]
fn render_inbox_json_threads_attaches_fan_out_auto_apply_when_present() -> anyhow::Result<()> {
    let t1 = test_thread_meta(false, false, false);
    let t2 = test_thread_meta(false, false, false);
    let mut auto_apply_summaries = std::collections::BTreeMap::new();
    auto_apply_summaries.insert(
        t1.thread_id,
        FanOutAutoApplyInboxSummary {
            task_id: "t-auto-apply".to_string(),
            status: "error".to_string(),
            stage: Some("check_patch".to_string()),
            patch_artifact_id: Some("artifact-7".to_string()),
            recovery_commands: Some(1),
            recovery_1: Some("read_patch_artifact: omne artifact read".to_string()),
            error: Some("git apply --check failed".to_string()),
        },
    );
    let fan_in_blockers = std::collections::BTreeMap::new();
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
        rows[0]["fan_out_auto_apply"]["task_id"].as_str(),
        Some("t-auto-apply")
    );
    assert!(rows[1]["fan_out_auto_apply"].is_null());
    Ok(())
}

#[test]
fn render_inbox_json_threads_includes_token_budget_snapshot_fields() -> anyhow::Result<()> {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_limit = Some(200);
    t1.token_budget_remaining = Some(0);
    t1.token_budget_utilization = Some(1.25);
    t1.token_budget_exceeded = Some(true);
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let fan_in_blockers = std::collections::BTreeMap::new();
    let fan_in_diagnostics = std::collections::BTreeMap::new();
    let subagent_pending = std::collections::BTreeMap::new();
    let rows = render_inbox_json_threads(
        [&t1],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["token_budget_limit"].as_u64(), Some(200));
    assert_eq!(rows[0]["token_budget_remaining"].as_u64(), Some(0));
    assert_eq!(rows[0]["token_budget_exceeded"].as_bool(), Some(true));
    assert_eq!(
        rows[0]["token_budget_warning_active"].as_bool(),
        Some(false)
    );
    let utilization = rows[0]["token_budget_utilization"]
        .as_f64()
        .expect("token_budget_utilization should be numeric");
    assert!((utilization - 1.25).abs() < f64::EPSILON);
    Ok(())
}

#[test]
fn render_inbox_json_threads_includes_token_budget_warning_active_when_threshold_reached()
-> anyhow::Result<()> {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_limit = Some(200);
    t1.token_budget_remaining = Some(10);
    t1.token_budget_utilization = Some(1.0);
    t1.token_budget_exceeded = Some(false);
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let fan_in_blockers = std::collections::BTreeMap::new();
    let fan_in_diagnostics = std::collections::BTreeMap::new();
    let subagent_pending = std::collections::BTreeMap::new();
    let rows = render_inbox_json_threads(
        [&t1],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(true));
    Ok(())
}

#[test]
fn render_inbox_json_threads_prefers_server_token_budget_warning_active() -> anyhow::Result<()> {
    let mut t1 = test_thread_meta(false, false, false);
    t1.token_budget_limit = Some(200);
    t1.token_budget_remaining = Some(190);
    t1.token_budget_utilization = Some(0.05);
    t1.token_budget_exceeded = Some(false);
    t1.token_budget_warning_active = Some(true);
    let auto_apply_summaries = std::collections::BTreeMap::new();
    let fan_in_blockers = std::collections::BTreeMap::new();
    let fan_in_diagnostics = std::collections::BTreeMap::new();
    let subagent_pending = std::collections::BTreeMap::new();
    let rows = render_inbox_json_threads(
        [&t1],
        &auto_apply_summaries,
        &fan_in_blockers,
        &fan_in_diagnostics,
        &subagent_pending,
        true,
    )?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["token_budget_warning_active"].as_bool(), Some(true));
    Ok(())
}
