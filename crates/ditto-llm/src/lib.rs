use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DittoError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("failed to run auth command: {0}")]
    AuthCommand(String),
    #[error("failed to parse json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DittoError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingIntensity {
    #[serde(
        alias = "none",
        alias = "disabled",
        alias = "off",
        alias = "not_supported"
    )]
    Unsupported,
    #[serde(alias = "low")]
    Small,
    #[default]
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(default)]
    pub thinking: ThinkingIntensity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderAuth {
    #[serde(rename = "api_key_env", alias = "env", alias = "api_key")]
    ApiKeyEnv {
        #[serde(default)]
        keys: Vec<String>,
    },
    #[serde(alias = "auth_command")]
    Command { command: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub model_whitelist: Vec<String>,
    #[serde(default)]
    pub auth: Option<ProviderAuth>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderCapabilities {
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub streaming: bool,
}

impl ProviderCapabilities {
    pub fn openai_responses() -> Self {
        Self {
            tools: true,
            vision: true,
            reasoning: true,
            json_schema: true,
            streaming: true,
        }
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;

    async fn list_models(&self) -> Result<Vec<String>>;
}

pub struct OpenAiProvider {
    name: String,
    base_url: String,
    bearer_token: String,
    model_whitelist: Vec<String>,
    capabilities: ProviderCapabilities,
}

impl OpenAiProvider {
    pub async fn from_config(
        name: impl Into<String>,
        config: &ProviderConfig,
        env: &Env,
    ) -> Result<Self> {
        let base_url = config.base_url.as_deref().ok_or_else(|| {
            DittoError::InvalidResponse("provider base_url is missing".to_string())
        })?;
        let auth = config
            .auth
            .clone()
            .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
        let bearer_token = resolve_auth_token(&auth, env).await?;

        Ok(Self {
            name: name.into(),
            base_url: base_url.to_string(),
            bearer_token,
            model_whitelist: config.model_whitelist.clone(),
            capabilities: ProviderCapabilities::openai_responses(),
        })
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        let client = OpenAiCompatibleClient::new(self.bearer_token.clone(), self.base_url.clone())?;
        let models = client.list_models().await?;
        Ok(filter_models_whitelist(models, &self.model_whitelist))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Env {
    pub dotenv: BTreeMap<String, String>,
}

impl Env {
    pub fn parse_dotenv(contents: &str) -> Self {
        Self {
            dotenv: parse_dotenv(contents),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if let Some(value) = self.dotenv.get(key) {
            return Some(value.clone());
        }
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}

pub fn parse_dotenv(contents: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::<String, String>::new();

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

        out.insert(key.to_string(), value);
    }

    out
}

pub fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = BTreeSet::<String>::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

pub fn select_model_config<'a>(
    models: &'a BTreeMap<String, ModelConfig>,
    model: &str,
) -> Option<&'a ModelConfig> {
    if let Some(config) = models.get(model) {
        return Some(config);
    }
    models.get("*")
}

#[derive(Clone)]
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    bearer_token: String,
}

impl OpenAiCompatibleClient {
    pub fn new(bearer_token: String, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            http,
            base_url,
            bearer_token,
        })
    }

    fn models_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{base}/models")
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        #[derive(Debug, Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            data: Vec<ModelItem>,
        }

        #[derive(Debug, Deserialize)]
        struct ModelItem {
            id: String,
        }

        let url = self.models_url();
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.bearer_token)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(DittoError::InvalidResponse(format!(
                "GET /models failed ({status})"
            )));
        }

        let parsed = response.json::<ModelsResponse>().await?;
        let mut out = parsed
            .data
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        out.sort();
        out.dedup();
        Ok(out)
    }
}

pub fn filter_models_whitelist(models: Vec<String>, whitelist: &[String]) -> Vec<String> {
    if whitelist.is_empty() {
        return models;
    }

    let allow = whitelist
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect::<BTreeSet<_>>();

    models
        .into_iter()
        .filter(|m| allow.contains(m))
        .collect::<Vec<_>>()
}

pub async fn list_available_models(provider: &ProviderConfig, env: &Env) -> Result<Vec<String>> {
    let base_url = provider
        .base_url
        .as_deref()
        .ok_or_else(|| DittoError::InvalidResponse("provider base_url is missing".to_string()))?;
    let auth = provider
        .auth
        .clone()
        .unwrap_or(ProviderAuth::ApiKeyEnv { keys: Vec::new() });
    let token = resolve_auth_token(&auth, env).await?;
    let client = OpenAiCompatibleClient::new(token, base_url.to_string())?;
    let models = client.list_models().await?;
    Ok(filter_models_whitelist(models, &provider.model_whitelist))
}

pub async fn resolve_auth_token(auth: &ProviderAuth, env: &Env) -> Result<String> {
    match auth {
        ProviderAuth::ApiKeyEnv { keys } => {
            const DEFAULT_KEYS: &[&str] = &["OPENAI_API_KEY", "CODE_PM_OPENAI_API_KEY"];
            if keys.is_empty() {
                for key in DEFAULT_KEYS {
                    if let Some(value) = env.get(key) {
                        return Ok(value);
                    }
                }
                return Err(DittoError::AuthCommand(format!(
                    "missing api key env (tried: {})",
                    DEFAULT_KEYS.join(", ")
                )));
            }
            for key in keys {
                if let Some(value) = env.get(key.as_str()) {
                    return Ok(value);
                }
            }
            Err(DittoError::AuthCommand(format!(
                "missing api key env (tried: {})",
                keys.join(", "),
            )))
        }
        ProviderAuth::Command { command } => {
            let (program, args) = command
                .split_first()
                .ok_or_else(|| DittoError::AuthCommand("command is empty".to_string()))?;
            let output = tokio::process::Command::new(program)
                .args(args)
                .output()
                .await
                .map_err(|err| DittoError::AuthCommand(format!("spawn {program}: {err}")))?;
            if !output.status.success() {
                return Err(DittoError::AuthCommand(format!(
                    "command failed with status {}",
                    output.status
                )));
            }

            #[derive(Deserialize)]
            struct AuthCommandOutput {
                #[serde(default)]
                api_key: Option<String>,
                #[serde(default)]
                token: Option<String>,
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let parsed = serde_json::from_str::<AuthCommandOutput>(stdout.trim())?;
            parsed
                .api_key
                .or(parsed.token)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| DittoError::AuthCommand("json missing api_key/token".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv_basic() {
        let parsed = parse_dotenv(
            r#"
# comment
export OPENAI_API_KEY="sk-test"
FOO=bar
EMPTY=
"#,
        );
        assert_eq!(
            parsed.get("OPENAI_API_KEY").map(String::as_str),
            Some("sk-test")
        );
        assert_eq!(parsed.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(parsed.get("EMPTY"), None);
    }

    #[test]
    fn thinking_intensity_defaults_to_medium() {
        let parsed = toml::from_str::<ModelConfig>("").expect("parse toml");
        assert_eq!(parsed.thinking, ThinkingIntensity::Medium);
    }

    #[test]
    fn selects_exact_then_wildcard_model_config() {
        let models = BTreeMap::from([
            (
                "*".to_string(),
                ModelConfig {
                    thinking: ThinkingIntensity::High,
                },
            ),
            (
                "gpt-4.1".to_string(),
                ModelConfig {
                    thinking: ThinkingIntensity::XHigh,
                },
            ),
        ]);
        assert_eq!(
            select_model_config(&models, "gpt-4.1").map(|c| c.thinking),
            Some(ThinkingIntensity::XHigh)
        );
        assert_eq!(
            select_model_config(&models, "other").map(|c| c.thinking),
            Some(ThinkingIntensity::High)
        );
    }
}
