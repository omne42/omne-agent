impl UiState {
    fn new(include_archived: bool) -> Self {
        Self {
            include_archived,
            only_fan_out_linkage_issue: false,
            only_fan_out_auto_apply_error: false,
            only_fan_in_dependency_blocked: false,
            only_subagent_proxy_approval: false,
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
            subagent_pending_summary: None,
            subagent_pending_summary_needs_refresh: false,
            input: String::new(),
            status: None,
            status_expires_at: None,
            current_context_tokens_estimate: None,
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
        let value =
            serde_json::to_value(app.thread_list_meta(self.include_archived, false).await?)?;
        let parsed =
            serde_json::from_value::<ThreadListMetaResponse>(value).context("parse threads")?;
        self.threads = self.apply_thread_picker_filters(parsed.threads);
        if self.selected_thread >= self.threads.len() {
            self.selected_thread = self.threads.len().saturating_sub(1);
        }
        Ok(())
    }

    fn apply_thread_picker_filters(&self, threads: Vec<ThreadMeta>) -> Vec<ThreadMeta> {
        threads
            .into_iter()
            .filter(|thread| {
                (!self.only_fan_out_linkage_issue || thread.has_fan_out_linkage_issue)
                    && (!self.only_fan_out_auto_apply_error || thread.has_fan_out_auto_apply_error)
                    && (!self.only_fan_in_dependency_blocked
                        || thread.has_fan_in_dependency_blocked)
                    && (!self.only_subagent_proxy_approval
                        || thread.pending_subagent_proxy_approvals > 0)
            })
            .collect::<Vec<_>>()
    }

    fn clear_thread_picker_filters(&mut self) -> bool {
        let changed = self.only_fan_out_linkage_issue
            || self.only_fan_out_auto_apply_error
            || self.only_fan_in_dependency_blocked
            || self.only_subagent_proxy_approval;
        self.only_fan_out_linkage_issue = false;
        self.only_fan_out_auto_apply_error = false;
        self.only_fan_in_dependency_blocked = false;
        self.only_subagent_proxy_approval = false;
        changed
    }

    async fn open_thread(
        &mut self,
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<()> {
        let resume = app.thread_resume(thread_id).await?;
        let last_seq = resume.last_seq;
        let since_seq = last_seq.saturating_sub(2_000);

        let resp = app
            .thread_subscribe(thread_id, since_seq, Some(2_000), Some(0))
            .await
            .context("subscribe initial events")?;

        self.reset_thread_state(thread_id, resp.last_seq);
        if let Ok(state) = app.thread_state(thread_id).await {
            self.thread_cwd = state
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            self.current_context_tokens_estimate = state.current_context_tokens_estimate;
            self.total_tokens_used = state.total_tokens_used;
            self.skip_token_usage_before_seq = Some(resp.last_seq);
        }
        for event in resp.events {
            self.apply_event(&event);
        }
        if let Err(err) = self.refresh_header(app, thread_id).await {
            self.set_status(format!("header refresh error: {err}"));
        } else {
            self.header_needs_refresh = false;
        }
        self.subagent_pending_summary_needs_refresh = true;
        let _ = self.refresh_subagent_pending_summary(app, thread_id).await;
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
        self.subagent_pending_summary = None;
        self.subagent_pending_summary_needs_refresh = true;
        self.current_context_tokens_estimate = None;
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
        let effective = &config.effective;

        self.header.mode = Some(effective.mode.trim().to_string()).filter(|s| !s.is_empty());
        self.header.role = Some(effective.role.trim().to_string()).filter(|s| !s.is_empty());

        self.header.provider = Self::extract_openai_provider(&config);
        self.header.model = Some(effective.model.trim().to_string()).filter(|s| !s.is_empty());
        self.header.thinking =
            Some(effective.thinking.trim().to_string()).filter(|s| !s.is_empty());
        self.header.model_context_window = effective.model_context_window;
        self.header.allowed_tools_count = effective.allowed_tools.as_ref().map(std::vec::Vec::len);
        self.header.execpolicy_rules_count = effective.execpolicy_rules.len();
        if let Ok(state) = app.thread_state(thread_id).await {
            self.thread_cwd = state
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            self.current_context_tokens_estimate = state.current_context_tokens_estimate;
            self.total_tokens_used = state.total_tokens_used;
        }

        self.header.mcp_enabled = env_truthy("OMNE_ENABLE_MCP");
        Ok(())
    }

    fn extract_openai_provider(config: &crate::ThreadConfigExplainResponse) -> Option<String> {
        for layer in config.layers.iter().rev() {
            let provider = layer.get("openai_provider").and_then(|v| v.as_str());
            if let Some(provider) = provider.map(str::trim).filter(|s| !s.is_empty()) {
                return Some(provider.to_string());
            }
        }
        None
    }

    fn apply_event(&mut self, event: &ThreadEvent) {
        if subagent_pending_summary_maybe_changed_by_event(event) {
            self.subagent_pending_summary_needs_refresh = true;
        }
        match &event.kind {
            ThreadEventKind::TurnStarted { turn_id, input, .. } => {
                self.cancel_turn_start();
                self.active_turn_id = Some(*turn_id);
                self.header_needs_refresh = true;
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
                self.record_token_usage(response_id.as_deref(), token_usage.as_ref(), event.seq.0);
                let mut streamed = None::<String>;
                let mut streamed_thinking = None::<String>;
                if let Some(turn_id) = turn_id
                    && self
                        .streaming
                        .as_ref()
                        .is_some_and(|s| s.turn_id == *turn_id)
                {
                    streamed = self.streaming.as_ref().map(|s| s.output_text.clone());
                    streamed_thinking = self.streaming.as_ref().map(|s| s.thinking.clone());
                    self.streaming = None;
                }
                if let Some(thinking) =
                    streamed_thinking.filter(|thinking| !thinking.trim().is_empty())
                {
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::Thinking,
                        text: thinking,
                    });
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
                self.header_needs_refresh = true;
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
            ThreadEventKind::TurnCompleted {
                turn_id, status, ..
            } => {
                if self.active_turn_id == Some(*turn_id) {
                    self.active_turn_id = None;
                }
                self.header_needs_refresh = true;
                self.push_transcript(TranscriptEntry {
                    role: TranscriptRole::System,
                    text: format!("[turn] {turn_id} {}", turn_status_str(*status)),
                });
            }
            ThreadEventKind::ToolStarted {
                tool_id,
                tool,
                params,
                ..
            } => {
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
                structured_error,
                error,
                result,
                ..
            } => {
                let info = self.tool_events.remove(tool_id);
                let name = info.as_deref().unwrap_or("tool");
                if *status == ToolStatus::Completed {
                    if should_suppress_tool_completed(name) || should_suppress_tool_started(name) {
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
                    let line = format_tool_status_line(
                        name,
                        *status,
                        structured_error.as_ref(),
                        error.as_deref(),
                        result.as_ref(),
                    );
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
                role,
                model,
                thinking,
                ..
            } => {
                if let Some(mode) = mode.as_deref().filter(|s| !s.trim().is_empty()) {
                    self.header.mode = Some(mode.to_string());
                }
                if let Some(role) = role.as_deref().filter(|s| !s.trim().is_empty()) {
                    self.header.role = Some(role.to_string());
                }
                if let Some(model) = model.as_deref().filter(|s| !s.trim().is_empty()) {
                    self.header.model = Some(model.to_string());
                }
                if let Some(thinking) = thinking.as_deref().filter(|s| !s.trim().is_empty()) {
                    self.header.thinking = Some(thinking.to_string());
                }
                self.header_needs_refresh = true;
            }
            _ => {}
        }
    }

    async fn refresh_subagent_pending_summary(
        &mut self,
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<()> {
        let attention = app.thread_attention(thread_id).await?;
        self.subagent_pending_summary =
            summarize_subagent_pending_summary(attention.pending_approvals.as_slice());
        self.subagent_pending_summary_needs_refresh = false;
        Ok(())
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
            self.status_expires_at = None;
        }
    }

    fn set_temporary_status(&mut self, msg: String, ttl: Duration) {
        if Self::is_error_message(&msg) {
            self.report_error(msg);
        } else {
            self.status = Some(msg);
            self.status_expires_at = Some(Instant::now() + ttl);
        }
    }

    fn clear_status(&mut self) {
        self.status = None;
        self.status_expires_at = None;
    }

    fn dismiss_non_error_status(&mut self) {
        if self
            .status
            .as_deref()
            .is_some_and(|status| !Self::is_error_message(status))
        {
            self.clear_status();
        }
    }

    fn expire_status_if_needed(&mut self, now: Instant) -> bool {
        if self
            .status_expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            self.clear_status();
            return true;
        }
        false
    }

    fn report_error(&mut self, msg: String) {
        self.status = Some(msg.clone());
        self.status_expires_at = None;
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

fn subagent_pending_summary_maybe_changed_by_event(event: &ThreadEvent) -> bool {
    matches!(
        &event.kind,
        ThreadEventKind::ApprovalRequested { .. }
            | ThreadEventKind::ApprovalDecided { .. }
            | ThreadEventKind::TurnStarted { .. }
            | ThreadEventKind::TurnCompleted { .. }
    )
}

fn summarize_subagent_pending_summary(
    approvals: &[omne_app_server_protocol::ThreadAttentionPendingApproval],
) -> Option<SubagentPendingSummary> {
    let mut total = 0usize;
    let mut states = std::collections::BTreeMap::<String, usize>::new();

    for pending in approvals {
        if !super::is_subagent_proxy_pending_approval(pending) {
            continue;
        }
        total = total.saturating_add(1);
        let state = pending
            .summary
            .as_ref()
            .and_then(|summary| summary.child_attention_state.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_string());
        *states.entry(state).or_default() += 1;
    }

    (total > 0).then_some(SubagentPendingSummary { total, states })
}
