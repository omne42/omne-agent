    impl UiState {
        fn new(include_archived: bool) -> Self {
            Self {
                include_archived,
                threads: Vec::new(),
                selected_thread: 0,
                active_thread: None,
                header: HeaderState::default(),
                header_needs_refresh: false,
                overlays: Vec::new(),
                inline_palette: None,
                last_seq: 0,
                transcript: VecDeque::new(),
                transcript_scroll: 0,
                transcript_follow: true,
                transcript_max_scroll: 0,
                transcript_viewport_height: 0,
                tool_events: HashMap::new(),
                streaming: None,
                active_turn_id: None,
                input: String::new(),
                status: None,
                total_tokens_used: 0,
                counted_usage_responses: HashSet::new(),
                skip_token_usage_before_seq: None,
                pending_action: None,
                model_fetch: None,
                model_fetch_pending: false,
                model_list: Vec::new(),
                model_list_loaded: false,
                thread_cwd: None,
                mode_catalog: Vec::new(),
                mode_catalog_loaded: false,
                skill_catalog: Vec::new(),
                skill_catalog_loaded: false,
                turn_start: None,
            }
        }

        fn activate_thread(&mut self, thread_id: ThreadId, last_seq: u64) {
            let since_seq = last_seq.saturating_sub(2_000);
            self.reset_thread_state(thread_id, since_seq);
        }

        async fn refresh_threads(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let value = app.thread_list_meta(self.include_archived).await?;
            let parsed =
                serde_json::from_value::<ThreadListMetaResponse>(value).context("parse threads")?;
            self.threads = parsed.threads;
            if self.selected_thread >= self.threads.len() {
                self.selected_thread = self.threads.len().saturating_sub(1);
            }
            Ok(())
        }

        async fn open_thread(
            &mut self,
            app: &mut super::App,
            thread_id: ThreadId,
        ) -> anyhow::Result<()> {
            let resume = app.thread_resume(thread_id).await?;
            let last_seq = resume["last_seq"].as_u64().unwrap_or(0);
            let since_seq = last_seq.saturating_sub(2_000);

            let resp = app
                .thread_subscribe(thread_id, since_seq, Some(2_000), Some(0))
                .await
                .context("subscribe initial events")?;

            self.reset_thread_state(thread_id, resp.last_seq);
            if let Ok(state) = app.thread_state(thread_id).await {
                self.thread_cwd = state
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                if let Some(tokens) = state.get("total_tokens_used").and_then(|v| v.as_u64()) {
                    self.total_tokens_used = tokens;
                    self.skip_token_usage_before_seq = Some(resp.last_seq);
                }
            }
            for event in resp.events {
                self.apply_event(&event);
            }
            if let Err(err) = self.refresh_header(app, thread_id).await {
                self.set_status(format!("header refresh error: {err}"));
            } else {
                self.header_needs_refresh = false;
            }
            Ok(())
        }

        fn reset_thread_state(&mut self, thread_id: ThreadId, last_seq: u64) {
            self.active_thread = Some(thread_id);
            self.header = HeaderState::default();
            self.header_needs_refresh = true;
            self.overlays.clear();
            self.inline_palette = None;
            self.last_seq = last_seq;
            self.transcript.clear();
            self.transcript_scroll = 0;
            self.transcript_follow = true;
            self.transcript_max_scroll = 0;
            self.transcript_viewport_height = 0;
            self.tool_events.clear();
            self.streaming = None;
            self.active_turn_id = None;
            self.total_tokens_used = 0;
            self.counted_usage_responses.clear();
            self.skip_token_usage_before_seq = None;
            self.pending_action = None;
            self.cancel_model_fetch();
            self.cancel_turn_start();
            self.model_list.clear();
            self.model_list_loaded = false;
            self.thread_cwd = None;
            self.mode_catalog.clear();
            self.mode_catalog_loaded = false;
            self.skill_catalog.clear();
            self.skill_catalog_loaded = false;
        }

        async fn refresh_header(
            &mut self,
            app: &mut super::App,
            thread_id: ThreadId,
        ) -> anyhow::Result<()> {
            let config = app.thread_config_explain(thread_id).await?;

            self.header.mode = config
                .get("effective")
                .and_then(|v| v.get("mode"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            self.header.provider = Self::extract_openai_provider(&config);
            self.header.model = config
                .get("effective")
                .and_then(|v| v.get("model"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            self.header.thinking = config
                .get("effective")
                .and_then(|v| v.get("thinking"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            self.header.model_context_window = config
                .get("effective")
                .and_then(|v| v.get("model_context_window"))
                .and_then(|v| v.as_u64());

            self.header.mcp_enabled = env_truthy("CODE_PM_ENABLE_MCP");
            Ok(())
        }

        fn extract_openai_provider(config: &Value) -> Option<String> {
            let layers = config.get("layers")?.as_array()?;
            for layer in layers.iter().rev() {
                let provider = layer.get("openai_provider").and_then(|v| v.as_str());
                if let Some(provider) = provider.map(str::trim).filter(|s| !s.is_empty()) {
                    return Some(provider.to_string());
                }
            }
            None
        }

        fn apply_event(&mut self, event: &ThreadEvent) {
            match &event.kind {
                ThreadEventKind::TurnStarted { turn_id, input, .. } => {
                    self.cancel_turn_start();
                    self.active_turn_id = Some(*turn_id);
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::User,
                        text: input.clone(),
                    });
                }
                ThreadEventKind::AssistantMessage {
                    turn_id,
                    text,
                    response_id,
                    token_usage,
                    ..
                } => {
                    self.record_token_usage(
                        response_id.as_deref(),
                        token_usage.as_ref(),
                        event.seq.0,
                    );
                    let mut streamed = None::<String>;
                    if let Some(turn_id) = turn_id
                        && self.streaming.as_ref().is_some_and(|s| s.turn_id == *turn_id)
                    {
                        streamed = self.streaming.as_ref().map(|s| s.text.clone());
                        self.streaming = None;
                    }
                    if let Some(streamed) = streamed {
                        let streamed_trim = streamed.trim();
                        let final_trim = text.trim();
                        if !streamed_trim.is_empty()
                            && (final_trim.is_empty() || !final_trim.starts_with(streamed_trim))
                        {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Assistant,
                                text: streamed,
                            });
                        }
                    }
                    if !text.trim().is_empty() {
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Assistant,
                            text: text.clone(),
                        });
                    }
                }
                ThreadEventKind::AgentStep {
                    response_id,
                    token_usage,
                    ..
                } => {
                    self.record_token_usage(
                        Some(response_id.as_str()),
                        token_usage.as_ref(),
                        event.seq.0,
                    );
                }
                ThreadEventKind::ModelRouted {
                    selected_model,
                    rule_source,
                    reason,
                    ..
                } => {
                    self.header.model = Some(selected_model.clone());
                    let mut line = format!(
                        "[model] {selected_model} ({})",
                        model_routing_rule_source_str(*rule_source)
                    );
                    if let Some(reason) = reason.as_deref().filter(|s| !s.trim().is_empty()) {
                        line.push_str(&format!(" - {reason}"));
                    }
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::System,
                        text: line,
                    });
                }
                ThreadEventKind::TurnCompleted { turn_id, status, .. } => {
                    if self.active_turn_id == Some(*turn_id) {
                        self.active_turn_id = None;
                    }
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::System,
                        text: format!("[turn] {turn_id} {}", turn_status_str(*status)),
                    });
                }
                ThreadEventKind::ToolStarted { tool_id, tool, params, .. } => {
                    self.tool_events.insert(*tool_id, tool.clone());
                    if !should_suppress_tool_started(tool) {
                        if let Some(line) = format_tool_started_line(tool, params.as_ref()) {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Tool,
                                text: line,
                            });
                        }
                    }
                }
                ThreadEventKind::ToolCompleted {
                    tool_id,
                    status,
                    error,
                    result,
                    ..
                } => {
                    let info = self.tool_events.remove(tool_id);
                    let name = info.as_deref().unwrap_or("tool");
                    if *status == ToolStatus::Completed {
                        if should_suppress_tool_completed(name)
                            || should_suppress_tool_started(name)
                        {
                            return;
                        }
                        if let Some(result) = result.as_ref().filter(|v| !v.is_null())
                            && let Some(line) = format_tool_result_line(name, result)
                        {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Tool,
                                text: line,
                            });
                        }
                    } else {
                        let mut line = format!("{name} {}", tool_status_str(*status));
                        if let Some(err) = error.as_deref().filter(|s| !s.trim().is_empty()) {
                            line.push_str(": ");
                            line.push_str(err);
                        }
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Error,
                            text: line,
                        });
                    }
                }
                ThreadEventKind::ProcessStarted { argv, cwd, .. } => {
                    if let Some(line) =
                        format_process_started_line(argv, cwd, self.thread_cwd.as_deref())
                    {
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Tool,
                            text: line,
                        });
                    }
                }
                ThreadEventKind::ThreadConfigUpdated {
                    mode,
                    model,
                    thinking,
                    ..
                } => {
                    if let Some(mode) = mode.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.mode = Some(mode.to_string());
                    }
                    if let Some(model) = model.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.model = Some(model.to_string());
                    }
                    if let Some(thinking) = thinking.as_deref().filter(|s| !s.trim().is_empty())
                    {
                        self.header.thinking = Some(thinking.to_string());
                    }
                    self.header_needs_refresh = true;
                }
                _ => {}
            }
        }

        fn push_transcript(&mut self, entry: TranscriptEntry) {
            const MAX_TRANSCRIPT_ITEMS: usize = 5000;
            if self.transcript.len() >= MAX_TRANSCRIPT_ITEMS {
                self.transcript.pop_front();
            }
            self.transcript.push_back(entry);
        }

        fn record_token_usage(
            &mut self,
            response_id: Option<&str>,
            usage: Option<&Value>,
            event_seq: u64,
        ) {
            let Some(tokens) = usage.and_then(usage_total_tokens) else {
                return;
            };
            if let Some(skip_before) = self.skip_token_usage_before_seq {
                if event_seq <= skip_before {
                    return;
                }
            }
            if let Some(response_id) = response_id.filter(|s| !s.trim().is_empty()) {
                if !self.counted_usage_responses.insert(response_id.to_string()) {
                    return;
                }
            }
            self.total_tokens_used = self.total_tokens_used.saturating_add(tokens);
        }

        fn set_status(&mut self, msg: String) {
            if Self::is_error_message(&msg) {
                self.report_error(msg);
            } else {
                self.status = Some(msg);
            }
        }

        fn report_error(&mut self, msg: String) {
            self.status = Some(msg.clone());
            if self.active_thread.is_some() {
                self.push_transcript(TranscriptEntry {
                    role: TranscriptRole::Error,
                    text: msg,
                });
            }
        }

        fn cancel_model_fetch(&mut self) {
            if let Some(fetch) = self.model_fetch.take() {
                fetch.handle.abort();
            }
            self.model_fetch_pending = false;
        }
    }
