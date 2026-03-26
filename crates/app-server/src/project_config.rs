use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use config_kit::{
    ConfigDocument, ConfigFormat, ConfigFormatSet, ConfigLoadOptions, try_load_config_document,
};
use ditto_core::config::{ModelConfig, ProviderConfig};
use ditto_core::contracts::AuthMethodKind;
use serde::Deserialize;

#[derive(Default)]
pub struct ProjectOpenAiOverrides {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub fallback_providers: Vec<String>,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub models: BTreeMap<String, ModelConfig>,
    pub routing: Option<ditto_core::config::ProviderRoutingConfig>,
    pub dotenv: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct ProjectUiOverrides {
    pub show_thinking: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
pub enum ProjectConfigSource {
    /// `.omne_data/config_local.toml` (gitignored)
    Local,
    /// `.omne_data/config.toml` (commit-safe)
    Shared,
}

impl ProjectConfigSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Shared => "shared",
        }
    }
}

pub struct LoadedProjectConfig {
    pub enabled: bool,
    pub config_path: PathBuf,
    pub config_source: ProjectConfigSource,
    pub config_present: bool,
    pub env_path: PathBuf,
    pub env_present: bool,
    pub load_error: Option<String>,
    pub openai: ProjectOpenAiOverrides,
    pub ui: ProjectUiOverrides,
}

pub(crate) const DEFAULT_OPENAI_PROVIDER: &str = "openai-codex-apikey";
pub(crate) const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

pub(crate) struct ProviderBaseUrlOverrideWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectConfigToml {
    #[serde(default)]
    project_config: ProjectConfigSection,
    #[serde(default)]
    openai: ProjectOpenAiSection,
    #[serde(default)]
    google: ProjectProviderNamespaceSection,
    #[serde(default)]
    gemini: ProjectProviderNamespaceSection,
    #[serde(default)]
    claude: ProjectProviderNamespaceSection,
    #[serde(default)]
    anthropic: ProjectProviderNamespaceSection,
    #[serde(default)]
    ui: ProjectUiSection,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectConfigSection {
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectOpenAiSection {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    fallback_providers: Vec<String>,
    #[serde(default)]
    providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    models: BTreeMap<String, ModelConfig>,
    #[serde(default)]
    routing: Option<ditto_core::config::ProviderRoutingConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectProviderNamespaceSection {
    #[serde(default)]
    providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectUiSection {
    #[serde(default)]
    show_thinking: Option<bool>,
}

#[derive(Default)]
struct DotenvOpenAiOverrides {
    provider: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    fallback_providers: Vec<String>,
    dotenv: BTreeMap<String, String>,
}

impl DotenvOpenAiOverrides {
    fn into_project_overrides(self) -> ProjectOpenAiOverrides {
        ProjectOpenAiOverrides {
            provider: self.provider,
            base_url: self.base_url,
            model: self.model,
            fallback_providers: self.fallback_providers,
            providers: BTreeMap::new(),
            models: BTreeMap::new(),
            routing: None,
            dotenv: self.dotenv,
        }
    }
}

fn clean_string_opt(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let v = v.trim();
        if v.is_empty() {
            None
        } else {
            Some(v.to_string())
        }
    })
}

fn clean_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let value = value.to_string();
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

fn parse_csv_list(value: &str) -> Vec<String> {
    clean_string_list(value.split(',').map(|v| v.trim().to_string()).collect())
}

fn provider_auth_from_preset_hint(
    provider_name_hint: &str,
    preset: ditto_core::runtime_registry::BuiltinProviderPreset,
) -> Option<ditto_core::config::ProviderAuth> {
    let hint = preset.auth_hint?;
    match hint.method {
        AuthMethodKind::ApiKeyHeader => {
            let mut keys = hint
                .env_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect::<Vec<_>>();
            if provider_name_hint == DEFAULT_OPENAI_PROVIDER
                && !keys.iter().any(|key| key == "OMNE_OPENAI_API_KEY")
            {
                keys.push("OMNE_OPENAI_API_KEY".to_string());
            }
            Some(ditto_core::config::ProviderAuth::HttpHeaderEnv {
                header: hint.header_name.unwrap_or("authorization").to_string(),
                keys,
                prefix: hint.prefix.map(|value| value.to_string()),
            })
        }
        AuthMethodKind::ApiKeyQuery => Some(ditto_core::config::ProviderAuth::QueryParamEnv {
            param: hint.query_param.unwrap_or("key").to_string(),
            keys: hint
                .env_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect::<Vec<_>>(),
            prefix: hint.prefix.map(|value| value.to_string()),
        }),
        _ => None,
    }
}

pub(crate) fn resolve_provider_config(
    provider_name_hint: &str,
    provider_overrides: Option<&ProviderConfig>,
) -> anyhow::Result<ProviderConfig> {
    let mut config = provider_overrides.cloned().unwrap_or_default();

    if let Some(preset) = ditto_core::runtime_registry::builtin_runtime_registry_catalog()
        .provider_preset(provider_name_hint)
    {
        if config
            .base_url
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            if let Some(default_base_url) = preset.default_base_url {
                config.base_url = Some(default_base_url.to_string());
            }
        }
        if config.auth.is_none() {
            config.auth = provider_auth_from_preset_hint(provider_name_hint, preset);
        }
    }

    if config
        .base_url
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
        && provider_name_hint == DEFAULT_OPENAI_PROVIDER
    {
        config.base_url = Some(DEFAULT_OPENAI_BASE_URL.to_string());
    }

    if config.auth.is_none() && provider_name_hint == DEFAULT_OPENAI_PROVIDER {
        config.auth = Some(ditto_core::config::ProviderAuth::HttpHeaderEnv {
            header: "authorization".to_string(),
            keys: vec![
                "OPENAI_API_KEY".to_string(),
                "OMNE_OPENAI_API_KEY".to_string(),
            ],
            prefix: Some("Bearer ".to_string()),
        });
    }

    Ok(config)
}

pub(crate) fn provider_overrides_base_url(
    provider_name_hint: &str,
    providers: &BTreeMap<String, ProviderConfig>,
) -> Option<String> {
    let provider = providers.get(provider_name_hint)?;
    provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn openai_provider_base_url_override_warning(
    provider_name_hint: &str,
    effective_base_url: &str,
) -> Option<ProviderBaseUrlOverrideWarning> {
    let effective_base_url = effective_base_url.trim();
    if effective_base_url.is_empty() {
        return None;
    }

    let preset = ditto_core::runtime_registry::builtin_runtime_registry_catalog()
        .provider_preset(provider_name_hint)?;
    let default_base_url = preset.default_base_url?.trim();
    if default_base_url.is_empty() || default_base_url == effective_base_url {
        return None;
    }

    Some(ProviderBaseUrlOverrideWarning {
        code: format!("provider_base_url_override:{provider_name_hint}"),
        message: format!(
            "provider base_url overrides builtin default: provider={provider_name_hint} effective_base_url={effective_base_url} default_base_url={default_base_url}"
        ),
    })
}

fn parse_dotenv_openai(contents: &str) -> DotenvOpenAiOverrides {
    let mut out = DotenvOpenAiOverrides::default();
    out.dotenv = ditto_core::config::parse_dotenv(contents);

    out.provider = out.dotenv.get("OMNE_OPENAI_PROVIDER").cloned();
    out.base_url = out.dotenv.get("OMNE_OPENAI_BASE_URL").cloned();
    out.model = out.dotenv.get("OMNE_OPENAI_MODEL").cloned();
    out.fallback_providers = out
        .dotenv
        .get("OMNE_OPENAI_FALLBACK_PROVIDERS")
        .map(|value| parse_csv_list(value))
        .unwrap_or_default();

    out
}

fn merge_provider_namespace_aliases(
    providers: &mut BTreeMap<String, ProviderConfig>,
    namespace: &str,
    namespace_providers: BTreeMap<String, ProviderConfig>,
) {
    let namespace = namespace.trim();
    if namespace.is_empty() {
        return;
    }

    for (raw_name, config) in namespace_providers {
        let name = raw_name.trim();
        if name.is_empty() {
            continue;
        }
        let canonical = format!("{namespace}.providers.{name}");
        providers.entry(canonical).or_insert_with(|| config.clone());
        providers
            .entry(format!("{namespace}.provider.{name}"))
            .or_insert_with(|| config.clone());
        providers
            .entry(format!("{namespace}.{name}"))
            .or_insert(config);
    }
}

fn insert_openai_provider_aliases(providers: &mut BTreeMap<String, ProviderConfig>) {
    let snapshot = providers
        .iter()
        .map(|(name, config)| (name.clone(), config.clone()))
        .collect::<Vec<_>>();
    for (name, config) in snapshot {
        if name.contains('.') {
            continue;
        }
        providers
            .entry(format!("openai.providers.{name}"))
            .or_insert_with(|| config.clone());
        providers
            .entry(format!("openai.provider.{name}"))
            .or_insert_with(|| config.clone());
        providers.entry(format!("openai.{name}")).or_insert(config);
    }
}

fn project_config_load_options() -> ConfigLoadOptions {
    ConfigLoadOptions::new().with_format(ConfigFormat::Toml)
}

fn try_load_project_config_document(path: &Path) -> (bool, Option<ConfigDocument>, Option<String>) {
    match try_load_config_document(path, project_config_load_options()) {
        Ok(Some(document)) => (true, Some(document), None),
        Ok(None) => (false, None, None),
        Err(err) => (true, None, Some(err.to_string())),
    }
}

pub async fn load_project_config(thread_root: &Path) -> LoadedProjectConfig {
    let omne_data_dir = thread_root.join(".omne_data");
    let config_local_toml_path = omne_data_dir.join("config_local.toml");
    let config_toml_path = omne_data_dir.join("config.toml");
    let env_path = omne_data_dir.join(".env");

    let mut load_error: Option<String> = None;

    let (config_source, config_path, config_present, config_document) = {
        let (local_present, local_document, local_error) =
            try_load_project_config_document(&config_local_toml_path);
        if let Some(err) = local_error {
            load_error = Some(err);
            (
                ProjectConfigSource::Local,
                config_local_toml_path.clone(),
                true,
                None,
            )
        } else if let Some(document) = local_document {
            (
                ProjectConfigSource::Local,
                config_local_toml_path.clone(),
                local_present,
                Some(document),
            )
        } else {
            let (shared_present, shared_document, shared_error) =
                try_load_project_config_document(&config_toml_path);
            if let Some(err) = shared_error {
                load_error = Some(err);
                (
                    ProjectConfigSource::Shared,
                    config_toml_path.clone(),
                    true,
                    None,
                )
            } else {
                (
                    ProjectConfigSource::Shared,
                    config_toml_path.clone(),
                    shared_present,
                    shared_document,
                )
            }
        }
    };

    let mut enabled = false;
    let mut config_openai_provider: Option<String> = None;
    let mut config_openai_base_url: Option<String> = None;
    let mut config_openai_model: Option<String> = None;
    let mut config_openai_fallback_providers: Vec<String> = Vec::new();
    let mut config_openai_providers: BTreeMap<String, ProviderConfig> = BTreeMap::new();
    let mut config_openai_models: BTreeMap<String, ModelConfig> = BTreeMap::new();
    let mut config_openai_routing: Option<ditto_core::config::ProviderRoutingConfig> = None;
    let mut config_ui_show_thinking: Option<bool> = None;

    if let Some(document) = config_document {
        match document
            .parse_as::<ProjectConfigToml>(ConfigFormatSet::TOML)
            .map_err(anyhow::Error::new)
        {
            Ok(parsed) => {
                enabled = parsed.project_config.enabled;
                config_openai_provider = clean_string_opt(parsed.openai.provider);
                config_openai_base_url = clean_string_opt(parsed.openai.base_url);
                config_openai_model = clean_string_opt(parsed.openai.model);
                config_openai_fallback_providers =
                    clean_string_list(parsed.openai.fallback_providers);
                config_openai_providers = parsed.openai.providers;
                insert_openai_provider_aliases(&mut config_openai_providers);
                merge_provider_namespace_aliases(
                    &mut config_openai_providers,
                    "google",
                    parsed.google.providers,
                );
                merge_provider_namespace_aliases(
                    &mut config_openai_providers,
                    "gemini",
                    parsed.gemini.providers,
                );
                merge_provider_namespace_aliases(
                    &mut config_openai_providers,
                    "claude",
                    parsed.claude.providers,
                );
                merge_provider_namespace_aliases(
                    &mut config_openai_providers,
                    "anthropic",
                    parsed.anthropic.providers,
                );
                config_openai_models = parsed.openai.models;
                config_openai_routing = parsed.openai.routing;
                config_ui_show_thinking = parsed.ui.show_thinking;
            }
            Err(err) => {
                let msg = err.to_string();
                load_error = Some(match load_error {
                    Some(existing) => format!("{existing}; {msg}"),
                    None => msg,
                });
            }
        }
    }

    if !enabled {
        return LoadedProjectConfig {
            enabled,
            config_path,
            config_source,
            config_present,
            env_path,
            env_present: false,
            load_error,
            openai: ProjectOpenAiOverrides::default(),
            ui: ProjectUiOverrides::default(),
        };
    }

    let (env_present, env_raw) = match tokio::fs::read_to_string(&env_path).await {
        Ok(raw) => (true, Some(raw)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (false, None),
        Err(err) => {
            let msg = format!("read {}: {err}", env_path.display());
            load_error = Some(match load_error {
                Some(existing) => format!("{existing}; {msg}"),
                None => msg,
            });
            (true, None)
        }
    };

    let dotenv_openai = env_raw
        .as_deref()
        .map(parse_dotenv_openai)
        .unwrap_or_default()
        .into_project_overrides();

    let ProjectOpenAiOverrides {
        provider: dotenv_provider,
        base_url: dotenv_base_url,
        model: dotenv_model,
        fallback_providers: dotenv_fallback_providers,
        dotenv,
        ..
    } = dotenv_openai;

    let fallback_providers = if dotenv_fallback_providers.is_empty() {
        config_openai_fallback_providers
    } else {
        dotenv_fallback_providers
    };

    let openai = ProjectOpenAiOverrides {
        provider: clean_string_opt(dotenv_provider).or(config_openai_provider),
        base_url: clean_string_opt(dotenv_base_url).or(config_openai_base_url),
        model: clean_string_opt(dotenv_model).or(config_openai_model),
        fallback_providers,
        providers: config_openai_providers,
        models: config_openai_models,
        routing: config_openai_routing,
        dotenv,
    };
    let ui = ProjectUiOverrides {
        show_thinking: config_ui_show_thinking,
    };

    LoadedProjectConfig {
        enabled,
        config_path,
        config_source,
        config_present,
        env_path,
        env_present,
        load_error,
        openai,
        ui,
    }
}

pub async fn load_project_openai_overrides(thread_root: &Path) -> ProjectOpenAiOverrides {
    load_project_config(thread_root).await.openai
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditto_core::config::ThinkingIntensity;

    #[tokio::test]
    async fn loads_provider_reasoning_and_auth_command_from_config_toml() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "openai-auth-command"
model = "codex-mini-latest"

[openai.providers.openai-auth-command]
base_url = "https://example.com/v9"
[openai.providers.openai-auth-command.auth]
type = "command"
command = ["node", "script.mjs"]

[openai.models."*"]
thinking = "small"
[openai.models."codex-mini-latest"]
thinking = "xhigh"
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.provider.as_deref(),
            Some("openai-auth-command")
        );
        assert!(loaded.openai.base_url.is_none());
        assert_eq!(loaded.openai.model.as_deref(), Some("codex-mini-latest"));
        assert_eq!(
            loaded.openai.models.get("*").map(|c| c.thinking),
            Some(ThinkingIntensity::Small)
        );
        assert_eq!(
            loaded
                .openai
                .models
                .get("codex-mini-latest")
                .map(|c| c.thinking),
            Some(ThinkingIntensity::XHigh)
        );
        assert_eq!(
            loaded
                .openai
                .providers
                .get("openai-auth-command")
                .and_then(|p| p.base_url.as_deref())
                .map(str::to_string),
            Some("https://example.com/v9".to_string())
        );
        assert_eq!(
            loaded
                .openai
                .providers
                .get("openai-auth-command")
                .and_then(|p| p.auth.as_ref())
                .and_then(|auth| match auth {
                    ditto_core::config::ProviderAuth::Command { command } => Some(command.clone()),
                    _ => None,
                }),
            Some(vec!["node".to_string(), "script.mjs".to_string()])
        );
        Ok(())
    }

    #[tokio::test]
    async fn loads_base_url_from_config_toml() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
base_url = "https://example.org/v1"
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.base_url.as_deref(),
            Some("https://example.org/v1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn dotenv_provider_overrides_config_provider_when_enabled() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "openai-codex-apikey"
"#,
        )
        .await?;
        tokio::fs::write(
            omne_data.join(".env"),
            r#"
OMNE_OPENAI_PROVIDER=openai-auth-command
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.provider.as_deref(),
            Some("openai-auth-command")
        );
        Ok(())
    }

    #[tokio::test]
    async fn dotenv_base_url_overrides_config_when_enabled() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
base_url = "https://config.example/v1"
"#,
        )
        .await?;
        tokio::fs::write(
            omne_data.join(".env"),
            r#"
OMNE_OPENAI_BASE_URL=https://env.example/v1
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.base_url.as_deref(),
            Some("https://env.example/v1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn loads_fallback_providers_from_config_toml() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
fallback_providers = ["openai-auth-command", "openai-codex-apikey", "openai-auth-command", ""]
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.fallback_providers,
            vec![
                "openai-auth-command".to_string(),
                "openai-codex-apikey".to_string()
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn loads_routing_from_config_toml() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai.routing.profiles.primary]
provider = "openai-codex-apikey"
default_model = "gpt-4.1"

[openai.routing.profiles.thinking]
provider = "openai-codex-apikey"
default_model = "o3"

[openai.routing.default.completion]
targets = [{ profile = "primary" }]

[openai.routing.default.thinking]
targets = [{ profile = "thinking", model = "o3" }]
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);

        let routing = loaded
            .openai
            .routing
            .as_ref()
            .expect("routing should be parsed");
        let completion = routing
            .resolve_plan(ditto_core::config::RoutingContext {
                role: Some("coder"),
                scenario: Some("default"),
                phase: ditto_core::config::RoutingPhase::Completion,
                seed_hash: Some(1),
            })
            .expect("completion plan");
        assert_eq!(completion.targets.len(), 1);
        assert_eq!(completion.targets[0].provider, "openai-codex-apikey");
        assert_eq!(completion.targets[0].model, "gpt-4.1");

        let thinking = routing
            .resolve_plan(ditto_core::config::RoutingContext {
                role: Some("coder"),
                scenario: Some("default"),
                phase: ditto_core::config::RoutingPhase::Thinking,
                seed_hash: Some(1),
            })
            .expect("thinking plan");
        assert_eq!(thinking.targets[0].model, "o3");
        Ok(())
    }

    #[tokio::test]
    async fn dotenv_fallback_providers_override_config_when_present() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
fallback_providers = ["openai-auth-command"]
"#,
        )
        .await?;
        tokio::fs::write(
            omne_data.join(".env"),
            r#"
OMNE_OPENAI_FALLBACK_PROVIDERS=openai-codex-apikey, openai-auth-command
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.fallback_providers,
            vec![
                "openai-codex-apikey".to_string(),
                "openai-auth-command".to_string()
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn loads_google_provider_namespace_and_aliases() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "google.providers.yunwu"

[openai.providers.openai-auth-command]
base_url = "https://example.com/v1"

[google.providers.yunwu]
base_url = "https://ditto-gateway.example/v1"
upstream_api = "gemini_generate_content"
normalize_to = "openai_chat_completions"
normalize_endpoint = "/v1/chat/completions"

[google.providers.yunwu.auth]
type = "http_header_env"
header = "Authorization"
prefix = "Bearer "
keys = ["YUNWU_API_KEY"]
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.provider.as_deref(),
            Some("google.providers.yunwu")
        );

        let google_provider = loaded
            .openai
            .providers
            .get("google.providers.yunwu")
            .expect("google provider should be present");
        assert_eq!(
            google_provider.upstream_api,
            Some(ditto_core::config::ProviderApi::GeminiGenerateContent)
        );
        assert_eq!(
            google_provider.normalize_to,
            Some(ditto_core::config::ProviderApi::OpenaiChatCompletions)
        );
        assert_eq!(
            google_provider.normalize_endpoint.as_deref(),
            Some("/v1/chat/completions")
        );
        assert!(matches!(
            google_provider.auth.as_ref(),
            Some(ditto_core::config::ProviderAuth::HttpHeaderEnv { header, keys, prefix })
                if header == "Authorization"
                    && keys == &vec!["YUNWU_API_KEY".to_string()]
                    && prefix.as_deref() == Some("Bearer ")
        ));

        assert!(
            loaded
                .openai
                .providers
                .contains_key("google.provider.yunwu")
        );
        assert!(
            loaded
                .openai
                .providers
                .contains_key("openai.providers.openai-auth-command")
        );

        Ok(())
    }

    #[tokio::test]
    async fn prefers_config_local_toml_over_shared_config_toml() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let omne_data = root.join(".omne_data");
        tokio::fs::create_dir_all(&omne_data).await?;

        tokio::fs::write(
            omne_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "shared-provider"
"#,
        )
        .await?;
        tokio::fs::write(
            omne_data.join("config_local.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "local-provider"
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert!(matches!(loaded.config_source, ProjectConfigSource::Local));
        assert_eq!(
            loaded
                .config_path
                .file_name()
                .and_then(|name| name.to_str()),
            Some("config_local.toml")
        );
        assert_eq!(loaded.openai.provider.as_deref(), Some("local-provider"));
        Ok(())
    }
}
