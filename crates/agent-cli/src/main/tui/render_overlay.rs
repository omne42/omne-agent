    fn draw_overlay(f: &mut ratatui::Frame, overlay: &Overlay) {
        match overlay {
            Overlay::CommandPalette(view) => {
                draw_command_palette_overlay(f, f.area(), view);
            }
            _ => {
                let area = centered_rect(90, 80, f.area());
                f.render_widget(Clear, area);
                match overlay {
                    Overlay::Approvals(view) => draw_approvals_overlay(f, area, view),
                    Overlay::Processes(view) => draw_processes_overlay(f, area, view),
                    Overlay::Artifacts(view) => draw_artifacts_overlay(f, area, view),
                    Overlay::Text(view) => draw_text_overlay(f, area, view),
                    Overlay::CommandPalette(_) => {}
                }
            }
        }
    }

    fn draw_command_palette_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &CommandPaletteOverlay,
    ) {
        if area.height == 0 {
            return;
        }
        let width = area.width.max(1) as usize;
        let max_lines = ((area.height as usize) * 60 / 100)
            .max(3)
            .min(area.height as usize);
        let palette_render = build_command_palette_lines(view, width, max_lines);
        if palette_render.lines.is_empty() {
            return;
        }
        let height = palette_render.lines.len().min(max_lines).max(1) as u16;
        let rect = ratatui::layout::Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(height)),
            width: area.width,
            height,
        };
        f.render_widget(Clear, rect);
        let paragraph = Paragraph::new(Text::from(palette_render.lines));
        f.render_widget(paragraph, rect);
    }

    fn draw_approvals_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &ApprovalsOverlay,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                "Approvals (↑↓ select y=approve n=deny m=remember r=refresh Esc=close) remember={}",
                view.remember
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);

        let items = view
            .approvals
            .iter()
            .map(|item| {
                let line = format!(
                    "{} {}",
                    item.request.approval_id,
                    item.request.action.trim()
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Pending"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        let selected = if view.approvals.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[0], &mut list_state(selected));

        let details = view
            .approvals
            .get(view.selected)
            .map(build_approval_details)
            .unwrap_or_else(|| "no approvals".to_string());
        let paragraph = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[1]);
    }

    fn draw_processes_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &ProcessesOverlay,
    ) {
        let block = Block::default().borders(Borders::ALL).title(
            "Processes (↑↓ select Enter=inspect k=kill x=interrupt r=refresh Esc=close)",
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);

        let items = view
            .processes
            .iter()
            .map(|process| {
                let argv0 = process.argv.first().map(String::as_str).unwrap_or("");
                let line = format!(
                    "[{}] {} {}",
                    process_status_str(process.status),
                    process.process_id,
                    argv0
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Processes"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        let selected = if view.processes.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[0], &mut list_state(selected));

        let details = view
            .processes
            .get(view.selected)
            .map(build_process_details)
            .unwrap_or_else(|| "no processes".to_string());
        let paragraph = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[1]);
    }

    fn build_process_details(process: &ProcessInfo) -> String {
        let mut out = String::new();
        out.push_str(&format!("process_id: {}\n", process.process_id));
        out.push_str(&format!("thread_id: {}\n", process.thread_id));
        out.push_str(&format!(
            "status: {}\n",
            process_status_str(process.status)
        ));
        if let Some(turn_id) = process.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str(&format!("started_at: {}\n", process.started_at));
        out.push_str(&format!("last_update_at: {}\n", process.last_update_at));
        if let Some(exit_code) = process.exit_code {
            out.push_str(&format!("exit_code: {exit_code}\n"));
        }
        out.push_str(&format!("cwd: {}\n", process.cwd));
        out.push_str(&format!("argv: {}\n", process.argv.join(" ")));
        out.push_str(&format!("stdout_path: {}\n", process.stdout_path));
        out.push_str(&format!("stderr_path: {}\n", process.stderr_path));
        out
    }

    fn draw_artifacts_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &ArtifactsOverlay,
    ) {
        let block =
            Block::default()
                .borders(Borders::ALL)
                .title("Artifacts (↑↓ select Enter=read r=refresh Esc=close)");
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);

        let items = view
            .artifacts
            .iter()
            .map(|meta| {
                let line = format!(
                    "[{}] {} {}",
                    meta.artifact_type.trim(),
                    meta.artifact_id,
                    meta.summary.trim()
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Artifacts"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        let selected = if view.artifacts.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[0], &mut list_state(selected));

        let details = view
            .artifacts
            .get(view.selected)
            .map(build_artifact_details)
            .unwrap_or_else(|| "no artifacts".to_string());
        let paragraph = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[1]);
    }

    fn build_artifact_details(meta: &ArtifactMetadata) -> String {
        let mut out = String::new();
        out.push_str(&format!("artifact_id: {}\n", meta.artifact_id));
        out.push_str(&format!("artifact_type: {}\n", meta.artifact_type));
        out.push_str(&format!("summary: {}\n", meta.summary));
        out.push_str(&format!("version: {}\n", meta.version));
        out.push_str(&format!("size_bytes: {}\n", meta.size_bytes));
        out.push_str(&format!(
            "updated_at_unix: {}\n",
            meta.updated_at.unix_timestamp()
        ));
        out.push_str(&format!("content_path: {}\n", meta.content_path));
        out
    }

    fn build_approval_details(item: &ApprovalItem) -> String {
        let mut out = String::new();
        out.push_str(&format!("approval_id: {}\n", item.request.approval_id));
        out.push_str(&format!("requested_at: {}\n", item.request.requested_at));
        out.push_str(&format!("action: {}\n", item.request.action));
        if let Some(turn_id) = item.request.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str("\nparams:\n");
        out.push_str(
            &serde_json::to_string_pretty(&item.request.params)
                .unwrap_or_else(|_| item.request.params.to_string()),
        );
        out
    }

    fn draw_text_overlay(f: &mut ratatui::Frame, area: ratatui::layout::Rect, view: &TextOverlay) {
        let block = Block::default().borders(Borders::ALL).title(view.title.as_str());
        let paragraph = Paragraph::new(view.text.as_str())
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((view.scroll, 0));
        f.render_widget(paragraph, area);
    }

    fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ])
            .split(r);

        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(popup_layout[1]);

        horizontal[1]
    }
