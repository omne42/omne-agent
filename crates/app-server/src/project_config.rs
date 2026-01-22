use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Default)]
pub struct ProjectOpenAiOverrides {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

pub struct LoadedProjectConfig {
    pub enabled: bool,
    pub config_toml_path: PathBuf,
    pub config_toml_present: bool,
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
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Default)]
struct DotenvOpenAiOverrides {
    openai_api_key: Option<String>,
    code_pm_openai_api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
}

impl DotenvOpenAiOverrides {
    fn into_project_overrides(self) -> ProjectOpenAiOverrides {
        ProjectOpenAiOverrides {
            api_key: self.openai_api_key.or(self.code_pm_openai_api_key),
            base_url: self.base_url,
            model: self.model,
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

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }

        let mut value = raw_value.trim().to_string();
        if let Some(stripped) = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        {
            value = stripped.to_string();
        }

        if value.trim().is_empty() {
            continue;
        }

        match key {
            "OPENAI_API_KEY" => out.openai_api_key = Some(value),
            "CODE_PM_OPENAI_API_KEY" => out.code_pm_openai_api_key = Some(value),
            "CODE_PM_OPENAI_BASE_URL" => out.base_url = Some(value),
            "CODE_PM_OPENAI_MODEL" => out.model = Some(value),
            _ => {}
        }
    }

    out
}

pub async fn load_project_config(thread_root: &Path) -> LoadedProjectConfig {
    let codepm_data_dir = thread_root.join(".codepm_data");
    let config_toml_path = codepm_data_dir.join("config.toml");
    let env_path = codepm_data_dir.join(".env");

    let mut load_error: Option<String> = None;

    let (config_toml_present, config_toml_raw) =
        match tokio::fs::read_to_string(&config_toml_path).await {
            Ok(raw) => (true, Some(raw)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (false, None),
            Err(err) => {
                load_error = Some(format!("read {}: {err}", config_toml_path.display()));
                (true, None)
            }
        };

    let mut enabled = false;
    let mut config_openai_base_url: Option<String> = None;
    let mut config_openai_model: Option<String> = None;

    if let Some(raw) = config_toml_raw {
        match toml::from_str::<ProjectConfigToml>(&raw) {
            Ok(parsed) => {
                enabled = parsed.project_config.enabled;
                config_openai_base_url = clean_string_opt(parsed.openai.base_url);
                config_openai_model = clean_string_opt(parsed.openai.model);
            }
            Err(err) => {
                let msg = format!("parse {}: {err}", config_toml_path.display());
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
            config_toml_path,
            config_toml_present,
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

    let openai = ProjectOpenAiOverrides {
        api_key: dotenv_openai.api_key,
        base_url: clean_string_opt(dotenv_openai.base_url).or(config_openai_base_url),
        model: clean_string_opt(dotenv_openai.model).or(config_openai_model),
    };

    LoadedProjectConfig {
        enabled,
        config_toml_path,
        config_toml_present,
        env_path,
        env_present,
        load_error,
        openai,
    }
}

pub async fn load_project_openai_overrides(thread_root: &Path) -> ProjectOpenAiOverrides {
    load_project_config(thread_root).await.openai
}
