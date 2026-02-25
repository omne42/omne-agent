    fn draw_thread_list(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        const MAX_TITLE_WIDTH: usize = 24;
        const MAX_CWD_WIDTH: usize = 24;
        const MAX_ATTN_WIDTH: usize = 7;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let filter = thread_picker_filter_label(
            state.only_fan_out_linkage_issue,
            state.only_fan_out_auto_apply_error,
            state.only_fan_in_dependency_blocked,
            state.only_subagent_proxy_approval,
        );
        let archived = if state.include_archived { "on" } else { "off" };
        let header = format!("threads [{filter}] archived={archived} (↑↓ Enter n h l a b s c r q)");
        f.render_widget(
            Paragraph::new(header).style(Style::default().fg(Color::Gray)),
            chunks[0],
        );

        let mut rows = Vec::with_capacity(state.threads.len());
        let mut max_updated_width = UnicodeWidthStr::width("Updated");
        let mut max_attn_width = UnicodeWidthStr::width("Attn");
        let mut max_title_width = UnicodeWidthStr::width("Title");
        let mut max_cwd_width = UnicodeWidthStr::width("CWD");

        for thread in &state.threads {
            let updated = format_updated_label(thread);
            let attn_badge = attention_badge(thread);
            let attn = right_elide(attn_badge.as_str(), MAX_ATTN_WIDTH);
            let title = thread
                .title
                .as_deref()
                .map(normalize_single_line)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".to_string());
            let title = right_elide(&title, MAX_TITLE_WIDTH);
            let cwd = thread
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| right_elide(s, MAX_CWD_WIDTH))
                .unwrap_or_else(|| "-".to_string());
            let message = thread
                .first_message
                .as_deref()
                .map(normalize_single_line)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".to_string());

            max_updated_width = max_updated_width.max(UnicodeWidthStr::width(updated.as_str()));
            max_attn_width = max_attn_width.max(UnicodeWidthStr::width(attn.as_str()));
            max_title_width = max_title_width.max(UnicodeWidthStr::width(title.as_str()));
            max_cwd_width = max_cwd_width.max(UnicodeWidthStr::width(cwd.as_str()));

            rows.push((updated, attn, title, cwd, message));
        }

        let column_header = format!(
            "{}  {}  {}  {}  Message",
            pad_to_width("Updated", max_updated_width),
            pad_to_width("Attn", max_attn_width),
            pad_to_width("Title", max_title_width),
            pad_to_width("CWD", max_cwd_width),
        );
        let header_line = Paragraph::new(column_header).style(Style::default().fg(Color::Gray));
        let header_area = ratatui::layout::Rect {
            x: chunks[1].x,
            y: chunks[1].y,
            width: chunks[1].width,
            height: 1,
        };
        f.render_widget(header_line, header_area);

        let list_area = ratatui::layout::Rect {
            x: chunks[1].x,
            y: chunks[1].y + 1,
            width: chunks[1].width,
            height: chunks[1].height.saturating_sub(1),
        };

        let items = rows
            .into_iter()
            .map(|(updated, attn, title, cwd, message)| {
                let line = format!(
                    "{}  {}  {}  {}  {}",
                    pad_to_width(&updated, max_updated_width),
                    pad_to_width(&attn, max_attn_width),
                    pad_to_width(&title, max_title_width),
                    pad_to_width(&cwd, max_cwd_width),
                    message
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");

        let selected = if state.threads.is_empty() {
            None
        } else {
            Some(state.selected_thread)
        };
        f.render_stateful_widget(list, list_area, &mut list_state(selected));
    }

    fn list_state(selected: Option<usize>) -> ratatui::widgets::ListState {
        let mut state = ratatui::widgets::ListState::default();
        state.select(selected);
        state
    }

    fn attention_badge(meta: &ThreadMeta) -> String {
        if meta.has_fan_out_auto_apply_error {
            return "auto!".to_string();
        }
        if meta.has_fan_out_linkage_issue {
            return "link!".to_string();
        }
        if meta.has_fan_in_dependency_blocked {
            return "fanin!".to_string();
        }
        if meta.pending_subagent_proxy_approvals > 0 {
            return subagent_pending_badge(meta.pending_subagent_proxy_approvals);
        }
        if meta.has_test_failed {
            return "test!".to_string();
        }
        if meta.has_diff_ready {
            return "diff".to_string();
        }
        if meta.has_plan_ready {
            return "plan".to_string();
        }
        match meta.attention_state.as_str() {
            "need_approval" => "approve".to_string(),
            "stuck" => "stuck".to_string(),
            "running" => "run".to_string(),
            "failed" => "failed".to_string(),
            "done" => "done".to_string(),
            "idle" => "idle".to_string(),
            "paused" => "paused".to_string(),
            "interrupted" => "intr".to_string(),
            "cancelled" => "cancel".to_string(),
            "archived" => "arch".to_string(),
            _ => "-".to_string(),
        }
    }

    fn subagent_pending_badge(count: usize) -> String {
        if count > 999 {
            "sub999+".to_string()
        } else {
            format!("sub{count}")
        }
    }
