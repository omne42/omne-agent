async fn handle_thread_models(server: &Server, params: ThreadModelsParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (thread_openai_provider, thread_model, thread_openai_base_url) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.openai_provider.clone(),
            state.model.clone(),
            state.openai_base_url.clone(),
        )
    };

    let project = crate::project_config::load_project_openai_overrides(&thread_root).await;
    let project_provider = project.provider.clone();
    let project_base_url = project.base_url.clone();
    let project_model = project.model.clone();

    let provider = thread_openai_provider
        .clone()
        .or(project_provider)
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_PROVIDER")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "openai-codex-apikey".to_string());

    let builtin_provider_config = builtin_openai_provider_config(&provider);
    let provider_overrides = project.providers.get(&provider);
    if builtin_provider_config.is_none() && provider_overrides.is_none() {
        anyhow::bail!(
            "unknown openai provider: {provider} (expected: openai-codex-apikey, openai-auth-command; or define [openai.providers.{provider}] in project config)"
        );
    }

    let mut provider_config = builtin_provider_config.unwrap_or_default();
    if let Some(overrides) = provider_overrides {
        provider_config = merge_provider_config(provider_config, overrides);
    }

    let base_url = thread_openai_base_url
        .clone()
        .or(project_base_url)
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.base_url.clone())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("openai provider {provider} is missing base_url"))?;

    let current_model = thread_model
        .clone()
        .or(project_model.clone())
        .or_else(|| {
            std::env::var("CODE_PM_OPENAI_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.default_model.clone())
        .unwrap_or_else(|| "gpt-4.1".to_string());

    let thinking = ditto_llm::select_model_config(&project.models, &current_model)
        .map(|cfg| cfg.thinking)
        .unwrap_or_default();

    let provider_for_listing = ditto_llm::ProviderConfig {
        base_url: Some(base_url.clone()),
        ..provider_config
    };

    let env = ditto_llm::Env {
        dotenv: project.dotenv,
    };
    let provider_client = ditto_llm::OpenAiProvider::from_config(
        provider.clone(),
        &provider_for_listing,
        &env,
    )
    .await
    .context("build provider client")?;
    let capabilities = ditto_llm::Provider::capabilities(&provider_client);

    let list_models = async {
        ditto_llm::Provider::list_models(&provider_client)
            .await
            .context("list /models")
    };
    let models_timeout = std::time::Duration::from_secs(2);
    let mut models_error: Option<String> = None;
    let models = match tokio::time::timeout(models_timeout, list_models).await {
        Ok(Ok(models)) => models,
        Ok(Err(err)) => {
            models_error = Some(err.to_string());
            Vec::new()
        }
        Err(_) => {
            models_error = Some("list /models timeout".to_string());
            Vec::new()
        }
    };

    let models = if models.is_empty() {
        let mut out = Vec::<String>::new();
        let mut seen = std::collections::HashSet::<String>::new();
        let mut push = |value: Option<String>| {
            let Some(value) = value else { return };
            let value = value.trim().to_string();
            if value.is_empty() {
                return;
            }
            if seen.insert(value.clone()) {
                out.push(value);
            }
        };

        if !provider_for_listing.model_whitelist.is_empty() {
            for model in &provider_for_listing.model_whitelist {
                push(Some(model.clone()));
            }
        } else {
            push(thread_model);
            push(project_model);
            push(provider_for_listing.default_model.clone());
            push(Some(current_model.clone()));
        }
        out
    } else if provider_for_listing.model_whitelist.is_empty() {
        models
    } else {
        let allow = provider_for_listing
            .model_whitelist
            .iter()
            .map(|m| m.as_str())
            .collect::<std::collections::HashSet<_>>();
        let filtered = models
            .into_iter()
            .filter(|model| allow.contains(model.as_str()))
            .collect::<Vec<_>>();
        if filtered.is_empty() {
            provider_for_listing.model_whitelist.clone()
        } else {
            filtered
        }
    };

    Ok(serde_json::json!({
        "provider": provider,
        "base_url": base_url,
        "current_model": current_model,
        "thinking": thinking,
        "default_model": provider_for_listing.default_model,
        "model_whitelist": provider_for_listing.model_whitelist,
        "capabilities": capabilities,
        "models": models,
        "models_error": models_error,
    }))
}

fn builtin_openai_provider_config(provider: &str) -> Option<ditto_llm::ProviderConfig> {
    match provider {
        "openai-codex-apikey" => Some(ditto_llm::ProviderConfig {
            base_url: Some("https://api.openai.com/v1".to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            http_headers: Default::default(),
            http_query_params: Default::default(),
            auth: Some(ditto_llm::ProviderAuth::ApiKeyEnv { keys: Vec::new() }),
            capabilities: None,
        }),
        "openai-auth-command" => Some(ditto_llm::ProviderConfig {
            base_url: Some("https://api.openai.com/v1".to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            http_headers: Default::default(),
            http_query_params: Default::default(),
            auth: Some(ditto_llm::ProviderAuth::Command { command: Vec::new() }),
            capabilities: None,
        }),
        _ => None,
    }
}

fn merge_provider_config(
    mut base: ditto_llm::ProviderConfig,
    overrides: &ditto_llm::ProviderConfig,
) -> ditto_llm::ProviderConfig {
    if let Some(base_url) = overrides
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        base.base_url = Some(base_url.to_string());
    }
    if let Some(default_model) = overrides
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        base.default_model = Some(default_model.to_string());
    }
    if !overrides.model_whitelist.is_empty() {
        base.model_whitelist = ditto_llm::normalize_string_list(overrides.model_whitelist.clone());
    }
    if !overrides.http_headers.is_empty() {
        base.http_headers.extend(overrides.http_headers.clone());
    }
    if !overrides.http_query_params.is_empty() {
        base.http_query_params
            .extend(overrides.http_query_params.clone());
    }
    if let Some(auth) = overrides.auth.clone() {
        base.auth = Some(auth);
    }
    if let Some(capabilities) = overrides.capabilities {
        base.capabilities = Some(capabilities);
    }
    base
}
