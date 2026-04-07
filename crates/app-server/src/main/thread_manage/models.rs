fn resolve_thread_models_provider_upstream_api(
    provider_config: &ditto_core::config::ProviderConfig,
) -> ditto_core::config::ProviderApi {
    if let Some(upstream_api) = provider_config.upstream_api {
        return upstream_api;
    }
    if provider_config
        .capabilities
        .is_some_and(|capabilities| capabilities.reasoning)
    {
        ditto_core::config::ProviderApi::OpenaiResponses
    } else {
        ditto_core::config::ProviderApi::OpenaiChatCompletions
    }
}

fn default_thread_models_capabilities(
    upstream_api: ditto_core::config::ProviderApi,
) -> ditto_core::config::ProviderCapabilities {
    if upstream_api.uses_openai_responses_client() {
        return ditto_core::config::ProviderCapabilities::openai_responses();
    }
    ditto_core::config::ProviderCapabilities {
        tools: true,
        vision: false,
        reasoning: false,
        json_schema: false,
        streaming: true,
        prompt_cache: true,
    }
}

fn resolve_thread_models_capabilities(
    provider_config: &ditto_core::config::ProviderConfig,
    upstream_api: ditto_core::config::ProviderApi,
) -> ditto_core::config::ProviderCapabilities {
    provider_config
        .capabilities
        .unwrap_or_else(|| default_thread_models_capabilities(upstream_api))
}

fn fallback_native_models(
    current_model: &str,
    default_model: Option<&str>,
    whitelist: &[String],
) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();

    for model in whitelist {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    for candidate in [Some(current_model), default_model] {
        let Some(candidate) = candidate else {
            continue;
        };
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }

    out
}

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
        .unwrap_or_else(|| crate::project_config::default_openai_provider_name().to_string());

    let provider_overrides = project.providers.get(&provider);
    let provider_config =
        crate::project_config::resolve_provider_config(&provider, provider_overrides)?;

    let base_url = thread_openai_base_url
        .or(project.base_url)
        .or_else(|| {
            std::env::var("OMNE_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.base_url.clone())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("provider {provider} is missing base_url"))?;

    let current_model = thread_model
        .or(project.model)
        .or_else(|| {
            std::env::var("OMNE_OPENAI_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.default_model.clone())
        .unwrap_or_else(|| "gpt-4.1".to_string());

    let thinking = ditto_core::config::select_model_config(&project.models, &current_model)
        .map(|cfg| cfg.thinking)
        .unwrap_or_default();

    let provider_for_listing = ditto_core::config::ProviderConfig {
        provider: provider_config.provider.clone(),
        enabled_capabilities: provider_config.enabled_capabilities.clone(),
        base_url: Some(base_url.clone()),
        default_model: provider_config
            .default_model
            .clone()
            .or_else(|| Some(current_model.clone())),
        model_whitelist: provider_config.model_whitelist.clone(),
        http_headers: provider_config.http_headers.clone(),
        http_query_params: provider_config.http_query_params.clone(),
        auth: provider_config.auth.clone(),
        capabilities: provider_config.capabilities,
        upstream_api: provider_config.upstream_api,
        normalize_to: provider_config.normalize_to,
        normalize_endpoint: provider_config.normalize_endpoint,
        openai_compatible: provider_config.openai_compatible.clone(),
    };

    let upstream_api = resolve_thread_models_provider_upstream_api(&provider_for_listing);
    let capabilities = resolve_thread_models_capabilities(&provider_for_listing, upstream_api);
    let env = ditto_core::config::Env {
        dotenv: project.dotenv,
    };

    let models = match upstream_api {
        ditto_core::config::ProviderApi::OpenaiResponses | ditto_core::config::ProviderApi::OpenaiChatCompletions => {
            let client = ditto_core::providers::OpenAI::from_config(&provider_for_listing, &env)
                .await
                .context("build provider client")?;
            let listed = client.list_model_ids().await.context("list /models")?;
            ditto_core::config::filter_models_whitelist(listed, &provider_for_listing.model_whitelist)
        }
        ditto_core::config::ProviderApi::GeminiGenerateContent
        | ditto_core::config::ProviderApi::AnthropicMessages => fallback_native_models(
            &current_model,
            provider_for_listing.default_model.as_deref(),
            &provider_for_listing.model_whitelist,
        ),
    };

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
