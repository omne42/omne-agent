use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ditto_llm::{ModelConfig, ProviderConfig, ThinkingIntensity};
use serde::Deserialize;

#[derive(Default)]
pub struct ProjectOpenAiOverrides {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub models: BTreeMap<String, ModelConfig>,
    pub dotenv: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug)]
pub enum ProjectConfigSource {
    /// `.codepm_data/config_local.toml` (gitignored)
    Local,
    /// `.codepm_data/config.toml` (commit-safe)
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
    providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    models: BTreeMap<String, ModelConfig>,
    #[serde(default)]
    model_reasoning_effort: BTreeMap<String, ThinkingIntensity>,
    #[serde(default)]
    auth_command: Option<ProjectOpenAiAuthCommandSection>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectOpenAiAuthCommandSection {
    #[serde(default)]
    command: Vec<String>,
}

#[derive(Default)]
struct DotenvOpenAiOverrides {
    provider: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    dotenv: BTreeMap<String, String>,
}

impl DotenvOpenAiOverrides {
    fn into_project_overrides(self) -> ProjectOpenAiOverrides {
        ProjectOpenAiOverrides {
            provider: self.provider,
            base_url: self.base_url,
            model: self.model,
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

fn parse_dotenv_openai(contents: &str) -> DotenvOpenAiOverrides {
    let mut out = DotenvOpenAiOverrides::default();
    out.dotenv = ditto_llm::parse_dotenv(contents);

    out.provider = out.dotenv.get("CODE_PM_OPENAI_PROVIDER").cloned();
    out.base_url = out.dotenv.get("CODE_PM_OPENAI_BASE_URL").cloned();
    out.model = out.dotenv.get("CODE_PM_OPENAI_MODEL").cloned();

    out
}

pub async fn load_project_config(thread_root: &Path) -> LoadedProjectConfig {
    let codepm_data_dir = thread_root.join(".codepm_data");
    let config_local_toml_path = codepm_data_dir.join("config_local.toml");
    let config_toml_path = codepm_data_dir.join("config.toml");
    let env_path = codepm_data_dir.join(".env");

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
    let mut config_openai_providers: BTreeMap<String, ProviderConfig> = BTreeMap::new();
    let mut config_openai_models: BTreeMap<String, ModelConfig> = BTreeMap::new();
    let mut config_openai_model_reasoning_effort: BTreeMap<String, ThinkingIntensity> =
        BTreeMap::new();
    let mut config_openai_auth_command: Option<Vec<String>> = None;

    if let Some(raw) = config_raw {
        match toml::from_str::<ProjectConfigToml>(&raw) {
            Ok(parsed) => {
                enabled = parsed.project_config.enabled;
                config_openai_provider = clean_string_opt(parsed.openai.provider);
                config_openai_base_url = clean_string_opt(parsed.openai.base_url);
                config_openai_model = clean_string_opt(parsed.openai.model);
                config_openai_providers = parsed.openai.providers;
                config_openai_models = parsed.openai.models;
                for (k, v) in parsed.openai.model_reasoning_effort {
                    let key = k.trim().to_string();
                    if !key.is_empty() {
                        config_openai_model_reasoning_effort.insert(key, v);
                    }
                }
                config_openai_auth_command = parsed.openai.auth_command.and_then(|section| {
                    let command = section
                        .command
                        .into_iter()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>();
                    if command.is_empty() {
                        None
                    } else {
                        Some(command)
                    }
                });
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

    let mut openai = ProjectOpenAiOverrides {
        provider: clean_string_opt(dotenv_openai.provider).or(config_openai_provider),
        base_url: clean_string_opt(dotenv_openai.base_url).or(config_openai_base_url),
        model: clean_string_opt(dotenv_openai.model).or(config_openai_model),
        providers: config_openai_providers,
        models: config_openai_models,
        dotenv: dotenv_openai.dotenv,
    };

    for (k, v) in config_openai_model_reasoning_effort {
        openai
            .models
            .entry(k)
            .or_insert_with(|| ModelConfig { thinking: v });
    }

    if let Some(command) = config_openai_auth_command {
        let provider = openai.provider.as_deref().unwrap_or("openai-auth-command");
        if provider == "openai-auth-command" {
            openai
                .providers
                .entry(provider.to_string())
                .or_default()
                .auth = Some(ditto_llm::ProviderAuth::Command { command });
        }
    }

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

    #[tokio::test]
    async fn loads_provider_reasoning_and_auth_command_from_config_toml() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let codepm_data = root.join(".codepm_data");
        tokio::fs::create_dir_all(&codepm_data).await?;

        tokio::fs::write(
            codepm_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "openai-auth-command"
base_url = "https://example.com/v9"
model = "codex-mini-latest"
model_reasoning_effort = { "*" = "small", "codex-mini-latest" = "xhigh" }

[openai.auth_command]
command = ["node", "script.mjs"]
"#,
        )
        .await?;

        let loaded = load_project_config(root).await;
        assert!(loaded.enabled);
        assert_eq!(
            loaded.openai.provider.as_deref(),
            Some("openai-auth-command")
        );
        assert_eq!(
            loaded.openai.base_url.as_deref(),
            Some("https://example.com/v9")
        );
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
    async fn dotenv_provider_overrides_config_provider_when_enabled() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        let codepm_data = root.join(".codepm_data");
        tokio::fs::create_dir_all(&codepm_data).await?;

        tokio::fs::write(
            codepm_data.join("config.toml"),
            r#"
[project_config]
enabled = true

[openai]
provider = "openai-codex-apikey"
"#,
        )
        .await?;
        tokio::fs::write(
            codepm_data.join(".env"),
            r#"
CODE_PM_OPENAI_PROVIDER=openai-auth-command
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
}
