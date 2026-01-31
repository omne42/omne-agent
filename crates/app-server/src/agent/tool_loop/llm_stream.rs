async fn run_llm_stream_once(
    llm: Arc<dyn ditto_llm::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    req: ditto_llm::GenerateRequest,
    max_openai_request_duration: Duration,
) -> Result<AgentLlmResponse, LlmAttemptFailure> {
    #[derive(Default)]
    struct ToolCallBuffer {
        name: Option<String>,
        arguments: String,
    }

    let mut emitted_output = false;

    let inner = async {
        let mut stream = llm.stream(req).await?;
        let mut response_id = String::new();
        let mut usage: Option<ditto_llm::Usage> = None;
        let mut output_items = Vec::<OpenAiItem>::new();
        let mut output_text = String::new();
        let mut tool_call_order = Vec::<String>::new();
        let mut tool_calls = std::collections::BTreeMap::<String, ToolCallBuffer>::new();
        let mut seen_tool_call_ids = std::collections::HashSet::<String>::new();
        let mut warnings = Vec::<ditto_llm::Warning>::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            match chunk {
                ditto_llm::StreamChunk::Warnings { warnings: w } => warnings.extend(w),
                ditto_llm::StreamChunk::ResponseId { id } => {
                    if response_id.is_empty() && !id.trim().is_empty() {
                        response_id = id;
                    }
                }
                ditto_llm::StreamChunk::TextDelta { text } => {
                    if text.is_empty() {
                        continue;
                    }
                    emitted_output = true;
                    output_text.push_str(&text);
                    if emit_deltas {
                        let delta = pm_core::redact_text(&text);
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
                ditto_llm::StreamChunk::ToolCallStart { id, name } => {
                    emitted_output = true;
                    let slot = tool_calls.entry(id.clone()).or_default();
                    if slot.name.is_none() && !name.trim().is_empty() {
                        slot.name = Some(name);
                    }
                    if seen_tool_call_ids.insert(id.clone()) {
                        tool_call_order.push(id);
                    }
                }
                ditto_llm::StreamChunk::ToolCallDelta {
                    id,
                    arguments_delta,
                } => {
                    emitted_output = true;
                    let slot = tool_calls.entry(id.clone()).or_default();
                    slot.arguments.push_str(&arguments_delta);
                    if seen_tool_call_ids.insert(id.clone()) {
                        tool_call_order.push(id);
                    }
                }
                ditto_llm::StreamChunk::ReasoningDelta { text } => {
                    if text.is_empty() {
                        continue;
                    }
                    emitted_output = true;
                    if emit_deltas {
                        let delta = pm_core::redact_text(&text);
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
                ditto_llm::StreamChunk::Usage(u) => usage = Some(u),
                ditto_llm::StreamChunk::FinishReason(_) => {}
            }
        }

        if response_id.trim().is_empty() {
            response_id = "<unknown>".to_string();
        }

        if !output_text.is_empty() {
            let output_text = pm_core::redact_text(&output_text);
            output_items.push(serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": output_text }],
            }));
        }

        for id in tool_call_order {
            let Some(call) = tool_calls.get(&id) else {
                continue;
            };
            let Some(name) = call.name.as_deref().filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            let args = if call.arguments.trim().is_empty() {
                "{}".to_string()
            } else {
                call.arguments.clone()
            };
            output_items.push(serde_json::json!({
                "type": "function_call",
                "name": name,
                "arguments": args,
                "call_id": id,
            }));
        }

        Ok::<_, ditto_llm::DittoError>(AgentLlmResponse {
            id: response_id,
            output: output_items,
            usage: usage.as_ref().and_then(token_usage_json_from_ditto_usage),
            warnings,
        })
    };

    match tokio::time::timeout(max_openai_request_duration, inner).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(err)) => Err(LlmAttemptFailure {
            error: err.into(),
            emitted_output,
        }),
        Err(_) => Err(LlmAttemptFailure {
            error: LlmAttemptError::TimedOut,
            emitted_output,
        }),
    }
}
