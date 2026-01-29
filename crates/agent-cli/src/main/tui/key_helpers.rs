    fn handle_key_text_overlay(key: KeyEvent, view: &mut TextOverlay) -> OverlayOp {
        match key.code {
            KeyCode::Up => {
                view.scroll = view.scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                view.scroll = view.scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                view.scroll = view.scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                view.scroll = view.scroll.saturating_add(10);
            }
            _ => {}
        }
        OverlayOp::None
    }

    fn handle_key_command_palette(
        key: KeyEvent,
        view: &mut CommandPaletteOverlay,
    ) -> Option<PaletteCommand> {
        match key.code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if !view.filtered.is_empty() {
                    view.selected = (view.selected + 1).min(view.filtered.len() - 1);
                }
            }
            KeyCode::Enter => {
                let selected = view.selected_action();
                if let Some(action) = selected {
                    if matches!(action, PaletteCommand::Noop)
                        && view.title == "model"
                        && !view.query.trim().is_empty()
                    {
                        return Some(PaletteCommand::SetModel(
                            view.query.trim().to_string(),
                        ));
                    }
                    return Some(action);
                }
                if view.title == "model" {
                    let query = view.query.trim();
                    if !query.is_empty() {
                        return Some(PaletteCommand::SetModel(query.to_string()));
                    }
                }
                return None;
            }
            KeyCode::Backspace => {
                view.query.pop();
                view.selected = 0;
                view.rebuild_filter();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                view.query.clear();
                view.selected = 0;
                view.rebuild_filter();
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    view.query.push(c);
                    view.selected = 0;
                    view.rebuild_filter();
                }
            }
            _ => {}
        }

        None
    }
