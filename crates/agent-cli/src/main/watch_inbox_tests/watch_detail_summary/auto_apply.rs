use super::*;

#[test]
fn fan_out_auto_apply_summary_omits_applied_payload() {
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
                applied: true,
                workspace_cwd: None,
                target_workspace_cwd: None,
                check_argv: vec![],
                apply_argv: vec![],
                patch_artifact_id: None,
                patch_read_cmd: None,
                failure_stage: None,
                recovery_hint: None,
                recovery_commands: vec![],
                error: None,
            },
        ),
        status: "completed".to_string(),
        reason: None,
    };

    assert!(fan_out_auto_apply_summary_from_payload(&payload).is_none());
}

#[test]
fn is_fan_out_auto_apply_error_matches_status() {
    let error = FanOutAutoApplyInboxSummary {
        task_id: "t1".to_string(),
        status: "error".to_string(),
        stage: None,
        patch_artifact_id: None,
        recovery_commands: None,
        recovery_1: None,
        error: None,
    };
    let non_error = FanOutAutoApplyInboxSummary {
        task_id: "t2".to_string(),
        status: "attempted_not_applied".to_string(),
        stage: None,
        patch_artifact_id: None,
        recovery_commands: None,
        recovery_1: None,
        error: None,
    };
    assert!(is_fan_out_auto_apply_error(&error));
    assert!(!is_fan_out_auto_apply_error(&non_error));
}
