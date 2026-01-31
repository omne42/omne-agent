    struct InputRender {
        lines: Vec<Line<'static>>,
        cursor_line: usize,
        cursor_col: usize,
    }

    struct PaletteRender {
        lines: Vec<Line<'static>>,
    }

    fn draw_thread_view(
        f: &mut ratatui::Frame,
        state: &mut UiState,
        area: ratatui::layout::Rect,
    ) {
        let width = area.width.max(1) as usize;
        let mut transcript_lines = Vec::<Line>::new();
        if state.scrollback_enabled {
            let mut entries = state.transcript.iter().enumerate().skip(state.transcript_flushed);
            if let Some((idx, entry)) = entries.next() {
                let mut lines = format_transcript_entry_lines(entry, width);
                if idx == state.transcript_flushed && state.transcript_flushed_line_offset > 0 {
                    let skip = state
                        .transcript_flushed_line_offset
                        .min(lines.len());
                    lines.drain(0..skip);
                }
                transcript_lines.extend(lines);
            }
            for (_idx, entry) in entries {
                transcript_lines.extend(format_transcript_entry_lines(entry, width));
            }
            if let Some(streaming) = &state.streaming {
                transcript_lines.extend(format_role_lines(
                    TranscriptRole::Assistant,
                    streaming.text.as_str(),
                    width,
                ));
            }
            // Keep one empty line between transcript and prompt so the input never "jumps" to the
            // very top when streaming finishes.
            transcript_lines.push(Line::from(Span::raw("")));
        } else {
            for entry in &state.transcript {
                transcript_lines.extend(format_transcript_entry_lines(entry, width));
            }
            if let Some(streaming) = &state.streaming {
                transcript_lines.extend(format_role_lines(
                    TranscriptRole::Assistant,
                    streaming.text.as_str(),
                    width,
                ));
            }
            // 顶部固定留一行空白，避免首条输出把输入框顶线盖住。
            transcript_lines.insert(0, Line::from(Span::raw("")));
        }

        const MAX_INLINE_PALETTE_ITEMS: usize = 12;
        const INLINE_PALETTE_MIN_LINES: usize = 4;

        let input_render = build_input_lines(&state.input, state.input_cursor, width);
        let input_lines = input_render.lines.len();
        let total_height = area.height as usize;

        let inline_view = state.inline_palette.as_ref().map(|inline| &inline.view);
        let footer_lines = if inline_view.is_some() { 0 } else { 1 };
        let reserved_bottom = input_lines.saturating_add(footer_lines);
        let palette_target_lines = inline_view
            .map(|view| {
                let items = if view.filtered.is_empty() {
                    1
                } else {
                    view.filtered.len().min(MAX_INLINE_PALETTE_ITEMS)
                };
                1 + items
            })
            .unwrap_or(0);
        let min_palette_lines = if inline_view.is_some() {
            INLINE_PALETTE_MIN_LINES
        } else {
            0
        };

        let max_bottom_pct = if inline_view.is_some() { 70 } else { 35 };
        let mut max_bottom = ((total_height * max_bottom_pct) / 100)
            .max(4)
            .min(total_height);
        let desired_bottom = reserved_bottom.saturating_add(palette_target_lines);
        let min_bottom_needed = reserved_bottom.saturating_add(min_palette_lines);
        max_bottom = max_bottom
            .max(desired_bottom)
            .max(min_bottom_needed)
            .min(total_height);

        let available_palette_lines = max_bottom.saturating_sub(reserved_bottom);
        let max_palette_lines = if inline_view.is_some() {
            available_palette_lines
                .min(palette_target_lines)
                .max(min_palette_lines.min(available_palette_lines))
        } else {
            0
        };

        let palette_render = inline_view
            .map(|view| build_command_palette_lines(view, width, max_palette_lines));

        let mut bottom_lines = Vec::<Line>::new();
        let input_start = bottom_lines.len();
        bottom_lines.extend(input_render.lines);
        let input_cursor = (
            input_start + input_render.cursor_line,
            input_render.cursor_col,
        );
        if let Some(palette) = palette_render {
            bottom_lines.extend(palette.lines);
        }
        if footer_lines > 0 {
            bottom_lines.push(build_footer_line(state, area.width));
        }

        let required_bottom = bottom_lines.len().max(1);
        let bottom_height = required_bottom.min(max_bottom).min(total_height);
        let max_top = total_height.saturating_sub(bottom_height);
        let top_height = if state.scrollback_enabled {
            transcript_lines.len().min(max_top)
        } else {
            max_top
        };

        let transcript_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: top_height as u16,
        };
        let bottom_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y.saturating_add(top_height as u16),
            width: area.width,
            height: bottom_height as u16,
        };

        let viewport_height = transcript_area.height as usize;
        let max_scroll = transcript_lines.len().saturating_sub(viewport_height);
        state.transcript_max_scroll = u16::try_from(max_scroll).unwrap_or(u16::MAX);
        state.transcript_viewport_height = transcript_area.height;

        let scroll = if state.transcript_follow {
            max_scroll
        } else {
            usize::from(state.transcript_scroll).min(max_scroll)
        };
        state.transcript_scroll = u16::try_from(scroll).unwrap_or(u16::MAX);

        if transcript_area.height > 0 {
            let paragraph = Paragraph::new(Text::from(transcript_lines));
            f.render_widget(
                paragraph.scroll((state.transcript_scroll, 0)),
                transcript_area,
            );
        }

        if bottom_area.height > 0 {
            let bottom_paragraph = Paragraph::new(Text::from(bottom_lines));
            f.render_widget(bottom_paragraph, bottom_area);

            if state.active_thread.is_some() {
                let cursor_target = match state.overlays.last() {
                    Some(_) => None,
                    None => Some(input_cursor),
                };
                if let Some((cursor_line, cursor_col)) = cursor_target {
                    if cursor_line < bottom_height {
                        let y = bottom_area.y.saturating_add(cursor_line as u16);
                        let x = bottom_area.x.saturating_add(cursor_col as u16);
                        let max_x = bottom_area
                            .x
                            .saturating_add(bottom_area.width.saturating_sub(1));
                        f.set_cursor_position((x.min(max_x), y));
                    }
                }
            }
        }
    }

    fn format_transcript_entry_lines(entry: &TranscriptEntry, width: usize) -> Vec<Line<'static>> {
        format_role_lines(entry.role, entry.text.as_str(), width)
    }

    fn format_role_lines(
        role: TranscriptRole,
        text: &str,
        width: usize,
    ) -> Vec<Line<'static>> {
        let (prefix, prefix_style, content_style) = match role {
            TranscriptRole::User => ("user: ", Style::default().fg(Color::Yellow), None),
            TranscriptRole::Assistant => ("assistant: ", Style::default().fg(Color::Green), None),
            TranscriptRole::System => ("system: ", Style::default().fg(Color::Cyan), None),
            TranscriptRole::Error => ("error: ", Style::default().fg(Color::Red), None),
            TranscriptRole::Tool => ("tool: ", Style::default().fg(Color::Blue), Some(Style::default().fg(Color::Blue))),
        };
        wrap_prefixed_lines(prefix, prefix_style, content_style, text, width)
    }

    fn wrap_prefixed_lines(
        prefix: &str,
        prefix_style: Style,
        content_style: Option<Style>,
        text: &str,
        width: usize,
    ) -> Vec<Line<'static>> {
        let width = width.max(1);
        let prefix_width = UnicodeWidthStr::width(prefix);
        let available_first = width.saturating_sub(prefix_width).max(1);
        let available_other = width;
        let mut out = Vec::<Line>::new();

        let mut first = true;
        for raw_line in text.split('\n') {
            let max_width = if first {
                available_first
            } else {
                available_other
            };
            let mut segments = wrap_plain_text(raw_line, max_width);
            if segments.is_empty() {
                segments.push(String::new());
            }
            for segment in segments {
                let lead = if first { prefix } else { "" };
                let lead_style = prefix_style;
                let body_style = content_style.unwrap_or_default();
                out.push(Line::from(vec![
                    Span::styled(lead.to_string(), lead_style),
                    Span::styled(segment, body_style),
                ]));
                first = false;
            }
            if first {
                out.push(Line::from(vec![Span::styled(
                    prefix.to_string(),
                    prefix_style,
                )]));
                first = false;
            }
        }

        if out.is_empty() {
            out.push(Line::from(vec![Span::styled(
                prefix.to_string(),
                prefix_style,
            )]));
        }
        out
    }

    fn wrap_plain_text(text: &str, max_width: usize) -> Vec<String> {
        let max_width = max_width.max(1);
        let mut out = Vec::new();
        let mut current = String::new();
        let mut current_width = 0usize;
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > max_width && !current.is_empty() {
                out.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        out.push(current);
        out
    }

    fn truncate_to_width(text: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }
        let mut out = String::new();
        let mut width = 0usize;
        let mut truncated = false;
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if width + ch_width > max_width {
                truncated = true;
                break;
            }
            out.push(ch);
            width += ch_width;
        }
        if truncated && max_width > 1 {
            if UnicodeWidthStr::width(out.as_str()) >= max_width {
                out.pop();
            }
            out.push('…');
        }
        out
    }

    fn build_palette_item_line(item: &PaletteItem, selected: bool, width: usize) -> Line<'static> {
        let width = width.max(1);
        let marker = if selected { ">" } else { " " };
        let mut base_style = Style::default();
        let mut detail_style = Style::default().fg(Color::DarkGray);
        if selected {
            base_style = base_style.add_modifier(Modifier::REVERSED);
            detail_style = detail_style.add_modifier(Modifier::REVERSED);
        }

        let mut spans = Vec::new();
        let marker_text = format!("{marker} ");
        let mut used = UnicodeWidthStr::width(marker_text.as_str());
        spans.push(Span::styled(marker_text, base_style));

        let mut remaining = width.saturating_sub(used);
        let label = truncate_to_width(item.label.as_str(), remaining);
        let label_width = UnicodeWidthStr::width(label.as_str());
        spans.push(Span::styled(label, base_style));
        used = used.saturating_add(label_width);
        remaining = width.saturating_sub(used);

        if let Some(detail) = item.detail.as_deref().filter(|d| !d.trim().is_empty()) {
            if remaining > 0 {
                let spacer = if remaining >= 2 { "  " } else { " " };
                let spacer_width = UnicodeWidthStr::width(spacer);
                if remaining >= spacer_width {
                    spans.push(Span::styled(spacer.to_string(), base_style));
                    used = used.saturating_add(spacer_width);
                    remaining = width.saturating_sub(used);
                    if remaining > 0 {
                        let detail = truncate_to_width(detail, remaining);
                        let detail_width = UnicodeWidthStr::width(detail.as_str());
                        spans.push(Span::styled(detail, detail_style));
                        used = used.saturating_add(detail_width);
                    }
                }
            }
        }

        let pad = width.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), base_style));
        }

        Line::from(spans)
    }

    fn build_command_palette_lines(
        view: &CommandPaletteOverlay,
        width: usize,
        max_lines: usize,
    ) -> PaletteRender {
        let mut lines = Vec::<Line>::new();
        if max_lines == 0 {
            return PaletteRender { lines };
        }
        let title = view.title.trim();
        let label = if title.is_empty() { "commands" } else { title };
        let label = format!("{}: ", label.to_ascii_lowercase());
        let query = view.query.as_str();
        let display_query = if query.trim().is_empty() {
            "(type to search)"
        } else {
            query
        };
        let header_text = truncate_to_width(&format!("{label}{display_query}"), width);
        let header_style = Style::default().fg(Color::Gray);
        let header_pad = width.saturating_sub(UnicodeWidthStr::width(header_text.as_str()));
        let mut header_spans = Vec::new();
        header_spans.push(Span::styled(header_text, header_style));
        if header_pad > 0 {
            header_spans.push(Span::styled(" ".repeat(header_pad), header_style));
        }
        lines.push(Line::from(header_spans));

        let available_items = max_lines.saturating_sub(1);
        if view.filtered.is_empty() {
            if available_items > 0 {
                let text = "  (no matches)";
                let style = Style::default().fg(Color::DarkGray);
                let pad = width.saturating_sub(UnicodeWidthStr::width(text));
                let mut spans = Vec::new();
                spans.push(Span::styled(text.to_string(), style));
                if pad > 0 {
                    spans.push(Span::styled(" ".repeat(pad), style));
                }
                lines.push(Line::from(spans));
            }
            return PaletteRender { lines };
        }

        if available_items == 0 {
            return PaletteRender { lines };
        }

        let visible = available_items.min(view.filtered.len()).max(1);
        let mut start = 0usize;
        if view.selected >= visible {
            start = view.selected + 1 - visible;
        }
        let end = (start + visible).min(view.filtered.len());

        for (offset, filtered_idx) in view.filtered[start..end].iter().enumerate() {
            if let Some(item) = view.items.get(*filtered_idx) {
                let selected = start + offset == view.selected;
                lines.push(build_palette_item_line(item, selected, width));
            }
        }

        PaletteRender { lines }
    }

    fn build_input_lines(input: &str, cursor: usize, width: usize) -> InputRender {
        const PADDING_LINES: usize = 1;
        let width = width.max(1);
        let prompt = "› ";
        let prompt_width = UnicodeWidthStr::width(prompt);
        let indent = " ".repeat(prompt_width);
        let available = width.saturating_sub(prompt_width).max(1);
        let input_bg = Style::default().bg(Color::Rgb(60, 60, 60));
        let prompt_style = input_bg.fg(Color::Gray);
        let input_style = input_bg;

        let mut segments = Vec::<String>::new();
        for raw_line in input.split('\n') {
            let wrapped = wrap_plain_text(raw_line, available);
            if wrapped.is_empty() {
                segments.push(String::new());
            } else {
                segments.extend(wrapped);
            }
        }
        if segments.is_empty() {
            segments.push(String::new());
        }

        let cursor = cursor.min(input.len());
        let mut cursor_segment_line = 0usize;
        let mut cursor_segment_col = 0usize;
        let mut bytes_seen = 0usize;
        for ch in input.chars() {
            let ch_len = ch.len_utf8();
            if bytes_seen + ch_len > cursor {
                break;
            }
            bytes_seen = bytes_seen.saturating_add(ch_len);
            if ch == '\n' {
                cursor_segment_line = cursor_segment_line.saturating_add(1);
                cursor_segment_col = 0;
                continue;
            }
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if cursor_segment_col + ch_width > available && cursor_segment_col > 0 {
                cursor_segment_line = cursor_segment_line.saturating_add(1);
                cursor_segment_col = 0;
            }
            cursor_segment_col = cursor_segment_col.saturating_add(ch_width);
        }

        let cursor_segment_line =
            cursor_segment_line.min(segments.len().saturating_sub(1));
        let cursor_line = cursor_segment_line + PADDING_LINES;
        let cursor_col = prompt_width
            .saturating_add(cursor_segment_col)
            .min(width.saturating_sub(1));

        let mut lines = Vec::<Line>::new();
        let padding = " ".repeat(width);
        for _ in 0..PADDING_LINES {
            lines.push(Line::from(Span::styled(padding.clone(), input_style)));
        }
        for (idx, segment) in segments.iter().enumerate() {
            let lead = if idx == 0 { prompt } else { indent.as_str() };
            let lead_style = prompt_style;
            let content_style = input_style;
            let used = UnicodeWidthStr::width(lead)
                .saturating_add(UnicodeWidthStr::width(segment.as_str()));
            let pad = width.saturating_sub(used);
            let mut spans = Vec::new();
            spans.push(Span::styled(lead.to_string(), lead_style));
            spans.push(Span::styled(segment.to_string(), content_style));
            if pad > 0 {
                spans.push(Span::styled(" ".repeat(pad), content_style));
            }
            lines.push(Line::from(spans));
        }
        for _ in 0..PADDING_LINES {
            lines.push(Line::from(Span::styled(padding.clone(), input_style)));
        }

        InputRender {
            lines,
            cursor_line,
            cursor_col,
        }
    }

    fn model_routing_rule_source_str(value: ModelRoutingRuleSource) -> &'static str {
        match value {
            ModelRoutingRuleSource::Subagent => "subagent",
            ModelRoutingRuleSource::ProjectOverride => "project_override",
            ModelRoutingRuleSource::KeywordRule => "keyword_rule",
            ModelRoutingRuleSource::Skill => "skill",
            ModelRoutingRuleSource::RoleDefault => "role_default",
            ModelRoutingRuleSource::GlobalDefault => "global_default",
        }
    }

    fn usage_total_tokens(usage: &Value) -> Option<u64> {
        let total_tokens = usage.get("total_tokens").and_then(Value::as_u64);
        let input_tokens = usage_input_tokens(usage);
        let output_tokens = usage_output_tokens(usage);
        total_tokens.or_else(|| match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        })
    }

    fn usage_input_tokens(usage: &Value) -> Option<u64> {
        usage.get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
    }

    fn usage_output_tokens(usage: &Value) -> Option<u64> {
        usage.get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(Value::as_u64)
    }

    fn usage_cache_input_tokens(usage: &Value) -> Option<u64> {
        usage.get("cache_input_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                usage.get("input_tokens_details")
                    .and_then(|details| details.get("cached_tokens"))
                    .and_then(Value::as_u64)
            })
            .or_else(|| {
                usage.get("prompt_tokens_details")
                    .and_then(|details| details.get("cached_tokens"))
                    .and_then(Value::as_u64)
            })
    }

    fn tool_status_str(value: ToolStatus) -> &'static str {
        match value {
            ToolStatus::Completed => "completed",
            ToolStatus::Failed => "failed",
            ToolStatus::Denied => "denied",
            ToolStatus::Cancelled => "cancelled",
        }
    }

    fn turn_status_str(value: TurnStatus) -> &'static str {
        match value {
            TurnStatus::Completed => "completed",
            TurnStatus::Interrupted => "interrupted",
            TurnStatus::Failed => "failed",
            TurnStatus::Cancelled => "cancelled",
            TurnStatus::Stuck => "stuck",
        }
    }
