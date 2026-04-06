use super::*;

#[test]
fn thread_git_snapshot_rpc_needs_approval_returns_actionable_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        },
    )
    .expect("serialize ThreadGitSnapshotNeedsApprovalResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/diff needs approval"));
    assert!(message.contains(&approval_id.to_string()));
}

#[test]
fn thread_git_snapshot_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        },
    )?;

    let outcome = parse_thread_git_snapshot_rpc_outcome("thread/diff", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::NeedsApproval {
            thread_id: t,
            approval_id: a
        } if t == thread_id && a == approval_id
    ));
    Ok(())
}

#[test]
fn thread_git_snapshot_rpc_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("sandbox_policy_denied".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                    omne_app_server_protocol::ProcessDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        thread_id,
                        remembered: None,
                        structured_error: None,
                        error_code: None,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/diff denied"));
    assert!(message.contains("sandbox_policy_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_patch_rpc_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("execpolicy_denied".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                    omne_app_server_protocol::ProcessDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        thread_id,
                        remembered: None,
                        structured_error: None,
                        error_code: None,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/patch denied"));
    assert!(message.contains("execpolicy_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_patch_rpc_artifact_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("allowed_tools_denied".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(
                    omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        tool: "artifact/write".to_string(),
                        allowed_tools: vec!["process/start".to_string()],
                        structured_error: None,
                        error_code: None,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/patch denied"));
    assert!(message.contains("allowed_tools_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_diff_rpc_artifact_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("allowed_tools_denied".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(
                    omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        tool: "artifact/write".to_string(),
                        allowed_tools: vec!["process/start".to_string()],
                        structured_error: None,
                        error_code: None,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/diff denied"));
    assert!(message.contains("allowed_tools_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_patch_rpc_artifact_mode_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("mode_denied".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(
                    omne_app_server_protocol::ArtifactModeDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        structured_error: None,
                        error_code: None,
                        mode: "artifact-deny".to_string(),
                        decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                        decision_source: "mode_permission".to_string(),
                        tool_override_hit: false,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/patch denied"));
    assert!(message.contains("mode_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_diff_rpc_artifact_mode_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("mode_denied".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(
                    omne_app_server_protocol::ArtifactModeDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        structured_error: None,
                        error_code: None,
                        mode: "artifact-deny".to_string(),
                        decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                        decision_source: "mode_permission".to_string(),
                        tool_override_hit: false,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/diff denied"));
    assert!(message.contains("mode_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_patch_rpc_artifact_unknown_mode_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("mode_unknown".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(
                    omne_app_server_protocol::ArtifactUnknownModeDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        structured_error: None,
                        error_code: None,
                        mode: "artifact-unknown".to_string(),
                        decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                        available: "other-mode".to_string(),
                        load_error: None,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/patch denied"));
    assert!(message.contains("mode_unknown"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_diff_rpc_artifact_unknown_mode_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            structured_error: None,
            error_code: Some("mode_unknown".to_string()),
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(
                    omne_app_server_protocol::ArtifactUnknownModeDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        structured_error: None,
                        error_code: None,
                        mode: "artifact-unknown".to_string(),
                        decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                        available: "other-mode".to_string(),
                        load_error: None,
                    },
                ),
            ),
        },
    )
    .expect("serialize ThreadGitSnapshotDeniedResponse");

    let err =
        parse_thread_git_snapshot_rpc_response("thread/diff", value).expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/diff denied"));
    assert!(message.contains("mode_unknown"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_git_snapshot_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
        denied: true,
        thread_id,
        structured_error: None,
        error_code: None,
        detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
            omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                omne_app_server_protocol::ProcessDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    thread_id,
                    remembered: None,
                    structured_error: None,
                    error_code: None,
                },
            ),
        ),
    })?;

    let outcome = parse_thread_git_snapshot_rpc_outcome("thread/diff", value)?;
    assert!(matches!(outcome, RpcGateOutcome::Denied { .. }));
    Ok(())
}

#[test]
fn thread_git_snapshot_rpc_timed_out_passthrough() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
            omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse {
                thread_id,
                process_id: omne_protocol::ProcessId::new(),
                stdout_path: "/tmp/stdout.log".to_string(),
                stderr_path: "/tmp/stderr.log".to_string(),
                timed_out: true,
                wait_seconds: 5,
            },
        ),
    )
    .expect("serialize ThreadGitSnapshotRpcResponse::TimedOut");

    let parsed = parse_thread_git_snapshot_rpc_response("thread/diff", value)?;
    assert!(matches!(
        parsed,
        omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
            omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse { wait_seconds: 5, .. }
        )
    ));
    Ok(())
}

#[test]
fn thread_git_snapshot_rpc_outcome_keeps_timed_out_passthrough() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
            omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse {
                thread_id,
                process_id: omne_protocol::ProcessId::new(),
                stdout_path: "/tmp/stdout.log".to_string(),
                stderr_path: "/tmp/stderr.log".to_string(),
                timed_out: true,
                wait_seconds: 5,
            },
        ),
    )?;

    let outcome = parse_thread_git_snapshot_rpc_outcome("thread/diff", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
                omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse { wait_seconds: 5, .. }
            )
        )
    ));
    Ok(())
}
