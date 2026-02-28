use super::*;

#[test]
fn thread_hook_run_rpc_needs_approval_returns_actionable_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value =
        serde_json::to_value(omne_app_server_protocol::ThreadHookRunNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
            hook: "run".to_string(),
        })
        .expect("serialize ThreadHookRunNeedsApprovalResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run needs approval"));
    assert!(message.contains(&approval_id.to_string()));
}

#[test]
fn thread_hook_run_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let approval_id = omne_protocol::ApprovalId::new();
    let value =
        serde_json::to_value(omne_app_server_protocol::ThreadHookRunNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
            hook: "run".to_string(),
        })?;

    let outcome = parse_thread_hook_run_rpc_outcome("thread/hook_run", value)?;
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
fn thread_hook_run_rpc_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: Some("sandbox_policy_denied".to_string()),
        config_path: None,
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
            omne_app_server_protocol::ProcessDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                thread_id,
                remembered: None,
                error_code: None,
            },
        ),
    })
    .expect("serialize ThreadHookRunDeniedResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run denied"));
    assert!(message.contains("sandbox_policy_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_hook_run_rpc_mode_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: Some("mode_denied".to_string()),
        config_path: None,
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(
            omne_app_server_protocol::ProcessModeDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                thread_id,
                mode: "hook-mode-deny".to_string(),
                decision: omne_app_server_protocol::ProcessModeDecision::Deny,
                decision_source: "mode_permission".to_string(),
                tool_override_hit: false,
                error_code: None,
            },
        ),
    })
    .expect("serialize ThreadHookRunDeniedResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run denied"));
    assert!(message.contains("mode_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_hook_run_rpc_mode_unknown_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: Some("mode_unknown".to_string()),
        config_path: None,
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(
            omne_app_server_protocol::ProcessUnknownModeDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                thread_id,
                mode: "hook-mode".to_string(),
                decision: omne_app_server_protocol::ProcessModeDecision::Deny,
                available: "other-mode".to_string(),
                load_error: None,
                error_code: None,
            },
        ),
    })
    .expect("serialize ThreadHookRunDeniedResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run denied"));
    assert!(message.contains("mode_unknown"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_hook_run_rpc_allowed_tools_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: Some("allowed_tools_denied".to_string()),
        config_path: Some(".omne_data/spec/workspace.yaml".to_string()),
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(
            omne_app_server_protocol::ProcessAllowedToolsDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                tool: "process/start".to_string(),
                allowed_tools: vec!["repo/search".to_string()],
                error_code: None,
            },
        ),
    })
    .expect("serialize ThreadHookRunDeniedResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run denied"));
    assert!(message.contains("allowed_tools_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_hook_run_rpc_execpolicy_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: Some("execpolicy_denied".to_string()),
        config_path: Some(".omne_data/spec/workspace.yaml".to_string()),
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(
            omne_app_server_protocol::ProcessExecPolicyDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                decision: omne_app_server_protocol::ExecPolicyDecision::Forbidden,
                matched_rules: vec![],
                justification: None,
                error_code: None,
            },
        ),
    })
    .expect("serialize ThreadHookRunDeniedResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run denied"));
    assert!(message.contains("execpolicy_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_hook_run_rpc_execpolicy_load_denied_returns_error() {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: Some("execpolicy_load_denied".to_string()),
        config_path: Some(".omne_data/spec/workspace.yaml".to_string()),
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(
            omne_app_server_protocol::ProcessExecPolicyLoadDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                mode: "coder".to_string(),
                error: "failed to load thread execpolicy rules".to_string(),
                details: "missing rules/missing.rules".to_string(),
                error_code: None,
            },
        ),
    })
    .expect("serialize ThreadHookRunDeniedResponse");

    let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
        .expect_err("expected error");
    let message = err.to_string();
    assert!(message.contains("thread/hook_run denied"));
    assert!(message.contains("execpolicy_load_denied"));
    assert!(message.contains("[rpc error_code]"));
}

#[test]
fn thread_hook_run_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
    let thread_id = omne_protocol::ThreadId::new();
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
        denied: true,
        thread_id,
        hook: "run".to_string(),
        error_code: None,
        config_path: None,
        detail: omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
            omne_app_server_protocol::ProcessDeniedResponse {
                tool_id: omne_protocol::ToolId::new(),
                denied: true,
                thread_id,
                remembered: None,
                error_code: None,
            },
        ),
    })?;

    let outcome = parse_thread_hook_run_rpc_outcome("thread/hook_run", value)?;
    assert!(matches!(outcome, RpcGateOutcome::Denied { .. }));
    Ok(())
}

#[test]
fn thread_hook_run_rpc_ok_passthrough() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
        omne_app_server_protocol::ThreadHookRunResponse {
            ok: true,
            skipped: false,
            hook: "run".to_string(),
            reason: None,
            searched: None,
            config_path: None,
            argv: None,
            process_id: None,
            stdout_path: None,
            stderr_path: None,
        },
    ))
    .expect("serialize ThreadHookRunRpcResponse::Ok");

    let parsed = parse_thread_hook_run_rpc_response("thread/hook_run", value)?;
    assert!(parsed.ok);
    assert_eq!(parsed.hook, "run");
    Ok(())
}

#[test]
fn thread_hook_run_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
    let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
        omne_app_server_protocol::ThreadHookRunResponse {
            ok: true,
            skipped: false,
            hook: "run".to_string(),
            reason: None,
            searched: None,
            config_path: None,
            argv: None,
            process_id: None,
            stdout_path: None,
            stderr_path: None,
        },
    ))?;

    let outcome = parse_thread_hook_run_rpc_outcome("thread/hook_run", value)?;
    assert!(matches!(
        outcome,
        RpcGateOutcome::Ok(omne_app_server_protocol::ThreadHookRunResponse { ok: true, .. })
    ));
    Ok(())
}
