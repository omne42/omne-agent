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
        omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxPolicyDenied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxNetworkDenied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail) => {
            detail.error_code.clone()
        }
        omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail) => {
            detail.error_code.clone()
        }
    }
}

async fn run_workspace_hook_inner(
    server: &Server,
    params: ThreadHookRunParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadHookRunRpcResponse> {
    let (_thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let config_dir = thread_root.join(".omne_data").join("spec");
    let yaml_path = config_dir.join("workspace.yaml");
    let yml_path = config_dir.join("workspace.yml");

    let (config_path, config_contents) = match tokio::fs::read_to_string(&yaml_path).await {
        Ok(contents) => (yaml_path, contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            match tokio::fs::read_to_string(&yml_path).await {
                Ok(contents) => (yml_path, contents),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
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
                        exit_code: None,
                        stdout_path: None,
                        stderr_path: None,
                    };
                    return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
                        response,
                    ));
                }
                Err(err) => {
                    return Err(err).with_context(|| format!("read {}", yml_path.display()));
                }
            }
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", yaml_path.display())),
    };

    let config: WorkspaceHooksConfig = serde_yaml::from_str(&config_contents)
        .with_context(|| format!("parse {}", config_path.display()))?;

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
            exit_code: None,
            stdout_path: None,
            stderr_path: None,
        };
        return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(response));
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
        return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::NeedsApproval(
            response,
        ));
    }

    if obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false) {
        let detail =
            serde_json::from_value::<omne_app_server_protocol::ThreadProcessDeniedDetail>(output)
                .context("parse thread/hook_run denied detail")?;
        let response = omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id: params.thread_id,
            hook: key.to_string(),
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
        .ok_or_else(|| anyhow::anyhow!("thread/hook_run missing process_id"))?;
    let process_id = serde_json::from_value::<ProcessId>(process_id)
        .context("parse thread/hook_run process_id response field")?;
    let stdout_path = obj
        .get("stdout_path")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let stderr_path = obj
        .get("stderr_path")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);

    // `thread/hook_run` must not return before the hook process exits; callers commonly spawn
    // `omne-app-server` in stdio mode for a single RPC, and disconnecting would terminate the
    // server (dropping the process actor and losing stdout/stderr + exit_code persistence).
    let exit_code = match wait_for_workspace_hook_exit(server, process_id).await {
        Ok(code) => code,
        Err(err) => {
            let response = omne_app_server_protocol::ThreadHookRunResponse {
                ok: false,
                skipped: false,
                hook: key.to_string(),
                reason: Some(err.to_string()),
                searched: None,
                config_path: Some(config_path.display().to_string()),
                argv: Some(argv),
                process_id: Some(process_id),
                exit_code: None,
                stdout_path,
                stderr_path,
            };
            return Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
                response,
            ));
        }
    };

    let (ok, reason) = match exit_code {
        Some(0) => (true, None),
        Some(code) => (
            false,
            Some(format!("workspace hook process exited with code {code}")),
        ),
        None => (
            false,
            Some("workspace hook process exited without an exit code".to_string()),
        ),
    };
    let response = omne_app_server_protocol::ThreadHookRunResponse {
        ok,
        skipped: false,
        hook: key.to_string(),
        reason,
        searched: None,
        config_path: Some(config_path.display().to_string()),
        argv: Some(argv),
        process_id: Some(process_id),
        exit_code,
        stdout_path,
        stderr_path,
    };
    Ok(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
        response,
    ))
}

fn workspace_hook_process_timeout() -> Option<Duration> {
    // 0 means "no timeout". Keep the default off; workspace hooks are user-defined and may run
    // longer than our advisory hook dispatch (which has a short cap).
    let value = std::env::var("OMNE_WORKSPACE_HOOK_PROCESS_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if value == 0 {
        None
    } else {
        // Clamp to avoid accidentally waiting "forever" due to a typo (e.g. ms vs secs).
        Some(Duration::from_secs(value.min(24 * 60 * 60)))
    }
}

async fn wait_for_workspace_hook_exit(
    server: &Server,
    process_id: ProcessId,
) -> anyhow::Result<Option<i32>> {
    let deadline = workspace_hook_process_timeout().map(|timeout| tokio::time::Instant::now() + timeout);
    loop {
        let entry = {
            let processes = server.processes.lock().await;
            processes.get(&process_id).cloned()
        };
        let Some(entry) = entry else {
            return Ok(None);
        };

        let (status, exit_code) = {
            let info = entry.info.lock().await;
            (info.status.clone(), info.exit_code)
        };

        if matches!(status, ProcessStatus::Exited | ProcessStatus::Abandoned) {
            return Ok(exit_code);
        }

        if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
            let _ = entry
                .cmd_tx
                .send(ProcessCommand::Kill {
                    reason: Some("workspace hook timeout".to_string()),
                })
                .await;
            anyhow::bail!("workspace hook process timed out: {}", process_id);
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
