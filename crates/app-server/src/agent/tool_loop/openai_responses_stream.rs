async fn resolve_openai_client_for_provider(
    provider_name: &str,
    provider_cache: &mut std::collections::BTreeMap<String, ProviderRuntime>,
    project_overrides: &ProjectOpenAiOverrides,
    base_url_override: Option<&str>,
    env: &ditto_llm::Env,
) -> anyhow::Result<(ProviderRuntime, Arc<ditto_llm::OpenAI>)> {
    let runtime = match provider_cache.get(provider_name).cloned() {
        Some(runtime) => runtime,
        None => {
            let runtime =
                build_provider_runtime(provider_name, project_overrides, base_url_override, env)
                    .await?;
            provider_cache.insert(provider_name.to_string(), runtime.clone());
            runtime
        }
    };

    let client = runtime.openai_responses_client.clone().ok_or_else(|| {
        anyhow::anyhow!("provider does not have an OpenAI Responses client: {provider_name}")
    })?;
    Ok((runtime, client))
}

async fn run_openai_stream_once(
    client: Arc<ditto_llm::OpenAI>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    request: ditto_llm::providers::openai::OpenAIResponsesRawRequest<'_>,
    max_openai_request_duration: Duration,
) -> Result<OpenAiRawLlmResponse, LlmAttemptFailure> {
    let mut emitted_output = false;

    let inner = async {
        let mut stream = client.create_response_stream_raw(&request).await?;
        let mut response_id = String::new();
        let mut usage: Option<Value> = None;
        let mut output_items = Vec::<Value>::new();
        let mut output_text = String::new();
        let mut reasoning_summary_text = String::new();

        while let Some(event) = stream.recv().await {
            let event = event?;
            match event {
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::Created {
                    response_id: id,
                } => {
                    if response_id.is_empty()
                        && let Some(id) = id.as_deref().filter(|v| !v.trim().is_empty())
                    {
                        response_id = id.to_string();
                    }
                }
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::OutputTextDelta(delta) => {
                    if delta.is_empty() {
                        continue;
                    }
                    emitted_output = true;
                    output_text.push_str(&delta);
                    if emit_deltas {
                        let response_id_snapshot = response_id.clone();
                        thread_rt.emit_notification(
                            "item/delta",
                            &serde_json::json!({
                                "thread_id": thread_id,
                                "turn_id": turn_id,
                                "response_id": response_id_snapshot,
                                "kind": "output_text",
                                "delta": delta,
                            }),
                        );
                    }
                }
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::ReasoningTextDelta(
                    delta,
                ) => {
                    if delta.is_empty() {
                        continue;
                    }
                    emitted_output = true;
                    if emit_deltas {
                        let response_id_snapshot = response_id.clone();
                        thread_rt.emit_notification(
                            "item/delta",
                            &serde_json::json!({
                                "thread_id": thread_id,
                                "turn_id": turn_id,
                                "response_id": response_id_snapshot,
                                "kind": "reasoning_text",
                                "delta": delta,
                            }),
                        );
                    }
                }
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::ReasoningSummaryTextDelta(
                    delta,
                ) => {
                    if !delta.is_empty() {
                        reasoning_summary_text.push_str(&delta);
                    }
                }
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::OutputItemDone(item) => {
                    emitted_output = true;
                    output_items.push(item);
                }
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::Failed { error, .. } => {
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error message");
                    return Err(ditto_llm::DittoError::InvalidResponse(format!(
                        "openai response.failed: {message}"
                    )));
                }
                ditto_llm::providers::openai::OpenAIResponsesRawEvent::Completed {
                    response_id: id,
                    usage: u,
                } => {
                    if response_id.is_empty()
                        && let Some(id) = id.as_deref().filter(|v| !v.trim().is_empty())
                    {
                        response_id = id.to_string();
                    }
                    usage = u;
                    break;
                }
            }
        }

        if response_id.trim().is_empty() {
            response_id = "<unknown>".to_string();
        }

        let reasoning_summary_text =
            resolve_reasoning_summary_text(reasoning_summary_text, &output_items);
        if emit_deltas && !reasoning_summary_text.trim().is_empty() {
            let response_id_snapshot = response_id.clone();
            thread_rt.emit_notification(
                "item/delta",
                &serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "response_id": response_id_snapshot,
                    "kind": "reasoning_summary_text",
                    "delta": reasoning_summary_text,
                }),
            );
        }

        Ok::<_, ditto_llm::DittoError>(OpenAiRawLlmResponse {
            id: response_id,
            output_text,
            output_items,
            usage,
        })
    };

    match tokio::time::timeout(max_openai_request_duration, inner).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(err)) => Err(LlmAttemptFailure {
            error: LlmAttemptError::Ditto(err),
            emitted_output,
        }),
        Err(_) => Err(LlmAttemptFailure {
            error: LlmAttemptError::TimedOut,
            emitted_output,
        }),
    }
}
