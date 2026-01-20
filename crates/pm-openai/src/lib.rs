use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    InputText {
        text: String,
    },
    OutputText {
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseItem {
    Message {
        role: String,
        content: Vec<ContentItem>,
    },
    FunctionCall {
        name: String,
        arguments: String,
        call_id: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Serialize)]
pub struct ResponsesApiRequest<'a> {
    pub model: &'a str,
    pub instructions: &'a str,
    pub input: &'a [ResponseItem],
    pub tools: &'a [Value],
    pub tool_choice: &'a str,
    pub parallel_tool_calls: bool,
    pub store: bool,
    pub stream: bool,
}

#[derive(Debug, Deserialize)]
pub struct ResponsesApiResponse {
    pub id: String,
    #[serde(default)]
    pub output: Vec<ResponseItem>,
    #[serde(default)]
    pub usage: Option<Value>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl Client {
    pub fn new(api_key: String) -> anyhow::Result<Self> {
        Self::new_with_base_url(api_key, "https://api.openai.com".to_string())
    }

    pub fn new_with_base_url(api_key: String, base_url: String) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .context("build reqwest client")?;
        Ok(Self {
            http,
            base_url,
            api_key,
        })
    }

    pub async fn create_response(
        &self,
        request: &ResponsesApiRequest<'_>,
    ) -> anyhow::Result<ResponsesApiResponse> {
        let url = format!("{}/v1/responses", self.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(request)
            .send()
            .await
            .context("send /v1/responses")?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("openai responses failed ({status}): {text}");
        }

        response
            .json::<ResponsesApiResponse>()
            .await
            .context("parse /v1/responses json")
    }
}

pub fn tool_function(name: &str, description: &str, parameters: Value) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}
