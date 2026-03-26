#[allow(clippy::too_many_arguments)]
async fn run_openai_responses_codex_parity_loop(
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    cancel: CancellationToken,
    turn_priority: TurnPriority,
    final_model: String,
    completion_provider_candidates: Vec<ProviderRouteTarget>,
    thinking_provider_candidates: Vec<ProviderRouteTarget>,
    mut provider_cache: std::collections::BTreeMap<String, ProviderRuntime>,
    model_configs: std::collections::BTreeMap<String, ditto_core::config::ModelConfig>,
    env: ditto_core::config::Env,
    tools: Vec<ditto_core::contracts::Tool>,
    instructions: String,
    turn_input: String,
    seed_input_items: Vec<OpenAiItem>,
    tool_model: Option<String>,
    completion_model_fallbacks: Vec<String>,
    thinking_model_fallbacks: Vec<String>,
    resolved_attachments: Vec<ResolvedAttachment>,
    pdf_file_id_upload_min_bytes: u64,
    rule_source: omne_protocol::ModelRoutingRuleSource,
    rule_id: Option<String>,
    cfg: ToolLoopConfig,
) -> anyhow::Result<ToolLoopOutcome> {
    fn content_part_to_openai_user_item(part: &ditto_core::contracts::ContentPart) -> Option<Value> {
        match part {
            ditto_core::contracts::ContentPart::Text { text } => {
                if text.is_empty() {
                    return None;
                }
                Some(serde_json::json!({ "type": "input_text", "text": text }))
            }
            ditto_core::contracts::ContentPart::Image { source } => {
                let image_url = match source {
                    ditto_core::contracts::ImageSource::Url { url } => url.clone(),
                    ditto_core::contracts::ImageSource::Base64 { media_type, data } => {
                        format!("data:{media_type};base64,{data}")
                    }
                };
                Some(serde_json::json!({ "type": "input_image", "image_url": image_url }))
            }
            ditto_core::contracts::ContentPart::File {
                filename,
                media_type,
                source,
            } => {
                if media_type != "application/pdf" {
                    return None;
                }

                let item = match source {
                    ditto_core::contracts::FileSource::Url { url } => {
                        serde_json::json!({ "type": "input_file", "file_url": url })
                    }
                    ditto_core::contracts::FileSource::Base64 { data } => serde_json::json!({
                        "type": "input_file",
                        "filename": filename.clone().unwrap_or_else(|| "file.pdf".to_string()),
                        "file_data": format!("data:{media_type};base64,{data}"),
                    }),
                    ditto_core::contracts::FileSource::FileId { file_id } => {
                        serde_json::json!({ "type": "input_file", "file_id": file_id })
                    }
                };
                Some(item)
            }
            _ => None,
        }
    }

    fn build_user_message_item(
        text: &str,
        attachment_parts: &[ditto_core::contracts::ContentPart],
    ) -> Option<Value> {
        let mut content = Vec::<Value>::new();
        if !text.trim().is_empty() {
            content.push(serde_json::json!({ "type": "input_text", "text": text }));
        }
        for part in attachment_parts {
            if let Some(item) = content_part_to_openai_user_item(part) {
                content.push(item);
            }
        }
        if content.is_empty() {
            return None;
        }
        Some(serde_json::json!({
            "type": "message",
            "role": "user",
            "content": content,
        }))
    }

    fn append_attachments_to_last_user_message(
        history: &mut [Value],
        attachment_parts: &[ditto_core::contracts::ContentPart],
    ) -> bool {
        if attachment_parts.is_empty() {
            return false;
        }

        let Some(last_user_idx) = history.iter().rposition(|item| {
            item.get("type").and_then(Value::as_str) == Some("message")
                && item.get("role").and_then(Value::as_str) == Some("user")
        }) else {
            return false;
        };

        let Some(obj) = history[last_user_idx].as_object_mut() else {
            return false;
        };
        let Some(content) = obj.get_mut("content").and_then(Value::as_array_mut) else {
            return false;
        };
        let mut added = false;
        for part in attachment_parts {
            if let Some(item) = content_part_to_openai_user_item(part) {
                content.push(item);
                added = true;
            }
        }
        added
    }

    fn parse_function_call_item(item: &Value) -> Option<(String, String, String)> {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return None;
        }
        let call_id = item.get("call_id").and_then(Value::as_str)?;
        let name = item.get("name").and_then(Value::as_str)?;
        let arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("{}");
        Some((name.to_string(), arguments.to_string(), call_id.to_string()))
    }

    async fn resolve_openai_client_for_target(
        target: &ProviderRouteTarget,
        provider_cache: &mut std::collections::BTreeMap<String, ProviderRuntime>,
        env: &ditto_core::config::Env,
    ) -> anyhow::Result<(ProviderRuntime, Arc<ditto_core::providers::OpenAI>)> {
        let runtime = match provider_cache.get(&target.id).cloned() {
            Some(runtime) => runtime,
            None => {
                let runtime = build_provider_runtime(target, env).await?;
                provider_cache.insert(target.id.clone(), runtime.clone());
                runtime
            }
        };

        let client = runtime.openai_responses_client.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "provider does not have an OpenAI Responses client: provider={} route_target={}",
                target.provider,
                target.id
            )
        })?;
        Ok((runtime, client))
    }

    async fn run_openai_stream_once(
        client: Arc<ditto_core::providers::OpenAI>,
        thread_rt: Arc<super::ThreadRuntime>,
        thread_id: ThreadId,
        turn_id: TurnId,
        emit_deltas: bool,
        show_thinking: bool,
        request: ditto_core::providers::openai::OpenAIResponsesRawRequest<'_>,
        max_openai_request_duration: Duration,
    ) -> Result<OpenAiRawLlmResponse, LlmAttemptFailure> {
        let mut emitted_output = false;

        let inner = async {
            let mut stream = client.create_response_stream_raw(&request).await?;
            let mut response_id = String::new();
            let mut usage: Option<Value> = None;
            let mut output_items = Vec::<Value>::new();
            let mut output_text = String::new();

            while let Some(event) = stream.recv().await {
                let event = event?;
                match event {
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::Created {
                        response_id: id,
                    } => {
                        if response_id.is_empty()
                            && let Some(id) = id.as_deref().filter(|v| !v.trim().is_empty())
                        {
                            response_id = id.to_string();
                        }
                    }
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::OutputTextDelta(delta) => {
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
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::ReasoningTextDelta(delta) => {
                        if delta.is_empty() {
                            continue;
                        }
                        if emit_deltas && show_thinking {
                            emitted_output = true;
                            let delta = omne_core::redact_text(&delta);
                            let response_id_snapshot = response_id.clone();
                            thread_rt.emit_notification(
                                "item/delta",
                                &serde_json::json!({
                                    "thread_id": thread_id,
                                    "turn_id": turn_id,
                                    "response_id": response_id_snapshot,
                                    "kind": "thinking",
                                    "thinking_kind": "text",
                                    "delta": delta,
                                }),
                            );
                        }
                    }
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::ReasoningSummaryTextDelta(delta) => {
                        if delta.is_empty() {
                            continue;
                        }
                        if emit_deltas && show_thinking {
                            emitted_output = true;
                            let delta = omne_core::redact_text(&delta);
                            let response_id_snapshot = response_id.clone();
                            thread_rt.emit_notification(
                                "item/delta",
                                &serde_json::json!({
                                    "thread_id": thread_id,
                                    "turn_id": turn_id,
                                    "response_id": response_id_snapshot,
                                    "kind": "thinking",
                                    "thinking_kind": "summary",
                                    "delta": delta,
                                }),
                            );
                        }
                    }
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::OutputItemDone(item) => {
                        emitted_output = true;
                        output_items.push(item);
                    }
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::Failed { error, .. } => {
                        let message = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error message");
                        return Err(ditto_core::error::DittoError::invalid_response_text(format!(
                            "openai response.failed: {message}"
                        )));
                    }
                    ditto_core::providers::openai::OpenAIResponsesRawEvent::Completed {
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

            Ok::<_, ditto_core::error::DittoError>(OpenAiRawLlmResponse {
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

    let mut openai_history = read_openai_responses_history(&server.thread_store, thread_id).await?;
    let seeded_from_events = openai_history.is_empty() && !seed_input_items.is_empty();
    let tool_phase_initial = tool_model.is_some();
    let bootstrap_candidates = if tool_phase_initial && !thinking_provider_candidates.is_empty() {
        &thinking_provider_candidates
    } else {
        &completion_provider_candidates
    };
    let bootstrap_target = bootstrap_candidates
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("provider routing resolved no bootstrap target"))?;
    let (bootstrap_runtime, bootstrap_client) =
        resolve_openai_client_for_target(&bootstrap_target, &mut provider_cache, &env).await?;

    let attachment_parts = if resolved_attachments.is_empty() {
        Vec::new()
    } else {
        attachments_to_ditto_parts_for_provider(
            thread_id,
            turn_id,
            &bootstrap_target.provider,
            &bootstrap_runtime,
            &resolved_attachments,
            pdf_file_id_upload_min_bytes,
        )
        .await?
    };

    if seeded_from_events {
        openai_history = seed_input_items;
        if !append_attachments_to_last_user_message(&mut openai_history, &attachment_parts) {
            tracing::debug!(
                thread_id = %thread_id,
                turn_id = %turn_id,
                "unable to append attachments to the last user message in seeded history"
            );
        }
        append_openai_responses_history_items(&server.thread_store, thread_id, &openai_history)
            .await?;
    } else if openai_history.is_empty() {
        let mut new_items = Vec::<Value>::new();
        if let Ok(context_refs) = load_turn_context_refs(&server, thread_id, turn_id).await
            && !context_refs.is_empty()
        {
            let ctx_items = context_refs_to_messages(
                &server,
                thread_id,
                turn_id,
                &context_refs,
                cancel.clone(),
            )
            .await?;
            new_items.extend(ctx_items);
        }

        if let Some(user_message) = build_user_message_item(&turn_input, &attachment_parts) {
            new_items.push(user_message);
        }
        openai_history = new_items;
        append_openai_responses_history_items(&server.thread_store, thread_id, &openai_history)
            .await?;
    } else {
        let mut new_items = Vec::<Value>::new();
        if let Ok(context_refs) = load_turn_context_refs(&server, thread_id, turn_id).await
            && !context_refs.is_empty()
        {
            let ctx_items = context_refs_to_messages(
                &server,
                thread_id,
                turn_id,
                &context_refs,
                cancel.clone(),
            )
            .await?;
            new_items.extend(ctx_items);
        }

        if let Some(user_message) = build_user_message_item(&turn_input, &attachment_parts) {
            new_items.push(user_message);
        }

        if !new_items.is_empty() {
            append_openai_responses_history_items(&server.thread_store, thread_id, &new_items)
                .await?;
            openai_history.extend(new_items);
        }
    }

    let mut last_response_id = String::new();
    let mut last_usage: Option<Value> = None;
    let mut last_text = String::new();
    let mut tool_calls_total = 0usize;
    let mut loop_detector = LoopDetector::new();
    let mut total_tokens_used = cfg.starting_total_tokens_used;
    let mut did_auto_compact = false;
    let mut attempted_auto_compact = false;
    let mut finished = false;
    let started_at = tokio::time::Instant::now();
    let mut active_provider_idx = 0usize;

    let phase_model_fallbacks = |tool_phase: bool| {
        let mut values = if tool_phase {
            thinking_model_fallbacks.clone()
        } else {
            completion_model_fallbacks.clone()
        };
        let primary = if tool_phase && !thinking_provider_candidates.is_empty() {
            thinking_provider_candidates.first()
        } else {
            completion_provider_candidates.first()
        };
        if let Some(target) = primary {
            values.extend(target.model_fallbacks.clone());
        }
        ditto_core::config::normalize_string_list(values)
    };

    let mut tool_phase_active = tool_phase_initial;
    let mut model = tool_model.clone().unwrap_or_else(|| final_model.clone());
    let mut model_candidates =
        build_model_candidates(&model, phase_model_fallbacks(tool_phase_active));
    let mut model_idx = 0usize;

    if !attempted_auto_compact
        && should_auto_compact(
            estimate_context_tokens(&instructions, &openai_history),
            cfg.auto_compact_token_limit,
        )
    {
        attempted_auto_compact = true;
        match compact_openai_responses_history(
            &server.thread_store,
            thread_id,
            &bootstrap_client,
            &model,
            &instructions,
            &openai_history,
        )
        .await
        {
            Ok(replacement) => {
                openai_history = replacement;
                did_auto_compact = true;
            }
            Err(err) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    turn_id = %turn_id,
                    error = %err,
                    "auto /responses/compact failed"
                );
            }
        }
    }

    for step_idx in 0..cfg.max_agent_steps {
        if cancel.is_cancelled() {
            return Err(AgentTurnError::Cancelled.into());
        }
        if started_at.elapsed() > cfg.max_turn_duration {
            return Err(AgentTurnError::BudgetExceeded {
                budget: "turn_seconds",
            }
            .into());
        }

        let tools_enabled = tool_model.is_none() || tool_phase_active;
        let emit_deltas = tool_model.is_none() || !tool_phase_active;
        let keep_assistant_messages = emit_deltas;

        let phase_provider_candidates =
            if tool_phase_active && !thinking_provider_candidates.is_empty() {
                &thinking_provider_candidates
            } else {
                &completion_provider_candidates
            };
        if phase_provider_candidates.is_empty() {
            anyhow::bail!("no usable provider candidates available for current phase");
        }
        let mut provider_index =
            active_provider_idx.min(phase_provider_candidates.len().saturating_sub(1));
        let mut attempts = 0usize;
        let mut failure_count = 0usize;
        let mut last_failure: Option<LlmAttemptFailure> = None;

        let (resp, active_target) = loop {
            if cancel.is_cancelled() {
                return Err(AgentTurnError::Cancelled.into());
            }
            if started_at.elapsed() > cfg.max_turn_duration {
                return Err(AgentTurnError::BudgetExceeded {
                    budget: "turn_seconds",
                }
                .into());
            }

            if provider_index >= phase_provider_candidates.len() {
                if let Some(failure) = last_failure.as_ref()
                    && llm_error_prefers_model_fallback(&failure.error)
                    && model_idx + 1 < model_candidates.len()
                {
                    let cause = llm_error_summary(&failure.error);
                    let prev = model.clone();
                    model_idx += 1;
                    model = model_candidates[model_idx].clone();
                    provider_index = 0;
                    attempts = 0;
                    failure_count = 0;
                    last_failure = None;

                    let reason = format!("model_fallback: from={prev} to={model}; cause={cause}");
                    let _ = thread_rt
                        .append_event(ThreadEventKind::ModelRouted {
                            turn_id,
                            selected_model: model.clone(),
                            rule_source,
                            reason: Some(reason),
                            rule_id: rule_id.clone(),
                        })
                        .await;
                    continue;
                }

                match last_failure {
                    Some(LlmAttemptFailure {
                        error: LlmAttemptError::TimedOut,
                        ..
                    }) => return Err(AgentTurnError::OpenAiRequestTimedOut.into()),
                    Some(LlmAttemptFailure { error, .. }) => {
                        return Err(anyhow::Error::new(error).context("llm stream failed"));
                    }
                    None => {
                        anyhow::bail!("no usable openai provider available for model={model}")
                    }
                }
            }

            let Some(target) = phase_provider_candidates.get(provider_index).cloned() else {
                provider_index = provider_index.saturating_add(1);
                continue;
            };
            let (runtime, client) =
                match resolve_openai_client_for_target(&target, &mut provider_cache, &env).await {
                    Ok((runtime, client)) => (runtime, client),
                    Err(_) => {
                        provider_index = provider_index.saturating_add(1);
                        continue;
                    }
                };

            if !runtime.capabilities.reasoning {
                provider_index = provider_index.saturating_add(1);
                continue;
            }
            if !model_allowed_by_whitelist(&model, &runtime.config.model_whitelist) {
                provider_index = provider_index.saturating_add(1);
                continue;
            }

            let reasoning_effort = match ditto_core::config::select_model_config(&model_configs, &model)
                .map(|cfg| cfg.thinking)
                .unwrap_or_default()
            {
                ThinkingIntensity::Unsupported => None,
                ThinkingIntensity::Small => Some(ditto_core::provider_options::ReasoningEffort::Low),
                ThinkingIntensity::Medium => Some(ditto_core::provider_options::ReasoningEffort::Medium),
                ThinkingIntensity::High => Some(ditto_core::provider_options::ReasoningEffort::High),
                ThinkingIntensity::XHigh => Some(ditto_core::provider_options::ReasoningEffort::XHigh),
            };
            let tool_choice = if tools_enabled {
                ditto_core::contracts::ToolChoice::Auto
            } else {
                ditto_core::contracts::ToolChoice::None
            };
            let tools_opt = if tools_enabled {
                Some(tools.as_slice())
            } else {
                None
            };

            let request = ditto_core::providers::openai::OpenAIResponsesRawRequest {
                model: &model,
                instructions: &instructions,
                input: &openai_history,
                tools: tools_opt,
                tool_choice: Some(&tool_choice),
                parallel_tool_calls: cfg.parallel_tool_calls,
                store: false,
                stream: true,
                reasoning_effort,
                reasoning_summary: None,
                response_format: cfg.response_format.as_ref(),
                include: vec!["reasoning.encrypted_content".to_string()],
                prompt_cache_key: Some(thread_id.to_string()),
                extra_headers: Default::default(),
            };

            attempts += 1;
            let _permit = LlmWorkerPool::global().acquire(turn_priority).await?;
            match run_openai_stream_once(
                client,
                thread_rt.clone(),
                thread_id,
                turn_id,
                emit_deltas,
                cfg.show_thinking,
                request,
                cfg.max_openai_request_duration,
            )
            .await
            {
                Ok(resp) => {
                    active_provider_idx = provider_index;
                    break (resp, target.clone());
                }
                Err(failure) => {
                    let should_fallback = llm_error_prefers_provider_fallback(&failure.error)
                        && provider_index + 1 < phase_provider_candidates.len();
                    let is_retryable = llm_error_is_retryable(&failure.error);
                    last_failure = Some(failure);

                    let Some(failure) = last_failure.as_ref() else {
                        anyhow::bail!("llm stream failed");
                    };
                    if failure.emitted_output {
                        let summary = llm_error_summary(&failure.error);
                        anyhow::bail!("llm stream failed after emitting output: {summary}");
                    }

                    if attempts >= cfg.llm_max_attempts {
                        if llm_error_prefers_model_fallback(&failure.error)
                            && model_idx + 1 < model_candidates.len()
                        {
                            let cause = llm_error_summary(&failure.error);
                            let prev = model.clone();
                            model_idx += 1;
                            model = model_candidates[model_idx].clone();
                            provider_index = 0;
                            attempts = 0;
                            failure_count = 0;
                            last_failure = None;

                            let reason =
                                format!("model_fallback: from={prev} to={model}; cause={cause}");
                            let _ = thread_rt
                                .append_event(ThreadEventKind::ModelRouted {
                                    turn_id,
                                    selected_model: model.clone(),
                                    rule_source,
                                    reason: Some(reason),
                                    rule_id: rule_id.clone(),
                                })
                                .await;
                            continue;
                        }

                        match &failure.error {
                            LlmAttemptError::TimedOut => {
                                return Err(AgentTurnError::OpenAiRequestTimedOut.into());
                            }
                            _ => {
                                let summary = llm_error_summary(&failure.error);
                                anyhow::bail!(
                                    "llm stream failed after {attempts} attempts: {summary}"
                                );
                            }
                        }
                    }

                    if should_fallback {
                        let prev = target.provider.clone();
                        provider_index += 1;
                        let next = phase_provider_candidates
                            .get(provider_index)
                            .map(|candidate| candidate.provider.clone())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        let cause = llm_error_summary(&failure.error);
                        let reason =
                            format!("provider_fallback: from={prev} to={next}; cause={cause}");
                        let _ = thread_rt
                            .append_event(ThreadEventKind::ModelRouted {
                                turn_id,
                                selected_model: model.clone(),
                                rule_source,
                                reason: Some(reason),
                                rule_id: rule_id.clone(),
                            })
                            .await;
                        continue;
                    }

                    if is_retryable {
                        failure_count += 1;
                        let delay = retry_backoff_delay(
                            failure_count,
                            cfg.llm_retry_base_delay,
                            cfg.llm_retry_max_delay,
                        );
                        if !delay.is_zero() {
                            tokio::select! {
                                _ = cancel.cancelled() => return Err(AgentTurnError::Cancelled.into()),
                                _ = tokio::time::sleep(delay) => {}
                            }
                        }
                        continue;
                    }

                    let summary = llm_error_summary(&failure.error);
                    anyhow::bail!("llm stream failed: {summary}");
                }
            }
        };

        let warnings_count = 0usize;
        let step_text = if keep_assistant_messages {
            resp.output_text.clone()
        } else {
            String::new()
        };
        last_response_id = resp.id.clone();
        last_usage = resp.usage.clone();
        if cfg.max_total_tokens > 0 {
            if let Some(tokens) = resp.usage.as_ref().and_then(usage_total_tokens) {
                total_tokens_used = total_tokens_used.saturating_add(tokens);
                if total_tokens_used > cfg.max_total_tokens {
                    return Err(AgentTurnError::TokenBudgetExceeded {
                        used: total_tokens_used,
                        limit: cfg.max_total_tokens,
                    }
                    .into());
                }
            }
        }

        let mut function_calls = Vec::new();
        if keep_assistant_messages {
            last_text = resp.output_text.clone();
        }

        for item in &resp.output_items {
            if let Some(call) = parse_function_call_item(item) {
                function_calls.push(call);
            }
        }

        if !resp.output_items.is_empty() {
            append_openai_responses_history_items(
                &server.thread_store,
                thread_id,
                &resp.output_items,
            )
            .await?;
            openai_history.extend(resp.output_items.clone());
        }

        if function_calls.is_empty() {
            let _ = thread_rt
                .append_event(ThreadEventKind::AgentStep {
                    turn_id,
                    step: step_idx.min(u32::MAX as usize) as u32,
                    model: model.clone(),
                    response_id: last_response_id.clone(),
                    text: if step_text.trim().is_empty() {
                        None
                    } else {
                        Some(truncate_chars(&step_text, 20_000))
                    },
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    token_usage: last_usage.clone(),
                    warnings_count: None,
                })
                .await;

            if tool_model.is_some() && tool_phase_active {
                tool_phase_active = false;
                active_provider_idx = 0;

                let prev = model.clone();
                model = final_model.clone();
                model_candidates = build_model_candidates(&model, phase_model_fallbacks(false));
                model_idx = 0;

                if prev != model {
                    let provider = completion_provider_candidates
                        .first()
                        .map(|target| target.provider.as_str())
                        .unwrap_or("<unknown>");
                    let reason =
                        format!("tool_model_final: from={prev} to={model}; provider={provider}");
                    let _ = thread_rt
                        .append_event(ThreadEventKind::ModelRouted {
                            turn_id,
                            selected_model: model.clone(),
                            rule_source,
                            reason: Some(reason),
                            rule_id: rule_id.clone(),
                        })
                        .await;
                }

                let msg = serde_json::json!({
                    "type": "message",
                    "role": "system",
                    "content": [{
                        "type": "input_text",
                        "text": "Tool phase complete. Provide the final answer to the user's request without calling tools."
                    }]
                });
                append_openai_responses_history_items(
                    &server.thread_store,
                    thread_id,
                    std::slice::from_ref(&msg),
                )
                .await?;
                openai_history.push(msg);
                continue;
            }

            finished = true;
            break;
        }

        let tool_calls_for_event = function_calls
            .iter()
            .map(|(tool_name, arguments, call_id)| {
                let arguments = omne_core::redact_text(arguments);
                let arguments = truncate_chars(&arguments, 10_000);
                omne_protocol::AgentStepToolCall {
                    name: tool_name.clone(),
                    call_id: call_id.clone(),
                    arguments,
                }
            })
            .collect::<Vec<_>>();
        let mut tool_results_for_event = Vec::<omne_protocol::AgentStepToolResult>::new();

        let signature = step_signature(&function_calls);
        if let Some(kind) = loop_detector.observe(signature) {
            return Err(AgentTurnError::LoopDetected { kind }.into());
        }

        let can_parallelize_read_only = cfg.parallel_tool_calls
            && function_calls.len() > 1
            && function_calls
                .iter()
                .all(|(tool_name, _, _)| tool_is_read_only(tool_name));

        let mut tool_output_items = Vec::<Value>::new();

        if can_parallelize_read_only {
            let batch_size = function_calls.len();
            if tool_calls_total + batch_size > cfg.max_tool_calls {
                return Err(AgentTurnError::BudgetExceeded {
                    budget: "tool_calls",
                }
                .into());
            }
            tool_calls_total += batch_size;

            let mut outputs = vec![None::<(String, Value, Vec<OpenAiItem>)>; batch_size];
            let mut calls = Vec::new();

            for (idx, (tool_name, arguments, call_id)) in function_calls.into_iter().enumerate() {
                let args_json: Value = match serde_json::from_str(&arguments) {
                    Ok(v) => v,
                    Err(err) => {
                        let output = serde_json::json!({
                            "error": "invalid tool arguments",
                            "details": err.to_string(),
                            "arguments": arguments,
                        });
                        outputs[idx] = Some((call_id, output, Vec::new()));
                        continue;
                    }
                };
                calls.push((idx, tool_name, args_json, call_id));
            }

            let results = stream::iter(calls)
                .map(|(idx, tool_name, args_json, call_id)| {
                    let server = server.clone();
                    let cancel = cancel.clone();
                    async move {
                        let outcome = run_tool_call(
                            &server,
                            thread_id,
                            Some(turn_id),
                            &tool_name,
                            args_json,
                            cancel,
                            false,
                        )
                        .await;
                        (idx, call_id, outcome)
                    }
                })
                .buffer_unordered(cfg.max_parallel_tool_calls)
                .collect::<Vec<_>>()
                .await;

            for (idx, call_id, outcome) in results {
                let (output_value, hook_messages) = match outcome {
                    Ok(outcome) => (outcome.output, outcome.hook_messages),
                    Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                };
                outputs[idx] = Some((call_id, output_value, hook_messages));
            }

            for (call_id, output_value, hook_messages) in outputs.into_iter().flatten() {
                let output_json = serde_json::to_string(&output_value)?;
                let output_preview = omne_core::redact_text(&output_json);
                let output_preview = truncate_chars(&output_preview, 10_000);
                tool_results_for_event.push(omne_protocol::AgentStepToolResult {
                    call_id: call_id.clone(),
                    output: output_preview,
                });

                tool_output_items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output_json,
                }));

                tool_output_items.extend(hook_messages);
            }
        } else {
            for (tool_name, arguments, call_id) in function_calls {
                tool_calls_total += 1;
                if tool_calls_total > cfg.max_tool_calls {
                    return Err(AgentTurnError::BudgetExceeded {
                        budget: "tool_calls",
                    }
                    .into());
                }
                let args_json: Value = match serde_json::from_str(&arguments) {
                    Ok(v) => v,
                    Err(err) => {
                        let output = serde_json::json!({
                            "error": "invalid tool arguments",
                            "details": err.to_string(),
                            "arguments": arguments,
                        });
                        tool_output_items.push(serde_json::json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": serde_json::to_string(&output)?,
                        }));
                        continue;
                    }
                };

                let outcome = run_tool_call(
                    &server,
                    thread_id,
                    Some(turn_id),
                    &tool_name,
                    args_json,
                    cancel.clone(),
                    false,
                )
                .await;
                let (output_value, hook_messages) = match outcome {
                    Ok(outcome) => (outcome.output, outcome.hook_messages),
                    Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                };
                let output_json = serde_json::to_string(&output_value)?;
                let output_preview = omne_core::redact_text(&output_json);
                let output_preview = truncate_chars(&output_preview, 10_000);
                tool_results_for_event.push(omne_protocol::AgentStepToolResult {
                    call_id: call_id.clone(),
                    output: output_preview,
                });

                tool_output_items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output_json,
                }));

                tool_output_items.extend(hook_messages);
            }
        }

        if !tool_output_items.is_empty() {
            append_openai_responses_history_items(
                &server.thread_store,
                thread_id,
                &tool_output_items,
            )
            .await?;
            openai_history.extend(tool_output_items.clone());
        }

        if !attempted_auto_compact
            && should_auto_compact(
                estimate_context_tokens(&instructions, &openai_history),
                cfg.auto_compact_token_limit,
            )
        {
            attempted_auto_compact = true;
            if !did_auto_compact
                && let Some(openai_client) = provider_cache
                    .get(&active_target.id)
                    .and_then(|runtime| runtime.openai_responses_client.as_deref())
            {
                match compact_openai_responses_history(
                    &server.thread_store,
                    thread_id,
                    openai_client,
                    &model,
                    &instructions,
                    &openai_history,
                )
                .await
                {
                    Ok(replacement) => {
                        openai_history = replacement;
                        did_auto_compact = true;
                    }
                    Err(err) => {
                        tracing::warn!(
                            thread_id = %thread_id,
                            turn_id = %turn_id,
                            error = %err,
                            "auto /responses/compact failed"
                        );
                    }
                }
            }
        }

        let _ = thread_rt
            .append_event(ThreadEventKind::AgentStep {
                turn_id,
                step: step_idx.min(u32::MAX as usize) as u32,
                model: model.clone(),
                response_id: last_response_id.clone(),
                text: if step_text.trim().is_empty() {
                    None
                } else {
                    Some(truncate_chars(&step_text, 20_000))
                },
                tool_calls: tool_calls_for_event,
                tool_results: tool_results_for_event,
                token_usage: last_usage.clone(),
                warnings_count: if warnings_count == 0 {
                    None
                } else {
                    Some(warnings_count.min(u32::MAX as usize) as u32)
                },
            })
            .await;
    }

    if !finished {
        return Err(AgentTurnError::BudgetExceeded { budget: "steps" }.into());
    }

    Ok(ToolLoopOutcome {
        model,
        last_response_id,
        last_usage,
        last_text,
    })
}
