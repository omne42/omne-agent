    fn draw_ui(f: &mut ratatui::Frame, state: &mut UiState) {
        let area = f.area();
        f.render_widget(Clear, area);
        match state.active_thread {
            None => {
                let show_footer = !matches!(state.overlays.last(), Some(Overlay::CommandPalette(_)));
                if show_footer {
                    let layout = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(1), Constraint::Length(1)])
                        .split(area);
                    draw_thread_list(f, state, layout[0]);
                    draw_footer(f, state, layout[1]);
                } else {
                    draw_thread_list(f, state, area);
                }
            }
            Some(_) => draw_thread_view(f, state, area),
        }

        if let Some(overlay) = state.overlays.last() {
            match overlay {
                Overlay::CommandPalette(_) if state.active_thread.is_none() => {
                    draw_overlay(f, overlay);
                }
                Overlay::CommandPalette(_) => {}
                _ => draw_overlay(f, overlay),
            }
        }
    }

    const BASELINE_TOKENS: u64 = 12_000;

    fn percent_of_context_window_remaining(total_tokens_used: u64, context_window: u64) -> u64 {
        if context_window <= BASELINE_TOKENS {
            return 0;
        }
        let effective_window = context_window.saturating_sub(BASELINE_TOKENS);
        let used = total_tokens_used.saturating_sub(BASELINE_TOKENS);
        let remaining = effective_window.saturating_sub(used);
        (remaining as f64 / effective_window as f64 * 100.0)
            .clamp(0.0, 100.0)
            .round() as u64
    }

    fn build_footer_line(state: &UiState, width: u16) -> Line<'static> {
        let context_left = match state.header.model_context_window {
            Some(window) => {
                let pct = percent_of_context_window_remaining(state.total_tokens_used, window);
                format!("{pct}% context left")
            }
            None => {
                if state.total_tokens_used > 0 {
                    format!("{} used", state.total_tokens_used)
                } else {
                    "100% context left".to_string()
                }
            }
        };

        let msg = match state.active_thread {
            Some(thread_id) => {
                let short = super::thread_id_short(thread_id);
                let mode = state.header.mode.as_deref().unwrap_or("-");
                let provider = state.header.provider.as_deref().unwrap_or("-");
                let model = state.header.model.as_deref().unwrap_or("-");
                let thinking = state.header.thinking.as_deref().unwrap_or("-");
                let mcp = if state.header.mcp_enabled { "on" } else { "off" };

                if width < 80 {
                    format!("{context_left}  th={short} mode={mode} model={model} (Ctrl-K)")
                } else {
                    format!(
                        "{context_left}  thread={short} agent={mode} provider={provider} model={model} thinking={thinking} mcp={mcp} (Ctrl-K=commands)"
                    )
                }
            }
            None => format!("{context_left}  threads (Ctrl-K=commands)"),
        };

        let style = Style::default().fg(Color::Gray);
        match state.status.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(status) => {
                let status_style = if UiState::is_error_message(status) {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Gray)
                };
                Line::from(vec![
                    Span::styled(msg, style),
                    Span::styled(" | ", style),
                    Span::styled(status.to_string(), status_style),
                ])
            }
            None => Line::from(Span::styled(msg, style)),
        }
    }

    fn draw_footer(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let paragraph = Paragraph::new(build_footer_line(state, area.width));
        f.render_widget(paragraph, area);
    }

    fn parse_rfc3339_timestamp(value: &str) -> Option<OffsetDateTime> {
        OffsetDateTime::parse(value, &Rfc3339).ok()
    }

    fn human_time_ago(ts: OffsetDateTime) -> String {
        let now = OffsetDateTime::now_utc();
        let delta = now - ts;
        let secs = delta.whole_seconds().max(0);
        if secs < 60 {
            if secs == 1 {
                format!("{secs} second ago")
            } else {
                format!("{secs} seconds ago")
            }
        } else if secs < 60 * 60 {
            let mins = secs / 60;
            if mins == 1 {
                format!("{mins} minute ago")
            } else {
                format!("{mins} minutes ago")
            }
        } else if secs < 60 * 60 * 24 {
            let hours = secs / 3600;
            if hours == 1 {
                format!("{hours} hour ago")
            } else {
                format!("{hours} hours ago")
            }
        } else {
            let days = secs / (60 * 60 * 24);
            if days == 1 {
                format!("{days} day ago")
            } else {
                format!("{days} days ago")
            }
        }
    }

    fn format_updated_label(meta: &ThreadMeta) -> String {
        let updated = meta
            .updated_at
            .as_deref()
            .and_then(parse_rfc3339_timestamp)
            .or_else(|| meta.created_at.as_deref().and_then(parse_rfc3339_timestamp));
        updated.map(human_time_ago).unwrap_or_else(|| "-".to_string())
    }

    fn normalize_single_line(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return "-".to_string();
        }
        trimmed
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn right_elide(value: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }
        if UnicodeWidthStr::width(value) <= max_width {
            return value.to_string();
        }
        if max_width <= 1 {
            return "…".to_string();
        }
        let tail_len = max_width.saturating_sub(1);
        let mut out = String::new();
        out.push('…');
        let mut tail = String::new();
        for ch in value.chars().rev() {
            if UnicodeWidthStr::width(tail.as_str()) >= tail_len {
                break;
            }
            tail.push(ch);
        }
        out.push_str(&tail.chars().rev().collect::<String>());
        out
    }

    fn pad_to_width(value: &str, width: usize) -> String {
        let mut out = value.to_string();
        let pad = width.saturating_sub(UnicodeWidthStr::width(value));
        if pad > 0 {
            out.push_str(&" ".repeat(pad));
        }
        out
    }

