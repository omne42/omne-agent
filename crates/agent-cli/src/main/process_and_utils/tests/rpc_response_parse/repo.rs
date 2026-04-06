use super::*;

#[test]
fn repo_rpc_needs_approval_returns_actionable_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(omne_app_server_protocol::RepoNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    })
    .expect("serialize RepoNeedsApprovalResponse");

    let err =
        parse_repo_rpc_response_typed::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)
            .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("repo/search needs approval"));
    assert!(message.contains(&approval_id.to_string()));
}

#[test]
fn repo_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(omne_app_server_protocol::RepoNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    })?;

    let outcome =
        parse_repo_rpc_outcome::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)?;
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
fn repo_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::RepoDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        remembered: None,
        structured_error: None,
        error_code: None,
    })?;

    let outcome =
        parse_repo_rpc_outcome::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Denied { detail }
            if detail.get("denied") == Some(&serde_json::Value::Bool(true))
    ));
    Ok(())
}

#[test]
fn repo_rpc_denied_returns_error_includes_error_code() {
    let value = serde_json::to_value(omne_app_server_protocol::RepoDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        remembered: None,
        structured_error: None,
        error_code: Some("mode_denied".to_string()),
    })
    .expect("serialize RepoDeniedResponse");

    let err =
        parse_repo_rpc_response_typed::<omne_app_server_protocol::RepoSearchResponse>(
            "repo/search",
            value,
        )
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("repo/search denied"));
    assert!(message.contains("mode_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn repo_rpc_ok_requires_typed_success_shape() {
    let value = serde_json::json!({
        "tool_id": "tool_123",
        "text": "hello",
    });
    let err =
        parse_repo_rpc_response_typed::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)
            .expect_err("expected error");
    assert!(err.to_string().contains("parse repo/search response"));
}
