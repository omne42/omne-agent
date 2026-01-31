    impl UiState {
        fn new(include_archived: bool) -> Self {
            Self {
                include_archived,
                scrollback_enabled: false,
                threads: Vec::new(),
                selected_thread: 0,
                active_thread: None,
                header: HeaderState::default(),
                header_needs_refresh: false,
                overlays: Vec::new(),
                inline_palette: None,
                last_seq: 0,
                transcript: VecDeque::new(),
                transcript_flushed: 0,
                transcript_flushed_line_offset: 0,
                transcript_scroll: 0,
                transcript_follow: true,
                transcript_max_scroll: 0,
                transcript_viewport_height: 0,
                tool_events: HashMap::new(),
                process_started_lines: HashMap::new(),
                streaming: None,
                streaming_entry_active: false,
                thinking_turn_id: None,
                active_turn_id: None,
                turn_inflight_started_at: None,
                turn_inflight_id: None,
                input: String::new(),
                input_cursor: 0,
                status: None,
                total_input_tokens_used: 0,
                total_cache_input_tokens_used: 0,
                total_output_tokens_used: 0,
                total_tokens_used: 0,
                token_usage_by_response: HashMap::new(),
                last_tokens_in_context_window: None,
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
                self.total_input_tokens_used = state
                    .get("input_tokens_used")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                self.total_cache_input_tokens_used = state
                    .get("cache_input_tokens_used")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                self.total_output_tokens_used = state
                    .get("output_tokens_used")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                self.total_tokens_used = state
                    .get("total_tokens_used")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                if self.total_tokens_used > 0
                    || self.total_input_tokens_used > 0
                    || self.total_output_tokens_used > 0
                    || self.total_cache_input_tokens_used > 0
                {
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
            self.transcript_flushed = 0;
            self.transcript_flushed_line_offset = 0;
            self.transcript_scroll = 0;
            self.transcript_follow = true;
            self.transcript_max_scroll = 0;
            self.transcript_viewport_height = 0;
            self.tool_events.clear();
            self.process_started_lines.clear();
            self.streaming = None;
            self.streaming_entry_active = false;
            self.thinking_turn_id = None;
            self.active_turn_id = None;
            self.turn_inflight_started_at = None;
            self.turn_inflight_id = None;
            self.total_input_tokens_used = 0;
            self.total_cache_input_tokens_used = 0;
            self.total_output_tokens_used = 0;
            self.total_tokens_used = 0;
            self.token_usage_by_response.clear();
            self.last_tokens_in_context_window = None;
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
                    if self.turn_inflight_started_at.is_some() && self.turn_inflight_id.is_none() {
                        self.turn_inflight_id = Some(*turn_id);
                    }
                    let normalized_input = normalize_user_turn_input_for_dedupe(input);
                    let should_dedupe = self.transcript.back().is_some_and(|last| {
                        matches!(last.role, TranscriptRole::User)
                            && normalize_user_turn_input_for_dedupe(&last.text) == normalized_input
                    });
                    if should_dedupe {
                        if let Some(last) = self.transcript.back_mut() {
                            last.text = input.clone();
                        }
                    } else {
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::User,
                            text: input.clone(),
                        });
                    }
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

                    let final_text = text.as_str();
                    let final_trim = final_text.trim();

                    if let Some(turn_id) = turn_id
                        && self.streaming.as_ref().is_some_and(|s| s.turn_id == *turn_id)
                    {
                        let streamed = self.streaming.as_ref().map(|s| s.text.clone());
                        self.streaming = None;
                        self.streaming_entry_active = false;

                        let Some(streamed) = streamed else {
                            return;
                        };
                        let streamed_trim = streamed.trim();
                        if final_trim.is_empty() {
                            return;
                        }
                        if streamed_trim.is_empty() {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Assistant,
                                text: final_text.to_string(),
                            });
                            return;
                        }

                        if final_text.starts_with(streamed.as_str()) {
                            let rest = &final_text[streamed.len()..];
                            if !rest.is_empty() {
                                match self.transcript.back_mut() {
                                    Some(last) if matches!(last.role, TranscriptRole::Assistant) => {
                                        last.text.push_str(rest);
                                    }
                                    _ => {
                                        self.push_transcript(TranscriptEntry {
                                            role: TranscriptRole::Assistant,
                                            text: rest.to_string(),
                                        });
                                    }
                                }
                            }
                            return;
                        }

                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Assistant,
                            text: final_text.to_string(),
                        });
                        return;
                    }

                    if !final_trim.is_empty() {
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Assistant,
                            text: final_text.to_string(),
                        });
                    }
                }
                ThreadEventKind::AgentStep {
                    response_id,
                    tool_calls,
                    tool_results,
                    token_usage,
                    ..
                } => {
                    self.record_token_usage(
                        Some(response_id.as_str()),
                        token_usage.as_ref(),
                        event.seq.0,
                    );
                    if !tool_results.is_empty() {
                        let mut call_info_by_id =
                            std::collections::HashMap::<&str, (&str, &str)>::new();
                        for call in tool_calls {
                            call_info_by_id.insert(
                                call.call_id.as_str(),
                                (call.name.as_str(), call.arguments.as_str()),
                            );
                        }
                        for result in tool_results {
                            let (name, args) = call_info_by_id
                                .get(result.call_id.as_str())
                                .copied()
                                .unwrap_or(("tool", "{}"));
                            if self.try_merge_process_tool_output(name, args, result.output.as_str()) {
                                continue;
                            }
                            if let Some(line) =
                                format_agent_step_tool_result(name, result.output.as_str())
                            {
                                self.push_transcript(TranscriptEntry {
                                    role: TranscriptRole::Tool,
                                    text: line,
                                });
                            }
                        }
                    }
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
                    let inflight_matches = self.turn_inflight_id == Some(*turn_id)
                        || (self.turn_inflight_id.is_none()
                            && self.active_turn_id == Some(*turn_id));
                    let duration = if inflight_matches {
                        self.turn_inflight_started_at.map(|start| start.elapsed())
                    } else {
                        None
                    };
                    if self.active_turn_id == Some(*turn_id) {
                        self.active_turn_id = None;
                    }
                    if inflight_matches {
                        self.turn_inflight_started_at = None;
                        self.turn_inflight_id = None;
                    }
                    let mut line = format!("[turn] {turn_id} {}", turn_status_str(*status));
                    if let Some(duration) = duration {
                        line.push_str(&format!(" ({})", format_elapsed(duration)));
                    }
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::System,
                        text: line,
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
                ThreadEventKind::ProcessStarted {
                    process_id,
                    argv,
                    cwd,
                    ..
                } => {
                    if let Some(line) =
                        format_process_started_line(argv, cwd, self.thread_cwd.as_deref())
                    {
                        self.process_started_lines.insert(*process_id, line.clone());
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Tool,
                            text: line,
                        });
                    }
                }
                ThreadEventKind::ThreadConfigUpdated {
                    mode,
                    openai_provider,
                    model,
                    thinking,
                    ..
                } => {
                    if let Some(mode) = mode.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.mode = Some(mode.to_string());
                    }
                    if let Some(provider) =
                        openai_provider.as_deref().filter(|s| !s.trim().is_empty())
                    {
                        self.header.provider = Some(provider.to_string());
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

        fn apply_live_event(&mut self, event: &ThreadEvent) -> bool {
            if self.active_thread != Some(event.thread_id) {
                return false;
            }
            if event.seq.0 <= self.last_seq {
                return false;
            }
            self.last_seq = event.seq.0;
            self.apply_event(event);
            true
        }

        fn push_transcript(&mut self, entry: TranscriptEntry) {
            self.streaming_entry_active = false;
            const MAX_TRANSCRIPT_ITEMS: usize = 5000;
            if self.transcript.len() >= MAX_TRANSCRIPT_ITEMS {
                self.transcript.pop_front();
                if self.transcript_flushed > 0 {
                    self.transcript_flushed = self.transcript_flushed.saturating_sub(1);
                } else {
                    self.transcript_flushed_line_offset = 0;
                }
            }
            self.transcript.push_back(entry);
        }

        fn try_merge_process_tool_output(&mut self, tool: &str, args: &str, output: &str) -> bool {
            let tool = normalize_agent_tool_name(tool);
            if tool != "process/inspect" && tool != "process/tail" {
                return false;
            }

            let Ok(args) = serde_json::from_str::<Value>(args) else {
                return false;
            };
            let Some(process_id) = args
                .get("process_id")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<ProcessId>().ok())
            else {
                return false;
            };

            let Ok(value) = serde_json::from_str::<Value>(output) else {
                return false;
            };
            if value.get("needs_approval").and_then(Value::as_bool) == Some(true) {
                return false;
            }
            if value.get("denied").and_then(Value::as_bool) == Some(true) {
                return false;
            }
            let Some(text) = extract_primary_tool_text(&value) else {
                return false;
            };
            let text = text.trim_end();
            if text.trim().is_empty() {
                return true;
            }

            let Some(started_line) = self.process_started_lines.get(&process_id).cloned() else {
                return false;
            };

            let found = self
                .transcript
                .iter_mut()
                .enumerate()
                .rev()
                .find(|(_idx, entry)| {
                    matches!(entry.role, TranscriptRole::Tool) && entry.text == started_line
                });

            match found {
                Some((idx, entry))
                    if idx > self.transcript_flushed
                        || (idx == self.transcript_flushed
                            && self.transcript_flushed_line_offset == 0) =>
                {
                    entry.text.push('\n');
                    entry.text.push_str(text);
                }
                _ => {
                    // If the original `$ cmd` line has already been flushed to terminal scrollback,
                    // we can't mutate what the user sees; fall back to emitting a fresh combined
                    // entry at the bottom.
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::Tool,
                        text: format!("{started_line}\n{text}"),
                    });
                }
            }

            self.process_started_lines.remove(&process_id);
            true
        }

        fn record_token_usage(
            &mut self,
            response_id: Option<&str>,
            usage: Option<&Value>,
            event_seq: u64,
        ) {
            let Some(usage) = usage else {
                return;
            };

            let input_tokens = usage_input_tokens(usage);
            let cache_input_tokens = usage_cache_input_tokens(usage);
            let output_tokens = usage_output_tokens(usage);
            let total_tokens = usage_total_tokens(usage);
            let has_numeric_usage = input_tokens.is_some()
                || cache_input_tokens.is_some()
                || output_tokens.is_some()
                || total_tokens.is_some();
            if !has_numeric_usage {
                // Some events store token usage as redacted strings; skip counting so that a later
                // event for the same response_id (often the final assistant message) can provide
                // real numbers.
                return;
            }

            self.last_tokens_in_context_window = total_tokens.or(self.last_tokens_in_context_window);

            if let Some(skip_before) = self.skip_token_usage_before_seq {
                if event_seq <= skip_before {
                    return;
                }
            }

            fn apply_delta(slot: &mut Option<u64>, next: Option<u64>) -> u64 {
                let Some(next) = next else {
                    return 0;
                };
                match *slot {
                    None => {
                        *slot = Some(next);
                        next
                    }
                    Some(prev) if next > prev => {
                        *slot = Some(next);
                        next - prev
                    }
                    _ => 0,
                }
            }

            if let Some(response_id) = response_id.filter(|s| !s.trim().is_empty()) {
                let entry = self
                    .token_usage_by_response
                    .entry(response_id.to_string())
                    .or_default();
                let delta_input = apply_delta(&mut entry.input_tokens, input_tokens);
                let delta_cache = apply_delta(&mut entry.cache_input_tokens, cache_input_tokens);
                let delta_output = apply_delta(&mut entry.output_tokens, output_tokens);
                let delta_total = apply_delta(&mut entry.total_tokens, total_tokens);

                self.total_input_tokens_used =
                    self.total_input_tokens_used.saturating_add(delta_input);
                self.total_cache_input_tokens_used = self
                    .total_cache_input_tokens_used
                    .saturating_add(delta_cache);
                self.total_output_tokens_used =
                    self.total_output_tokens_used.saturating_add(delta_output);
                self.total_tokens_used = self.total_tokens_used.saturating_add(delta_total);
                return;
            }

            self.total_input_tokens_used =
                self.total_input_tokens_used.saturating_add(input_tokens.unwrap_or(0));
            self.total_cache_input_tokens_used = self
                .total_cache_input_tokens_used
                .saturating_add(cache_input_tokens.unwrap_or(0));
            self.total_output_tokens_used =
                self.total_output_tokens_used.saturating_add(output_tokens.unwrap_or(0));
            self.total_tokens_used =
                self.total_tokens_used.saturating_add(total_tokens.unwrap_or(0));
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

        fn clear_input(&mut self) {
            self.input.clear();
            self.input_cursor = 0;
        }

        fn set_input(&mut self, value: String) {
            self.input = value;
            self.input_cursor = self.input.len();
        }

        fn clamp_input_cursor(&mut self) {
            self.input_cursor = self.input_cursor.min(self.input.len());
            while !self.input.is_char_boundary(self.input_cursor) {
                self.input_cursor = self.input_cursor.saturating_sub(1);
            }
        }

        fn move_input_cursor_left(&mut self) {
            self.clamp_input_cursor();
            if self.input_cursor == 0 {
                return;
            }
            self.input_cursor = prev_char_boundary(&self.input, self.input_cursor);
        }

        fn move_input_cursor_right(&mut self) {
            self.clamp_input_cursor();
            if self.input_cursor >= self.input.len() {
                return;
            }
            self.input_cursor = next_char_boundary(&self.input, self.input_cursor);
        }

        fn insert_input_char(&mut self, value: char) {
            self.clamp_input_cursor();
            self.input.insert(self.input_cursor, value);
            self.input_cursor = self.input_cursor.saturating_add(value.len_utf8());
        }

        fn insert_input_str(&mut self, value: &str) {
            self.clamp_input_cursor();
            self.input.insert_str(self.input_cursor, value);
            self.input_cursor = self.input_cursor.saturating_add(value.len());
        }

        fn input_backspace(&mut self) {
            self.clamp_input_cursor();
            if self.input_cursor == 0 {
                return;
            }
            let prev = prev_char_boundary(&self.input, self.input_cursor);
            self.input.drain(prev..self.input_cursor);
            self.input_cursor = prev;
        }

        fn input_delete(&mut self) {
            self.clamp_input_cursor();
            if self.input_cursor >= self.input.len() {
                return;
            }
            let end = next_char_boundary(&self.input, self.input_cursor);
            self.input.drain(self.input_cursor..end);
        }
    }

    fn prev_char_boundary(input: &str, cursor: usize) -> usize {
        let cursor = cursor.min(input.len());
        let mut idx = cursor;
        while idx > 0 {
            idx = idx.saturating_sub(1);
            if input.is_char_boundary(idx) {
                return idx;
            }
        }
        0
    }

    fn next_char_boundary(input: &str, cursor: usize) -> usize {
        let cursor = cursor.min(input.len());
        let mut idx = cursor.saturating_add(1);
        while idx <= input.len() {
            if input.is_char_boundary(idx) {
                return idx;
            }
            idx = idx.saturating_add(1);
        }
        input.len()
    }

    fn normalize_user_turn_input_for_dedupe(input: &str) -> &str {
        let mut out = input.trim();
        loop {
            let trimmed = out.trim_end();
            let Some(last_ch) = trimmed.chars().last() else {
                return trimmed;
            };
            if matches!(last_ch, '.' | '。' | '!' | '！' | '?' | '？') {
                let end = trimmed.len().saturating_sub(last_ch.len_utf8());
                out = &trimmed[..end];
                continue;
            }
            return trimmed;
        }
    }
