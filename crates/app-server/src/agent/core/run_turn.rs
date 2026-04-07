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

fn resolve_role_permission_mode_name(
    role_name: &str,
    role_catalog: &omne_core::roles::RoleCatalog,
) -> Option<String> {
    role_catalog
        .permission_mode_name(role_name)
        .map(ToString::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RoleInputDirective {
    role_name: String,
    content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleDirectiveHandling {
    InjectIntoSystem,
    AutoCompactThenUser,
    UserMessage,
}

fn resolve_role_directive_handling(
    is_first_turn: bool,
    near_compaction: bool,
) -> RoleDirectiveHandling {
    if is_first_turn {
        RoleDirectiveHandling::InjectIntoSystem
    } else if near_compaction {
        RoleDirectiveHandling::AutoCompactThenUser
    } else {
        RoleDirectiveHandling::UserMessage
    }
}

fn parse_role_input_directive(
    input: &str,
    role_catalog: &omne_core::roles::RoleCatalog,
) -> Option<RoleInputDirective> {
    let body = input.strip_prefix("@{")?;
    let close_idx = body.find('}')?;
    let role_name = body[..close_idx].trim();
    if role_name.is_empty() || role_catalog.role(role_name).is_none() {
        return None;
    }

    let suffix = &body[(close_idx + 1)..];
    let mut suffix_chars = suffix.chars();
    let first = suffix_chars.next()?;
    if !first.is_whitespace() {
        return None;
    }
    let content = suffix_chars.as_str().trim_start();
    if content.is_empty() {
        return None;
    }

    Some(RoleInputDirective {
        role_name: role_name.to_string(),
        content: content.to_string(),
    })
}

fn append_mode_scenario_prompt(
    instructions: &mut String,
    mode_name: &str,
    mode_catalog: &omne_core::modes::ModeCatalog,
) {
    let Some(mode) = mode_catalog.mode(mode_name) else {
        return;
    };
    let description = mode.description.trim();
    if description.is_empty() {
        return;
    }

    instructions.push_str("\n\n# Scenario mode\n\n");
    instructions.push_str(&format!("Active mode: `{mode_name}`\n\n{description}\n"));
}

fn append_role_identity_prompt(
    instructions: &mut String,
    role_name: &str,
    role_catalog: &omne_core::roles::RoleCatalog,
    mode_catalog: &omne_core::modes::ModeCatalog,
) {
    let permission_mode_name = resolve_role_permission_mode_name(role_name, role_catalog);
    let role_description = permission_mode_name
        .as_deref()
        .and_then(|mode_name| mode_catalog.mode(mode_name))
        .map(|mode| mode.description.trim())
        .filter(|description| !description.is_empty());

    instructions.push_str("\n\n# Role identity\n\n");
    instructions.push_str(&format!("Active role: `{role_name}`\n"));
    if let Some(description) = role_description {
        instructions.push('\n');
        instructions.push_str(description);
        instructions.push('\n');
    }
}

fn replace_latest_user_message_text(input_items: &mut [OpenAiItem], new_text: &str) -> bool {
    for item in input_items.iter_mut().rev() {
        if item.get("type").and_then(Value::as_str) != Some("message")
            || item.get("role").and_then(Value::as_str) != Some("user")
        {
            continue;
        }

        let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts.iter_mut() {
            if part.get("type").and_then(Value::as_str) != Some("input_text") {
                continue;
            }
            if let Some(obj) = part.as_object_mut() {
                obj.insert("text".to_string(), Value::String(new_text.to_string()));
                return true;
            }
        }
    }

    false
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

struct BuiltSystemPrompt {
    text: String,
    sha256: String,
    source: String,
}

fn system_prompt_sha256(text: &str) -> String {
    omne_integrity_primitives::hash_sha256(text.as_bytes()).to_string()
}

async fn build_system_prompt_from_sources(thread_cwd: Option<&str>) -> BuiltSystemPrompt {
    let mut instructions = DEFAULT_INSTRUCTIONS.to_string();
    let mut sources = vec!["default".to_string()];

    if let Some(user_instructions_path) = resolve_user_instructions_path() {
        if let Ok(contents) = tokio::fs::read_to_string(&user_instructions_path).await {
            let contents = omne_core::redact_text(&contents);
            instructions.push_str("\n\n# User instructions\n\n");
            instructions.push_str(&format!(
                "_Source: {}_\n\n",
                user_instructions_path.display()
            ));
            instructions.push_str(&contents);
            sources.push(format!(
                "user_instructions:{}",
                user_instructions_path.display()
            ));
        }
    }

    if let Some(cwd) = thread_cwd {
        let agents_path = PathBuf::from(cwd).join("AGENTS.md");
        if let Ok(contents) = tokio::fs::read_to_string(&agents_path).await {
            let contents = omne_core::redact_text(&contents);
            instructions.push_str("\n\n# Project instructions (AGENTS.md)\n\n");
            instructions.push_str(&contents);
            sources.push(format!("project_agents:{}", agents_path.display()));
        }
    }

    BuiltSystemPrompt {
        sha256: system_prompt_sha256(&instructions),
        text: instructions,
        source: sources.join(" | "),
    }
}

async fn resolve_or_persist_thread_system_prompt_snapshot(
    thread_rt: &Arc<super::ThreadRuntime>,
    thread_cwd: Option<&str>,
) -> anyhow::Result<String> {
    let (existing_sha256, existing_text) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.system_prompt_sha256.clone(),
            state.system_prompt_text.clone(),
        )
    };

    match (existing_sha256, existing_text) {
        (Some(saved_sha256), Some(saved_text)) => {
            if system_prompt_sha256(&saved_text) != saved_sha256 {
                anyhow::bail!("thread system prompt snapshot is corrupted (hash mismatch)");
            }
            Ok(saved_text)
        }
        (Some(saved_sha256), None) => {
            let built = build_system_prompt_from_sources(thread_cwd).await;
            if saved_sha256 != built.sha256 {
                anyhow::bail!(
                    "thread system prompt snapshot text is missing and current sources no longer match sha256={}",
                    saved_sha256
                );
            }
            Ok(built.text)
        }
        (None, _) => {
            let built = build_system_prompt_from_sources(thread_cwd).await;
            thread_rt
                .append_event(ThreadEventKind::ThreadSystemPromptSnapshot {
                    prompt_sha256: built.sha256,
                    prompt_text: built.text.clone(),
                    source: Some(built.source),
                })
                .await?;
            Ok(built.text)
        }
    }
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
        thread_role,
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
            state.role.clone(),
            state.model.clone(),
            state.openai_base_url.clone(),
            state.show_thinking,
            state.cwd.clone(),
            state.allowed_tools.clone(),
        )
    };

    let thread_root = match thread_cwd.as_deref() {
        Some(thread_cwd) => {
            Some(omne_core::resolve_dir(Path::new(thread_cwd), Path::new(".")).await?)
        }
        None => None,
    };
    let mode_catalog = if let Some(thread_root) = thread_root.as_deref() {
        omne_core::modes::ModeCatalog::load(thread_root).await
    } else {
        omne_core::modes::ModeCatalog::builtin()
    };
    let role_catalog = omne_core::roles::RoleCatalog::builtin();
    let effective_allowed_tools = if thread_root.is_some() {
        let role_permission_mode_name =
            resolve_role_permission_mode_name(&thread_role, &role_catalog)
                .unwrap_or_else(|| thread_mode.clone());
        match (
            mode_catalog.mode(&thread_mode),
            mode_catalog.mode(&role_permission_mode_name),
        ) {
            (Some(mode), Some(role_permission_mode)) => Some(
                omne_core::allowed_tools::effective_permissions_for_mode_and_role(
                    mode,
                    role_permission_mode,
                    allowed_tools.as_deref(),
                ),
            ),
            _ => allowed_tools.clone(),
        }
    } else {
        allowed_tools.clone()
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

    let mode_show_thinking = mode_catalog
        .mode(&thread_mode)
        .and_then(|mode| mode.ui.show_thinking);
    let show_thinking = thread_show_thinking
        .or(mode_show_thinking)
        .or(project_ui.show_thinking)
        .unwrap_or(true);

    let directives = load_turn_directives(&server, thread_id, turn_id)
        .await
        .unwrap_or_default();
    let has_plan_directive = directives
        .iter()
        .any(|directive| matches!(directive, omne_protocol::TurnDirective::Plan));

    let route_role = resolve_turn_role_for_routing(has_plan_directive, &thread_mode);
    let route_scenario_override = std::env::var("OMNE_OPENAI_ROUTING_SCENARIO")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let route_scenario = route_scenario_override
        .as_deref()
        .or(Some(thread_mode.as_str()));
    let route_seed_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        thread_id.hash(&mut hasher);
        route_role.hash(&mut hasher);
        hasher.finish()
    };

    let legacy_primary_provider = project_overrides
        .provider
        .clone()
        .or_else(|| {
            std::env::var("OMNE_OPENAI_PROVIDER")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| crate::project_config::default_openai_provider_name().to_string());
    let legacy_fallback_providers = std::env::var("OMNE_OPENAI_FALLBACK_PROVIDERS")
        .ok()
        .map(|value| parse_csv_list(&value))
        .unwrap_or_else(|| project_overrides.fallback_providers.clone());
    let env_model_override = std::env::var("OMNE_OPENAI_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let legacy_default_model = project_overrides
        .model
        .clone()
        .or_else(|| env_model_override.clone())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let base_url_override = thread_openai_base_url
        .clone()
        .or(project_overrides.base_url.clone())
        .or_else(|| {
            std::env::var("OMNE_OPENAI_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });

    let route_selection = resolve_provider_route_selection(
        &project_overrides,
        route_role,
        route_scenario,
        Some(route_seed_hash),
        &legacy_primary_provider,
        legacy_fallback_providers,
        &legacy_default_model,
        base_url_override.as_deref(),
    )?;
    let completion_primary_target = route_selection
        .completion_targets
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("provider routing resolved no completion target"))?;

    let forced_model = thread_model.is_some();
    let global_default_model = if forced_model {
        thread_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string())
    } else if let Some(model) = env_model_override.clone() {
        model
    } else if route_selection.explicit {
        completion_primary_target.model.clone()
    } else {
        legacy_default_model.clone()
    };

    let env = ditto_core::config::Env {
        dotenv: std::mem::take(&mut project_overrides.dotenv),
    };

    // Cache provider runtimes across turns to keep HTTP connections sticky. Some OpenAI-compatible
    // gateways implement prompt caching per-backend-instance, so new TCP connections can lead to
    // cache misses even with stable `prompt_cache_key`.
    let provider_runtime_cache_key = provider_runtime_cache_key(&completion_primary_target, &env);
    let cached_provider_runtime = {
        let cache = server.provider_runtimes.lock().await;
        cache.get(&provider_runtime_cache_key).cloned()
    };
    let provider_runtime = match cached_provider_runtime {
        Some(runtime) => runtime,
        None => {
            let runtime = build_provider_runtime(&completion_primary_target, &env).await?;
            let mut cache = server.provider_runtimes.lock().await;
            cache.insert(provider_runtime_cache_key, runtime.clone());
            runtime
        }
    };
    let model_client = provider_runtime.client.clone();

    let primary_provider = completion_primary_target.provider.clone();
    let completion_provider_candidates = route_selection.completion_targets.clone();
    let mut thinking_provider_candidates = if route_selection.thinking_targets.is_empty() {
        completion_provider_candidates.clone()
    } else {
        route_selection.thinking_targets.clone()
    };
    let mut provider_cache = std::collections::BTreeMap::<String, ProviderRuntime>::new();
    provider_cache.insert(
        completion_primary_target.id.clone(),
        provider_runtime.clone(),
    );

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
    let auto_compact_threshold_pct = parse_env_u64(
        "OMNE_AGENT_AUTO_SUMMARY_THRESHOLD_PCT",
        crate::model_limits::DEFAULT_AUTO_COMPACT_THRESHOLD_PCT,
        1,
        crate::model_limits::MAX_AUTO_COMPACT_THRESHOLD_PCT,
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
                    serde_json::from_str::<ditto_core::provider_options::ResponseFormat>(raw)
                        .context("parse OMNE_AGENT_RESPONSE_FORMAT_JSON")?,
                )
            }
        }
        Err(_) => None,
    };

    if response_format.is_some() && !provider_runtime.capabilities.json_schema {
        thread_rt
            .emit_item_delta_warning_once(
                format!("json_schema_unsupported:{primary_provider}"),
                thread_id,
                turn_id,
                format!(
                    "response_format is set, but provider does not advertise json_schema support; request will be forwarded but may be ignored or error (provider={primary_provider})"
                ),
            )
            .await;
    }

    if let Some(effective_base_url) = provider_runtime.config.base_url.as_deref()
        && let Some(warning) = crate::project_config::openai_provider_base_url_override_warning(
            &primary_provider,
            effective_base_url,
        )
    {
        thread_rt
            .emit_item_delta_warning_once(warning.code, thread_id, turn_id, warning.message)
            .await;
    }

    let mut turn_input = input;
    let role_input_directive = parse_role_input_directive(&turn_input, &role_catalog);
    if let Some(directive) = role_input_directive.as_ref() {
        turn_input = directive.content.clone();
    }

    let mut instructions =
        resolve_or_persist_thread_system_prompt_snapshot(&thread_rt, thread_cwd.as_deref()).await?;
    append_mode_scenario_prompt(&mut instructions, &thread_mode, &mode_catalog);

    if let Some(skills) = load_skills_from_input(&turn_input, thread_cwd.as_deref()).await? {
        instructions.push_str(&skills);
    }

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
            let ctx_items = context_refs_to_messages(
                &server,
                thread_id,
                turn_id,
                &context_refs,
                cancel.clone(),
            )
            .await;
            match ctx_items {
                Ok(ctx_items) => {
                    insert_context_before_last_user_message(&mut input_items, ctx_items)
                }
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
    if role_input_directive.is_some() {
        let _ = replace_latest_user_message_text(&mut input_items, &turn_input);
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
            effective_allowed_tools.as_deref(),
            &attachments,
            max_attachment_bytes,
        )
        .await?
    };
    let mut starting_total_tokens_used =
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

    let pre_route_model_config =
        ditto_core::config::select_model_config(&project_overrides.models, &global_default_model);
    let pre_route_limits = resolve_model_limits(&global_default_model, pre_route_model_config);
    let pre_route_auto_compact_token_limit =
        crate::model_limits::effective_auto_compact_token_limit(
            pre_route_limits.context_window,
            pre_route_limits.auto_compact_token_limit,
            auto_compact_threshold_pct,
        );

    let mut context_tokens_estimate = estimate_context_tokens(&instructions, &input_items);
    if let Some(directive) = role_input_directive.as_ref() {
        let turn_count = load_turn_started_count(&server, thread_id).await?;
        let is_first_turn = turn_count <= 1;
        let near_compaction =
            should_auto_compact(context_tokens_estimate, pre_route_auto_compact_token_limit);
        match resolve_role_directive_handling(is_first_turn, near_compaction) {
            RoleDirectiveHandling::InjectIntoSystem => {
                append_role_identity_prompt(
                    &mut instructions,
                    &directive.role_name,
                    &role_catalog,
                    &mode_catalog,
                );
                context_tokens_estimate = estimate_context_tokens(&instructions, &input_items);
            }
            RoleDirectiveHandling::AutoCompactThenUser => {
                let summary_cfg = AutoCompactSummaryConfig {
                    source_max_chars: auto_summary_source_max_chars,
                    tail_items: auto_summary_tail_items,
                };
                let ctx = AutoCompactSummaryContext {
                    server: &server,
                    thread_id,
                    turn_id,
                    model: &global_default_model,
                    llm: model_client.clone(),
                    turn_priority,
                    max_openai_request_duration,
                    max_total_tokens,
                    total_tokens_used: &mut starting_total_tokens_used,
                    input_items: &mut input_items,
                };
                let did_auto_summary = auto_compact_summary(ctx, summary_cfg).await?;
                if did_auto_summary {
                    context_tokens_estimate = estimate_context_tokens(&instructions, &input_items);
                }
            }
            RoleDirectiveHandling::UserMessage => {}
        }
    }

    let router_config = match thread_root.as_deref() {
        Some(thread_root) => omne_core::router::load_router_config(thread_root).await?,
        None => None,
    };
    let route_plan = omne_core::router::plan_route(
        router_config.as_ref().map(|loaded| &loaded.config),
        omne_core::router::RouteIntent {
            role: Some(route_role),
            input: &turn_input,
            global_default_model: &global_default_model,
            forced: forced_model,
            context_tokens_estimate,
        },
    );

    let final_model = route_plan.selected_model.clone();
    let env_tool_model_override = std::env::var("OMNE_AGENT_TOOL_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let thinking_primary_model = route_selection
        .thinking_targets
        .first()
        .map(|target| target.model.clone());
    let tool_model = if forced_model {
        None
    } else {
        env_tool_model_override.or(thinking_primary_model)
    };
    if tool_model.is_none() {
        thinking_provider_candidates.clear();
    }

    let effective_tool_schema_model = tool_model.as_deref().unwrap_or(final_model.as_str());
    let tool_specs = build_tools_for_turn(
        effective_allowed_tools.as_deref(),
        Some(effective_tool_schema_model),
        Some(thread_role.as_str()),
        thread_root.as_deref(),
    );
    let tool_count = tool_specs.len();
    let tool_schema_bytes = tool_specs_total_json_bytes(&tool_specs);
    tracing::info!(
        thread_id = %thread_id,
        turn_id = %turn_id,
        model = %effective_tool_schema_model,
        tool_count,
        tool_schema_bytes,
        "prepared tool schemas"
    );
    let tools = tool_specs_to_ditto_tools(&tool_specs).context("parse tool schemas")?;

    let env_model_fallbacks = std::env::var("OMNE_AGENT_FALLBACK_MODELS")
        .ok()
        .map(|value| parse_csv_list(&value))
        .unwrap_or_default();
    let completion_model_fallbacks = ditto_core::config::normalize_string_list({
        let mut values = route_selection.completion_model_fallbacks.clone();
        values.extend(env_model_fallbacks.clone());
        values
    });
    let thinking_model_fallbacks = ditto_core::config::normalize_string_list({
        let mut values = if route_selection.thinking_model_fallbacks.is_empty() {
            completion_model_fallbacks.clone()
        } else {
            route_selection.thinking_model_fallbacks.clone()
        };
        values.extend(env_model_fallbacks);
        values
    });

    let reason = route_plan
        .reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{value}; provider={primary_provider}"))
        .or_else(|| Some(format!("provider={primary_provider}")));

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
        let tool_provider = thinking_provider_candidates
            .first()
            .map(|target| target.provider.as_str())
            .unwrap_or(primary_provider.as_str());
        let reason =
            format!("tool_model: from={final_model} to={tool_model}; provider={tool_provider}");
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
    let model_config = ditto_core::config::select_model_config(&project_overrides.models, &final_model);
    let limits = resolve_model_limits(&final_model, model_config);
    let auto_compact_token_limit = crate::model_limits::effective_auto_compact_token_limit(
        limits.context_window,
        limits.auto_compact_token_limit,
        auto_compact_threshold_pct,
    );
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
        auto_compact_token_limit,
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
        completion_provider_candidates,
        thinking_provider_candidates,
        provider_cache,
        model_configs: project_overrides.models.clone(),
        env,
        tools,
        instructions,
        turn_input,
        input_items,
        tool_model,
        completion_model_fallbacks,
        thinking_model_fallbacks,
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
    llm: Arc<dyn ditto_core::llm_core::model::LanguageModel>,
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
