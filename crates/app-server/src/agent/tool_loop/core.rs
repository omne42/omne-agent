#[derive(Debug, Clone)]
struct ToolLoopConfig {
    max_agent_steps: usize,
    max_tool_calls: usize,
    max_turn_duration: Duration,
    max_openai_request_duration: Duration,
    llm_max_attempts: usize,
    llm_retry_base_delay: Duration,
    llm_retry_max_delay: Duration,
    max_total_tokens: u64,
    starting_total_tokens_used: u64,
    auto_compact_token_limit: Option<u64>,
    auto_summary_threshold_pct: u64,
    auto_summary_source_max_chars: usize,
    auto_summary_tail_items: usize,
    parallel_tool_calls: bool,
    max_parallel_tool_calls: usize,
    response_format: Option<ditto_llm::ResponseFormat>,
}

#[derive(Debug)]
struct ToolLoopOutcome {
    model: String,
    last_response_id: String,
    last_usage: Option<Value>,
    last_text: String,
}

struct ToolLoop {
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    cancel: CancellationToken,
    turn_priority: TurnPriority,
    approval_policy: omne_agent_protocol::ApprovalPolicy,
    final_model: String,
    provider: String,
    provider_candidates: Vec<String>,
    provider_cache: std::collections::BTreeMap<String, ProviderRuntime>,
    provider_config: ditto_llm::ProviderConfig,
    project_overrides: ProjectOpenAiOverrides,
    base_url_override: Option<String>,
    env: ditto_llm::Env,
    tools: Vec<ditto_llm::Tool>,
    instructions: String,
    input_items: Vec<OpenAiItem>,
    tool_model: Option<String>,
    model_fallbacks: Vec<String>,
    model_client: Arc<dyn ditto_llm::LanguageModel>,
    resolved_attachments: Vec<ResolvedAttachment>,
    pdf_file_id_upload_min_bytes: u64,
    rule_source: omne_agent_protocol::ModelRoutingRuleSource,
    rule_id: Option<String>,
    thinking_override: Option<String>,
    cfg: ToolLoopConfig,
}

fn max_steps_prompt(max_agent_steps: usize) -> String {
    format!(
        "[max_steps] reached max_agent_steps={max_agent_steps} for this turn.\n\nHard requirements:\n- Do NOT call any tools.\n- Only write a text response to the user.\n- Briefly summarize progress, what is incomplete, and ask the user whether to continue in a new message."
    )
}

fn parse_thinking_override(value: Option<&str>) -> Option<ThinkingIntensity> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "unsupported" => Some(ThinkingIntensity::Unsupported),
            "small" => Some(ThinkingIntensity::Small),
            "medium" => Some(ThinkingIntensity::Medium),
            "high" => Some(ThinkingIntensity::High),
            "xhigh" => Some(ThinkingIntensity::XHigh),
            _ => None,
        })
}

fn resolve_reasoning_effort(
    thinking_override: Option<&str>,
    project_models: &std::collections::BTreeMap<String, ditto_llm::ModelConfig>,
    model: &str,
) -> Option<ditto_llm::ReasoningEffort> {
    let thinking = parse_thinking_override(thinking_override)
        .or_else(|| ditto_llm::select_model_config(project_models, model).map(|cfg| cfg.thinking))
        .unwrap_or_default();

    match thinking {
        ThinkingIntensity::Unsupported => None,
        ThinkingIntensity::Small => Some(ditto_llm::ReasoningEffort::Low),
        ThinkingIntensity::Medium => Some(ditto_llm::ReasoningEffort::Medium),
        ThinkingIntensity::High => Some(ditto_llm::ReasoningEffort::High),
        ThinkingIntensity::XHigh => Some(ditto_llm::ReasoningEffort::XHigh),
    }
}

impl ToolLoop {
    async fn run(self) -> anyhow::Result<ToolLoopOutcome> {
        let ToolLoop {
            server,
            thread_rt,
            thread_id,
            turn_id,
            cancel,
            turn_priority,
            approval_policy,
            final_model,
            provider,
            provider_candidates,
            mut provider_cache,
            provider_config,
            project_overrides,
            base_url_override,
            env,
            tools,
            instructions,
            mut input_items,
            tool_model,
            model_fallbacks,
            model_client,
            resolved_attachments,
            pdf_file_id_upload_min_bytes,
            rule_source,
            rule_id,
            thinking_override,
            cfg,
        } = self;

        let mut last_response_id = String::new();
        let mut last_usage: Option<Value> = None;
        let mut last_text = String::new();
        let mut tool_calls_total = 0usize;
        let mut loop_detector = LoopDetector::new();
        let mut total_tokens_used = cfg.starting_total_tokens_used;
        let mut did_auto_summary = false;
        let mut attempted_auto_summary = false;
        let mut finished = false;
        let started_at = tokio::time::Instant::now();
        let mut active_provider_idx = 0usize;
        let mut attachment_parts_cache =
            std::collections::BTreeMap::<String, Vec<ditto_llm::ContentPart>>::new();

        let mut tool_phase_active = tool_model.is_some();
        let mut model = tool_model.clone().unwrap_or_else(|| final_model.clone());
        let mut model_candidates = build_model_candidates(&model, model_fallbacks.clone());
        if !provider_config.model_whitelist.is_empty() {
            model_candidates.retain(|candidate| {
                model_allowed_by_whitelist(candidate, &provider_config.model_whitelist)
            });
        }
        let mut model_idx = 0usize;

        if !attempted_auto_summary
            && should_auto_compact(
                total_tokens_used,
                cfg.auto_compact_token_limit,
                cfg.max_total_tokens,
                cfg.auto_summary_threshold_pct,
            )
        {
            attempted_auto_summary = true;
            let summary_cfg = AutoCompactSummaryConfig {
                source_max_chars: cfg.auto_summary_source_max_chars,
                tail_items: cfg.auto_summary_tail_items,
            };
            let ctx = AutoCompactSummaryContext {
                server: &server,
                thread_id,
                turn_id,
                model: &model,
                llm: model_client.clone(),
                turn_priority,
                max_openai_request_duration: cfg.max_openai_request_duration,
                max_total_tokens: cfg.max_total_tokens,
                total_tokens_used: &mut total_tokens_used,
                input_items: &mut input_items,
            };
            if !did_auto_summary && auto_compact_summary(ctx, summary_cfg).await? {
                did_auto_summary = true;
            }
        }

        let max_steps_message = max_steps_prompt(cfg.max_agent_steps);

        for step_idx in 0..=cfg.max_agent_steps {
            if cancel.is_cancelled() {
                return Err(AgentTurnError::Cancelled.into());
            }
            if started_at.elapsed() > cfg.max_turn_duration {
                return Err(AgentTurnError::BudgetExceeded {
                    budget: "turn_seconds",
                }
                .into());
            }

            let force_text_only = step_idx >= cfg.max_agent_steps;
            if force_text_only {
                tool_phase_active = false;
                if model != final_model {
                    model = final_model.clone();
                    model_candidates = build_model_candidates(&model, model_fallbacks.clone());
                    if !provider_config.model_whitelist.is_empty() {
                        model_candidates.retain(|candidate| {
                            model_allowed_by_whitelist(candidate, &provider_config.model_whitelist)
                        });
                    }
                    model_idx = 0;
                }
            }

            let mut base_messages =
                response_items_to_ditto_messages(&instructions, &input_items, &[]);
            if force_text_only {
                base_messages.push(ditto_llm::Message::system(max_steps_message.clone()));
            }

            let tools_enabled = !force_text_only && (tool_model.is_none() || tool_phase_active);
            let emit_deltas = force_text_only || tool_model.is_none() || !tool_phase_active;
            let keep_assistant_messages = emit_deltas;

            let mut provider_index =
                active_provider_idx.min(provider_candidates.len().saturating_sub(1));
            let mut attempts = 0usize;
            let mut failure_count = 0usize;
            let mut last_failure: Option<LlmAttemptFailure> = None;

            let resp = loop {
                if cancel.is_cancelled() {
                    return Err(AgentTurnError::Cancelled.into());
                }
                if started_at.elapsed() > cfg.max_turn_duration {
                    return Err(AgentTurnError::BudgetExceeded {
                        budget: "turn_seconds",
                    }
                    .into());
                }
                if provider_index >= provider_candidates.len() {
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
                            return Err(anyhow::Error::new(error).context("llm stream failed"))
                        }
                        None => {
                            anyhow::bail!("no usable openai provider available for model={model}")
                        }
                    }
                }

                let provider_name = provider_candidates
                    .get(provider_index)
                    .cloned()
                    .unwrap_or_else(|| provider.clone());
                let runtime = match provider_cache.get(&provider_name).cloned() {
                    Some(runtime) => runtime,
                    None => match build_provider_runtime(
                        &provider_name,
                        &project_overrides,
                        base_url_override.as_deref(),
                        &env,
                    )
                    .await
                    {
                        Ok(runtime) => {
                            provider_cache.insert(provider_name.clone(), runtime.clone());
                            runtime
                        }
                        Err(err) => {
                            tracing::warn!(
                                thread_id = %thread_id,
                                turn_id = %turn_id,
                                provider = provider_name,
                                error = %err,
                                "failed to build provider client; skipping"
                            );
                            provider_index = provider_index.saturating_add(1);
                            continue;
                        }
                    },
                };

                if !model_allowed_by_whitelist(&model, &runtime.config.model_whitelist) {
                    provider_index = provider_index.saturating_add(1);
                    continue;
                }

                let reasoning_effort = if runtime.capabilities.reasoning {
                    resolve_reasoning_effort(
                        thinking_override.as_deref(),
                        &project_overrides.models,
                        &model,
                    )
                } else {
                    None
                };

                let provider_options = ditto_llm::ProviderOptions {
                    reasoning_effort,
                    response_format: cfg.response_format.clone(),
                    parallel_tool_calls: Some(cfg.parallel_tool_calls),
                };
                if !resolved_attachments.is_empty()
                    && !attachment_parts_cache.contains_key(&provider_name)
                {
                    let parts = attachments_to_ditto_parts_for_provider(
                        thread_id,
                        turn_id,
                        provider_name.as_str(),
                        &runtime,
                        &resolved_attachments,
                        pdf_file_id_upload_min_bytes,
                    )
                    .await?;
                    attachment_parts_cache.insert(provider_name.clone(), parts);
                }

                let attachment_parts = attachment_parts_cache
                    .get(&provider_name)
                    .map(|parts| parts.as_slice())
                    .unwrap_or(&[]);
                let messages = apply_attachments_to_messages(base_messages.clone(), attachment_parts);
                let mut req_base = ditto_llm::GenerateRequest::from(messages);
                req_base.model = Some(model.clone());
                if tools_enabled {
                    req_base.tools = Some(tools.clone());
                    req_base.tool_choice = Some(ditto_llm::ToolChoice::Auto);
                } else {
                    req_base.tools = None;
                    req_base.tool_choice = Some(ditto_llm::ToolChoice::None);
                }

                let req = req_base
                    .with_provider_options(provider_options)
                    .context("encode provider_options")?;

                attempts += 1;
                let _permit = LlmWorkerPool::global().acquire(turn_priority).await?;
                match run_llm_stream_once(
                    runtime.client.clone(),
                    thread_rt.clone(),
                    thread_id,
                    turn_id,
                    emit_deltas,
                    req,
                    cfg.max_openai_request_duration,
                )
                .await
                {
                    Ok(resp) => {
                        active_provider_idx = provider_index;
                        break resp;
                    }
                    Err(failure) => {
                        let should_fallback = llm_error_prefers_provider_fallback(&failure.error)
                            && provider_index + 1 < provider_candidates.len();
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
                                    return Err(AgentTurnError::OpenAiRequestTimedOut.into())
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
                            let prev = provider_name.clone();
                            provider_index += 1;
                            let next = provider_candidates
                                .get(provider_index)
                                .cloned()
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

                        if !is_retryable {
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

                            let summary = llm_error_summary(&failure.error);
                            anyhow::bail!("llm stream failed: {summary}");
                        }

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
                    }
                }
            };

            if !resp.warnings.is_empty() {
                log_llm_warnings(thread_id, turn_id, &resp.warnings);
            }
            let warnings_count = resp.warnings.len();
            let step_text = if keep_assistant_messages {
                extract_assistant_text(&resp.output)
            } else {
                String::new()
            };
            last_response_id = resp.id.clone();
            last_usage = resp.usage.clone();
            if cfg.max_total_tokens > 0 {
                if let Some(tokens) = resp.usage.as_ref().and_then(usage_total_tokens) {
                    total_tokens_used = total_tokens_used.saturating_add(tokens);
                    if total_tokens_used > cfg.max_total_tokens {
                        return Err(
                            AgentTurnError::TokenBudgetExceeded {
                                used: total_tokens_used,
                                limit: cfg.max_total_tokens,
                            }
                            .into(),
                        );
                    }
                }
            }

            let mut function_calls = Vec::new();
            if keep_assistant_messages {
                last_text = extract_assistant_text(&resp.output);
            }

            for item in resp.output {
                if force_text_only
                    && item.get("type").and_then(Value::as_str) == Some("function_call")
                {
                    continue;
                }
                if item.get("type").and_then(Value::as_str) == Some("function_call")
                    && let Some(name) = item.get("name").and_then(Value::as_str)
                    && let Some(call_id) = item.get("call_id").and_then(Value::as_str)
                {
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    function_calls.push((
                        name.to_string(),
                        arguments.to_string(),
                        call_id.to_string(),
                    ));
                    input_items.push(item);
                } else if item.get("type").and_then(Value::as_str) == Some("message") {
                    if keep_assistant_messages {
                        input_items.push(item);
                    }
                } else {
                    input_items.push(item);
                }
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
                        warnings_count: if warnings_count == 0 {
                            None
                        } else {
                            Some(warnings_count.min(u32::MAX as usize) as u32)
                        },
                    })
                    .await;

                if tool_model.is_some() && tool_phase_active {
                    tool_phase_active = false;

                    let prev = model.clone();
                    model = final_model.clone();
                    model_candidates = build_model_candidates(&model, model_fallbacks.clone());
                    if !provider_config.model_whitelist.is_empty() {
                        model_candidates.retain(|candidate| {
                            model_allowed_by_whitelist(candidate, &provider_config.model_whitelist)
                        });
                    }
                    model_idx = 0;

                    if prev != model {
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

                    input_items.push(serde_json::json!({
                        "type": "message",
                        "role": "system",
                        "content": [{
                            "type": "input_text",
                            "text": "Tool phase complete. Provide the final answer to the user's request without calling tools.",
                        }]
                    }));

                    continue;
                }
                finished = true;
                break;
            }

            let tool_calls_for_event = function_calls
                .iter()
                .map(|(tool_name, arguments, call_id)| {
                    let arguments = omne_agent_core::redact_text(arguments);
                    let arguments = truncate_chars(&arguments, 10_000);
                    omne_agent_protocol::AgentStepToolCall {
                        name: tool_name.clone(),
                        call_id: call_id.clone(),
                        arguments,
                    }
                })
                .collect::<Vec<_>>();
            let mut tool_results_for_event = Vec::<omne_agent_protocol::AgentStepToolResult>::new();

            let signature = step_signature(&function_calls);
            if let Some(kind) = loop_detector.observe(signature) {
                match gate_doom_loop(
                    server.as_ref(),
                    &thread_rt,
                    thread_id,
                    turn_id,
                    approval_policy,
                    kind,
                    signature,
                    &tool_calls_for_event,
                    cancel.clone(),
                )
                .await?
                {
                    DoomLoopDecision::Approved => {
                        loop_detector.recent.clear();
                    }
                    DoomLoopDecision::Denied { remembered } => {
                        for (tool_name, _arguments, call_id) in &function_calls {
                            let output_value = serde_json::json!({
                                "denied": true,
                                "error": "doom_loop denied",
                                "kind": kind,
                                "signature": signature,
                                "tool": tool_name,
                            });
                            let output_json = serde_json::to_string(&output_value)?;
                            let output_preview = omne_agent_core::redact_text(&output_json);
                            let output_preview = truncate_chars(&output_preview, 10_000);
                            tool_results_for_event.push(omne_agent_protocol::AgentStepToolResult {
                                call_id: call_id.clone(),
                                output: output_preview,
                            });

                            input_items.push(serde_json::json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": output_json,
                            }));
                        }

                        last_text = format!(
                            "[doom_loop] detected repeated tool calls (kind={kind}). Stopped executing tools after approval was denied{}.",
                            if remembered { " (remembered)" } else { "" }
                        );
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
                        finished = true;
                        break;
                    }
                }
            }

            let can_parallelize_read_only = cfg.parallel_tool_calls
                && function_calls.len() > 1
                && function_calls
                    .iter()
                    .all(|(tool_name, _, _)| tool_is_read_only(tool_name));

            if can_parallelize_read_only {
                let batch_size = function_calls.len();
                if tool_calls_total + batch_size > cfg.max_tool_calls {
                    return Err(AgentTurnError::BudgetExceeded {
                        budget: "tool_calls",
                    }
                    .into());
                }
                tool_calls_total += batch_size;

                let mut outputs =
                    vec![None::<(String, Value, Vec<OpenAiItem>)>; batch_size];
                let mut calls = Vec::new();

                for (idx, (tool_name, arguments, call_id)) in
                    function_calls.into_iter().enumerate()
                {
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
                                true,
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
                    let output_preview = omne_agent_core::redact_text(&output_json);
                    let output_preview = truncate_chars(&output_preview, 10_000);
                    tool_results_for_event.push(omne_agent_protocol::AgentStepToolResult {
                        call_id: call_id.clone(),
                        output: output_preview,
                    });

                    input_items.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output_json,
                    }));
                    for message in hook_messages {
                        input_items.push(message);
                    }
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
                            input_items.push(serde_json::json!({
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
                        true,
                    )
                    .await;
                    let (output_value, hook_messages) = match outcome {
                        Ok(outcome) => (outcome.output, outcome.hook_messages),
                        Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                    };

                    let output_json = serde_json::to_string(&output_value)?;
                    let output_preview = omne_agent_core::redact_text(&output_json);
                    let output_preview = truncate_chars(&output_preview, 10_000);
                    tool_results_for_event.push(omne_agent_protocol::AgentStepToolResult {
                        call_id: call_id.clone(),
                        output: output_preview,
                    });

                    input_items.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output_json,
                    }));
                    for message in hook_messages {
                        input_items.push(message);
                    }
                }
            }

            if !attempted_auto_summary
                && should_auto_compact(
                    total_tokens_used,
                    cfg.auto_compact_token_limit,
                    cfg.max_total_tokens,
                    cfg.auto_summary_threshold_pct,
                )
            {
                attempted_auto_summary = true;
                let summary_cfg = AutoCompactSummaryConfig {
                    source_max_chars: cfg.auto_summary_source_max_chars,
                    tail_items: cfg.auto_summary_tail_items,
                };
                let ctx = AutoCompactSummaryContext {
                    server: &server,
                    thread_id,
                    turn_id,
                    model: &model,
                    llm: model_client.clone(),
                    turn_priority,
                    max_openai_request_duration: cfg.max_openai_request_duration,
                    max_total_tokens: cfg.max_total_tokens,
                    total_tokens_used: &mut total_tokens_used,
                    input_items: &mut input_items,
                };
                if !did_auto_summary && auto_compact_summary(ctx, summary_cfg).await? {
                    did_auto_summary = true;
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
}
