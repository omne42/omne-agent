use super::*;

#[test]
fn artifact_rpc_needs_approval_returns_actionable_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ArtifactNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    })
    .expect("serialize ArtifactNeedsApprovalResponse");

    let err = parse_artifact_rpc_response_typed::<omne_app_server_protocol::ArtifactWriteResponse>(
        "artifact/write",
        value,
    )
    .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("artifact/write needs approval"));
    assert!(message.contains(&approval_id.to_string()));
    assert!(message.contains(&format!(
        "omne approval decide {} {} --approve",
        thread_id, approval_id
    )));
    assert!(!message.contains("--thread-id"));
}

#[test]
fn artifact_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ArtifactNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    })?;

    let outcome =
        parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactWriteResponse>(
            "artifact/write",
            value,
        )?;
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
fn artifact_rpc_denied_returns_error() {
    let value = serde_json::to_value(omne_app_server_protocol::ArtifactDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        structured_error: None,
        error_code: Some("allowed_tools_denied".to_string()),
        remembered: None,
    })
    .expect("serialize ArtifactDeniedResponse");

    let err = parse_artifact_rpc_response_typed::<omne_app_server_protocol::ArtifactWriteResponse>(
        "artifact/write",
        value,
    )
    .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("artifact/write denied"));
    assert!(message.contains("allowed_tools_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn artifact_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ArtifactDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        structured_error: None,
        error_code: None,
        remembered: None,
    })?;

    let outcome =
        parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactWriteResponse>(
            "artifact/write",
            value,
        )?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Denied { detail }
            if detail.get("denied") == Some(&serde_json::Value::Bool(true))
    ));
    Ok(())
}

#[test]
fn artifact_rpc_ok_passthrough() -> anyhow::Result<()> {
    let tool_id = omne_protocol::ToolId::new();
    let artifact_id = omne_protocol::ArtifactId::new();
    let value = serde_json::json!({
        "tool_id": tool_id,
        "artifact_id": artifact_id,
        "created": true,
        "content_path": "/tmp/artifact.txt",
        "metadata_path": "/tmp/artifact.json",
        "metadata": {
            "artifact_id": artifact_id,
            "artifact_type": "report",
            "summary": "summary",
            "created_at": "1970-01-01T00:00:00Z",
            "updated_at": "1970-01-01T00:00:00Z",
            "version": 1,
            "content_path": "/tmp/artifact.txt",
            "size_bytes": 12
        }
    });

    let parsed: omne_app_server_protocol::ArtifactWriteResponse =
        parse_artifact_rpc_response_typed("artifact/write", value)?;
    assert_eq!(parsed.tool_id, tool_id);
    assert_eq!(parsed.artifact_id, artifact_id);
    assert_eq!(parsed.metadata.version, 1);
    Ok(())
}

#[test]
fn artifact_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
    let tool_id = omne_protocol::ToolId::new();
    let artifact_id = omne_protocol::ArtifactId::new();
    let value = serde_json::json!({
        "tool_id": tool_id,
        "artifact_id": artifact_id,
        "created": true,
        "content_path": "/tmp/artifact.txt",
        "metadata_path": "/tmp/artifact.json",
        "metadata": {
            "artifact_id": artifact_id,
            "artifact_type": "report",
            "summary": "summary",
            "created_at": "1970-01-01T00:00:00Z",
            "updated_at": "1970-01-01T00:00:00Z",
            "version": 1,
            "content_path": "/tmp/artifact.txt",
            "size_bytes": 12
        }
    });

    let outcome =
        parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactWriteResponse>(
            "artifact/write",
            value,
        )?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(omne_app_server_protocol::ArtifactWriteResponse {
            tool_id: t,
            artifact_id: a,
            created: true,
            ..
        }) if t == tool_id && a == artifact_id
    ));
    Ok(())
}

#[test]
fn artifact_rpc_outcome_denied_false_is_not_misclassified() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ArtifactDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: false,
        structured_error: None,
        error_code: None,
        remembered: None,
    })?;

    let outcome =
        parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactDeniedResponse>(
            "artifact/write",
            value,
        )?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(omne_app_server_protocol::ArtifactDeniedResponse {
            denied: false,
            ..
        })
    ));
    Ok(())
}
