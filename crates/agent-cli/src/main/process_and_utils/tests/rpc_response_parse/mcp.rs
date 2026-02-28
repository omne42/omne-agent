use super::*;

#[test]
fn mcp_rpc_failed_response_passthrough() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::McpFailedResponse {
        tool_id: omne_protocol::ToolId::new(),
        failed: true,
        error: "server timeout".to_string(),
        server: "demo".to_string(),
    })
    .expect("serialize McpFailedResponse");

    let parsed: McpActionOrFailedResponse = parse_mcp_rpc_response_typed("mcp/call", value)?;
    match parsed {
        McpActionOrFailedResponse::Failed(response) => {
            assert!(response.failed);
            assert_eq!(response.server, "demo");
        }
        McpActionOrFailedResponse::Action(_) => anyhow::bail!("expected failed response"),
    }
    Ok(())
}

#[test]
fn mcp_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(omne_app_server_protocol::McpNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    })?;

    let outcome = parse_mcp_rpc_outcome::<McpListServersOrFailedResponse>("mcp/list_servers", value)?;
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
fn mcp_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::McpDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        remembered: None,
        error_code: None,
    })?;

    let outcome = parse_mcp_rpc_outcome::<McpListServersOrFailedResponse>("mcp/list_servers", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Denied { detail }
            if detail.get("denied") == Some(&serde_json::Value::Bool(true))
    ));
    Ok(())
}

#[test]
fn mcp_rpc_denied_returns_error_includes_error_code() {
    let value = serde_json::to_value(omne_app_server_protocol::McpDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        remembered: None,
        error_code: Some("allowed_tools_denied".to_string()),
    })
    .expect("serialize McpDeniedResponse");

    let err = parse_mcp_rpc_response_typed::<McpListServersOrFailedResponse>(
        "mcp/list_servers",
        value,
    )
    .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("mcp/list_servers denied"));
    assert!(message.contains("allowed_tools_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn mcp_rpc_outcome_keeps_failed_passthrough() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::McpFailedResponse {
        tool_id: omne_protocol::ToolId::new(),
        failed: true,
        error: "server timeout".to_string(),
        server: "demo".to_string(),
    })?;

    let outcome = parse_mcp_rpc_outcome::<McpActionOrFailedResponse>("mcp/call", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(McpActionOrFailedResponse::Failed(response))
            if response.failed && response.server == "demo"
    ));
    Ok(())
}

#[test]
fn mcp_rpc_ok_requires_typed_success_shape() {
    let value = serde_json::json!({
        "foo": "bar",
    });
    let err = parse_mcp_rpc_response_typed::<McpListServersOrFailedResponse>(
        "mcp/list_servers",
        value,
    )
    .expect_err("expected error");
    assert!(err.to_string().contains("parse mcp/list_servers response"));
}

