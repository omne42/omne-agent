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

async fn handle_thread_hook_run(server: &Server, params: ThreadHookRunParams) -> anyhow::Result<Value> {
    let (_thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let config_dir = thread_root.join(".codepm_data").join("spec");
    let yaml_path = config_dir.join("workspace.yaml");
    let yml_path = config_dir.join("workspace.yml");

    let (config_path, config_contents) = match tokio::fs::read_to_string(&yaml_path).await {
        Ok(contents) => (yaml_path, contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            match tokio::fs::read_to_string(&yml_path).await {
                Ok(contents) => (yml_path, contents),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(serde_json::json!({
                        "ok": true,
                        "skipped": true,
                        "hook": thread_hook_key(params.hook),
                        "reason": "workspace hook config not found",
                        "searched": [
                            yaml_path.display().to_string(),
                            yml_path.display().to_string(),
                        ],
                    }));
                }
                Err(err) => return Err(err).with_context(|| format!("read {}", yml_path.display())),
            }
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", yaml_path.display())),
    };

    let config: WorkspaceHooksConfig =
        serde_yaml::from_str(&config_contents).with_context(|| format!("parse {}", config_path.display()))?;

    let key = thread_hook_key(params.hook);
    let argv = config.hooks.get(key).cloned().filter(|argv| !argv.is_empty());
    let Some(argv) = argv else {
        return Ok(serde_json::json!({
            "ok": true,
            "skipped": true,
            "hook": key,
            "reason": "workspace hook not configured",
            "config_path": config_path.display().to_string(),
        }));
    };

    let output = handle_process_start(
        server,
        ProcessStartParams {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            argv: argv.clone(),
            cwd: None,
        },
    )
    .await?;

    let Some(obj) = output.as_object() else {
        return Ok(output);
    };

    if obj
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(serde_json::json!({
            "needs_approval": true,
            "thread_id": params.thread_id,
            "approval_id": obj.get("approval_id"),
            "hook": key,
        }));
    }

    if obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false) {
        let mut out = obj.clone();
        out.insert("hook".to_string(), serde_json::json!(key));
        out.insert(
            "config_path".to_string(),
            serde_json::json!(config_path.display().to_string()),
        );
        return Ok(Value::Object(out));
    }

    Ok(serde_json::json!({
        "ok": true,
        "hook": key,
        "argv": argv,
        "config_path": config_path.display().to_string(),
        "process_id": obj.get("process_id"),
        "stdout_path": obj.get("stdout_path"),
        "stderr_path": obj.get("stderr_path"),
    }))
}
