use ditto_core::config::ProviderApi;
use ditto_server::config_editing::{
    ConfigScope as DittoConfigScope, ModelDeleteRequest, ModelListRequest, ModelShowRequest,
    ModelUpsertRequest, ProviderAuthType, ProviderDeleteRequest, ProviderListRequest,
    ProviderNamespace as DittoProviderNamespace, ProviderShowRequest, ProviderUpsertRequest,
    complete_model_upsert_request_interactive, complete_provider_upsert_request_interactive,
    delete_model_config, delete_provider_config, list_model_configs, list_provider_configs,
    show_model_config, show_provider_config, upsert_model_config, upsert_provider_config,
};

async fn run_provider_add(cli: &Cli, args: ProviderAddArgs) -> anyhow::Result<()> {
    let use_interactive = !args.no_interactive || args.interactive;
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let mut request = ProviderUpsertRequest {
        name: args.name,
        config_path: None,
        root,
        scope,
        namespace: map_provider_namespace(args.namespace),
        provider: None,
        enabled_capabilities: Vec::new(),
        base_url: args.base_url,
        default_model: args.default_model,
        upstream_api: args.upstream_api.map(map_provider_api),
        normalize_to: args.normalize_to.map(map_provider_api),
        normalize_endpoint: args.normalize_endpoint,
        auth_type: map_provider_auth_type(args.auth_type),
        auth_keys: args.auth_keys,
        auth_param: args.auth_param,
        auth_header: args.auth_header,
        auth_prefix: args.auth_prefix,
        auth_command: args.auth_command,
        set_default: args.set_default,
        set_default_model: args.set_default_model,
        tools: args.tools,
        vision: args.vision,
        reasoning: args.reasoning,
        json_schema: args.json_schema,
        streaming: args.streaming,
        prompt_cache: args.prompt_cache,
        discover_models: args.discover_models,
        discovery_api_key: args.api_key,
        model_whitelist: Vec::new(),
        register_models: args.register_models,
        model_limit: args.model_limit,
    };
    if use_interactive {
        request = complete_provider_upsert_request_interactive(request)?;
    }

    let report = upsert_provider_config(request).await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_model_add(cli: &Cli, args: ModelAddArgs) -> anyhow::Result<()> {
    let use_interactive = !args.no_interactive || args.interactive;
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let mut request = ModelUpsertRequest {
        name: args.name,
        config_path: None,
        root,
        scope,
        provider: args.provider,
        fallback_providers: args.fallback_providers,
        set_default: args.set_default,
        thinking: args.thinking,
        context_window: args.context_window,
        auto_compact_token_limit: args.auto_compact_token_limit,
        prompt_cache: args.prompt_cache,
    };
    if use_interactive {
        request = complete_model_upsert_request_interactive(request)?;
    }

    let report = upsert_model_config(request).await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_provider_list(cli: &Cli, args: ProviderListArgs) -> anyhow::Result<()> {
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let report = list_provider_configs(ProviderListRequest {
        config_path: None,
        root,
        scope,
        namespace: args.namespace.map(map_provider_namespace),
    })
    .await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_provider_show(cli: &Cli, args: ProviderShowArgs) -> anyhow::Result<()> {
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let report = show_provider_config(ProviderShowRequest {
        name: args.name,
        config_path: None,
        root,
        scope,
        namespace: map_provider_namespace(args.namespace),
    })
    .await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_provider_delete(cli: &Cli, args: ProviderDeleteArgs) -> anyhow::Result<()> {
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let report = delete_provider_config(ProviderDeleteRequest {
        name: args.name,
        config_path: None,
        root,
        scope,
        namespace: map_provider_namespace(args.namespace),
    })
    .await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_model_list(cli: &Cli, args: ModelListArgs) -> anyhow::Result<()> {
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let report = list_model_configs(ModelListRequest {
        config_path: None,
        root,
        scope,
    })
    .await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_model_show(cli: &Cli, args: ModelShowArgs) -> anyhow::Result<()> {
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let report = show_model_config(ModelShowRequest {
        name: args.name,
        config_path: None,
        root,
        scope,
    })
    .await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

async fn run_model_delete(cli: &Cli, args: ModelDeleteArgs) -> anyhow::Result<()> {
    let (scope, root) = resolve_scope_and_root(cli, args.scope)?;
    let report = delete_model_config(ModelDeleteRequest {
        name: args.name,
        config_path: None,
        root,
        scope,
    })
    .await?;
    print_json_or_pretty(args.json, &serde_json::to_value(report)?)
}

fn resolve_scope_and_root(
    cli: &Cli,
    scope: ConfigScope,
) -> anyhow::Result<(DittoConfigScope, Option<std::path::PathBuf>)> {
    let mapped_scope = map_config_scope(scope);
    let root = match scope {
        ConfigScope::Global => None,
        ConfigScope::Workspace => Some(resolve_pm_root(cli)?),
        ConfigScope::Auto => {
            if cli.omne_root.is_some() || std::env::var_os("OMNE_ROOT").is_some() {
                Some(resolve_pm_root(cli)?)
            } else {
                let cwd = std::env::current_dir()?;
                let workspace_root = cwd.join(".omne_data");
                if workspace_root.exists() {
                    Some(resolve_pm_root(cli)?)
                } else {
                    None
                }
            }
        }
    };
    Ok((mapped_scope, root))
}

fn map_config_scope(scope: ConfigScope) -> DittoConfigScope {
    match scope {
        ConfigScope::Auto => DittoConfigScope::Auto,
        ConfigScope::Workspace => DittoConfigScope::Workspace,
        ConfigScope::Global => DittoConfigScope::Global,
    }
}

fn map_provider_namespace(namespace: ProviderNamespace) -> DittoProviderNamespace {
    match namespace {
        ProviderNamespace::Openai => DittoProviderNamespace::Openai,
        ProviderNamespace::Google => DittoProviderNamespace::Google,
        ProviderNamespace::Gemini => DittoProviderNamespace::Gemini,
        ProviderNamespace::Claude => DittoProviderNamespace::Claude,
        ProviderNamespace::Anthropic => DittoProviderNamespace::Anthropic,
    }
}

fn map_provider_api(api: ProviderApiArg) -> ProviderApi {
    match api {
        ProviderApiArg::OpenaiChatCompletions => ProviderApi::OpenaiChatCompletions,
        ProviderApiArg::OpenaiResponses => ProviderApi::OpenaiResponses,
        ProviderApiArg::GeminiGenerateContent => ProviderApi::GeminiGenerateContent,
        ProviderApiArg::AnthropicMessages => ProviderApi::AnthropicMessages,
    }
}

fn map_provider_auth_type(auth_type: ProviderAuthTypeArg) -> ProviderAuthType {
    match auth_type {
        ProviderAuthTypeArg::ApiKeyEnv => ProviderAuthType::ApiKeyEnv,
        ProviderAuthTypeArg::QueryParamEnv => ProviderAuthType::QueryParamEnv,
        ProviderAuthTypeArg::HttpHeaderEnv => ProviderAuthType::HttpHeaderEnv,
        ProviderAuthTypeArg::Command => ProviderAuthType::Command,
    }
}
