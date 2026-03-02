async fn run_llm_stream_once(
    llm: Arc<dyn ditto_llm::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_llm::GenerateRequest,
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
                        ditto_llm::ContentPart::Text { text: chunk } => text.push_str(chunk),
                        ditto_llm::ContentPart::Image { .. } => {
                            *non_text_counts
                                .entry("image".to_string())
                                .or_insert(Value::Number(0u64.into())) =
                                Value::Number(
                                    non_text_counts
                                        .get("image")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0)
                                        .saturating_add(1)
                                        .into(),
                                );
                        }
                        ditto_llm::ContentPart::File { .. } => {
                            *non_text_counts
                                .entry("file".to_string())
                                .or_insert(Value::Number(0u64.into())) =
                                Value::Number(
                                    non_text_counts
                                        .get("file")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0)
                                        .saturating_add(1)
                                        .into(),
                                );
                        }
                        ditto_llm::ContentPart::ToolCall { .. } => {
                            *non_text_counts
                                .entry("tool_call".to_string())
                                .or_insert(Value::Number(0u64.into())) =
                                Value::Number(
                                    non_text_counts
                                        .get("tool_call")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0)
                                        .saturating_add(1)
                                        .into(),
                                );
                        }
                        ditto_llm::ContentPart::ToolResult { .. } => {
                            *non_text_counts
                                .entry("tool_result".to_string())
                                .or_insert(Value::Number(0u64.into())) =
                                Value::Number(
                                    non_text_counts
                                        .get("tool_result")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0)
                                        .saturating_add(1)
                                        .into(),
                                );
                        }
                        ditto_llm::ContentPart::Reasoning { .. } => {
                            *non_text_counts
                                .entry("reasoning".to_string())
                                .or_insert(Value::Number(0u64.into())) =
                                Value::Number(
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
                "provider_options": req.provider_options.clone().unwrap_or(Value::Null),
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
                                    if let ditto_llm::ContentPart::Text { text } = part {
                                        content.push_str(text);
                                    }
                                }
                                let role = match msg.role {
                                    ditto_llm::Role::System => "system",
                                    ditto_llm::Role::User => "user",
                                    ditto_llm::Role::Assistant => "assistant",
                                    ditto_llm::Role::Tool => "tool",
                                };
                                if content.trim().is_empty() {
                                    continue;
                                }
                                body_messages.push(serde_json::json!({ "role": role, "content": content }));
                            }

                            let tools = req.tools.as_ref().map(|tools| {
                                tools.iter().map(|t| {
                                    serde_json::json!({
                                        "type": "function",
                                        "function": {
                                            "name": t.name,
                                            "description": t.description,
                                            "parameters": t.parameters,
                                        }
                                    })
                                }).collect::<Vec<_>>()
                            });
                            let tool_choice = req.tool_choice.as_ref().map(|choice| match choice {
                                ditto_llm::ToolChoice::Auto => Value::String("auto".to_string()),
                                ditto_llm::ToolChoice::None => Value::String("none".to_string()),
                                ditto_llm::ToolChoice::Required => Value::String("required".to_string()),
                                ditto_llm::ToolChoice::Tool { name } => serde_json::json!({
                                    "type": "function",
                                    "function": { "name": name }
                                }),
                            });

                            let mut body = serde_json::Map::<String, Value>::new();
                            body.insert("model".to_string(), Value::String(model.to_string()));
                            body.insert("stream".to_string(), Value::Bool(true));
                            body.insert("messages".to_string(), Value::Array(body_messages));
                            if let Some(tools) = tools {
                                body.insert("tools".to_string(), Value::Array(tools));
                            }
                            if let Some(tool_choice) = tool_choice {
                                body.insert("tool_choice".to_string(), tool_choice);
                            }
                            if let Some(options) = req.provider_options.as_ref().and_then(Value::as_object) {
                                if let Some(parallel_tool_calls) =
                                    options.get("parallel_tool_calls").and_then(Value::as_bool)
                                {
                                    body.insert(
                                        "parallel_tool_calls".to_string(),
                                        Value::Bool(parallel_tool_calls),
                                    );
                                }
                                if let Some(response_format) = options.get("response_format") {
                                    body.insert("response_format".to_string(), response_format.clone());
                                }
                                if let Some(reasoning_effort) = options.get("reasoning_effort") {
                                    body.insert("reasoning_effort".to_string(), reasoning_effort.clone());
                                }
                                if let Some(prompt_cache_key) = options.get("prompt_cache_key") {
                                    body.insert("prompt_cache_key".to_string(), prompt_cache_key.clone());
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
            let mut disable_debug_file = false;
            if let Some(file) = debug_file.as_mut() {
                use tokio::io::AsyncWriteExt;
                debug_seq += 1;

                let line = match &chunk {
                    ditto_llm::StreamChunk::Warnings { warnings } => {
                        serde_json::json!({ "seq": debug_seq, "type": "warnings", "count": warnings.len() })
                    }
                    ditto_llm::StreamChunk::ResponseId { id } => {
                        serde_json::json!({ "seq": debug_seq, "type": "response_id", "id": id })
                    }
                    ditto_llm::StreamChunk::TextDelta { text } => {
                        let text = truncate_chars(&omne_core::redact_text(text), 4000);
                        serde_json::json!({ "seq": debug_seq, "type": "text_delta", "len": text.len(), "text": text })
                    }
                    ditto_llm::StreamChunk::ToolCallStart { id, name } => {
                        serde_json::json!({ "seq": debug_seq, "type": "tool_call_start", "id": id, "name": name })
                    }
                    ditto_llm::StreamChunk::ToolCallDelta { id, arguments_delta } => {
                        let delta = truncate_chars(&omne_core::redact_text(arguments_delta), 4000);
                        serde_json::json!({ "seq": debug_seq, "type": "tool_call_delta", "id": id, "len": delta.len(), "arguments_delta": delta })
                    }
                    ditto_llm::StreamChunk::ReasoningDelta { text } => {
                        serde_json::json!({ "seq": debug_seq, "type": "reasoning_delta", "len": text.len() })
                    }
                    ditto_llm::StreamChunk::FinishReason(reason) => {
                        serde_json::json!({ "seq": debug_seq, "type": "finish_reason", "reason": format!("{reason:?}") })
                    }
                    ditto_llm::StreamChunk::Usage(usage) => {
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
                ditto_llm::StreamChunk::ToolCallStart { id, name } => {
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
                ditto_llm::StreamChunk::ToolCallDelta {
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
                ditto_llm::StreamChunk::ReasoningDelta { text } => {
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
                ditto_llm::StreamChunk::Usage(u) => {
                    debug_counts.usage = debug_counts.usage.saturating_add(1);
                    usage = Some(u);
                }
                ditto_llm::StreamChunk::FinishReason(_) => {
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

        // Some OpenAI-compatible providers can return an "empty" SSE stream for large requests
        // (finish_reason + [DONE], but no delta.content/tool_calls). This breaks CLI output and
        // history because the tool loop sees no assistant text. Fallback to non-streaming generate.
        if llm.provider() == "openai-compatible" && output_items.is_empty() {
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

            let mut resp = run_llm_generate_inner(
                llm.clone(),
                thread_rt.clone(),
                thread_id,
                turn_id,
                emit_deltas,
                show_thinking,
                req_for_generate,
            )
            .await?;
            resp.warnings.push(ditto_llm::Warning::Compatibility {
                feature: "stream.empty_output".to_string(),
                details: "streaming completed without emitting text/tool deltas; fell back to non-streaming generate".to_string(),
            });

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
    llm: Arc<dyn ditto_llm::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_llm::GenerateRequest,
) -> Result<AgentLlmResponse, ditto_llm::DittoError> {
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
            ditto_llm::ContentPart::Text { text } => output_text.push_str(text),
            ditto_llm::ContentPart::Reasoning { text } => reasoning_text.push_str(text),
            ditto_llm::ContentPart::ToolCall { id, name, arguments } => {
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
    if !reasoning_text.is_empty() && emit_deltas && show_thinking {
        let delta = omne_core::redact_text(&reasoning_text);
        thread_rt.emit_notification(
            "item/delta",
            &serde_json::json!({
                "thread_id": thread_id,
                "turn_id": turn_id,
                "response_id": response_id.clone(),
                "kind": "thinking",
                "delta": delta,
            }),
        );
    }
    if !output_text.is_empty() {
        let output_text = omne_core::redact_text(&output_text);
        output_items.push(serde_json::json!({
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": output_text }],
        }));

        if emit_deltas {
            let delta = omne_core::redact_text(&output_text);
            thread_rt.emit_notification(
                "item/delta",
                &serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "response_id": response_id.clone(),
                    "kind": "output_text",
                    "delta": delta,
                }),
            );
        }
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
    llm: Arc<dyn ditto_llm::LanguageModel>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    emit_deltas: bool,
    show_thinking: bool,
    req: ditto_llm::GenerateRequest,
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
        Ok::<_, ditto_llm::DittoError>(resp)
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
    impl ditto_llm::LanguageModel for EmptyStreamThenGenerate {
        fn provider(&self) -> &str {
            "openai-compatible"
        }

        fn model_id(&self) -> &str {
            "stub"
        }

        async fn generate(
            &self,
            _request: ditto_llm::GenerateRequest,
        ) -> ditto_llm::Result<ditto_llm::GenerateResponse> {
            Ok(ditto_llm::GenerateResponse {
                content: vec![ditto_llm::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                usage: ditto_llm::Usage {
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
            _request: ditto_llm::GenerateRequest,
        ) -> ditto_llm::Result<ditto_llm::StreamResult> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ditto_llm::StreamChunk::ResponseId {
                    id: "resp_stream_1".to_string(),
                }),
                Ok(ditto_llm::StreamChunk::FinishReason(ditto_llm::FinishReason::Stop)),
            ])))
        }
    }

    #[tokio::test]
    async fn empty_openai_compatible_stream_falls_back_to_generate() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = omne_core::ThreadStore::new(omne_core::PmPaths::new(
            dir.path().join(".omne_data"),
        ));
        let handle = store.create_thread(std::path::PathBuf::from("/tmp")).await?;
        let thread_id = handle.thread_id();
        let (notify_tx, _notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let req = ditto_llm::GenerateRequest::from(vec![ditto_llm::Message::user("ping")]);
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
                .any(|w| matches!(w, ditto_llm::Warning::Compatibility { feature, .. } if feature == "stream.empty_output"))
        );

        Ok(())
    }

    #[derive(Clone)]
    struct ReasoningStreamEmitsDeltas;

    #[async_trait]
    impl ditto_llm::LanguageModel for ReasoningStreamEmitsDeltas {
        fn provider(&self) -> &str {
            "openai-compatible"
        }

        fn model_id(&self) -> &str {
            "stub"
        }

        async fn generate(
            &self,
            _request: ditto_llm::GenerateRequest,
        ) -> ditto_llm::Result<ditto_llm::GenerateResponse> {
            Ok(ditto_llm::GenerateResponse {
                content: vec![ditto_llm::ContentPart::Text {
                    text: "OK".to_string(),
                }],
                provider_metadata: Some(serde_json::json!({ "id": "resp_gen_1" })),
                ..Default::default()
            })
        }

        async fn stream(
            &self,
            _request: ditto_llm::GenerateRequest,
        ) -> ditto_llm::Result<ditto_llm::StreamResult> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ditto_llm::StreamChunk::ResponseId {
                    id: "resp_stream_1".to_string(),
                }),
                Ok(ditto_llm::StreamChunk::ReasoningDelta {
                    text: "thinking...".to_string(),
                }),
                Ok(ditto_llm::StreamChunk::TextDelta {
                    text: "OK".to_string(),
                }),
                Ok(ditto_llm::StreamChunk::FinishReason(ditto_llm::FinishReason::Stop)),
            ])))
        }
    }

    #[tokio::test]
    async fn reasoning_delta_emits_item_delta_thinking_when_enabled() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let store = omne_core::ThreadStore::new(omne_core::PmPaths::new(
            dir.path().join(".omne_data"),
        ));
        let handle = store.create_thread(std::path::PathBuf::from("/tmp")).await?;
        let thread_id = handle.thread_id();
        let (notify_tx, mut notify_rx) = tokio::sync::broadcast::channel(16);
        let thread_rt = Arc::new(crate::ThreadRuntime::new(handle, notify_tx));

        let turn_id = TurnId::new();
        let req = ditto_llm::GenerateRequest::from(vec![ditto_llm::Message::user("ping")]);
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
