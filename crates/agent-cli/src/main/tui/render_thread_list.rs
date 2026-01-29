    fn draw_thread_list(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        const MAX_TITLE_WIDTH: usize = 24;
        const MAX_CWD_WIDTH: usize = 24;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let header = "threads (↑↓ Enter=open n=new r=refresh q/Ctrl-C=quit)";
        f.render_widget(
            Paragraph::new(header).style(Style::default().fg(Color::Gray)),
            chunks[0],
        );

        let mut rows = Vec::with_capacity(state.threads.len());
        let mut max_updated_width = UnicodeWidthStr::width("Updated");
        let mut max_title_width = UnicodeWidthStr::width("Title");
        let mut max_cwd_width = UnicodeWidthStr::width("CWD");

        for thread in &state.threads {
            let updated = format_updated_label(thread);
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
            max_title_width = max_title_width.max(UnicodeWidthStr::width(title.as_str()));
            max_cwd_width = max_cwd_width.max(UnicodeWidthStr::width(cwd.as_str()));

            rows.push((updated, title, cwd, message));
        }

        let column_header = format!(
            "{}  {}  {}  Message",
            pad_to_width("Updated", max_updated_width),
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
            .map(|(updated, title, cwd, message)| {
                let line = format!(
                    "{}  {}  {}  {}",
                    pad_to_width(&updated, max_updated_width),
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
