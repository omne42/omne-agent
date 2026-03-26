use super::*;

#[test]
fn checkpoint_restore_rpc_needs_approval_returns_actionable_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let checkpoint_id = omne_protocol::CheckpointId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse {
            thread_id,
            checkpoint_id,
            needs_approval: true,
            approval_id,
            plan: omne_app_server_protocol::ThreadCheckpointPlan {
                create: 1,
                modify: 2,
                delete: 3,
            },
        },
    )
    .expect("serialize ThreadCheckpointRestoreNeedsApprovalResponse");

    let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/checkpoint/restore needs approval"));
    assert!(message.contains(&approval_id.to_string()));
}

#[test]
fn checkpoint_restore_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let checkpoint_id = omne_protocol::CheckpointId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse {
            thread_id,
            checkpoint_id,
            needs_approval: true,
            approval_id,
            plan: omne_app_server_protocol::ThreadCheckpointPlan {
                create: 1,
                modify: 2,
                delete: 3,
            },
        },
    )?;

    let outcome = parse_checkpoint_restore_rpc_outcome("thread/checkpoint/restore", value)?;
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
fn checkpoint_restore_rpc_denied_returns_error() {
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
            thread_id: omne_protocol::ThreadId::new(),
            checkpoint_id: omne_protocol::CheckpointId::new(),
            denied: true,
            error_code: Some("mode_denied".to_string()),
            sandbox_policy: None,
            mode: Some("coder".to_string()),
            decision: None,
            available: None,
            load_error: None,
            sandbox_writable_roots: None,
        },
    )
    .expect("serialize ThreadCheckpointRestoreDeniedResponse");

    let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/checkpoint/restore denied"));
    assert!(message.contains("mode_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn checkpoint_restore_rpc_approval_denied_returns_error() {
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
            thread_id: omne_protocol::ThreadId::new(),
            checkpoint_id: omne_protocol::CheckpointId::new(),
            denied: true,
            error_code: Some("approval_denied".to_string()),
            sandbox_policy: None,
            mode: None,
            decision: None,
            available: None,
            load_error: None,
            sandbox_writable_roots: None,
        },
    )
    .expect("serialize ThreadCheckpointRestoreDeniedResponse");

    let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/checkpoint/restore denied"));
    assert!(message.contains("approval_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn checkpoint_restore_rpc_mode_unknown_returns_error() {
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
            thread_id: omne_protocol::ThreadId::new(),
            checkpoint_id: omne_protocol::CheckpointId::new(),
            denied: true,
            error_code: Some("mode_unknown".to_string()),
            sandbox_policy: None,
            mode: Some("checkpoint-restore-mode".to_string()),
            decision: Some(omne_app_server_protocol::ThreadCheckpointDecision::Deny),
            available: Some("other-mode".to_string()),
            load_error: None,
            sandbox_writable_roots: None,
        },
    )
    .expect("serialize ThreadCheckpointRestoreDeniedResponse");

    let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/checkpoint/restore denied"));
    assert!(message.contains("mode_unknown"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn checkpoint_restore_rpc_sandbox_policy_denied_returns_error() {
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
            thread_id: omne_protocol::ThreadId::new(),
            checkpoint_id: omne_protocol::CheckpointId::new(),
            denied: true,
            error_code: Some("sandbox_policy_denied".to_string()),
            sandbox_policy: Some(policy_meta::WriteScope::ReadOnly),
            mode: None,
            decision: None,
            available: None,
            load_error: None,
            sandbox_writable_roots: None,
        },
    )
    .expect("serialize ThreadCheckpointRestoreDeniedResponse");

    let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/checkpoint/restore denied"));
    assert!(message.contains("sandbox_policy_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn checkpoint_restore_rpc_sandbox_writable_roots_unsupported_returns_error() {
    let value = serde_json::to_value(
        omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
            thread_id: omne_protocol::ThreadId::new(),
            checkpoint_id: omne_protocol::CheckpointId::new(),
            denied: true,
            error_code: Some("sandbox_writable_roots_unsupported".to_string()),
            sandbox_policy: None,
            mode: None,
            decision: None,
            available: None,
            load_error: None,
            sandbox_writable_roots: Some(vec![".".to_string()]),
        },
    )
    .expect("serialize ThreadCheckpointRestoreDeniedResponse");

    let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/checkpoint/restore denied"));
    assert!(message.contains("sandbox_writable_roots_unsupported"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn checkpoint_restore_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
        thread_id: omne_protocol::ThreadId::new(),
        checkpoint_id: omne_protocol::CheckpointId::new(),
        denied: true,
        error_code: None,
        sandbox_policy: None,
        mode: Some("coder".to_string()),
        decision: None,
        available: None,
        load_error: None,
        sandbox_writable_roots: None,
    })?;

    let outcome = parse_checkpoint_restore_rpc_outcome("thread/checkpoint/restore", value)?;
    assert!(matches!(outcome, RpcGateOutcome::Denied { .. }));
    Ok(())
}

#[test]
fn checkpoint_restore_rpc_ok_passthrough() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let checkpoint_id = omne_protocol::CheckpointId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadCheckpointRestoreResponse {
        thread_id,
        checkpoint_id,
        restored: true,
        plan: omne_app_server_protocol::ThreadCheckpointPlan {
            create: 0,
            modify: 0,
            delete: 0,
        },
        duration_ms: 42,
    })
    .expect("serialize ThreadCheckpointRestoreResponse");

    let parsed = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)?;
    assert!(parsed.restored);
    assert_eq!(parsed.duration_ms, 42);
    Ok(())
}

#[test]
fn checkpoint_restore_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let checkpoint_id = omne_protocol::CheckpointId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadCheckpointRestoreResponse {
        thread_id,
        checkpoint_id,
        restored: true,
        plan: omne_app_server_protocol::ThreadCheckpointPlan {
            create: 0,
            modify: 0,
            delete: 0,
        },
        duration_ms: 42,
    })?;

    let outcome = parse_checkpoint_restore_rpc_outcome("thread/checkpoint/restore", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(omne_app_server_protocol::ThreadCheckpointRestoreResponse {
            restored: true,
            duration_ms: 42,
            ..
        })
    ));
    Ok(())
}
