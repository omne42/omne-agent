use anyhow::Context;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::io::StreamReader;

use crate::{ApiError, RateLimits, TokenUsage, header_string};

#[derive(Debug)]
pub struct HttpError {
    pub status: reqwest::StatusCode,
    pub body: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "openai http error ({})", self.status)?;
        if !self.body.trim().is_empty() {
            write!(f, ": {}", self.body)?;
        }
        Ok(())
    }
}

impl std::error::Error for HttpError {}

#[derive(Debug, Serialize)]
pub struct ResponsesApiRequestRaw<'a> {
    pub model: &'a str,
    pub instructions: &'a str,
    pub input: &'a [Value],
    pub tools: &'a [Value],
    pub tool_choice: &'a str,
    pub parallel_tool_calls: bool,
    pub store: bool,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<&'a Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponsesApiResponseRaw {
    pub id: String,
    #[serde(default)]
    pub output: Vec<Value>,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
    #[serde(flatten)]
    pub other: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct CompactionInput<'a> {
    pub model: &'a str,
    pub input: &'a [Value],
    pub instructions: &'a str,
}

#[derive(Debug, Deserialize)]
struct CompactionResponse {
    output: Vec<Value>,
}

#[derive(Debug, Clone)]
pub enum ResponseEventRaw {
    Created {
        response_id: Option<String>,
    },
    ModelsEtag(String),
    RateLimits(RateLimits),
    OutputTextDelta(String),
    OutputItemDone(Value),
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

pub struct ResponseEventStreamRaw {
    rx_event: mpsc::Receiver<anyhow::Result<ResponseEventRaw>>,
    task: tokio::task::JoinHandle<()>,
}

impl ResponseEventStreamRaw {
    pub async fn recv(&mut self) -> Option<anyhow::Result<ResponseEventRaw>> {
        self.rx_event.recv().await
    }
}

impl Drop for ResponseEventStreamRaw {
    fn drop(&mut self) {
        self.task.abort();
    }
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

fn parse_response_event(event: ResponsesStreamEvent) -> anyhow::Result<Option<ResponseEventRaw>> {
    match event.kind.as_str() {
        "response.created" => {
            let response_id = event
                .response
                .as_ref()
                .and_then(|resp| resp.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(Some(ResponseEventRaw::Created { response_id }))
        }
        "response.output_text.delta" => Ok(event.delta.map(ResponseEventRaw::OutputTextDelta)),
        "response.output_item.done" => Ok(event.item.map(ResponseEventRaw::OutputItemDone)),
        "response.reasoning_text.delta" => {
            if let (Some(delta), Some(content_index)) = (event.delta, event.content_index) {
                return Ok(Some(ResponseEventRaw::ReasoningTextDelta {
                    delta,
                    content_index,
                }));
            }
            Ok(None)
        }
        "response.reasoning_summary_text.delta" => {
            if let (Some(delta), Some(summary_index)) = (event.delta, event.summary_index) {
                return Ok(Some(ResponseEventRaw::ReasoningSummaryTextDelta {
                    delta,
                    summary_index,
                }));
            }
            Ok(None)
        }
        "response.failed" => {
            let Some(resp) = event.response else {
                return Ok(Some(ResponseEventRaw::Failed {
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

            Ok(Some(ResponseEventRaw::Failed { response_id, error }))
        }
        "response.completed" => {
            if let Some(resp_val) = event.response {
                let completed: ResponseCompleted =
                    serde_json::from_value(resp_val).context("parse response.completed payload")?;
                return Ok(Some(ResponseEventRaw::Completed {
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
                return Ok(Some(ResponseEventRaw::Completed {
                    response_id: done.id,
                    usage: done.usage,
                }));
            }
            Ok(Some(ResponseEventRaw::Completed {
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
    tx_event: mpsc::Sender<anyhow::Result<ResponseEventRaw>>,
) where
    R: tokio::io::AsyncBufRead + Unpin,
{
    if let Some(etag) = header_string(&headers, "x-models-etag") {
        let _ = tx_event.send(Ok(ResponseEventRaw::ModelsEtag(etag))).await;
    }
    if let Some(rate_limits) = RateLimits::from_headers(&headers) {
        let _ = tx_event
            .send(Ok(ResponseEventRaw::RateLimits(rate_limits)))
            .await;
    }

    process_sse(lines, tx_event).await;
}

async fn process_sse<R>(
    mut lines: tokio::io::Lines<R>,
    tx_event: mpsc::Sender<anyhow::Result<ResponseEventRaw>>,
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
                                ResponseEventRaw::Completed { .. }
                                    | ResponseEventRaw::Failed { .. }
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

impl crate::Client {
    pub fn responses_compact_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/responses/compact") {
            base.to_string()
        } else if base.ends_with("/responses") {
            format!("{base}/compact")
        } else {
            format!("{base}/responses/compact")
        }
    }

    pub async fn compact_responses_history(
        &self,
        input: &CompactionInput<'_>,
    ) -> anyhow::Result<Vec<Value>> {
        let url = self.responses_compact_url();
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(input)
            .send()
            .await
            .context("send /responses/compact")?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(HttpError { status, body: text }));
        }

        let parsed = response
            .json::<CompactionResponse>()
            .await
            .context("parse /responses/compact json")?;
        Ok(parsed.output)
    }

    pub async fn create_response_stream_raw(
        &self,
        request: &ResponsesApiRequestRaw<'_>,
    ) -> anyhow::Result<ResponseEventStreamRaw> {
        if !request.stream {
            anyhow::bail!("stream=true is required for create_response_stream_raw");
        }

        let url = self.responses_url();
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .header("Accept", "text/event-stream")
            .json(request)
            .send()
            .await
            .context("send /responses (stream)")?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(HttpError { status, body: text }));
        }

        let headers = response.headers().clone();
        let byte_stream = response.bytes_stream().map_err(std::io::Error::other);
        let reader = StreamReader::new(byte_stream);
        let lines = tokio::io::BufReader::new(reader).lines();

        let (tx_event, rx_event) = mpsc::channel::<anyhow::Result<ResponseEventRaw>>(512);
        let task = tokio::spawn(process_response_stream(lines, headers, tx_event));

        Ok(ResponseEventStreamRaw { rx_event, task })
    }
}
