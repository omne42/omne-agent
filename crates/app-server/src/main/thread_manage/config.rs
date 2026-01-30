use crate::model_limits::resolve_model_limits;

async fn handle_thread_configure(
    server: &Server,
    params: ThreadConfigureParams,
) -> anyhow::Result<Value> {
    let (rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (
        current_approval_policy,
        current_sandbox_policy,
        current_sandbox_writable_roots,
        current_sandbox_network_access,
        current_mode,
        current_openai_provider,
        current_model,
        current_thinking,
        current_openai_base_url,
        current_allowed_tools,
    ) = {
        let handle = rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.sandbox_network_access,
            state.mode.clone(),
            state.openai_provider.clone(),
            state.model.clone(),
            state.thinking.clone(),
            state.openai_base_url.clone(),
            state.allowed_tools.clone(),
        )
    };

    let approval_policy = params.approval_policy.unwrap_or(current_approval_policy);
    let sandbox_writable_roots = params.sandbox_writable_roots.map(|roots| {
        roots
            .into_iter()
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>()
    });
    let sandbox_writable_roots = match sandbox_writable_roots {
        Some(roots) => {
            let mut out = Vec::<String>::new();
            let mut seen = std::collections::BTreeSet::<String>::new();
            for root in roots {
                let resolved =
                    pm_core::resolve_dir_unrestricted(&thread_root, Path::new(&root)).await?;
                let resolved = resolved.display().to_string();
                if seen.insert(resolved.clone()) {
                    out.push(resolved);
                }
            }
            Some(out)
        }
        None => None,
    };
    let sandbox_network_access = params.sandbox_network_access;
    let mode = params
        .mode
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    if let Some(mode) = mode.as_deref() {
        let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
        if catalog.mode(mode).is_none() {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            anyhow::bail!("unknown mode: {mode} (available: {available})");
        }
    }
    let openai_provider = params
        .openai_provider
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    if let Some(provider) = openai_provider.as_deref() {
        let project = crate::project_config::load_project_openai_overrides(&thread_root).await;
        let builtin = matches!(provider, "openai-codex-apikey" | "openai-auth-command");
        let configured = project.providers.contains_key(provider);
        if !builtin && !configured {
            anyhow::bail!(
                "unknown openai provider: {provider} (expected: openai-codex-apikey, openai-auth-command; or define [openai.providers.{provider}] in project config)"
            );
        }
    }
    let model = params.model.filter(|s| !s.trim().is_empty());
    let thinking = params
        .thinking
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| match value.to_ascii_lowercase().as_str() {
            "small" | "medium" | "high" | "xhigh" | "unsupported" => Ok(value.to_string()),
            other => anyhow::bail!(
                "invalid thinking: {other} (expected: small|medium|high|xhigh|unsupported)"
            ),
        })
        .transpose()?;
    let openai_base_url = params.openai_base_url.filter(|s| !s.trim().is_empty());
    let allowed_tools = match params.allowed_tools {
        None => None,
        Some(None) => Some(None),
        Some(Some(tools)) => Some(Some(normalize_allowed_tools(tools)?)),
    };
    let allowed_tools_changed = match &allowed_tools {
        None => false,
        Some(None) => current_allowed_tools.is_some(),
        Some(Some(tools)) => current_allowed_tools.as_ref() != Some(tools),
    };

    let changed = approval_policy != current_approval_policy
        || params
            .sandbox_policy
            .is_some_and(|p| p != current_sandbox_policy)
        || sandbox_writable_roots
            .as_ref()
            .is_some_and(|roots| roots != &current_sandbox_writable_roots)
        || sandbox_network_access
            .is_some_and(|access| access != current_sandbox_network_access)
        || mode.as_ref().is_some_and(|m| m != &current_mode)
        || openai_provider.as_ref() != current_openai_provider.as_ref()
        || model.as_ref() != current_model.as_ref()
        || thinking.as_ref() != current_thinking.as_ref()
        || openai_base_url.as_ref() != current_openai_base_url.as_ref()
        || allowed_tools_changed;

    if changed {
        rt.append_event(pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy: params.sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            openai_provider,
            model,
            thinking,
            openai_base_url,
            allowed_tools,
        })
        .await?;
    }
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_thread_config_explain(
    server: &Server,
    params: ThreadConfigExplainParams,
) -> anyhow::Result<Value> {
    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let thread_cwd = events
        .iter()
        .find_map(|event| match &event.kind {
            pm_protocol::ThreadEventKind::ThreadCreated { cwd, .. } => Some(cwd.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", params.thread_id))?;
    let thread_root = pm_core::resolve_dir(Path::new(&thread_cwd), Path::new(".")).await?;
    let mode_catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;

    let default_model = "gpt-4.1".to_string();
    let default_openai_provider = "openai-codex-apikey".to_string();
    let default_openai_base_url = "https://api.openai.com/v1".to_string();
    let default_mode = "coder".to_string();
    let default_thinking = thinking_label(ditto_llm::ThinkingIntensity::default()).to_string();

    let mut effective_approval_policy = pm_protocol::ApprovalPolicy::AutoApprove;
    let mut effective_sandbox_policy = pm_protocol::SandboxPolicy::WorkspaceWrite;
    let mut effective_sandbox_writable_roots = Vec::<String>::new();
    let mut effective_sandbox_network_access = pm_protocol::SandboxNetworkAccess::Deny;
    let mut effective_mode = default_mode.clone();
    let mut effective_openai_provider = default_openai_provider.clone();
    let mut effective_model = default_model.clone();
    let mut effective_thinking = default_thinking.clone();
    let mut effective_openai_base_url = default_openai_base_url.clone();
    let mut effective_allowed_tools: Option<Vec<String>> = None;
    let mut layers = vec![serde_json::json!({
        "source": "default",
        "approval_policy": effective_approval_policy,
        "sandbox_policy": effective_sandbox_policy,
        "sandbox_writable_roots": effective_sandbox_writable_roots,
        "sandbox_network_access": effective_sandbox_network_access,
        "mode": effective_mode,
        "openai_provider": effective_openai_provider,
        "model": effective_model,
        "thinking": effective_thinking,
        "openai_base_url": effective_openai_base_url,
        "allowed_tools": effective_allowed_tools,
    })];

    let env_provider = std::env::var("CODE_PM_OPENAI_PROVIDER")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_model = std::env::var("CODE_PM_OPENAI_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_openai_base_url = std::env::var("CODE_PM_OPENAI_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    if env_provider.is_some() || env_model.is_some() || env_openai_base_url.is_some() {
        if let Some(provider) = env_provider.as_deref() {
            effective_openai_provider = provider.to_string();
        }
        if let Some(model) = env_model.as_deref() {
            effective_model = model.to_string();
        }
        if let Some(openai_base_url) = env_openai_base_url.as_deref() {
            effective_openai_base_url = openai_base_url.to_string();
        }
        layers.push(serde_json::json!({
            "source": "env",
            "openai_provider": effective_openai_provider,
            "model": effective_model,
            "thinking": effective_thinking,
            "openai_base_url": effective_openai_base_url,
        }));
    }

    let project = crate::project_config::load_project_config(&thread_root).await;
    let project_thinking_for_model = |model: &str| -> String {
        if project.enabled {
            let thinking = ditto_llm::select_model_config(&project.openai.models, model)
                .map(|cfg| cfg.thinking)
                .unwrap_or_default();
            thinking_label(thinking).to_string()
        } else {
            default_thinking.clone()
        }
    };

    if project.enabled {
        if let Some(provider) = project.openai.provider.as_deref() {
            effective_openai_provider = provider.to_string();
        }
        if let Some(model) = project.openai.model.as_deref() {
            effective_model = model.to_string();
        }
        if let Some(openai_base_url) = project.openai.base_url.as_deref() {
            effective_openai_base_url = openai_base_url.to_string();
        } else if env_openai_base_url.is_none() {
            if let Some(provider_base_url) = project
                .openai
                .providers
                .get(&effective_openai_provider)
                .and_then(|provider| provider.base_url.clone())
                .or_else(|| match effective_openai_provider.as_str() {
                    "openai-codex-apikey" | "openai-auth-command" => {
                        Some(default_openai_base_url.clone())
                    }
                    _ => None,
                })
            {
                effective_openai_base_url = provider_base_url;
            }
        }
        effective_thinking = project_thinking_for_model(&effective_model);
        layers.push(serde_json::json!({
            "source": "project",
            "enabled": true,
            "config_path": project.config_path.display().to_string(),
            "config_source": project.config_source.as_str(),
            "config_present": project.config_present,
            "env_path": project.env_path.display().to_string(),
            "env_present": project.env_present,
            "load_error": project.load_error,
            "openai_provider": effective_openai_provider,
            "model": effective_model,
            "thinking": effective_thinking,
            "openai_base_url": effective_openai_base_url,
        }));
    } else if project.config_present || project.load_error.is_some() {
        layers.push(serde_json::json!({
            "source": "project",
            "enabled": false,
            "config_path": project.config_path.display().to_string(),
            "config_source": project.config_source.as_str(),
            "config_present": project.config_present,
            "env_path": project.env_path.display().to_string(),
            "env_present": project.env_present,
            "load_error": project.load_error,
        }));
    }

    let provider_base_url_for = |provider: &str| -> Option<String> {
        if project.enabled {
            if let Some(provider_base_url) = project
                .openai
                .providers
                .get(provider)
                .and_then(|provider| provider.base_url.clone())
                .filter(|s| !s.trim().is_empty())
            {
                return Some(provider_base_url);
            }
        }
        if matches!(provider, "openai-codex-apikey" | "openai-auth-command") {
            return Some(default_openai_base_url.clone());
        }
        None
    };
    let mut openai_base_url_forced = env_openai_base_url.is_some()
        || (project.enabled && project.openai.base_url.as_ref().is_some_and(|s| !s.trim().is_empty()));

    let mut thinking_override: Option<String> = None;
    for event in events {
        if let pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            openai_provider,
            model,
            thinking,
            openai_base_url,
            allowed_tools,
        } = event.kind
        {
            let ts = event.timestamp.format(&Rfc3339)?;
            effective_approval_policy = approval_policy;
            if let Some(policy) = sandbox_policy {
                effective_sandbox_policy = policy;
            }
            if let Some(roots) = sandbox_writable_roots {
                effective_sandbox_writable_roots = roots;
            }
            if let Some(access) = sandbox_network_access {
                effective_sandbox_network_access = access;
            }
            if let Some(mode) = mode {
                effective_mode = mode;
            }
            if let Some(provider) = openai_provider {
                effective_openai_provider = provider;
                if !openai_base_url_forced
                    && let Some(provider_base_url) =
                        provider_base_url_for(&effective_openai_provider)
                {
                    effective_openai_base_url = provider_base_url;
                }
            }
            if let Some(model) = model {
                effective_model = model;
                if thinking_override.is_none() {
                    effective_thinking = project_thinking_for_model(&effective_model);
                }
            }
            if let Some(thinking) = thinking {
                effective_thinking = thinking.clone();
                thinking_override = Some(thinking);
            }
            if let Some(openai_base_url) = openai_base_url {
                effective_openai_base_url = openai_base_url;
                openai_base_url_forced = true;
            }
            if let Some(allowed_tools) = allowed_tools {
                effective_allowed_tools = allowed_tools;
            }
            layers.push(serde_json::json!({
                "source": "thread",
                "seq": event.seq.0,
                "timestamp": ts,
                "approval_policy": approval_policy,
                "sandbox_policy": effective_sandbox_policy,
                "sandbox_writable_roots": effective_sandbox_writable_roots,
                "sandbox_network_access": effective_sandbox_network_access,
                "mode": effective_mode,
                "openai_provider": effective_openai_provider,
                "model": effective_model,
                "thinking": effective_thinking,
                "openai_base_url": effective_openai_base_url,
                "allowed_tools": effective_allowed_tools,
            }));
        }
    }

    let (mode_catalog_source, mode_catalog_path) = match &mode_catalog.source {
        pm_core::modes::ModeCatalogSource::Builtin => ("builtin", None),
        pm_core::modes::ModeCatalogSource::Env(path) => ("env", Some(path.display().to_string())),
        pm_core::modes::ModeCatalogSource::Project(path) => ("project", Some(path.display().to_string())),
    };
    let available_modes = mode_catalog
        .mode_names()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let effective_mode_name = effective_mode.clone();
    let effective_mode_def = mode_catalog.mode(&effective_mode).map(|mode| {
        serde_json::json!({
            "name": effective_mode_name,
            "description": mode.description.as_str(),
            "permissions": {
                "read": mode.permissions.read,
                "edit": {
                    "decision": mode.permissions.edit.decision,
                    "allow_globs": &mode.permissions.edit.allow_globs,
                    "deny_globs": &mode.permissions.edit.deny_globs,
                },
                "command": mode.permissions.command,
                "process": {
                    "inspect": mode.permissions.process.inspect,
                    "kill": mode.permissions.process.kill,
                    "interact": mode.permissions.process.interact,
                },
                "artifact": mode.permissions.artifact,
                "browser": mode.permissions.browser,
            },
            "tool_overrides": &mode.tool_overrides,
        })
    });

    let model_config = if project.enabled {
        ditto_llm::select_model_config(&project.openai.models, &effective_model)
    } else {
        None
    };
    let limits = resolve_model_limits(&effective_model, model_config);
    let model_context_window = limits
        .context_window
        .map(|window| crate::model_limits::effective_context_window_for_model(&effective_model, window));

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "effective": {
            "approval_policy": effective_approval_policy,
            "sandbox_policy": effective_sandbox_policy,
            "sandbox_writable_roots": effective_sandbox_writable_roots,
            "sandbox_network_access": effective_sandbox_network_access,
            "mode": effective_mode,
            "openai_provider": effective_openai_provider,
            "model": effective_model,
            "thinking": effective_thinking,
            "openai_base_url": effective_openai_base_url,
            "allowed_tools": effective_allowed_tools,
            "model_context_window": model_context_window,
            "auto_compact_token_limit": limits.auto_compact_token_limit,
        },
        "mode_catalog": {
            "source": mode_catalog_source,
            "path": mode_catalog_path,
            "load_error": mode_catalog.load_error,
            "modes": available_modes,
        },
        "effective_mode_def": effective_mode_def,
        "layers": layers,
    }))
}

fn thinking_label(value: ditto_llm::ThinkingIntensity) -> &'static str {
    match value {
        ditto_llm::ThinkingIntensity::Unsupported => "unsupported",
        ditto_llm::ThinkingIntensity::Small => "small",
        ditto_llm::ThinkingIntensity::Medium => "medium",
        ditto_llm::ThinkingIntensity::High => "high",
        ditto_llm::ThinkingIntensity::XHigh => "xhigh",
    }
}

const KNOWN_ALLOWED_TOOLS: &[&str] = &[
    "file/read",
    "file/glob",
    "file/grep",
    "file/write",
    "file/patch",
    "file/edit",
    "file/delete",
    "fs/mkdir",
    "repo/search",
    "repo/index",
    "repo/symbols",
    "mcp/list_servers",
    "mcp/list_tools",
    "mcp/list_resources",
    "mcp/call",
    "artifact/write",
    "artifact/list",
    "artifact/read",
    "artifact/delete",
    "process/start",
    "process/list",
    "process/inspect",
    "process/kill",
    "process/interrupt",
    "process/tail",
    "process/follow",
];

fn normalize_allowed_tools(tools: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for tool in tools {
        let trimmed = tool.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !KNOWN_ALLOWED_TOOLS.contains(&trimmed) {
            let known = KNOWN_ALLOWED_TOOLS.join(", ");
            anyhow::bail!("unknown tool: {trimmed} (known tools: {known})");
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}
