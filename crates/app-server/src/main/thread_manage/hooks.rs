#[derive(Debug, Deserialize)]
struct WorkspaceHooksConfig {
    #[serde(default)]
    hooks: HashMap<String, Vec<String>>,
}

fn thread_hook_key(hook: WorkspaceHookName) -> &'static str {
    match hook {
        WorkspaceHookName::Setup => "setup",
        WorkspaceHookName::Run => "run",
        WorkspaceHookName::Archive => "archive",
    }
}

fn thread_hook_run_denied_error_code(
    detail: &omne_app_server_protocol::ThreadProcessDeniedDetail,
) -> Option<String> {
    match detail {
        omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxPolicyDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxNetworkDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
        omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail) => detail
            .structured_error
            .as_ref()
            .and_then(|value| value.catalog_code().map(ToOwned::to_owned))
            .or_else(|| detail.error_code.clone()),
    }
}

async fn run_workspace_hook_inner(
    server: &Server,
    params: ThreadHookRunParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadHookRunRpcResponse> {
    #[cfg(test)]
    let _env_lock = crate::app_server_process_env_lock().lock().await;

    let (_thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let config_dir = thread_root.join(".omne_data").join("spec");
    let yaml_path = config_dir.join("workspace.yaml");
    let yml_path = config_dir.join("workspace.yml");

    let (config_path, config) = if let Some(config) =
        config_kit::try_load_typed_config_file::<WorkspaceHooksConfig>(
            &yaml_path,
            config_kit::ConfigLoadOptions::new(),
            config_kit::ConfigFormatSet::YAML,
        )
        .with_context(|| format!("load {}", yaml_path.display()))?
    {
        (yaml_path, config)
    } else if let Some(config) = config_kit::try_load_typed_config_file::<WorkspaceHooksConfig>(
        &yml_path,
        config_kit::ConfigLoadOptions::new(),
        config_kit::ConfigFormatSet::YAML,
    )
    .with_context(|| format!("load {}", yml_path.display()))?
    {
        (yml_path, config)
    } else {
        let response = omne_app_server_protocol::ThreadHookRunResponse {
            ok: true,
            skipped: true,
            hook: thread_hook_key(params.hook).to_string(),
            reason: Some("workspace hook config not found".to_string()),
            searched: Some(vec![
                yaml_path.display().to_string(),
                yml_path.display().to_string(),
            ]),
            config_path: None,
            argv: None,
            process_id: None,
            stdout_path: None,
            stderr_path: None,
        };
        return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
            response,
        ));
    };

    let key = thread_hook_key(params.hook);
    let argv = config
        .hooks
        .get(key)
        .cloned()
        .filter(|argv| !argv.is_empty());
    let Some(argv) = argv else {
        let response = omne_app_server_protocol::ThreadHookRunResponse {
            ok: true,
            skipped: true,
            hook: key.to_string(),
            reason: Some("workspace hook not configured".to_string()),
            searched: None,
            config_path: Some(config_path.display().to_string()),
            argv: None,
            process_id: None,
            stdout_path: None,
            stderr_path: None,
        };
        return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
            response,
        ));
    };

    let output = handle_process_start(
        server,
        ProcessStartParams {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            argv: argv.clone(),
            cwd: None,
            timeout_ms: None,
        },
    )
    .await?;

    let Some(obj) = output.as_object() else {
        anyhow::bail!("unexpected thread/hook_run process/start response shape");
    };

    if obj
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let approval_id = obj
            .get("approval_id")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("thread/hook_run missing approval_id"))?;
        let approval_id = serde_json::from_value(approval_id)
            .context("parse thread/hook_run approval_id response field")?;
        let response = omne_app_server_protocol::ThreadHookRunNeedsApprovalResponse {
            needs_approval: true,
            thread_id: params.thread_id,
            approval_id,
            hook: key.to_string(),
        };
        return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::NeedsApproval(response));
    }

    if obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false) {
        let detail = serde_json::from_value::<omne_app_server_protocol::ThreadProcessDeniedDetail>(
            output.clone(),
        )
        .context("parse thread/hook_run denied detail")?;
        let response = omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id: params.thread_id,
            hook: key.to_string(),
            structured_error: structured_error_from_result_value(&output),
            error_code: thread_hook_run_denied_error_code(&detail),
            config_path: Some(config_path.display().to_string()),
            detail,
        };
        return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(
            response,
        ));
    }

    let process_id = obj
        .get("process_id")
        .cloned()
        .map(serde_json::from_value::<ProcessId>)
        .transpose()
        .context("parse thread/hook_run process_id response field")?;
    let stdout_path = obj
        .get("stdout_path")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let stderr_path = obj
        .get("stderr_path")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let response = omne_app_server_protocol::ThreadHookRunResponse {
        ok: true,
        skipped: false,
        hook: key.to_string(),
        reason: None,
        searched: None,
        config_path: Some(config_path.display().to_string()),
        argv: Some(argv),
        process_id,
        stdout_path,
        stderr_path,
    };
    Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
        response,
    ))
}

async fn run_auto_workspace_hook(
    server: &Server,
    thread_id: ThreadId,
    hook: WorkspaceHookName,
) -> omne_app_server_protocol::ThreadAutoHookResponse {
    match run_workspace_hook_inner(
        server,
        ThreadHookRunParams {
            thread_id,
            turn_id: None,
            approval_id: None,
            hook,
        },
    )
    .await
    {
        Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::NeedsApproval(response)) => {
            omne_app_server_protocol::ThreadAutoHookResponse::NeedsApproval(response)
        }
        Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response)) => {
            omne_app_server_protocol::ThreadAutoHookResponse::Denied(response)
        }
        Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(response)) => {
            omne_app_server_protocol::ThreadAutoHookResponse::Ok(response)
        }
        Err(err) => omne_app_server_protocol::ThreadAutoHookResponse::Error(
            omne_app_server_protocol::ThreadHookRunErrorResponse {
                ok: false,
                hook: thread_hook_key(hook).to_string(),
                error: err.to_string(),
                structured_error: catalog_structured_error_with(
                    "thread_hook_run_failed",
                    |message| {
                        message.try_with_value_arg("hook", thread_hook_key(hook))?;
                        message.try_with_value_arg("error", err.to_string())?;
                        Ok(())
                    },
                )
                .ok(),
            },
        ),
    }
}

async fn handle_thread_hook_run(
    server: &Server,
    params: ThreadHookRunParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadHookRunRpcResponse> {
    run_workspace_hook_inner(server, params).await
}
