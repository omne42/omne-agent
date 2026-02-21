use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ditto_llm::{ModelConfig, ProviderConfig};
use serde::Deserialize;

#[derive(Default)]
pub struct ProjectOpenAiOverrides {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub fallback_providers: Vec<String>,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub models: BTreeMap<String, ModelConfig>,
    pub dotenv: BTreeMap<String, String>,
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
}

#[derive(Debug, Default, Deserialize)]
struct ProjectConfigToml {
    #[serde(default)]
    project_config: ProjectConfigSection,
    #[serde(default)]
    openai: ProjectOpenAiSection,
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

fn parse_dotenv_openai(contents: &str) -> DotenvOpenAiOverrides {
    let mut out = DotenvOpenAiOverrides::default();
    out.dotenv = ditto_llm::parse_dotenv(contents);

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

pub async fn load_project_config(thread_root: &Path) -> LoadedProjectConfig {
    let omne_data_dir = thread_root.join(".omne_data");
    let config_local_toml_path = omne_data_dir.join("config_local.toml");
    let config_toml_path = omne_data_dir.join("config.toml");
    let env_path = omne_data_dir.join(".env");

    let mut load_error: Option<String> = None;

    let (config_source, config_path, config_present, config_raw) =
        match tokio::fs::read_to_string(&config_local_toml_path).await {
            Ok(raw) => (
                ProjectConfigSource::Local,
                config_local_toml_path.clone(),
                true,
                Some(raw),
            ),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                match tokio::fs::read_to_string(&config_toml_path).await {
                    Ok(raw) => (
                        ProjectConfigSource::Shared,
                        config_toml_path.clone(),
                        true,
                        Some(raw),
                    ),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => (
                        ProjectConfigSource::Shared,
                        config_toml_path.clone(),
                        false,
                        None,
                    ),
                    Err(err) => {
                        load_error = Some(format!("read {}: {err}", config_toml_path.display()));
                        (
                            ProjectConfigSource::Shared,
                            config_toml_path.clone(),
                            true,
                            None,
                        )
                    }
                }
            }
            Err(err) => {
                load_error = Some(format!("read {}: {err}", config_local_toml_path.display()));
                (
                    ProjectConfigSource::Local,
                    config_local_toml_path.clone(),
                    true,
                    None,
                )
            }
        };

    let mut enabled = false;
    let mut config_openai_provider: Option<String> = None;
    let mut config_openai_base_url: Option<String> = None;
    let mut config_openai_model: Option<String> = None;
    let mut config_openai_fallback_providers: Vec<String> = Vec::new();
    let mut config_openai_providers: BTreeMap<String, ProviderConfig> = BTreeMap::new();
    let mut config_openai_models: BTreeMap<String, ModelConfig> = BTreeMap::new();

    if let Some(raw) = config_raw {
        match toml::from_str::<ProjectConfigToml>(&raw) {
            Ok(parsed) => {
                enabled = parsed.project_config.enabled;
                config_openai_provider = clean_string_opt(parsed.openai.provider);
                config_openai_base_url = clean_string_opt(parsed.openai.base_url);
                config_openai_model = clean_string_opt(parsed.openai.model);
                config_openai_fallback_providers =
                    clean_string_list(parsed.openai.fallback_providers);
                config_openai_providers = parsed.openai.providers;
                config_openai_models = parsed.openai.models;
            }
            Err(err) => {
                let msg = format!("parse {}: {err}", config_path.display());
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
        dotenv,
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
    }
}

pub async fn load_project_openai_overrides(thread_root: &Path) -> ProjectOpenAiOverrides {
    load_project_config(thread_root).await.openai
}

#[cfg(test)]
mod tests {
    use super::*;
    use ditto_llm::ThinkingIntensity;

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
                    ditto_llm::ProviderAuth::Command { command } => Some(command.clone()),
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
}
