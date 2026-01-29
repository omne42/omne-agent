    impl UiState {
        async fn handle_key_overlay(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            let mut status = None::<String>;
            let mut decided = None::<ApprovalId>;
            let mut set_pending_action = None::<PendingAction>;
            let mut palette_command = None::<PaletteCommand>;
            let op;

            let close_with_q =
                key.code == KeyCode::Char('q')
                    && !matches!(self.overlays.last(), Some(Overlay::CommandPalette(_)));
            let closing_palette = matches!(self.overlays.last(), Some(Overlay::CommandPalette(_)));
            if key.code == KeyCode::Esc || close_with_q {
                if closing_palette {
                    self.cancel_model_fetch();
                }
                self.overlays.pop();
                return Ok(false);
            }

            {
                let Some(overlay) = self.overlays.last_mut() else {
                    return Ok(false);
                };
                match overlay {
                    Overlay::Approvals(view) => {
                        (op, status, decided) =
                            handle_key_approvals_overlay(app, key, view).await?;
                    }
                    Overlay::Processes(view) => {
                        (op, status, set_pending_action) =
                            handle_key_processes_overlay(app, key, view).await?;
                    }
                    Overlay::Artifacts(view) => {
                        (op, status, set_pending_action) =
                            handle_key_artifacts_overlay(app, key, view).await?;
                    }
                    Overlay::Text(view) => {
                        op = handle_key_text_overlay(key, view);
                    }
                    Overlay::CommandPalette(view) => {
                        palette_command = handle_key_command_palette(key, view);
                        op = OverlayOp::None;
                    }
                }
            }

            if let Some(msg) = status {
                self.set_status(msg);
            }

            if let Some(pending) = set_pending_action {
                self.pending_action = Some(pending);
            }

            if let Some(approval_id) = decided {
                if self
                    .pending_action
                    .as_ref()
                    .is_some_and(|pending| pending.approval_id() == approval_id)
                {
                    self.resume_pending_action(app).await?;
                }
            }

            match op {
                OverlayOp::None => {}
                OverlayOp::Push(overlay) => {
                    self.overlays.push(overlay);
                }
            }

            if let Some(command) = palette_command {
                return self.execute_palette_command(app, command).await;
            }

            Ok(false)
        }

        async fn handle_key(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            if let Some(status) = self.status.take() {
                if Self::is_error_message(&status) {
                    self.status = Some(status);
                }
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                let mut cleared_input = false;
                if self.active_thread.is_some() && !self.input.trim().is_empty() {
                    self.input.clear();
                    self.transcript_follow = true;
                    self.update_inline_palette(app).await?;
                    cleared_input = true;
                }
                if !self.overlays.is_empty() {
                    let closing_palette =
                        matches!(self.overlays.last(), Some(Overlay::CommandPalette(_)));
                    if closing_palette {
                        self.cancel_model_fetch();
                    }
                    self.overlays.pop();
                    return Ok(false);
                }
                if self.active_thread.is_some() {
                    if cleared_input {
                        return Ok(false);
                    }
                    return Ok(true);
                }
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
                if self.active_thread.is_some() {
                    self.insert_command_trigger();
                    self.update_inline_palette(app).await?;
                } else {
                    self.toggle_command_palette();
                }
                return Ok(false);
            }
            if self.active_thread.is_none()
                && self.overlays.is_empty()
                && !key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('/')
            {
                self.toggle_command_palette();
                return Ok(false);
            }

            if !self.overlays.is_empty() {
                return self.handle_key_overlay(app, key).await;
            }

            match self.active_thread {
                None => self.handle_key_threads(app, key).await,
                Some(_) => self.handle_key_thread_view(app, key).await,
            }
        }

        async fn handle_key_threads(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
                KeyCode::Char('r') => {
                    self.refresh_threads(app).await?;
                }
                KeyCode::Char('n') => {
                    let started = app.thread_start(None).await?;
                    let thread_id: ThreadId =
                        serde_json::from_value(started["thread_id"].clone())
                            .context("thread_id missing")?;
                    self.open_thread(app, thread_id).await?;
                }
                KeyCode::Up => {
                    self.selected_thread = self.selected_thread.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.threads.is_empty() {
                        self.selected_thread =
                            (self.selected_thread + 1).min(self.threads.len() - 1);
                    }
                }
                KeyCode::Enter => {
                    let Some(meta) = self.threads.get(self.selected_thread).cloned() else {
                        return Ok(false);
                    };
                    self.open_thread(app, meta.thread_id).await?;
                }
                _ => {}
            }
            Ok(false)
        }

        async fn handle_key_thread_view(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('a') => {
                        self.open_approvals_overlay(app).await?;
                        return Ok(false);
                    }
                    KeyCode::Char('p') => {
                        self.open_processes_overlay(app).await?;
                        return Ok(false);
                    }
                    KeyCode::Char('o') => {
                        self.open_artifacts_overlay(app).await?;
                        return Ok(false);
                    }
                    _ => {}
                }
            }

            let inline_active = self.inline_palette.is_some();
            let mut input_changed = false;

            match key.code {
                KeyCode::Tab if key.modifiers.is_empty() => {
                    self.cycle_thinking(app).await?;
                }
                KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                    if self.transcript_scroll >= self.transcript_max_scroll {
                        self.transcript_scroll = self.transcript_max_scroll;
                        self.transcript_follow = true;
                    }
                }
                KeyCode::Up => {
                    if inline_active {
                        self.move_inline_selection(-1);
                    } else {
                        self.transcript_follow = false;
                        self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if inline_active {
                        self.move_inline_selection(1);
                    } else {
                        self.transcript_follow = false;
                        self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                        if self.transcript_scroll >= self.transcript_max_scroll {
                            self.transcript_scroll = self.transcript_max_scroll;
                            self.transcript_follow = true;
                        }
                    }
                }
                KeyCode::PageUp => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self
                        .transcript_scroll
                        .saturating_sub(self.transcript_page());
                }
                KeyCode::PageDown => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self
                        .transcript_scroll
                        .saturating_add(self.transcript_page());
                    if self.transcript_scroll >= self.transcript_max_scroll {
                        self.transcript_scroll = self.transcript_max_scroll;
                        self.transcript_follow = true;
                    }
                }
                KeyCode::Home => {
                    self.transcript_follow = false;
                    self.transcript_scroll = 0;
                }
                KeyCode::End => {
                    self.transcript_scroll = self.transcript_max_scroll;
                    self.transcript_follow = true;
                }
                KeyCode::Esc => {
                    if !self.input.trim().is_empty() {
                        self.input.clear();
                        self.transcript_follow = true;
                        input_changed = true;
                    } else {
                        return Ok(true);
                    }
                }
                KeyCode::Enter
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    self.input.push('\n');
                    input_changed = true;
                }
                KeyCode::Enter => {
                    if inline_active {
                        if let Some(command) = self.inline_selected_action() {
                            return self.execute_inline_command(app, command).await;
                        }
                        if let Some(inline) = self.inline_palette.as_ref() {
                            if inline.kind == InlinePaletteKind::Model {
                                let query = inline.view.query.trim();
                                if !query.is_empty() {
                                    return self
                                        .execute_inline_command(
                                            app,
                                            PaletteCommand::SetModel(query.to_string()),
                                        )
                                        .await;
                                }
                            }
                        }
                        return Ok(false);
                    }
                    let input = self.input.clone();
                    if input.trim().is_empty() {
                        return Ok(false);
                    }
                    let Some(thread_id) = self.active_thread else {
                        return Ok(false);
                    };
                    if self.turn_start.is_some() {
                        self.set_status("turn/start pending".to_string());
                        return Ok(false);
                    }
                    let rpc_handle = app.rpc_handle();
                    let pending = match spawn_turn_start(rpc_handle, thread_id, input, None) {
                        Ok(pending) => pending,
                        Err(err) => {
                            self.set_status(format!("turn/start error: {err}"));
                            return Ok(false);
                        }
                    };
                    self.input.clear();
                    self.turn_start = Some(pending);
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    input_changed = true;
                }
                KeyCode::Char(c) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.input.push(c);
                        input_changed = true;
                    }
                }
                _ => {}
            }
            if input_changed {
                self.transcript_follow = true;
                self.update_inline_palette(app).await?;
            }
            Ok(false)
        }

        async fn return_to_thread_picker(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            self.active_thread = None;
            self.header = HeaderState::default();
            self.header_needs_refresh = false;
            self.overlays.clear();
            self.inline_palette = None;
            self.input.clear();
            self.streaming = None;
            self.active_turn_id = None;
            self.total_tokens_used = 0;
            self.counted_usage_responses.clear();
            self.skip_token_usage_before_seq = None;
            self.pending_action = None;
            self.cancel_turn_start();
            self.transcript_scroll = 0;
            self.transcript_follow = true;
            self.transcript_max_scroll = 0;
            self.transcript_viewport_height = 0;
            self.model_list.clear();
            self.model_list_loaded = false;
            self.thread_cwd = None;
            self.mode_catalog.clear();
            self.mode_catalog_loaded = false;
            self.skill_catalog.clear();
            self.skill_catalog_loaded = false;
            self.refresh_threads(app).await?;
            Ok(())
        }

        fn toggle_command_palette(&mut self) {
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                self.cancel_model_fetch();
                self.overlays.pop();
                return;
            }

            self.transcript_follow = true;
            self.overlays
                .push(Overlay::CommandPalette(build_root_palette(self)));
        }

        fn replace_top_command_palette(&mut self, palette: CommandPaletteOverlay) {
            match self.overlays.last_mut() {
                Some(Overlay::CommandPalette(view)) => {
                    *view = palette;
                }
                _ => {
                    self.overlays.push(Overlay::CommandPalette(palette));
                }
            }
        }

        async fn execute_palette_command(
            &mut self,
            app: &mut super::App,
            command: PaletteCommand,
        ) -> anyhow::Result<bool> {
            match command {
                PaletteCommand::Quit => return Ok(true),
                PaletteCommand::Noop => {}
                PaletteCommand::OpenRoot => {
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::Help => {
                    self.overlays.pop();
                    self.overlays.push(Overlay::Text(TextOverlay {
                        title: "Help".to_string(),
                        text: tui_help_text(),
                        scroll: 0,
                    }));
                }
                PaletteCommand::NewThread => {
                    let started = match app.thread_start(None).await {
                        Ok(v) => v,
                        Err(err) => {
                            self.set_status(format!("thread/start error: {err}"));
                            return Ok(false);
                        }
                    };
                    let thread_id: ThreadId =
                        serde_json::from_value(started["thread_id"].clone()).context("thread_id missing")?;
                    self.open_thread(app, thread_id).await?;
                }
                PaletteCommand::ThreadPicker => {
                    self.return_to_thread_picker(app).await?;
                }
                PaletteCommand::RefreshThreads => {
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status("refreshed".to_string());
                    }
                }
                PaletteCommand::OpenApprovals => {
                    self.overlays.pop();
                    self.open_approvals_overlay(app).await?;
                }
                PaletteCommand::OpenProcesses => {
                    self.overlays.pop();
                    self.open_processes_overlay(app).await?;
                }
                PaletteCommand::OpenArtifacts => {
                    self.overlays.pop();
                    self.open_artifacts_overlay(app).await?;
                }
                PaletteCommand::PickMode => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    let config = match app.thread_config_explain(thread_id).await {
                        Ok(v) => v,
                        Err(err) => {
                            self.set_status(format!("thread/config/explain error: {err}"));
                            return Ok(false);
                        }
                    };
                    let modes = config
                        .get("mode_catalog")
                        .and_then(|v| v.get("modes"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    self.replace_top_command_palette(build_mode_palette(
                        modes,
                        self.header.mode.as_deref(),
                    ));
                }
                PaletteCommand::PickModel => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    if self.model_fetch.is_some() {
                        self.set_status("model list already loading".to_string());
                        return Ok(false);
                    }
                    let rpc_handle = app.rpc_handle();
                    self.model_fetch_pending = true;
                    self.replace_top_command_palette(build_model_loading_palette());
                    self.model_fetch = Some(spawn_model_fetch(
                        rpc_handle,
                        thread_id,
                        Duration::from_secs(2),
                    ));
                    self.set_status("loading models...".to_string());
                }
                PaletteCommand::PickApprovalPolicy => {
                    self.replace_top_command_palette(build_approval_policy_palette());
                }
                PaletteCommand::PickSandboxPolicy => {
                    self.replace_top_command_palette(build_sandbox_policy_palette());
                }
                PaletteCommand::PickSandboxNetworkAccess => {
                    self.replace_top_command_palette(build_sandbox_network_access_palette());
                }
                PaletteCommand::SetMode(mode) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: Some(mode.clone()),
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set mode error: {err}"));
                    } else {
                        self.set_status(format!("mode={mode}"));
                        let _ = self.refresh_header(app, thread_id).await;
                    }
                }
                PaletteCommand::SetModel(model) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
                            model: Some(model.clone()),
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set model error: {err}"));
                    } else {
                        self.set_status(format!("model={model}"));
                        let _ = self.refresh_header(app, thread_id).await;
                    }
                }
                PaletteCommand::SetApprovalPolicy(approval_policy) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: Some(approval_policy),
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set approval_policy error: {err}"));
                    } else {
                        self.set_status(format!(
                            "approval_policy={}",
                            approval_policy_label(approval_policy)
                        ));
                    }
                }
                PaletteCommand::SetSandboxPolicy(sandbox_policy) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: Some(sandbox_policy),
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set sandbox_policy error: {err}"));
                    } else {
                        self.set_status(format!(
                            "sandbox_policy={}",
                            sandbox_policy_label(sandbox_policy)
                        ));
                    }
                }
                PaletteCommand::SetSandboxNetworkAccess(sandbox_network_access) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: Some(sandbox_network_access),
                            mode: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set sandbox_network_access error: {err}"));
                    } else {
                        self.set_status(format!(
                            "sandbox_network_access={}",
                            sandbox_network_access_label(sandbox_network_access)
                        ));
                    }
                }
                PaletteCommand::InsertSkill(_) => {}
            }

            Ok(false)
        }

        fn transcript_page(&self) -> u16 {
            self.transcript_viewport_height.saturating_sub(1).max(1)
        }
    }
