use crate::model_limits::{effective_auto_compact_token_limit, resolve_model_limits};

#[derive(Debug)]
enum ThreadConfigureSpecError {
    UnknownMode {
        mode: String,
        available: String,
    },
    UnknownRole {
        role: String,
        available: String,
    },
    UnknownAllowedTool {
        tool: String,
        known: String,
    },
    AllowedToolDeniedByMode {
        mode: String,
        tool: String,
    },
    AllowedToolDeniedByRole {
        role: String,
        permission_mode: String,
        tool: String,
    },
    AllowedToolDecisionMappingMissing {
        tool: String,
    },
}

impl ThreadConfigureSpecError {
    fn error_code(&self) -> &'static str {
        match self {
            Self::UnknownMode { .. } => "mode_unknown",
            Self::UnknownRole { .. } => "role_unknown",
            Self::UnknownAllowedTool { .. } => "allowed_tools_unknown_tool",
            Self::AllowedToolDeniedByMode { .. } => "allowed_tools_mode_denied",
            Self::AllowedToolDeniedByRole { .. } => "allowed_tools_mode_denied",
            Self::AllowedToolDecisionMappingMissing { .. } => "allowed_tools_mapping_missing",
        }
    }
}

impl std::fmt::Display for ThreadConfigureSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMode { mode, available } => {
                write!(f, "unknown mode: {mode} (available: {available})")
            }
            Self::UnknownRole { role, available } => {
                write!(f, "unknown role: {role} (available: {available})")
            }
            Self::UnknownAllowedTool { tool, known } => {
                write!(
                    f,
                    "unknown tool in allowed_tools: {tool} (known tools: {known})"
                )
            }
            Self::AllowedToolDeniedByMode { mode, tool } => {
                write!(
                    f,
                    "allowed_tools tool is denied by mode: mode={mode} tool={tool}"
                )
            }
            Self::AllowedToolDeniedByRole {
                role,
                permission_mode,
                tool,
            } => {
                write!(
                    f,
                    "allowed_tools tool is denied by role: role={role} permission_mode={permission_mode} tool={tool}"
                )
            }
            Self::AllowedToolDecisionMappingMissing { tool } => {
                write!(
                    f,
                    "tool decision mapping is missing for allowed_tools entry: {tool}"
                )
            }
        }
    }
}

impl std::error::Error for ThreadConfigureSpecError {}

#[derive(Debug)]
enum ThreadConfigureInputError {
    InvalidThinking { value: String },
    SandboxWritableRootInvalid { root: String, message: String },
}

impl ThreadConfigureInputError {
    fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidThinking { .. } => "thinking_invalid",
            Self::SandboxWritableRootInvalid { .. } => "sandbox_writable_root_invalid",
        }
    }
}

impl std::fmt::Display for ThreadConfigureInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidThinking { value } => write!(
                f,
                "invalid thinking: {value} (expected: small|medium|high|xhigh|unsupported)"
            ),
            Self::SandboxWritableRootInvalid { root, message } => {
                write!(f, "invalid sandbox writable root: {root} ({message})")
            }
        }
    }
}

impl std::error::Error for ThreadConfigureInputError {}

fn thread_configure_error_code(err: &anyhow::Error) -> Option<&'static str> {
    for cause in err.chain() {
        if let Some(spec) = cause.downcast_ref::<ThreadConfigureSpecError>() {
            return Some(spec.error_code());
        }
        if let Some(input) = cause.downcast_ref::<ThreadConfigureInputError>() {
            return Some(input.error_code());
        }
    }
    None
}

fn validate_thread_configure_mode(
    mode: &str,
    catalog: &omne_core::modes::ModeCatalog,
) -> anyhow::Result<()> {
    if catalog.mode(mode).is_some() {
        return Ok(());
    }
    let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
    Err(ThreadConfigureSpecError::UnknownMode {
        mode: mode.to_string(),
        available,
    }
    .into())
}

fn validate_thread_configure_role(
    role: &str,
    role_catalog: &omne_core::roles::RoleCatalog,
) -> anyhow::Result<()> {
    if role_catalog.role(role).is_some() {
        return Ok(());
    }
    let available = available_role_names(role_catalog);
    Err(ThreadConfigureSpecError::UnknownRole {
        role: role.to_string(),
        available,
    }
    .into())
}

fn available_role_names(role_catalog: &omne_core::roles::RoleCatalog) -> String {
    let mut names = std::collections::BTreeSet::<String>::new();
    for role in role_catalog.role_names() {
        names.insert(role.to_string());
    }
    names.into_iter().collect::<Vec<_>>().join(", ")
}

fn resolve_role_permission_mode_name(
    role_name: &str,
    role_catalog: &omne_core::roles::RoleCatalog,
) -> Option<String> {
    role_catalog
        .permission_mode_name(role_name)
        .map(ToString::to_string)
}

fn normalize_thread_configure_allowed_tools(tools: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::<String>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for tool in tools {
        let trimmed = tool.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !omne_core::allowed_tools::is_known_allowed_tool(trimmed) {
            let known = omne_core::allowed_tools::known_allowed_tools().join(", ");
            return Err(ThreadConfigureSpecError::UnknownAllowedTool {
                tool: trimmed.to_string(),
                known,
            }
            .into());
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

fn parse_thread_configure_thinking(thinking: Option<String>) -> anyhow::Result<Option<String>> {
    let Some(value) = thinking else {
        return Ok(None);
    };
    let value = value.trim().to_string();
    if value.is_empty() {
        return Ok(None);
    }
    let lowered = value.to_ascii_lowercase();
    if matches!(
        lowered.as_str(),
        "small" | "medium" | "high" | "xhigh" | "unsupported"
    ) {
        return Ok(Some(value));
    }
    Err(ThreadConfigureInputError::InvalidThinking { value }.into())
}

fn normalize_thread_configure_text_value(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn validate_thread_configure_allowed_tools_for_mode_and_role(
    mode_name: &str,
    role_name: Option<&str>,
    tools: &[String],
    role_catalog: &omne_core::roles::RoleCatalog,
    mode_catalog: &omne_core::modes::ModeCatalog,
) -> anyhow::Result<()> {
    let mode = mode_catalog.mode(mode_name).ok_or_else(|| {
        let available = mode_catalog.mode_names().collect::<Vec<_>>().join(", ");
        anyhow::anyhow!(ThreadConfigureSpecError::UnknownMode {
            mode: mode_name.to_string(),
            available,
        })
    })?;
    let role_permission_mode = if let Some(role_name) = role_name {
        let permission_mode_name = resolve_role_permission_mode_name(role_name, role_catalog)
            .ok_or_else(|| {
                let available = available_role_names(role_catalog);
                anyhow::anyhow!(ThreadConfigureSpecError::UnknownRole {
                    role: role_name.to_string(),
                    available,
                })
            })?;
        let role_permission_mode = mode_catalog.mode(&permission_mode_name).ok_or_else(|| {
            let available = available_role_names(role_catalog);
            anyhow::anyhow!(ThreadConfigureSpecError::UnknownRole {
                role: role_name.to_string(),
                available,
            })
        })?;
        Some((permission_mode_name, role_permission_mode))
    } else {
        None
    };

    for tool in tools {
        let mode_decision = omne_core::allowed_tools::effective_mode_decision_for_tool(mode, tool)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    ThreadConfigureSpecError::AllowedToolDecisionMappingMissing {
                        tool: tool.clone(),
                    }
                )
            })?;
        if mode_decision == omne_core::modes::Decision::Deny {
            return Err(ThreadConfigureSpecError::AllowedToolDeniedByMode {
                mode: mode_name.to_string(),
                tool: tool.clone(),
            }
            .into());
        }

        if let Some((permission_mode_name, role_permission_mode)) = role_permission_mode.as_ref() {
            let role_decision = omne_core::allowed_tools::effective_mode_decision_for_tool(
                role_permission_mode,
                tool,
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    ThreadConfigureSpecError::AllowedToolDecisionMappingMissing {
                        tool: tool.clone(),
                    }
                )
            })?;
            if role_decision == omne_core::modes::Decision::Deny {
                return Err(ThreadConfigureSpecError::AllowedToolDeniedByRole {
                    role: role_name.unwrap_or_default().to_string(),
                    permission_mode: permission_mode_name.clone(),
                    tool: tool.clone(),
                }
                .into());
            }
        }
    }

    Ok(())
}

async fn handle_thread_configure(
    server: &Server,
    params: ThreadConfigureParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadConfigureResponse> {
    let (rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (
        current_approval_policy,
        current_sandbox_policy,
        current_sandbox_writable_roots,
        current_sandbox_network_access,
        current_mode,
        current_role,
        current_model,
        current_thinking,
        current_show_thinking,
        current_openai_base_url,
        current_allowed_tools,
        current_execpolicy_rules,
    ) = {
        let handle = rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.sandbox_network_access,
            state.mode.clone(),
            state.role.clone(),
            state.model.clone(),
            state.thinking.clone(),
            state.show_thinking,
            state.openai_base_url.clone(),
            state.allowed_tools.clone(),
            state.execpolicy_rules.clone(),
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
                let resolved = omne_core::resolve_dir_unrestricted(&thread_root, Path::new(&root))
                    .await
                    .map_err(|err| {
                        anyhow::Error::new(ThreadConfigureInputError::SandboxWritableRootInvalid {
                            root: root.clone(),
                            message: err.to_string(),
                        })
                    })?;
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
    let role = params
        .role
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    let allowed_tools_requested = matches!(params.allowed_tools, Some(Some(_)));
    let role_catalog = if role.is_some() || allowed_tools_requested {
        Some(omne_core::roles::RoleCatalog::builtin())
    } else {
        None
    };
    let mode_catalog = if mode.is_some() || role.is_some() || allowed_tools_requested {
        Some(omne_core::modes::ModeCatalog::load(&thread_root).await)
    } else {
        None
    };
    if let Some(mode_name) = mode.as_deref() {
        let catalog = mode_catalog
            .as_ref()
            .expect("mode catalog must be loaded when mode is provided");
        validate_thread_configure_mode(mode_name, catalog)?;
    }
    if let Some(role_name) = role.as_deref() {
        let roles = role_catalog
            .as_ref()
            .expect("role catalog must be loaded when role is provided");
        validate_thread_configure_role(role_name, roles)?;
    }
    let model = normalize_thread_configure_text_value(params.model);
    let thinking = parse_thread_configure_thinking(params.thinking)?;
    let show_thinking = params.show_thinking;
    let openai_base_url = normalize_thread_configure_text_value(params.openai_base_url);
    let allowed_tools = match params.allowed_tools {
        None => None,
        Some(None) => Some(None),
        Some(Some(tools)) => {
            let tools = normalize_thread_configure_allowed_tools(tools)?;
            let effective_mode_name = mode.as_deref().unwrap_or(current_mode.as_str());
            let modes = mode_catalog
                .as_ref()
                .expect("mode catalog must be loaded when allowed_tools are provided");
            let roles = role_catalog
                .as_ref()
                .expect("role catalog must be loaded when allowed_tools are provided");
            let effective_role_name = role.as_deref().or_else(|| {
                if roles.role(current_role.as_str()).is_some() {
                    Some(current_role.as_str())
                } else {
                    None
                }
            });
            if let Some(role_name) = effective_role_name {
                validate_thread_configure_role(role_name, roles)?;
            }
            validate_thread_configure_allowed_tools_for_mode_and_role(
                effective_mode_name,
                effective_role_name,
                &tools,
                roles,
                modes,
            )?;
            Some(Some(tools))
        }
    };
    let execpolicy_rules = params.execpolicy_rules.map(normalize_execpolicy_rules);
    let allowed_tools_changed = match &allowed_tools {
        None => false,
        Some(None) => current_allowed_tools.is_some(),
        Some(Some(tools)) => current_allowed_tools.as_ref() != Some(tools),
    };
    let model_changed = if params.clear_model {
        current_model.is_some()
    } else {
        model
            .as_ref()
            .is_some_and(|value| current_model.as_ref() != Some(value))
    };
    let thinking_changed = if params.clear_thinking {
        current_thinking.is_some()
    } else {
        thinking
            .as_ref()
            .is_some_and(|value| current_thinking.as_ref() != Some(value))
    };
    let show_thinking_changed = if params.clear_show_thinking {
        current_show_thinking.is_some()
    } else {
        show_thinking.is_some_and(|value| current_show_thinking != Some(value))
    };
    let openai_base_url_changed = if params.clear_openai_base_url {
        current_openai_base_url.is_some()
    } else {
        openai_base_url
            .as_ref()
            .is_some_and(|value| current_openai_base_url.as_ref() != Some(value))
    };
    let execpolicy_rules_changed = if params.clear_execpolicy_rules {
        !current_execpolicy_rules.is_empty()
    } else {
        execpolicy_rules
            .as_ref()
            .is_some_and(|rules| rules != &current_execpolicy_rules)
    };

    let changed = approval_policy != current_approval_policy
        || params
            .sandbox_policy
            .is_some_and(|p| p != current_sandbox_policy)
        || sandbox_writable_roots
            .as_ref()
            .is_some_and(|roots| roots != &current_sandbox_writable_roots)
        || sandbox_network_access.is_some_and(|access| access != current_sandbox_network_access)
        || mode.as_ref().is_some_and(|m| m != &current_mode)
        || role.as_ref().is_some_and(|r| r != &current_role)
        || model_changed
        || thinking_changed
        || show_thinking_changed
        || openai_base_url_changed
        || allowed_tools_changed
        || execpolicy_rules_changed;

    if changed {
        rt.append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy: params.sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            role,
            model,
            clear_model: params.clear_model,
            thinking,
            clear_thinking: params.clear_thinking,
            show_thinking,
            clear_show_thinking: params.clear_show_thinking,
            openai_base_url,
            clear_openai_base_url: params.clear_openai_base_url,
            allowed_tools,
            execpolicy_rules,
            clear_execpolicy_rules: params.clear_execpolicy_rules,
        })
        .await?;
    }
    Ok(omne_app_server_protocol::ThreadConfigureResponse { ok: true })
}

async fn handle_thread_config_explain(
    server: &Server,
    params: ThreadConfigExplainParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadConfigExplainResponse> {
    let events = server
        .thread_store
        .read_events_since(params.thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", params.thread_id))?;

    let thread_cwd = events
        .iter()
        .find_map(|event| match &event.kind {
            omne_protocol::ThreadEventKind::ThreadCreated { cwd, .. } => Some(cwd.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", params.thread_id))?;
    let thread_root = omne_core::resolve_dir(Path::new(&thread_cwd), Path::new(".")).await?;
    let mode_catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let role_catalog = omne_core::roles::RoleCatalog::builtin();

    let default_model = "gpt-4.1".to_string();
    let default_openai_provider = crate::project_config::default_openai_provider_name().to_string();
    let default_openai_base_url = crate::project_config::default_openai_base_url().to_string();
    let default_mode = "code".to_string();
    let default_role = "coder".to_string();
    let default_thinking = thinking_label(ditto_core::config::ThinkingIntensity::default()).to_string();
    let default_show_thinking = true;

    let mut effective_approval_policy = omne_protocol::ApprovalPolicy::AutoApprove;
    let mut effective_sandbox_policy = policy_meta::WriteScope::WorkspaceWrite;
    let mut effective_sandbox_writable_roots = Vec::<String>::new();
    let mut effective_sandbox_network_access = omne_protocol::SandboxNetworkAccess::Deny;
    let mut effective_mode = default_mode.clone();
    let mut effective_role = default_role.clone();
    let mut effective_openai_provider = default_openai_provider.clone();
    let mut effective_model = default_model.clone();
    let mut effective_thinking = default_thinking.clone();
    let mut effective_show_thinking = default_show_thinking;
    let mut effective_openai_base_url = default_openai_base_url.clone();
    let mut effective_allowed_tools: Option<Vec<String>> = None;
    let mut effective_execpolicy_rules = Vec::<String>::new();
    let mut layers = vec![serde_json::json!({
        "source": "default",
        "approval_policy": effective_approval_policy,
        "sandbox_policy": effective_sandbox_policy,
        "sandbox_writable_roots": effective_sandbox_writable_roots,
        "sandbox_network_access": effective_sandbox_network_access,
        "mode": effective_mode,
        "role": effective_role,
        "openai_provider": effective_openai_provider,
        "model": effective_model,
        "thinking": effective_thinking,
        "show_thinking": effective_show_thinking,
        "openai_base_url": effective_openai_base_url,
        "allowed_tools": effective_allowed_tools,
        "execpolicy_rules": effective_execpolicy_rules,
    })];

    let env_provider = std::env::var("OMNE_OPENAI_PROVIDER")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_model = std::env::var("OMNE_OPENAI_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let env_openai_base_url = std::env::var("OMNE_OPENAI_BASE_URL")
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
            "show_thinking": effective_show_thinking,
            "openai_base_url": effective_openai_base_url,
        }));
    }

    let project = crate::project_config::load_project_config(&thread_root).await;
    let project_thinking_for_model = |model: &str| -> String {
        if project.enabled {
            let thinking = ditto_core::config::select_model_config(&project.openai.models, model)
                .map(|cfg| cfg.thinking)
                .unwrap_or_default();
            thinking_label(thinking).to_string()
        } else {
            default_thinking.clone()
        }
    };
    let project_show_thinking_default = if project.enabled {
        project.ui.show_thinking.unwrap_or(default_show_thinking)
    } else {
        default_show_thinking
    };
    let mode_show_thinking_for_mode = |mode: &str| -> Option<bool> {
        mode_catalog
            .mode(mode)
            .and_then(|mode| mode.ui.show_thinking)
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
            if let Some(provider_base_url) = crate::project_config::provider_overrides_base_url(
                &effective_openai_provider,
                &project.openai.providers,
            ) {
                effective_openai_base_url = provider_base_url;
            }
        }
        effective_thinking = project_thinking_for_model(&effective_model);
        effective_show_thinking = project_show_thinking_default;
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
            "show_thinking": effective_show_thinking,
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

    if let Some(mode_override) = mode_show_thinking_for_mode(&effective_mode) {
        effective_show_thinking = mode_override;
        layers.push(serde_json::json!({
            "source": "mode",
            "mode": effective_mode.clone(),
            "show_thinking": effective_show_thinking,
        }));
    }

    let base_model = effective_model.clone();
    let base_openai_base_url = effective_openai_base_url.clone();

    if let Some(meta) =
        latest_artifact_metadata_by_type(server, params.thread_id, "preset_applied").await?
    {
        layers.push(serde_json::json!({
            "source": "preset",
            "artifact_id": meta.artifact_id,
            "artifact_type": meta.artifact_type,
            "summary": meta.summary,
            "updated_at": meta.updated_at.format(&Rfc3339)?,
            "provenance": meta.provenance,
        }));
    }

    let mut thinking_override: Option<String> = None;
    let mut show_thinking_override: Option<bool> = None;
    for event in events {
        if let omne_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            role,
            model,
            clear_model,
            thinking,
            clear_thinking,
            show_thinking,
            clear_show_thinking,
            openai_base_url,
            clear_openai_base_url,
            allowed_tools,
            execpolicy_rules,
            clear_execpolicy_rules,
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
                if show_thinking_override.is_none() {
                    effective_show_thinking = mode_show_thinking_for_mode(&effective_mode)
                        .unwrap_or(project_show_thinking_default);
                }
            }
            if let Some(role) = role {
                effective_role = role;
            }
            if clear_model {
                effective_model = base_model.clone();
                if thinking_override.is_none() {
                    effective_thinking = project_thinking_for_model(&effective_model);
                }
            } else if let Some(model) = model {
                effective_model = model;
                if thinking_override.is_none() {
                    effective_thinking = project_thinking_for_model(&effective_model);
                }
            }
            if clear_thinking {
                thinking_override = None;
                effective_thinking = project_thinking_for_model(&effective_model);
            } else if let Some(thinking) = thinking {
                effective_thinking = thinking.clone();
                thinking_override = Some(thinking);
            }
            if clear_show_thinking {
                show_thinking_override = None;
                effective_show_thinking = mode_show_thinking_for_mode(&effective_mode)
                    .unwrap_or(project_show_thinking_default);
            } else if let Some(show_thinking) = show_thinking {
                effective_show_thinking = show_thinking;
                show_thinking_override = Some(show_thinking);
            }
            if clear_openai_base_url {
                effective_openai_base_url = base_openai_base_url.clone();
            } else if let Some(openai_base_url) = openai_base_url {
                effective_openai_base_url = openai_base_url;
            }
            if let Some(allowed_tools) = allowed_tools {
                effective_allowed_tools = allowed_tools;
            }
            if clear_execpolicy_rules {
                effective_execpolicy_rules.clear();
            } else if let Some(execpolicy_rules) = execpolicy_rules {
                effective_execpolicy_rules = execpolicy_rules;
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
                "role": effective_role,
                "openai_provider": effective_openai_provider,
                "model": effective_model,
                "thinking": effective_thinking,
                "show_thinking": effective_show_thinking,
                "openai_base_url": effective_openai_base_url,
                "allowed_tools": effective_allowed_tools,
                "execpolicy_rules": effective_execpolicy_rules,
            }));
        }
    }

    let effective_role_permission_mode =
        resolve_role_permission_mode_name(&effective_role, &role_catalog)
            .unwrap_or_else(|| effective_mode.clone());
    let effective_permissions = mode_catalog.mode(&effective_mode).and_then(|mode| {
        mode_catalog
            .mode(&effective_role_permission_mode)
            .map(|role_permission_mode| {
                omne_core::allowed_tools::effective_permissions_for_mode_and_role(
                    mode,
                    role_permission_mode,
                    effective_allowed_tools.as_deref(),
                )
            })
    });
    let role_resolution_source = if role_catalog.role(&effective_role).is_some() {
        "role_catalog"
    } else {
        "none"
    };
    let available_roles = role_catalog
        .role_names()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    layers.push(serde_json::json!({
        "source": "role_catalog",
        "catalog_source": "builtin",
        "effective_role": effective_role.clone(),
        "permission_mode": effective_role_permission_mode.clone(),
        "resolution_source": role_resolution_source,
        "available_roles": available_roles,
    }));

    let (mode_catalog_source, mode_catalog_path) = match &mode_catalog.source {
        omne_core::modes::ModeCatalogSource::Builtin => ("builtin", None),
        omne_core::modes::ModeCatalogSource::Env(path) => ("env", Some(path.display().to_string())),
        omne_core::modes::ModeCatalogSource::Project(path) => {
            ("project", Some(path.display().to_string()))
        }
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
            "ui": {
                "show_thinking": mode.ui.show_thinking,
            },
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
        ditto_core::config::select_model_config(&project.openai.models, &effective_model)
    } else {
        None
    };
    let limits = resolve_model_limits(&effective_model, model_config);
    let auto_compact_threshold_pct = std::env::var("OMNE_AGENT_AUTO_SUMMARY_THRESHOLD_PCT")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(|value| value.min(crate::model_limits::MAX_AUTO_COMPACT_THRESHOLD_PCT))
        .filter(|value| *value > 0)
        .unwrap_or(crate::model_limits::DEFAULT_AUTO_COMPACT_THRESHOLD_PCT);
    let auto_compact_token_limit = effective_auto_compact_token_limit(
        limits.context_window,
        limits.auto_compact_token_limit,
        auto_compact_threshold_pct,
    );

    Ok(omne_app_server_protocol::ThreadConfigExplainResponse {
        thread_id: params.thread_id,
        effective: omne_app_server_protocol::ThreadConfigExplainEffective {
            approval_policy: effective_approval_policy,
            sandbox_policy: effective_sandbox_policy,
            sandbox_writable_roots: effective_sandbox_writable_roots,
            sandbox_network_access: effective_sandbox_network_access,
            mode: effective_mode,
            role: effective_role,
            model: effective_model,
            thinking: effective_thinking,
            show_thinking: effective_show_thinking,
            openai_base_url: effective_openai_base_url,
            allowed_tools: effective_allowed_tools,
            execpolicy_rules: effective_execpolicy_rules,
            permission_mode: effective_role_permission_mode,
            effective_permissions,
            model_context_window: limits.context_window,
            auto_compact_token_limit,
        },
        mode_catalog: omne_app_server_protocol::ThreadConfigExplainModeCatalog {
            source: mode_catalog_source.to_string(),
            path: mode_catalog_path,
            load_error: mode_catalog.load_error,
            modes: available_modes,
        },
        effective_mode_def,
        layers,
    })
}

async fn latest_artifact_metadata_by_type(
    server: &Server,
    thread_id: ThreadId,
    artifact_type: &str,
) -> anyhow::Result<Option<ArtifactMetadata>> {
    let dir = user_artifacts_dir_for_thread(server, thread_id);
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };

    let mut latest: Option<ArtifactMetadata> = None;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .with_context(|| format!("read {}", dir.display()))?
    {
        let ty = entry
            .file_type()
            .await
            .with_context(|| format!("stat {}", entry.path().display()))?;
        if !ty.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".metadata.json") {
            continue;
        }

        let meta = match read_artifact_metadata(&path).await {
            Ok(meta) => meta,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "skip bad artifact metadata");
                continue;
            }
        };
        if meta.artifact_type != artifact_type {
            continue;
        }
        let replace = latest.as_ref().is_none_or(|current| {
            meta.updated_at > current.updated_at
                || (meta.updated_at == current.updated_at && meta.artifact_id > current.artifact_id)
        });
        if replace {
            latest = Some(meta);
        }
    }

    Ok(latest)
}

fn thinking_label(value: ditto_core::config::ThinkingIntensity) -> &'static str {
    match value {
        ditto_core::config::ThinkingIntensity::Unsupported => "unsupported",
        ditto_core::config::ThinkingIntensity::Small => "small",
        ditto_core::config::ThinkingIntensity::Medium => "medium",
        ditto_core::config::ThinkingIntensity::High => "high",
        ditto_core::config::ThinkingIntensity::XHigh => "xhigh",
    }
}

fn normalize_execpolicy_rules(rules: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for rule in rules {
        let trimmed = rule.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = trimmed.to_string();
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}
