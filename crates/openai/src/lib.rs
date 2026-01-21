use std::collections::BTreeMap;

use anyhow::Context;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

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
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub input_tokens_details: Option<Value>,
    #[serde(default)]
    pub output_tokens_details: Option<Value>,
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiError {
    #[serde(rename = "type", default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub param: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RateLimitBucket {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub reset: Option<String>,
}

impl RateLimitBucket {
    fn is_empty(&self) -> bool {
        self.limit.is_none() && self.remaining.is_none() && self.reset.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimits {
    pub requests: RateLimitBucket,
    pub tokens: RateLimitBucket,
}

impl RateLimits {
    pub fn from_headers(headers: &reqwest::header::HeaderMap) -> Option<Self> {
        let requests = RateLimitBucket {
            limit: header_u64(headers, "x-ratelimit-limit-requests"),
            remaining: header_u64(headers, "x-ratelimit-remaining-requests"),
            reset: header_string(headers, "x-ratelimit-reset-requests"),
        };
        let tokens = RateLimitBucket {
            limit: header_u64(headers, "x-ratelimit-limit-tokens"),
            remaining: header_u64(headers, "x-ratelimit-remaining-tokens"),
            reset: header_string(headers, "x-ratelimit-reset-tokens"),
        };

        let out = Self { requests, tokens };
        if out.requests.is_empty() && out.tokens.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

#[derive(Debug, Clone)]
pub enum ResponseEvent {
    Created {
        response_id: Option<String>,
    },
    ModelsEtag(String),
    RateLimits(RateLimits),
    OutputTextDelta(String),
    OutputItemDone(ResponseItem),
    ReasoningTextDelta {
        delta: String,
        content_index: i64,
    },
    ReasoningSummaryTextDelta {
        delta: String,
        summary_index: i64,
    },
    Failed {
        response_id: Option<String>,
        error: ApiError,
    },
    Completed {
        response_id: Option<String>,
        usage: Option<TokenUsage>,
    },
}

pub struct ResponseEventStream {
    rx_event: mpsc::Receiver<anyhow::Result<ResponseEvent>>,
    task: tokio::task::JoinHandle<()>,
}

impl ResponseEventStream {
    pub async fn recv(&mut self) -> Option<anyhow::Result<ResponseEvent>> {
        self.rx_event.recv().await
    }
}

impl Drop for ResponseEventStream {
    fn drop(&mut self) {
        self.task.abort();
    }
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

    pub async fn create_response_stream(
        &self,
        request: &ResponsesApiRequest<'_>,
    ) -> anyhow::Result<ResponseEventStream> {
        if !request.stream {
            anyhow::bail!("stream=true is required for create_response_stream");
        }

        let url = format!("{}/v1/responses", self.base_url.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .json(request)
            .send()
            .await
            .context("send /v1/responses (stream)")?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("openai responses stream failed ({status}): {text}");
        }

        let headers = response.headers().clone();
        let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
        let reader = StreamReader::new(byte_stream);
        let lines = tokio::io::BufReader::new(reader).lines();

        let (tx_event, rx_event) = mpsc::channel::<anyhow::Result<ResponseEvent>>(512);
        let task = tokio::spawn(process_response_stream(lines, headers, tx_event));

        Ok(ResponseEventStream { rx_event, task })
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

fn header_string(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn header_u64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    header_string(headers, name).and_then(|v| v.parse::<u64>().ok())
}

#[derive(Debug, Deserialize)]
struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    response: Option<Value>,
    #[serde(default)]
    item: Option<Value>,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    summary_index: Option<i64>,
    #[serde(default)]
    content_index: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ResponseCompleted {
    id: String,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct ResponseDone {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

fn parse_response_event(event: ResponsesStreamEvent) -> anyhow::Result<Option<ResponseEvent>> {
    match event.kind.as_str() {
        "response.created" => {
            let response_id = event
                .response
                .as_ref()
                .and_then(|resp| resp.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(Some(ResponseEvent::Created { response_id }))
        }
        "response.output_text.delta" => Ok(event.delta.map(ResponseEvent::OutputTextDelta)),
        "response.output_item.done" => {
            if let Some(item_val) = event.item {
                let item: ResponseItem = serde_json::from_value(item_val)
                    .context("parse ResponseItem from response.output_item.done")?;
                return Ok(Some(ResponseEvent::OutputItemDone(item)));
            }
            Ok(None)
        }
        "response.reasoning_text.delta" => {
            if let (Some(delta), Some(content_index)) = (event.delta, event.content_index) {
                return Ok(Some(ResponseEvent::ReasoningTextDelta {
                    delta,
                    content_index,
                }));
            }
            Ok(None)
        }
        "response.reasoning_summary_text.delta" => {
            if let (Some(delta), Some(summary_index)) = (event.delta, event.summary_index) {
                return Ok(Some(ResponseEvent::ReasoningSummaryTextDelta {
                    delta,
                    summary_index,
                }));
            }
            Ok(None)
        }
        "response.failed" => {
            let Some(resp) = event.response else {
                return Ok(Some(ResponseEvent::Failed {
                    response_id: None,
                    error: ApiError {
                        r#type: None,
                        code: None,
                        message: Some("openai response.failed event received".to_string()),
                        param: None,
                    },
                }));
            };

            let response_id = resp
                .get("id")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());

            let error = match resp.get("error") {
                Some(error) => serde_json::from_value::<ApiError>(error.clone())
                    .context("parse response.failed error payload")?,
                None => ApiError {
                    r#type: None,
                    code: None,
                    message: Some(resp.to_string()),
                    param: None,
                },
            };

            Ok(Some(ResponseEvent::Failed { response_id, error }))
        }
        "response.completed" => {
            if let Some(resp_val) = event.response {
                let completed: ResponseCompleted =
                    serde_json::from_value(resp_val).context("parse response.completed payload")?;
                return Ok(Some(ResponseEvent::Completed {
                    response_id: Some(completed.id),
                    usage: completed.usage,
                }));
            }
            anyhow::bail!("openai response.completed missing response payload");
        }
        "response.done" => {
            if let Some(resp_val) = event.response {
                let done: ResponseDone =
                    serde_json::from_value(resp_val).context("parse response.done payload")?;
                return Ok(Some(ResponseEvent::Completed {
                    response_id: done.id,
                    usage: done.usage,
                }));
            }
            Ok(Some(ResponseEvent::Completed {
                response_id: None,
                usage: None,
            }))
        }
        _ => Ok(None),
    }
}

async fn process_response_stream<R>(
    lines: tokio::io::Lines<R>,
    headers: reqwest::header::HeaderMap,
    tx_event: mpsc::Sender<anyhow::Result<ResponseEvent>>,
) where
    R: tokio::io::AsyncBufRead + Unpin,
{
    if let Some(etag) = header_string(&headers, "x-models-etag") {
        let _ = tx_event.send(Ok(ResponseEvent::ModelsEtag(etag))).await;
    }
    if let Some(rate_limits) = RateLimits::from_headers(&headers) {
        let _ = tx_event
            .send(Ok(ResponseEvent::RateLimits(rate_limits)))
            .await;
    }

    process_sse(lines, tx_event).await;
}

async fn process_sse<R>(
    mut lines: tokio::io::Lines<R>,
    tx_event: mpsc::Sender<anyhow::Result<ResponseEvent>>,
) where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut data = String::new();
    let mut saw_terminal = false;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let line = line.trim_end_matches('\r');
                if line.is_empty() {
                    if data.is_empty() {
                        continue;
                    }
                    if data == "[DONE]" {
                        break;
                    }
                    let parsed = serde_json::from_str::<ResponsesStreamEvent>(&data)
                        .context("parse responses stream event")
                        .and_then(parse_response_event);
                    data.clear();

                    match parsed {
                        Ok(Some(event)) => {
                            saw_terminal |= matches!(
                                event,
                                ResponseEvent::Completed { .. } | ResponseEvent::Failed { .. }
                            );
                            if tx_event.send(Ok(event)).await.is_err() {
                                return;
                            }
                            if saw_terminal {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            let _ = tx_event.send(Err(err)).await;
                            return;
                        }
                    }
                    continue;
                }

                if let Some(rest) = line.strip_prefix("data:") {
                    let rest = rest.trim_start();
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(rest);
                }
            }
            Ok(None) => {
                if !saw_terminal {
                    let _ = tx_event
                        .send(Err(anyhow::anyhow!(
                            "stream closed before response.completed"
                        )))
                        .await;
                }
                return;
            }
            Err(err) => {
                let _ = tx_event.send(Err(err.into())).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::stream;
    use reqwest::header::{HeaderMap, HeaderValue};

    async fn collect_from_sse(sse: &str) -> Vec<anyhow::Result<ResponseEvent>> {
        let (tx, mut rx) = mpsc::channel::<anyhow::Result<ResponseEvent>>(16);
        let stream = stream::iter([Ok::<_, std::io::Error>(Bytes::from(sse.to_owned()))]);
        let reader = StreamReader::new(stream);
        let lines = tokio::io::BufReader::new(reader).lines();
        process_sse(lines, tx).await;

        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    }

    #[tokio::test]
    async fn parses_output_text_and_completed() -> anyhow::Result<()> {
        let sse = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            "event: response.output_item.done\n",
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"name\":\"file_read\",\"arguments\":\"{}\",\"call_id\":\"c1\"}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp1\",\"usage\":{\"total_tokens\":123}}}\n\n",
        );

        let events = collect_from_sse(sse).await;
        assert_eq!(events.len(), 3);

        match &events[0] {
            Ok(ResponseEvent::OutputTextDelta(delta)) => assert_eq!(delta, "Hello"),
            other => anyhow::bail!("unexpected event: {other:?}"),
        }

        match &events[1] {
            Ok(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                name, call_id, ..
            })) => {
                assert_eq!(name, "file_read");
                assert_eq!(call_id, "c1");
            }
            other => anyhow::bail!("unexpected event: {other:?}"),
        }

        match &events[2] {
            Ok(ResponseEvent::Completed { response_id, usage }) => {
                assert_eq!(response_id.as_deref(), Some("resp1"));
                assert_eq!(usage.as_ref().and_then(|u| u.total_tokens), Some(123));
            }
            other => anyhow::bail!("unexpected event: {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn parses_failed_event() -> anyhow::Result<()> {
        let sse = concat!(
            "event: response.failed\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp1\",\"error\":{\"type\":\"rate_limit_error\",\"code\":\"rate_limit_exceeded\",\"message\":\"nope\"}}}\n\n",
        );

        let events = collect_from_sse(sse).await;
        assert_eq!(events.len(), 1);

        match &events[0] {
            Ok(ResponseEvent::Failed { response_id, error }) => {
                assert_eq!(response_id.as_deref(), Some("resp1"));
                assert_eq!(error.r#type.as_deref(), Some("rate_limit_error"));
                assert_eq!(error.code.as_deref(), Some("rate_limit_exceeded"));
                assert_eq!(error.message.as_deref(), Some("nope"));
            }
            other => anyhow::bail!("unexpected event: {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn errors_when_stream_closes_without_completed() -> anyhow::Result<()> {
        let sse = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
        );
        let events = collect_from_sse(sse).await;
        assert_eq!(events.len(), 2);
        events[0].as_ref().expect("first event ok");
        assert!(events[1].is_err());
        Ok(())
    }

    #[test]
    fn parses_rate_limits_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-limit-requests",
            HeaderValue::from_static("100"),
        );
        headers.insert(
            "x-ratelimit-remaining-requests",
            HeaderValue::from_static("99"),
        );
        headers.insert("x-ratelimit-reset-requests", HeaderValue::from_static("1s"));
        headers.insert("x-ratelimit-limit-tokens", HeaderValue::from_static("1000"));
        headers.insert(
            "x-ratelimit-remaining-tokens",
            HeaderValue::from_static("900"),
        );
        headers.insert("x-ratelimit-reset-tokens", HeaderValue::from_static("2s"));

        let limits = RateLimits::from_headers(&headers).expect("limits present");
        assert_eq!(limits.requests.limit, Some(100));
        assert_eq!(limits.requests.remaining, Some(99));
        assert_eq!(limits.requests.reset.as_deref(), Some("1s"));
        assert_eq!(limits.tokens.limit, Some(1000));
        assert_eq!(limits.tokens.remaining, Some(900));
        assert_eq!(limits.tokens.reset.as_deref(), Some("2s"));
    }
}
