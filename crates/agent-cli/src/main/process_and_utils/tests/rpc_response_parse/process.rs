use super::*;

#[test]
fn process_rpc_denied_returns_error() {
    let value = serde_json::to_value(omne_app_server_protocol::ProcessDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        thread_id: omne_protocol::ThreadId::new(),
        remembered: None,
        structured_error: None,
        error_code: Some("sandbox_policy_denied".to_string()),
    })
    .expect("serialize ProcessDeniedResponse");

    let err = parse_process_rpc_response_typed::<omne_app_server_protocol::ProcessTailResponse>(
        "process/tail",
        value,
    )
    .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("process/tail denied"));
    assert!(message.contains("sandbox_policy_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn process_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ProcessDeniedResponse {
        tool_id: omne_protocol::ToolId::new(),
        denied: true,
        thread_id: omne_protocol::ThreadId::new(),
        remembered: None,
        structured_error: None,
        error_code: None,
    })?;

    let outcome = parse_process_rpc_outcome::<omne_app_server_protocol::ProcessTailResponse>(
        "process/tail",
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
fn process_rpc_ok_passthrough() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ProcessTailResponse {
        tool_id: omne_protocol::ToolId::new(),
        text: "hello".to_string(),
    })
    .expect("serialize ProcessTailResponse");

    let parsed: omne_app_server_protocol::ProcessTailResponse =
        parse_process_rpc_response_typed("process/tail", value)?;
    assert_eq!(parsed.text, "hello");
    Ok(())
}

#[test]
fn process_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ProcessTailResponse {
        tool_id: omne_protocol::ToolId::new(),
        text: "hello".to_string(),
    })?;

    let outcome = parse_process_rpc_outcome::<omne_app_server_protocol::ProcessTailResponse>(
        "process/tail",
        value,
    )?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(omne_app_server_protocol::ProcessTailResponse { text, .. })
            if text == "hello"
    ));
    Ok(())
}

#[test]
fn process_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ProcessNeedsApprovalResponse {
        needs_approval: true,
        thread_id,
        approval_id,
    })?;

    let outcome = parse_process_rpc_outcome::<omne_app_server_protocol::ProcessTailResponse>(
        "process/tail",
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
