async fn run_llm_stream_once(
    llm: Arc<dyn ditto_core::llm_core::model::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_core::contracts::GenerateRequest,
    max_openai_request_duration: Duration,
) -> Result<AgentLlmResponse, LlmAttemptFailure> {
    #[derive(Default)]
    struct StreamDebugCounts {
        text_delta: u64,
        tool_call_start: u64,
        tool_call_delta: u64,
        reasoning_delta: u64,
        usage: u64,
        finish_reason: u64,
    }

    #[derive(Default)]
    struct ToolCallBuffer {
        name: Option<String>,
        arguments: String,
    }

    let mut emitted_output = false;
    let req_for_timeout_fallback = req.clone();

    let inner = async {
        let req_for_generate = req.clone();
        let debug_stream = parse_env_bool("OMNE_DEBUG_LLM_STREAM", false);
        let mut debug_counts = StreamDebugCounts::default();
        let mut debug_seq: u64 = 0;

        let request_summary = if debug_stream {
            let tool_names = req
                .tools
                .as_ref()
                .map(|tools| tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>())
                .unwrap_or_default();

            let mut messages = Vec::<Value>::new();
            for msg in &req.messages {
                let mut text = String::new();
                let mut non_text_counts = serde_json::Map::<String, Value>::new();
                for part in &msg.content {
                    match part {
                        ditto_core::contracts::ContentPart::Text { text: chunk } => text.push_str(chunk),
                        ditto_core::contracts::ContentPart::Image { .. } => {
                            *non_text_counts
                                .entry("image".to_string())
                                .or_insert(Value::Number(0u64.into())) = Value::Number(
                                non_text_counts
                                    .get("image")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .saturating_add(1)
                                    .into(),
                            );
                        }
                        ditto_core::contracts::ContentPart::File { .. } => {
                            *non_text_counts
                                .entry("file".to_string())
                                .or_insert(Value::Number(0u64.into())) = Value::Number(
                                non_text_counts
                                    .get("file")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .saturating_add(1)
                                    .into(),
                            );
                        }
                        ditto_core::contracts::ContentPart::ToolCall { .. } => {
                            *non_text_counts
                                .entry("tool_call".to_string())
                                .or_insert(Value::Number(0u64.into())) = Value::Number(
                                non_text_counts
                                    .get("tool_call")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .saturating_add(1)
                                    .into(),
                            );
                        }
                        ditto_core::contracts::ContentPart::ToolResult { .. } => {
                            *non_text_counts
                                .entry("tool_result".to_string())
                                .or_insert(Value::Number(0u64.into())) = Value::Number(
                                non_text_counts
                                    .get("tool_result")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .saturating_add(1)
                                    .into(),
                            );
                        }
                        ditto_core::contracts::ContentPart::Reasoning { .. } => {
                            *non_text_counts
                                .entry("reasoning".to_string())
                                .or_insert(Value::Number(0u64.into())) = Value::Number(
                                non_text_counts
                                    .get("reasoning")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0)
                                    .saturating_add(1)
                                    .into(),
                            );
                        }
                    }
                }
                let text_redacted = truncate_chars(&omne_core::redact_text(&text), 400);
                messages.push(serde_json::json!({
                    "role": format!("{:?}", msg.role),
                    "text_len": text.len(),
                    "text_preview": text_redacted,
                    "non_text_parts": Value::Object(non_text_counts),
                }));
            }

            Some(serde_json::json!({
                "model": req.model.as_deref().unwrap_or(""),
                "temperature": req.temperature,
                "max_tokens": req.max_tokens,
                "top_p": req.top_p,
                "seed": req.seed,
                "presence_penalty": req.presence_penalty,
                "frequency_penalty": req.frequency_penalty,
                "stop_sequences_count": req.stop_sequences.as_ref().map(|v| v.len()).unwrap_or(0),
                "messages_count": req.messages.len(),
                "messages": messages,
                "tools_count": tool_names.len(),
                "tools_first_10": tool_names.into_iter().take(10).collect::<Vec<_>>(),
                "tool_choice": req
                    .tool_choice
                    .as_ref()
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_default(),
                "provider_options": req
                    .provider_options
                    .as_ref()
                    .map(|value| value.as_value().clone())
                    .unwrap_or(Value::Null),
            }))
        } else {
            None
        };

        let mut debug_file: Option<tokio::fs::File> = if debug_stream {
            let thread_dir = {
                let handle = thread_rt.handle.lock().await;
                handle
                    .log_path()
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
            };
            let dir = thread_dir.join("runtime").join("llm_stream");
            if let Err(err) = tokio::fs::create_dir_all(&dir).await {
                tracing::warn!(
                    thread_id = %thread_id,
                    turn_id = %turn_id,
                    error = %err,
                    "failed to create runtime llm_stream debug dir"
                );
                None
            } else {
                let path = dir.join(format!("{turn_id}.jsonl"));
                match tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .await
                {
                    Ok(mut file) => {
                        use tokio::io::AsyncWriteExt;
                        debug_seq += 1;
                        let line = serde_json::json!({
                            "seq": debug_seq,
                            "type": "attempt_start",
                            "thread_id": thread_id,
                            "turn_id": turn_id,
                        });
                        if let Ok(mut raw) = serde_json::to_string(&line) {
                            raw.push('\n');
                            if let Err(err) = file.write_all(raw.as_bytes()).await {
                                tracing::warn!(
                                    thread_id = %thread_id,
                                    turn_id = %turn_id,
                                    error = %err,
                                    "failed to write llm_stream debug header"
                                );
                            }
                        }
                        if let Some(summary) = request_summary.as_ref() {
                            debug_seq += 1;
                            let line = serde_json::json!({
                                "seq": debug_seq,
                                "type": "request_summary",
                                "summary": summary,
                            });
                            if let Ok(mut raw) = serde_json::to_string(&line) {
                                raw.push('\n');
                                if let Err(err) = file.write_all(raw.as_bytes()).await {
                                    tracing::warn!(
                                        thread_id = %thread_id,
                                        turn_id = %turn_id,
                                        error = %err,
                                        "failed to write llm_stream request_summary"
                                    );
                                }
                            }
                        }

                        // Best-effort: dump an OpenAI-compatible chat/completions body for offline repro.
                        if let Some(summary) = request_summary.as_ref() {
                            let body_path = dir.join(format!("{turn_id}.request_body.json"));
                            let model = summary.get("model").and_then(Value::as_str).unwrap_or("");
                            let mut body_messages = Vec::<Value>::new();
                            for msg in &req.messages {
                                let mut content = String::new();
                                for part in &msg.content {
                                    if let ditto_core::contracts::ContentPart::Text { text } = part {
                                        content.push_str(text);
                                    }
                                }
                                let role = match msg.role {
                                    ditto_core::contracts::Role::System => "system",
                                    ditto_core::contracts::Role::User => "user",
                                    ditto_core::contracts::Role::Assistant => "assistant",
                                    ditto_core::contracts::Role::Tool => "tool",
                                };
                                if content.trim().is_empty() {
                                    continue;
                                }
                                body_messages
                                    .push(serde_json::json!({ "role": role, "content": content }));
                            }

                            let tools = req.tools.as_ref().map(|tools| {
                                tools
                                    .iter()
                                    .map(|t| {
                                        serde_json::json!({
                                            "type": "function",
                                            "function": {
                                                "name": t.name,
                                                "description": t.description,
                                                "parameters": t.parameters,
                                            }
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            });
                            let tool_choice = req.tool_choice.as_ref().map(|choice| match choice {
                                ditto_core::contracts::ToolChoice::Auto => Value::String("auto".to_string()),
                                ditto_core::contracts::ToolChoice::None => Value::String("none".to_string()),
                                ditto_core::contracts::ToolChoice::Required => {
                                    Value::String("required".to_string())
                                }
                                ditto_core::contracts::ToolChoice::Tool { name } => serde_json::json!({
                                    "type": "function",
                                    "function": { "name": name }
                                }),
                            });

                            let mut body = serde_json::Map::<String, Value>::new();
                            body.insert("model".to_string(), Value::String(model.to_string()));
                            body.insert("stream".to_string(), Value::Bool(true));
                            if let Some(user) =
                                req.user.as_deref().map(str::trim).filter(|s| !s.is_empty())
                            {
                                body.insert("user".to_string(), Value::String(user.to_string()));
                            }
                            body.insert("messages".to_string(), Value::Array(body_messages));
                            if let Some(tools) = tools {
                                body.insert("tools".to_string(), Value::Array(tools));
                            }
                            if let Some(tool_choice) = tool_choice {
                                body.insert("tool_choice".to_string(), tool_choice);
                            }
                            if let Some(options) = req
                                .provider_options
                                .as_ref()
                                .and_then(|value| value.as_value().as_object())
                            {
                                if let Some(parallel_tool_calls) =
                                    options.get("parallel_tool_calls").and_then(Value::as_bool)
                                {
                                    body.insert(
                                        "parallel_tool_calls".to_string(),
                                        Value::Bool(parallel_tool_calls),
                                    );
                                }
                                if let Some(response_format) = options.get("response_format") {
                                    body.insert(
                                        "response_format".to_string(),
                                        response_format.clone(),
                                    );
                                }
                                if let Some(reasoning_effort) = options.get("reasoning_effort") {
                                    body.insert(
                                        "reasoning_effort".to_string(),
                                        reasoning_effort.clone(),
                                    );
                                }
                                if let Some(prompt_cache_key) = options.get("prompt_cache_key") {
                                    body.insert(
                                        "prompt_cache_key".to_string(),
                                        prompt_cache_key.clone(),
                                    );
                                }
                            }

                            if let Ok(raw) = serde_json::to_vec_pretty(&Value::Object(body)) {
                                // Ignore write errors; debug-only.
                                let _ = tokio::fs::write(&body_path, raw).await;
                            }
                        }
                        Some(file)
                    }
                    Err(err) => {
                        tracing::warn!(
                            thread_id = %thread_id,
                            turn_id = %turn_id,
                            error = %err,
                            "failed to open runtime llm_stream debug file"
                        );
                        None
                    }
                }
            }
        } else {
            None
        };

        let mut stream = match llm.stream(req).await {
            Ok(stream) => stream,
            Err(err) if llm_stream_error_prefers_generate_fallback(&err) => {
                return fallback_stream_to_generate(
                    llm.clone(),
                    thread_rt.clone(),
                    thread_id,
                    turn_id,
                    emit_deltas,
                    show_thinking,
                    req_for_generate,
                    "stream.empty_response_error",
                    "streaming failed before emitting output; fell back to non-streaming generate after upstream returned an empty-response error"
                        .to_string(),
                )
                .await;
            }
            Err(err) => return Err(err),
        };
        let mut response_id = String::new();
        let mut usage: Option<ditto_core::contracts::Usage> = None;
        let mut output_items = Vec::<OpenAiItem>::new();
        let mut output_text = String::new();
        let mut tool_call_order = Vec::<String>::new();
        let mut tool_calls = std::collections::BTreeMap::<String, ToolCallBuffer>::new();
        let mut seen_tool_call_ids = std::collections::HashSet::<String>::new();
        let mut warnings = Vec::<ditto_core::contracts::Warning>::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) if !emitted_output && llm_stream_error_prefers_generate_fallback(&err) => {
                    return fallback_stream_to_generate(
                        llm.clone(),
                        thread_rt.clone(),
                        thread_id,
                        turn_id,
                        emit_deltas,
                        show_thinking,
                        req_for_generate,
                        "stream.empty_response_error",
                        "streaming failed before emitting output; fell back to non-streaming generate after upstream returned an empty-response error"
                            .to_string(),
                    )
                    .await;
                }
                Err(err) => return Err(err),
            };
            let mut disable_debug_file = false;
            if let Some(file) = debug_file.as_mut() {
                use tokio::io::AsyncWriteExt;
                debug_seq += 1;

                let line = match &chunk {
                    ditto_core::contracts::StreamChunk::Warnings { warnings } => {
                        serde_json::json!({ "seq": debug_seq, "type": "warnings", "count": warnings.len() })
                    }
                    ditto_core::contracts::StreamChunk::ResponseId { id } => {
                        serde_json::json!({ "seq": debug_seq, "type": "response_id", "id": id })
                    }
                    ditto_core::contracts::StreamChunk::TextDelta { text } => {
                        let text = truncate_chars(&omne_core::redact_text(text), 4000);
                        serde_json::json!({ "seq": debug_seq, "type": "text_delta", "len": text.len(), "text": text })
                    }
                    ditto_core::contracts::StreamChunk::ToolCallStart { id, name } => {
                        serde_json::json!({ "seq": debug_seq, "type": "tool_call_start", "id": id, "name": name })
                    }
                    ditto_core::contracts::StreamChunk::ToolCallDelta {
                        id,
                        arguments_delta,
                    } => {
                        let delta = truncate_chars(&omne_core::redact_text(arguments_delta), 4000);
                        serde_json::json!({ "seq": debug_seq, "type": "tool_call_delta", "id": id, "len": delta.len(), "arguments_delta": delta })
                    }
                    ditto_core::contracts::StreamChunk::ReasoningDelta { text } => {
                        serde_json::json!({ "seq": debug_seq, "type": "reasoning_delta", "len": text.len() })
                    }
                    ditto_core::contracts::StreamChunk::FinishReason(reason) => {
                        serde_json::json!({ "seq": debug_seq, "type": "finish_reason", "reason": format!("{reason:?}") })
                    }
                    ditto_core::contracts::StreamChunk::Usage(usage) => {
                        serde_json::json!({ "seq": debug_seq, "type": "usage", "usage": usage })
                    }
                };

                if let Ok(mut raw) = serde_json::to_string(&line) {
                    raw.push('\n');
                    if let Err(err) = file.write_all(raw.as_bytes()).await {
                        tracing::warn!(
                            thread_id = %thread_id,
                            turn_id = %turn_id,
                            error = %err,
                            "failed to write llm_stream debug line"
                        );
                        disable_debug_file = true;
                    }
                }
            }
            if disable_debug_file {
                debug_file = None;
            }

            match chunk {
                ditto_core::contracts::StreamChunk::Warnings { warnings: w } => warnings.extend(w),
                ditto_core::contracts::StreamChunk::ResponseId { id } => {
                    if response_id.is_empty() && !id.trim().is_empty() {
                        response_id = id;
                    }
                }
                ditto_core::contracts::StreamChunk::TextDelta { text } => {
                    if text.is_empty() {
                        continue;
                    }
                    debug_counts.text_delta = debug_counts.text_delta.saturating_add(1);
                    emitted_output = true;
                    output_text.push_str(&text);
                    if emit_deltas {
                        let delta = omne_core::redact_text(&text);
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
                ditto_core::contracts::StreamChunk::ToolCallStart { id, name } => {
                    debug_counts.tool_call_start = debug_counts.tool_call_start.saturating_add(1);
                    emitted_output = true;
                    let slot = tool_calls.entry(id.clone()).or_default();
                    if slot.name.is_none() && !name.trim().is_empty() {
                        slot.name = Some(name);
                    }
                    if seen_tool_call_ids.insert(id.clone()) {
                        tool_call_order.push(id);
                    }
                }
                ditto_core::contracts::StreamChunk::ToolCallDelta {
                    id,
                    arguments_delta,
                } => {
                    debug_counts.tool_call_delta = debug_counts.tool_call_delta.saturating_add(1);
                    emitted_output = true;
                    let slot = tool_calls.entry(id.clone()).or_default();
                    slot.arguments.push_str(&arguments_delta);
                    if seen_tool_call_ids.insert(id.clone()) {
                        tool_call_order.push(id);
                    }
                }
                ditto_core::contracts::StreamChunk::ReasoningDelta { text } => {
                    debug_counts.reasoning_delta = debug_counts.reasoning_delta.saturating_add(1);
                    if emit_deltas && show_thinking && !text.is_empty() {
                        emitted_output = true;
                        let delta = omne_core::redact_text(&text);
                        let response_id_snapshot = response_id.clone();
                        thread_rt.emit_notification(
                            "item/delta",
                            &serde_json::json!({
                                "thread_id": thread_id,
                                "turn_id": turn_id,
                                "response_id": response_id_snapshot,
                                "kind": "thinking",
                                "delta": delta,
                            }),
                        );
                    }
                }
                ditto_core::contracts::StreamChunk::Usage(u) => {
                    debug_counts.usage = debug_counts.usage.saturating_add(1);
                    usage = Some(u);
                }
                ditto_core::contracts::StreamChunk::FinishReason(_) => {
                    debug_counts.finish_reason = debug_counts.finish_reason.saturating_add(1);
                }
            }
        }

        if response_id.trim().is_empty() {
            response_id = "<unknown>".to_string();
        }

        if let Some(file) = debug_file.as_mut() {
            use tokio::io::AsyncWriteExt;
            debug_seq += 1;
            let line = serde_json::json!({
                "seq": debug_seq,
                "type": "attempt_summary",
                "response_id": response_id.clone(),
                "output_text_len": output_text.len(),
                "warnings_count": warnings.len(),
                "counts": {
                    "text_delta": debug_counts.text_delta,
                    "tool_call_start": debug_counts.tool_call_start,
                    "tool_call_delta": debug_counts.tool_call_delta,
                    "reasoning_delta": debug_counts.reasoning_delta,
                    "usage": debug_counts.usage,
                    "finish_reason": debug_counts.finish_reason,
                }
            });
            if let Ok(mut raw) = serde_json::to_string(&line) {
                raw.push('\n');
                let _ = file.write_all(raw.as_bytes()).await;
            }
        }

        if !output_text.is_empty() {
            let output_text = omne_core::redact_text(&output_text);
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

        // Some providers can return an "empty" SSE stream for large requests
        // (finish_reason + [DONE], but no delta.content/tool_calls). This breaks CLI output and
        // history because the tool loop sees no assistant text. Fallback to non-streaming generate.
        if output_items.is_empty() {
            if let Some(file) = debug_file.as_mut() {
                use tokio::io::AsyncWriteExt;
                debug_seq += 1;
                let line = serde_json::json!({
                    "seq": debug_seq,
                    "type": "fallback_generate_start",
                });
                if let Ok(mut raw) = serde_json::to_string(&line) {
                    raw.push('\n');
                    let _ = file.write_all(raw.as_bytes()).await;
                }
            }

            let resp = fallback_stream_to_generate(
                llm.clone(),
                thread_rt.clone(),
                thread_id,
                turn_id,
                emit_deltas,
                show_thinking,
                req_for_generate,
                "stream.empty_output",
                "streaming completed without emitting text/tool deltas; fell back to non-streaming generate"
                    .to_string(),
            )
            .await?;

            if let Some(file) = debug_file.as_mut() {
                use tokio::io::AsyncWriteExt;
                debug_seq += 1;
                let line = serde_json::json!({
                    "seq": debug_seq,
                    "type": "fallback_generate_done",
                    "response_id": resp.id,
                    "output_items_count": resp.output.len(),
                });
                if let Ok(mut raw) = serde_json::to_string(&line) {
                    raw.push('\n');
                    let _ = file.write_all(raw.as_bytes()).await;
                }
            }

            return Ok(resp);
        }

        Ok::<_, ditto_core::error::DittoError>(AgentLlmResponse {
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
        Err(_) if !emitted_output => {
            // YUNWU_GEMINI_STREAM_TIMEOUT_FALLBACK:
            // Some upstreams keep the streaming request open without ever
            // yielding text/tool deltas. If that happens, preserve the turn by
            // doing one non-streaming generate() retry instead of bubbling a
            // timeout after zero observable output.
            fallback_stream_to_generate(
                llm,
                thread_rt,
                thread_id,
                turn_id,
                emit_deltas,
                show_thinking,
                req_for_timeout_fallback,
                "stream.timeout_before_output",
                "streaming timed out before emitting text/tool deltas; fell back to non-streaming generate"
                    .to_string(),
            )
            .await
            .map_err(|err| LlmAttemptFailure {
                error: err.into(),
                emitted_output,
            })
        }
        Err(_) => Err(LlmAttemptFailure {
            error: LlmAttemptError::TimedOut,
            emitted_output,
        }),
    }
}

fn tool_call_arguments_to_openai_string(arguments: &Value) -> String {
    match arguments {
        Value::String(raw) => {
            let raw = raw.trim();
            if raw.is_empty() {
                "{}".to_string()
            } else {
                raw.to_string()
            }
        }
        other => other.to_string(),
    }
}

fn llm_stream_error_prefers_generate_fallback(err: &ditto_core::error::DittoError) -> bool {
    // YUNWU_GEMINI_STREAM_FALLBACK:
    // Omne sees this after the provider-level streaming call fails before
    // producing any deltas. We only trigger fallback for the known
    // "empty_response" family so we don't silently mask real stream failures.
    #[derive(serde::Deserialize)]
    struct ApiErrorEnvelope {
        error: ApiErrorBody,
    }

    #[derive(serde::Deserialize)]
    struct ApiErrorBody {
        #[serde(default)]
        message: String,
        #[serde(default, rename = "type")]
        error_type: String,
        #[serde(default)]
        code: String,
    }

    fn is_empty_response_api_body(raw: &str) -> bool {
        let Ok(parsed) = serde_json::from_str::<ApiErrorEnvelope>(raw) else {
            return false;
        };
        if parsed.error.code.trim() == "channel:empty_response" {
            return true;
        }
        parsed.error.error_type.trim() == "channel_error"
            && parsed
                .error
                .message
                .to_ascii_lowercase()
                .contains("no meaningful content in candidates")
    }

    match err {
        ditto_core::error::DittoError::Api { body, .. } => is_empty_response_api_body(body),
        ditto_core::error::DittoError::InvalidResponse(message) => message
            .freeform_text()
            .is_some_and(is_empty_response_api_body),
        _ => false,
    }
}

async fn fallback_stream_to_generate(
    llm: Arc<dyn ditto_core::llm_core::model::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_core::contracts::GenerateRequest,
    warning_feature: &str,
    warning_details: String,
) -> Result<AgentLlmResponse, ditto_core::error::DittoError> {
    // YUNWU_GEMINI_STREAM_FALLBACK:
    // This is the Omne-side fallback path for reviewer/tool-loop flows. It
    // preserves the turn by converting a stream-path failure with zero emitted
    // output into one non-streaming generate() attempt.
    let mut resp = run_llm_generate_inner(
        llm,
        thread_rt,
        thread_id,
        turn_id,
        emit_deltas,
        show_thinking,
        req,
    )
    .await?;
    resp.warnings.push(ditto_core::contracts::Warning::Compatibility {
        feature: warning_feature.to_string(),
        details: warning_details,
    });
    Ok(resp)
}

fn response_id_from_provider_metadata(metadata: &Option<Value>) -> Option<String> {
    metadata
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("id"))
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn run_llm_generate_inner(
    llm: Arc<dyn ditto_core::llm_core::model::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_core::contracts::GenerateRequest,
) -> Result<AgentLlmResponse, ditto_core::error::DittoError> {
    let resp = llm.generate(req).await?;

    let mut response_id =
        response_id_from_provider_metadata(&resp.provider_metadata).unwrap_or_default();
    if response_id.trim().is_empty() {
        response_id = "<unknown>".to_string();
    }

    let mut output_text = String::new();
    let mut reasoning_text = String::new();
    let mut tool_calls = Vec::<(String, String, String)>::new();

    for part in &resp.content {
        match part {
            ditto_core::contracts::ContentPart::Text { text } => output_text.push_str(text),
            ditto_core::contracts::ContentPart::Reasoning { text } => reasoning_text.push_str(text),
            ditto_core::contracts::ContentPart::ToolCall {
                id,
                name,
                arguments,
            } => {
                tool_calls.push((
                    id.to_string(),
                    name.to_string(),
                    tool_call_arguments_to_openai_string(arguments),
                ));
            }
            _ => {}
        }
    }

    let mut output_items = Vec::<OpenAiItem>::new();
    let mut assistant_content = Vec::<Value>::new();
    if !reasoning_text.is_empty() {
        let reasoning_text = omne_core::redact_text(&reasoning_text);
        assistant_content.push(serde_json::json!({
            "type": "reasoning_text",
            "text": reasoning_text.clone(),
        }));

        if emit_deltas && show_thinking {
            thread_rt.emit_notification(
                "item/delta",
                &serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "response_id": response_id.clone(),
                    "kind": "thinking",
                    "delta": reasoning_text,
                }),
            );
        }
    }
    if !output_text.is_empty() {
        let output_text = omne_core::redact_text(&output_text);
        assistant_content.push(serde_json::json!({
            "type": "output_text",
            "text": output_text.clone(),
        }));

        if emit_deltas {
            thread_rt.emit_notification(
                "item/delta",
                &serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "response_id": response_id.clone(),
                    "kind": "output_text",
                    "delta": output_text,
                }),
            );
        }
    }
    if !assistant_content.is_empty() {
        output_items.push(serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": assistant_content,
        }));
    }

    for (call_id, name, arguments) in tool_calls {
        output_items.push(serde_json::json!({
            "type": "function_call",
            "name": name,
            "arguments": arguments,
            "call_id": call_id,
        }));
    }

    Ok(AgentLlmResponse {
        id: response_id,
        output: output_items,
        usage: token_usage_json_from_ditto_usage(&resp.usage),
        warnings: resp.warnings,
    })
}

async fn run_llm_generate_once(
    llm: Arc<dyn ditto_core::llm_core::model::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_core::contracts::GenerateRequest,
    max_openai_request_duration: Duration,
) -> Result<AgentLlmResponse, LlmAttemptFailure> {
    let mut emitted_output = false;
    let inner = async {
        let resp = run_llm_generate_inner(
            llm.clone(),
            thread_rt.clone(),
            thread_id,
            turn_id,
            emit_deltas,
            show_thinking,
            req,
        )
        .await?;
        emitted_output = !resp.output.is_empty();
        Ok::<_, ditto_core::error::DittoError>(resp)
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

#[cfg(test)]
mod llm_stream_tests {
    use super::*;
    use async_trait::async_trait;
    use futures_util::stream;

    #[derive(Clone)]
    struct EmptyStreamThenGenerate;

    #[async_trait]
    impl ditto_core::llm_core::model::LanguageModel for EmptyStreamThenGenerate {
        fn provider(&self) -> &str {
            "openai-compatible"
        }

        fn model_id(&self) -> &str {
            "stub"
        }

        async fn generate(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::contracts::GenerateResponse> {
            Ok(ditto_core::contracts::GenerateResponse {
                content: vec![ditto_core::contracts::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                usage: ditto_core::contracts::Usage {
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    total_tokens: Some(2),
                    ..Default::default()
                },
                provider_metadata: Some(serde_json::json!({ "id": "resp_gen_1" })),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::llm_core::model::StreamResult> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ditto_core::contracts::StreamChunk::ResponseId {
                    id: "resp_stream_1".to_string(),
                }),
                Ok(ditto_core::contracts::StreamChunk::FinishReason(
                    ditto_core::contracts::FinishReason::Stop,
                )),
            ])))
        }
    }

    #[tokio::test]
    async fn empty_openai_compatible_stream_falls_back_to_generate() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store =
            omne_core::ThreadStore::new(omne_core::PmPaths::new(dir.path().join(".omne_data")));
        let handle = store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        let (notify_tx, _notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let req = ditto_core::contracts::GenerateRequest::from(vec![ditto_core::contracts::Message::user("ping")]);
        let resp = run_llm_stream_once(
            Arc::new(EmptyStreamThenGenerate),
            thread_rt,
            thread_id,
            TurnId::new(),
            false,
            false,
            req,
            Duration::from_secs(5),
        )
        .await
        .map_err(|err| anyhow::anyhow!("llm attempt failed: {:?}", err.error))?;

        assert_eq!(resp.id, "resp_gen_1");
        assert_eq!(extract_assistant_text(&resp.output), "OK");
        assert!(
            resp.warnings
                .iter()
                .any(|w| matches!(w, ditto_core::contracts::Warning::Compatibility { feature, .. } if feature == "stream.empty_output"))
        );

        Ok(())
    }

    #[derive(Clone)]
    struct EmptyResponseStreamThenGenerate;

    #[async_trait]
    impl ditto_core::llm_core::model::LanguageModel for EmptyResponseStreamThenGenerate {
        fn provider(&self) -> &str {
            "google"
        }

        fn model_id(&self) -> &str {
            "gemini-3.1-pro-preview"
        }

        async fn generate(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::contracts::GenerateResponse> {
            Ok(ditto_core::contracts::GenerateResponse {
                content: vec![ditto_core::contracts::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                provider_metadata: Some(serde_json::json!({ "id": "resp_gen_429" })),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::llm_core::model::StreamResult> {
            Err(ditto_core::error::DittoError::Api {
                status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                body: "{\"error\":{\"message\":\"received empty response from Gemini: no meaningful content in candidates\",\"code\":\"channel:empty_response\"}}".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn empty_response_stream_error_falls_back_to_generate() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store =
            omne_core::ThreadStore::new(omne_core::PmPaths::new(dir.path().join(".omne_data")));
        let handle = store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        let (notify_tx, _notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let req = ditto_core::contracts::GenerateRequest::from(vec![ditto_core::contracts::Message::user("ping")]);
        let resp = run_llm_stream_once(
            Arc::new(EmptyResponseStreamThenGenerate),
            thread_rt,
            thread_id,
            TurnId::new(),
            false,
            false,
            req,
            Duration::from_secs(5),
        )
        .await
        .map_err(|err| anyhow::anyhow!("llm attempt failed: {:?}", err.error))?;

        assert_eq!(resp.id, "resp_gen_429");
        assert_eq!(extract_assistant_text(&resp.output), "OK");
        assert!(resp.warnings.iter().any(|w| matches!(
            w,
            ditto_core::contracts::Warning::Compatibility { feature, .. }
                if feature == "stream.empty_response_error"
        )));

        Ok(())
    }

    #[derive(Clone)]
    struct TimeoutBeforeOutputStreamThenGenerate;

    #[async_trait]
    impl ditto_core::llm_core::model::LanguageModel for TimeoutBeforeOutputStreamThenGenerate {
        fn provider(&self) -> &str {
            "google"
        }

        fn model_id(&self) -> &str {
            "gemini-3.1-pro-preview"
        }

        async fn generate(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::contracts::GenerateResponse> {
            Ok(ditto_core::contracts::GenerateResponse {
                content: vec![ditto_core::contracts::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                provider_metadata: Some(serde_json::json!({ "id": "resp_gen_timeout" })),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::llm_core::model::StreamResult> {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(Box::pin(stream::pending()))
        }
    }

    #[tokio::test]
    async fn timeout_before_output_stream_falls_back_to_generate() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store =
            omne_core::ThreadStore::new(omne_core::PmPaths::new(dir.path().join(".omne_data")));
        let handle = store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        let (notify_tx, _notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let req = ditto_core::contracts::GenerateRequest::from(vec![ditto_core::contracts::Message::user("ping")]);
        let resp = run_llm_stream_once(
            Arc::new(TimeoutBeforeOutputStreamThenGenerate),
            thread_rt,
            thread_id,
            TurnId::new(),
            false,
            false,
            req,
            Duration::from_millis(10),
        )
        .await
        .map_err(|err| anyhow::anyhow!("llm attempt failed: {:?}", err.error))?;

        assert_eq!(resp.id, "resp_gen_timeout");
        assert_eq!(extract_assistant_text(&resp.output), "OK");
        assert!(resp.warnings.iter().any(|w| matches!(
            w,
            ditto_core::contracts::Warning::Compatibility { feature, .. }
                if feature == "stream.timeout_before_output"
        )));

        Ok(())
    }

    #[derive(Clone)]
    struct CaptureGenerateRequestOnFallback {
        generated_request: std::sync::Arc<std::sync::Mutex<Option<ditto_core::contracts::GenerateRequest>>>,
    }

    #[async_trait]
    impl ditto_core::llm_core::model::LanguageModel for CaptureGenerateRequestOnFallback {
        fn provider(&self) -> &str {
            "openai-compatible"
        }

        fn model_id(&self) -> &str {
            "stub"
        }

        async fn generate(
            &self,
            request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::contracts::GenerateResponse> {
            *self.generated_request.lock().expect("mutex poisoned") = Some(request);
            Ok(ditto_core::contracts::GenerateResponse {
                content: vec![ditto_core::contracts::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                provider_metadata: Some(serde_json::json!({ "id": "resp_gen_1" })),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::llm_core::model::StreamResult> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ditto_core::contracts::StreamChunk::ResponseId {
                    id: "resp_stream_1".to_string(),
                }),
                Ok(ditto_core::contracts::StreamChunk::FinishReason(
                    ditto_core::contracts::FinishReason::Stop,
                )),
            ])))
        }
    }

    #[tokio::test]
    async fn fallback_generate_keeps_prompt_cache_key() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store =
            omne_core::ThreadStore::new(omne_core::PmPaths::new(dir.path().join(".omne_data")));
        let handle = store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        let (notify_tx, _notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let llm = CaptureGenerateRequestOnFallback {
            generated_request: captured.clone(),
        };

        let req = ditto_core::provider_options::request_with_provider_options(
            ditto_core::contracts::GenerateRequest::from(vec![ditto_core::contracts::Message::user("ping")]),
            ditto_core::provider_options::ProviderOptions {
                prompt_cache_key: Some(thread_id.to_string()),
                ..Default::default()
            },
        )?;

        let _resp = run_llm_stream_once(
            Arc::new(llm),
            thread_rt,
            thread_id,
            TurnId::new(),
            false,
            false,
            req,
            Duration::from_secs(5),
        )
        .await
        .map_err(|err| anyhow::anyhow!("llm attempt failed: {:?}", err.error))?;

        let generated_req = captured
            .lock()
            .expect("mutex poisoned")
            .clone()
            .expect("fallback generate should be called");
        let parsed =
            ditto_core::provider_options::request_parsed_provider_options_for(&generated_req, "openai-compatible")?
                .expect("provider options should exist");
        let expected = thread_id.to_string();
        assert_eq!(parsed.prompt_cache_key.as_deref(), Some(expected.as_str()));

        Ok(())
    }

    #[derive(Clone)]
    struct ReasoningStreamEmitsDeltas;

    #[async_trait]
    impl ditto_core::llm_core::model::LanguageModel for ReasoningStreamEmitsDeltas {
        fn provider(&self) -> &str {
            "openai-compatible"
        }

        fn model_id(&self) -> &str {
            "stub"
        }

        async fn generate(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::contracts::GenerateResponse> {
            Ok(ditto_core::contracts::GenerateResponse {
                content: vec![ditto_core::contracts::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                provider_metadata: Some(serde_json::json!({ "id": "resp_gen_1" })),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::llm_core::model::StreamResult> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ditto_core::contracts::StreamChunk::ResponseId {
                    id: "resp_stream_1".to_string(),
                }),
                Ok(ditto_core::contracts::StreamChunk::ReasoningDelta {
                    text: "thinking...".to_string(),
                }),
                Ok(ditto_core::contracts::StreamChunk::TextDelta {
                    text: "OK".to_string(),
                }),
                Ok(ditto_core::contracts::StreamChunk::FinishReason(
                    ditto_core::contracts::FinishReason::Stop,
                )),
            ])))
        }
    }

    #[tokio::test]
    async fn reasoning_delta_emits_item_delta_thinking_when_enabled() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store =
            omne_core::ThreadStore::new(omne_core::PmPaths::new(dir.path().join(".omne_data")));
        let handle = store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        let (notify_tx, mut notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let turn_id = TurnId::new();
        let req = ditto_core::contracts::GenerateRequest::from(vec![ditto_core::contracts::Message::user("ping")]);
        let resp = run_llm_stream_once(
            Arc::new(ReasoningStreamEmitsDeltas),
            thread_rt,
            thread_id,
            turn_id,
            true,
            true,
            req,
            Duration::from_secs(5),
        )
        .await
        .map_err(|err| anyhow::anyhow!("llm attempt failed: {:?}", err.error))?;

        assert_eq!(resp.id, "resp_stream_1");
        assert_eq!(extract_assistant_text(&resp.output), "OK");

        let thread_id_str = thread_id.to_string();
        let turn_id_str = turn_id.to_string();
        let mut saw_thinking = false;
        let mut saw_output = false;
        while let Ok(line) = notify_rx.try_recv() {
            let value: Value = serde_json::from_str(&line)?;
            if value.get("method").and_then(Value::as_str) != Some("item/delta") {
                continue;
            }
            let Some(params) = value.get("params").and_then(Value::as_object) else {
                continue;
            };
            if params.get("thread_id").and_then(Value::as_str) != Some(thread_id_str.as_str()) {
                continue;
            }
            if params.get("turn_id").and_then(Value::as_str) != Some(turn_id_str.as_str()) {
                continue;
            }
            let kind = params.get("kind").and_then(Value::as_str).unwrap_or("");
            let delta = params.get("delta").and_then(Value::as_str).unwrap_or("");
            if kind == "thinking" && delta.contains("thinking") {
                saw_thinking = true;
            }
            if kind == "output_text" && delta.contains("OK") {
                saw_output = true;
            }
        }

        assert!(saw_thinking, "expected item/delta kind=thinking");
        assert!(saw_output, "expected item/delta kind=output_text");

        Ok(())
    }
}
