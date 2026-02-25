async fn handle_thread_models(
    server: &Server,
    params: ThreadModelsParams,
) -> anyhow::Result<omne_app_server_protocol::ThreadModelsResponse> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (thread_model, thread_openai_base_url) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.model.clone(), state.openai_base_url.clone())
    };

    let project = crate::project_config::load_project_openai_overrides(&thread_root).await;

    let provider = project
        .provider
        .or_else(|| {
            std::env::var("OMNE_OPENAI_PROVIDER")
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
        .or(project.base_url)
        .or_else(|| {
            std::env::var("OMNE_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.base_url.clone())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("openai provider {provider} is missing base_url"))?;

    let current_model = thread_model
        .or(project.model)
        .or_else(|| {
            std::env::var("OMNE_OPENAI_MODEL")
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
        default_model: provider_config.default_model,
        model_whitelist: provider_config.model_whitelist.clone(),
        http_headers: provider_config.http_headers,
        http_query_params: provider_config.http_query_params,
        auth: provider_config.auth,
        capabilities: provider_config.capabilities,
    };

    let env = ditto_llm::Env {
        dotenv: project.dotenv,
    };
    let provider_client = ditto_llm::OpenAiModelsProvider::from_config(
        provider.clone(),
        &provider_for_listing,
        &env,
    )
    .await
    .context("build provider client")?;
    let capabilities = ditto_llm::Provider::capabilities(&provider_client);
    let models = ditto_llm::Provider::list_models(&provider_client)
        .await
        .context("list /models")?;

    Ok(omne_app_server_protocol::ThreadModelsResponse {
        provider,
        base_url,
        current_model,
        thinking: thinking_label(thinking).to_string(),
        default_model: provider_for_listing.default_model,
        model_whitelist: provider_for_listing.model_whitelist,
        capabilities: omne_app_server_protocol::ThreadModelCapabilities {
            tools: capabilities.tools,
            vision: capabilities.vision,
            reasoning: capabilities.reasoning,
            json_schema: capabilities.json_schema,
            streaming: capabilities.streaming,
            prompt_cache: capabilities.prompt_cache,
        },
        models,
    })
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
