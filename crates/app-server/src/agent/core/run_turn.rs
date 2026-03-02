fn apply_plan_parallel_tool_call_overrides(
    has_plan_directive: bool,
    parallel_tool_calls: bool,
    max_parallel_tool_calls: usize,
) -> (bool, usize) {
    if has_plan_directive {
        (false, 1)
    } else {
        (parallel_tool_calls, max_parallel_tool_calls)
    }
}

fn resolve_turn_role_for_routing(has_plan_directive: bool, thread_mode: &str) -> &str {
    if has_plan_directive {
        "architect"
    } else {
        thread_mode
    }
}

fn summarize_plan_artifact(text: &str) -> String {
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("plan");
    let summary = first_line.chars().take(120).collect::<String>();
    if summary.is_empty() {
        "plan".to_string()
    } else {
        summary
    }
}

async fn write_plan_artifact_if_needed(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    has_plan_directive: bool,
    last_text: &str,
) -> anyhow::Result<bool> {
    if !has_plan_directive || last_text.trim().is_empty() {
        return Ok(false);
    }

    let summary = summarize_plan_artifact(last_text);
    crate::handle_artifact_write(
        server,
        crate::ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "plan".to_string(),
            summary,
            text: last_text.to_string(),
        },
    )
    .await?;
    Ok(true)
}

pub async fn run_agent_turn(
    server: Arc<super::Server>,
    thread_rt: Arc<super::ThreadRuntime>,
    turn_id: TurnId,
    input: String,
    cancel: CancellationToken,
    turn_priority: omne_protocol::TurnPriority,
) -> anyhow::Result<()> {
    let (
        thread_id,
        thread_mode,
        thread_model,
        thread_openai_base_url,
        thread_show_thinking,
        thread_cwd,
        allowed_tools,
    ) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            handle.thread_id(),
            state.mode.clone(),
            state.model.clone(),
            state.openai_base_url.clone(),
            state.show_thinking,
            state.cwd.clone(),
            state.allowed_tools.clone(),
        )
    };

    let thread_root = match thread_cwd.as_deref() {
        Some(thread_cwd) => Some(omne_core::resolve_dir(Path::new(thread_cwd), Path::new(".")).await?),
        None => None,
    };

    let (mut project_overrides, project_ui) = if let Some(thread_root) = thread_root.as_deref() {
        let loaded = crate::project_config::load_project_config(thread_root).await;
        (loaded.openai, loaded.ui)
    } else {
        (
            ProjectOpenAiOverrides::default(),
            crate::project_config::ProjectUiOverrides::default(),
        )
    };

    let mode_show_thinking = if let Some(thread_root) = thread_root.as_deref() {
        let catalog = omne_core::modes::ModeCatalog::load(thread_root).await;
        catalog
            .mode(&thread_mode)
            .and_then(|mode| mode.ui.show_thinking)
    } else {
        None
    };
    let show_thinking = thread_show_thinking
        .or(mode_show_thinking)
        .or(project_ui.show_thinking)
        .unwrap_or(true);

    let provider = project_overrides
        .provider
        .clone()
        .or_else(|| {
            std::env::var("OMNE_OPENAI_PROVIDER")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_OPENAI_PROVIDER.to_string());

    let builtin_provider_config = builtin_openai_provider_config(&provider);
    let provider_overrides = project_overrides.providers.get(&provider);
    if builtin_provider_config.is_none() && provider_overrides.is_none() {
        anyhow::bail!(
            "unknown openai provider: {provider} (expected: openai-codex-apikey, openai-auth-command; or define [openai.providers.{provider}] in project config)"
        );
    }

    let mut provider_config = builtin_provider_config.unwrap_or_default();
    if let Some(overrides) = provider_overrides {
        provider_config = merge_provider_config(provider_config, overrides);
    }

    let forced_model = thread_model.is_some();
    let global_default_model = thread_model
        .or(project_overrides.model.clone())
        .or_else(|| {
            std::env::var("OMNE_OPENAI_MODEL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .or(provider_config.default_model.clone())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let base_url_override = thread_openai_base_url
        .clone()
        .or(project_overrides.base_url.clone())
        .or_else(|| {
            std::env::var("OMNE_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });
    let base_url = base_url_override
        .clone()
        .or(provider_config.base_url.clone())
        .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
    let base_url = base_url.trim().to_string();
    if base_url.is_empty() {
        anyhow::bail!("openai provider {provider} is missing base_url");
    }

    let provider_capabilities = provider_config
        .capabilities
        .unwrap_or_else(ditto_llm::ProviderCapabilities::openai_responses);
    if !provider_capabilities.tools {
        anyhow::bail!(
            "provider does not support tools: provider={provider} (omne requires tool calling; set [openai.providers.{provider}.capabilities.tools]=true)"
        );
    }
    if provider_capabilities.reasoning && !provider_capabilities.streaming {
        anyhow::bail!(
            "provider does not support streaming (required for reasoning providers): provider={provider} (set [openai.providers.{provider}.capabilities.streaming]=true or choose a streaming-capable provider)"
        );
    }

    let env = ditto_llm::Env {
        dotenv: std::mem::take(&mut project_overrides.dotenv),
    };
    let provider_for_llm = ditto_llm::ProviderConfig {
        base_url: Some(base_url),
        default_model: provider_config.default_model.clone(),
        model_whitelist: provider_config.model_whitelist.clone(),
        http_headers: provider_config.http_headers.clone(),
        http_query_params: provider_config.http_query_params.clone(),
        auth: provider_config.auth.clone(),
        capabilities: Some(provider_capabilities),
    };
    let (model_client, openai_responses_client, file_uploader) = if provider_capabilities.reasoning {
        let openai = Arc::new(
            ditto_llm::OpenAI::from_config(&provider_for_llm, &env)
                .await
                .context("build OpenAI Responses client")?,
        );
        let model_client: Arc<dyn ditto_llm::LanguageModel> = openai.clone();
        let file_uploader: Arc<dyn FileUploader> = openai.clone();
        (model_client, Some(openai), Some(file_uploader))
    } else {
        let chat = Arc::new(
            ditto_llm::OpenAICompatible::from_config(&provider_for_llm, &env)
                .await
                .context("build OpenAI-compatible Chat Completions client")?,
        );
        let model_client: Arc<dyn ditto_llm::LanguageModel> = chat.clone();
        let file_uploader: Arc<dyn FileUploader> = chat;
        (model_client, None, Some(file_uploader))
    };

    let fallbacks = std::env::var("OMNE_OPENAI_FALLBACK_PROVIDERS")
        .ok()
        .map(|value| parse_csv_list(&value))
        .unwrap_or_else(|| project_overrides.fallback_providers.clone());
    let provider_candidates = build_provider_candidates(&provider, fallbacks);
    let mut provider_cache = std::collections::BTreeMap::<String, ProviderRuntime>::new();
    provider_cache.insert(
        provider.clone(),
        ProviderRuntime {
            config: provider_for_llm,
            capabilities: provider_capabilities,
            client: model_client.clone(),
            openai_responses_client,
            file_uploader,
        },
    );

    let tool_specs = build_tools();
    let tools = tool_specs_to_ditto_tools(&tool_specs).context("parse tool schemas")?;

    let max_agent_steps = parse_env_usize(
        "OMNE_AGENT_MAX_STEPS",
        DEFAULT_MAX_AGENT_STEPS,
        1,
        MAX_MAX_AGENT_STEPS,
    );
    let max_tool_calls = parse_env_usize(
        "OMNE_AGENT_MAX_TOOL_CALLS",
        DEFAULT_MAX_TOOL_CALLS,
        1,
        MAX_MAX_TOOL_CALLS,
    );
    let max_turn_duration = Duration::from_secs(parse_env_u64(
        "OMNE_AGENT_MAX_TURN_SECONDS",
        DEFAULT_MAX_TURN_SECONDS,
        1,
        MAX_MAX_TURN_SECONDS,
    ));
    let max_openai_request_duration = Duration::from_secs(parse_env_u64(
        "OMNE_AGENT_MAX_OPENAI_REQUEST_SECONDS",
        DEFAULT_MAX_OPENAI_REQUEST_SECONDS,
        1,
        MAX_MAX_OPENAI_REQUEST_SECONDS,
    ));
    let llm_max_attempts = parse_env_usize(
        "OMNE_AGENT_LLM_MAX_ATTEMPTS",
        DEFAULT_LLM_MAX_ATTEMPTS,
        1,
        MAX_LLM_MAX_ATTEMPTS,
    );
    let llm_retry_base_delay = Duration::from_millis(parse_env_u64(
        "OMNE_AGENT_LLM_RETRY_BASE_DELAY_MS",
        DEFAULT_LLM_RETRY_BASE_DELAY_MS,
        0,
        MAX_LLM_RETRY_DELAY_MS,
    ));
    let llm_retry_max_delay = Duration::from_millis(parse_env_u64(
        "OMNE_AGENT_LLM_RETRY_MAX_DELAY_MS",
        DEFAULT_LLM_RETRY_MAX_DELAY_MS,
        0,
        MAX_LLM_RETRY_DELAY_MS,
    ));
    let max_total_tokens = parse_env_u64(
        "OMNE_AGENT_MAX_TOTAL_TOKENS",
        DEFAULT_MAX_TOTAL_TOKENS,
        0,
        MAX_MAX_TOTAL_TOKENS,
    );
    let auto_summary_threshold_pct = parse_env_u64(
        "OMNE_AGENT_AUTO_SUMMARY_THRESHOLD_PCT",
        DEFAULT_AUTO_SUMMARY_THRESHOLD_PCT,
        1,
        MAX_AUTO_SUMMARY_THRESHOLD_PCT,
    );
    let auto_summary_source_max_chars = parse_env_usize(
        "OMNE_AGENT_AUTO_SUMMARY_SOURCE_MAX_CHARS",
        DEFAULT_AUTO_SUMMARY_SOURCE_MAX_CHARS,
        1,
        MAX_AUTO_SUMMARY_SOURCE_MAX_CHARS,
    );
    let auto_summary_tail_items = parse_env_usize(
        "OMNE_AGENT_AUTO_SUMMARY_TAIL_ITEMS",
        DEFAULT_AUTO_SUMMARY_TAIL_ITEMS,
        0,
        MAX_AUTO_SUMMARY_TAIL_ITEMS,
    );
    let mut parallel_tool_calls = parse_env_bool("OMNE_AGENT_PARALLEL_TOOL_CALLS", false);
    let mut max_parallel_tool_calls = parse_env_usize(
        "OMNE_AGENT_MAX_PARALLEL_TOOL_CALLS",
        DEFAULT_MAX_PARALLEL_TOOL_CALLS,
        1,
        MAX_MAX_PARALLEL_TOOL_CALLS,
    );
    let response_format = match std::env::var("OMNE_AGENT_RESPONSE_FORMAT_JSON") {
        Ok(raw) => {
            let raw = raw.trim();
            if raw.is_empty() {
                None
            } else {
                Some(
                    serde_json::from_str::<ditto_llm::ResponseFormat>(raw)
                        .context("parse OMNE_AGENT_RESPONSE_FORMAT_JSON")?,
                )
            }
        }
        Err(_) => None,
    };

    if response_format.is_some() && !provider_capabilities.json_schema {
        thread_rt
            .emit_item_delta_warning_once(
                format!("json_schema_unsupported:{provider}"),
                thread_id,
                turn_id,
                format!(
                    "response_format is set, but provider does not advertise json_schema support; request will be forwarded but may be ignored or error (provider={provider})"
                ),
            )
            .await;
    }

    let mut instructions = DEFAULT_INSTRUCTIONS.to_string();

    if let Some(user_instructions_path) = resolve_user_instructions_path() {
        if let Ok(contents) = tokio::fs::read_to_string(&user_instructions_path).await {
            let contents = omne_core::redact_text(&contents);
            instructions.push_str("\n\n# User instructions\n\n");
            instructions.push_str(&format!(
                "_Source: {}_\n\n",
                user_instructions_path.display()
            ));
            instructions.push_str(&contents);
        }
    }

    if let Some(cwd) = thread_cwd.as_deref() {
        let agents_path = PathBuf::from(cwd).join("AGENTS.md");
        if let Ok(contents) = tokio::fs::read_to_string(&agents_path).await {
            let contents = omne_core::redact_text(&contents);
            instructions.push_str("\n\n# Project instructions (AGENTS.md)\n\n");
            instructions.push_str(&contents);
        }
    }

    if let Some(skills) = load_skills_from_input(&input, thread_cwd.as_deref()).await? {
        instructions.push_str(&skills);
    }

    let directives = load_turn_directives(&server, thread_id, turn_id)
        .await
        .unwrap_or_default();
    let has_plan_directive = directives
        .iter()
        .any(|directive| matches!(directive, omne_protocol::TurnDirective::Plan));
    (parallel_tool_calls, max_parallel_tool_calls) = apply_plan_parallel_tool_call_overrides(
        has_plan_directive,
        parallel_tool_calls,
        max_parallel_tool_calls,
    );
    if has_plan_directive {
        instructions.push_str("\n\n# Turn directive (/plan)\n\n");
        instructions.push_str(
            "This turn is planning-oriented. Produce a concrete execution plan and avoid side effects or destructive actions unless the user explicitly overrides this intent.\n",
        );
    }

    let session_start_hook_contexts =
        super::run_session_start_hooks(server.as_ref(), thread_id, turn_id).await;
    if !session_start_hook_contexts.is_empty() {
        instructions.push_str("\n\n# Additional context (hooks/session_start)\n\n");
        for ctx in &session_start_hook_contexts {
            if let Some(summary) = ctx.summary.as_deref() {
                instructions.push_str(&format!("## {}\n\n", summary.trim()));
            }
            instructions.push_str(ctx.text.trim());
            instructions.push_str("\n\n");
        }
    }

    let mut input_items = build_conversation(&server, thread_id).await?;
    if let Ok(context_refs) = load_turn_context_refs(&server, thread_id, turn_id).await {
        if !context_refs.is_empty() {
            let ctx_items =
                context_refs_to_messages(&server, thread_id, turn_id, &context_refs, cancel.clone())
                    .await;
            match ctx_items {
                Ok(ctx_items) => insert_context_before_last_user_message(&mut input_items, ctx_items),
                Err(err) => {
                    input_items.push(serde_json::json!({
                        "type": "message",
                        "role": "system",
                        "content": [{
                            "type": "input_text",
                            "text": format!("[context_refs] failed to resolve: {}", err),
                        }]
                    }));
                }
            }
        }
    }

    let attachments = load_turn_attachments(&server, thread_id, turn_id).await?;
    let max_attachments = parse_env_usize(
        "OMNE_AGENT_MAX_ATTACHMENTS",
        DEFAULT_AGENT_MAX_ATTACHMENTS,
        0,
        MAX_AGENT_MAX_ATTACHMENTS,
    );
    if max_attachments > 0 && attachments.len() > max_attachments {
        anyhow::bail!(
            "too many attachments: count={} max={}",
            attachments.len(),
            max_attachments
        );
    }
    let max_attachment_bytes = parse_env_u64(
        "OMNE_AGENT_MAX_ATTACHMENT_BYTES",
        DEFAULT_AGENT_MAX_ATTACHMENT_BYTES,
        0,
        MAX_AGENT_MAX_ATTACHMENT_BYTES,
    );
    let pdf_file_id_upload_min_bytes = parse_env_u64(
        "OMNE_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES",
        DEFAULT_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES,
        0,
        MAX_AGENT_PDF_FILE_ID_UPLOAD_MIN_BYTES,
    );
    let resolved_attachments = if attachments.is_empty() {
        Vec::new()
    } else {
        resolve_turn_attachments(
            thread_root.as_deref(),
            thread_mode.as_str(),
            allowed_tools.as_deref(),
            &attachments,
            max_attachment_bytes,
        )
        .await?
    };
    let context_tokens_estimate = estimate_context_tokens(&instructions, &input_items);

    let router_config = match thread_root.as_deref() {
        Some(thread_root) => omne_core::router::load_router_config(thread_root).await?,
        None => None,
    };
    let route_role = resolve_turn_role_for_routing(has_plan_directive, &thread_mode);
    let route_plan = omne_core::router::plan_route(
        router_config.as_ref().map(|loaded| &loaded.config),
        omne_core::router::RouteIntent {
            role: Some(route_role),
            input: &input,
            global_default_model: &global_default_model,
            forced: forced_model,
            context_tokens_estimate,
        },
    );

    let final_model = route_plan.selected_model.clone();
    if !model_allowed_by_whitelist(&final_model, &provider_config.model_whitelist) {
        anyhow::bail!(
            "model not allowed by provider whitelist: provider={provider} model={final_model}"
        );
    }

    let tool_model = if forced_model {
        None
    } else {
        std::env::var("OMNE_AGENT_TOOL_MODEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let tool_model = tool_model.filter(|candidate| candidate != &final_model);
    if let Some(tool_model) = tool_model.as_deref() {
        if !model_allowed_by_whitelist(tool_model, &provider_config.model_whitelist) {
            anyhow::bail!(
                "tool model not allowed by provider whitelist: provider={provider} model={tool_model}"
            );
        }
    }

    let model_fallbacks = std::env::var("OMNE_AGENT_FALLBACK_MODELS")
        .ok()
        .map(|value| parse_csv_list(&value))
        .unwrap_or_default();

    let reason = route_plan
        .reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{value}; provider={provider}"))
        .or_else(|| Some(format!("provider={provider}")));

    let _ = thread_rt
        .append_event(ThreadEventKind::ModelRouted {
            turn_id,
            selected_model: final_model.clone(),
            rule_source: route_plan.rule_source,
            reason,
            rule_id: route_plan.rule_id.clone(),
        })
        .await;

    if let Some(tool_model) = tool_model.as_ref() {
        let reason = format!("tool_model: from={final_model} to={tool_model}; provider={provider}");
        let _ = thread_rt
            .append_event(ThreadEventKind::ModelRouted {
                turn_id,
                selected_model: tool_model.clone(),
                rule_source: route_plan.rule_source,
                reason: Some(reason),
                rule_id: route_plan.rule_id.clone(),
            })
            .await;
    }
    let model_config = ditto_llm::select_model_config(&project_overrides.models, &final_model);
    let limits = resolve_model_limits(&final_model, model_config);
    let starting_total_tokens_used =
        match thread_total_tokens_used(&server.thread_store, thread_id).await {
            Ok(total) => total,
            Err(err) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    error = %err,
                    "failed to compute total token usage"
                );
                0
            }
        };
    let cfg = ToolLoopConfig {
        max_agent_steps,
        max_tool_calls,
        max_turn_duration,
        max_openai_request_duration,
        llm_max_attempts,
        llm_retry_base_delay,
        llm_retry_max_delay,
        max_total_tokens,
        starting_total_tokens_used,
        auto_compact_token_limit: limits.auto_compact_token_limit,
        auto_summary_threshold_pct,
        auto_summary_source_max_chars,
        auto_summary_tail_items,
        parallel_tool_calls,
        max_parallel_tool_calls,
        response_format,
        show_thinking,
    };

    let ToolLoopOutcome {
        model,
        last_response_id,
        last_usage,
        last_text,
    } = ToolLoop {
        server: server.clone(),
        thread_rt: thread_rt.clone(),
        thread_id,
        turn_id,
        cancel,
        turn_priority,
        final_model,
        provider,
        provider_candidates,
        provider_cache,
        provider_config,
        project_overrides,
        base_url_override,
        env,
        tools,
        instructions,
        turn_input: input,
        input_items,
        tool_model,
        model_fallbacks,
        model_client,
        resolved_attachments,
        pdf_file_id_upload_min_bytes,
        rule_source: route_plan.rule_source,
        rule_id: route_plan.rule_id,
        cfg,
    }
    .run()
    .await?;

    if !last_text.is_empty() {
        let _ = thread_rt
            .append_event(ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id),
                text: last_text.clone(),
                model: Some(model),
                response_id: Some(last_response_id),
                token_usage: last_usage,
            })
            .await;
    }

    let _ =
        write_plan_artifact_if_needed(&server, thread_id, turn_id, has_plan_directive, &last_text)
            .await;

    Ok(())
}

struct AutoCompactSummaryContext<'a> {
    server: &'a super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    model: &'a str,
    llm: Arc<dyn ditto_llm::LanguageModel>,
    turn_priority: TurnPriority,
    max_openai_request_duration: Duration,
    max_total_tokens: u64,
    total_tokens_used: &'a mut u64,
    input_items: &'a mut Vec<OpenAiItem>,
}

#[derive(Clone, Copy)]
struct AutoCompactSummaryConfig {
    source_max_chars: usize,
    tail_items: usize,
}
