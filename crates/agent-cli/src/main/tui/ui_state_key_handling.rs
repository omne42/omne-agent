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
            self.dismiss_non_error_status();
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
                if !self.overlays.is_empty() {
                    let _ = self.toggle_overlay_command_palette();
                } else if self.active_thread.is_some() {
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
                KeyCode::Char('h') => {
                    self.include_archived = !self.include_archived;
                    self.refresh_threads(app).await?;
                    self.set_status(format!(
                        "thread list: include_archived={}",
                        if self.include_archived { "on" } else { "off" }
                    ));
                }
                KeyCode::Char('l') => {
                    self.only_fan_out_linkage_issue = !self.only_fan_out_linkage_issue;
                    self.refresh_threads(app).await?;
                    self.set_status(format!(
                        "thread filter: fan_out_linkage_issue={}",
                        if self.only_fan_out_linkage_issue {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                }
                KeyCode::Char('a') => {
                    self.only_fan_out_auto_apply_error = !self.only_fan_out_auto_apply_error;
                    self.refresh_threads(app).await?;
                    self.set_status(format!(
                        "thread filter: fan_out_auto_apply_error={}",
                        if self.only_fan_out_auto_apply_error {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                }
                KeyCode::Char('b') => {
                    self.only_fan_in_dependency_blocked = !self.only_fan_in_dependency_blocked;
                    self.refresh_threads(app).await?;
                    self.set_status(format!(
                        "thread filter: fan_in_dependency_blocked={}",
                        if self.only_fan_in_dependency_blocked {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                }
                KeyCode::Char('s') => {
                    self.only_subagent_proxy_approval = !self.only_subagent_proxy_approval;
                    self.refresh_threads(app).await?;
                    self.set_status(format!(
                        "thread filter: subagent_proxy_approval={}",
                        if self.only_subagent_proxy_approval {
                            "on"
                        } else {
                            "off"
                        }
                    ));
                }
                KeyCode::Char('c') => {
                    let changed = self.clear_thread_picker_filters();
                    self.refresh_threads(app).await?;
                    self.set_status(if changed {
                        "thread filters cleared".to_string()
                    } else {
                        "thread filters already clear".to_string()
                    });
                }
                KeyCode::Char('n') => {
                    let started = app.thread_start(None).await?;
                    self.open_thread(app, started.thread_id).await?;
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
                        if self.execute_inline_list_command_from_query(app).await? {
                            return Ok(false);
                        }
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
            self.subagent_pending_summary = None;
            self.subagent_pending_summary_needs_refresh = false;
            self.current_context_tokens_estimate = None;
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

        fn toggle_overlay_command_palette(&mut self) -> bool {
            if self.overlays.is_empty() {
                return false;
            }
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                self.cancel_model_fetch();
                self.overlays.pop();
                return true;
            }
            if let Some(palette) = build_overlay_palette(self) {
                let context = palette.context_label();
                self.overlays.push(Overlay::CommandPalette(palette));
                self.set_temporary_status(
                    format!("overlay commands: {context}"),
                    Duration::from_secs(2),
                );
                return true;
            }
            self.set_temporary_status(
                "overlay commands unavailable".to_string(),
                Duration::from_secs(2),
            );
            false
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
            if let Some(key) = overlay_palette_command_key(&command) {
                if self.apply_overlay_palette_local_key(key) {
                    return Ok(false);
                }
                self.forward_palette_key_to_underlying_overlay(app, key).await?;
                return Ok(false);
            }

            match command {
                PaletteCommand::Quit => return Ok(true),
                PaletteCommand::ClosePalette => {
                    if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                        self.overlays.pop();
                    }
                }
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
                    self.open_thread(app, started.thread_id).await?;
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
                PaletteCommand::ToggleIncludeArchived => {
                    self.include_archived = !self.include_archived;
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status(format!(
                            "thread list: include_archived={}",
                            if self.include_archived { "on" } else { "off" }
                        ));
                    }
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::ToggleLinkageFilter => {
                    self.only_fan_out_linkage_issue = !self.only_fan_out_linkage_issue;
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status(format!(
                            "thread filter: fan_out_linkage_issue={}",
                            if self.only_fan_out_linkage_issue {
                                "on"
                            } else {
                                "off"
                            }
                        ));
                    }
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::ToggleAutoApplyErrorFilter => {
                    self.only_fan_out_auto_apply_error = !self.only_fan_out_auto_apply_error;
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status(format!(
                            "thread filter: fan_out_auto_apply_error={}",
                            if self.only_fan_out_auto_apply_error {
                                "on"
                            } else {
                                "off"
                            }
                        ));
                    }
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::ToggleFanInDependencyBlockedFilter => {
                    self.only_fan_in_dependency_blocked = !self.only_fan_in_dependency_blocked;
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status(format!(
                            "thread filter: fan_in_dependency_blocked={}",
                            if self.only_fan_in_dependency_blocked {
                                "on"
                            } else {
                                "off"
                            }
                        ));
                    }
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::ToggleSubagentProxyApprovalFilter => {
                    self.only_subagent_proxy_approval = !self.only_subagent_proxy_approval;
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status(format!(
                            "thread filter: subagent_proxy_approval={}",
                            if self.only_subagent_proxy_approval {
                                "on"
                            } else {
                                "off"
                            }
                        ));
                    }
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::ClearThreadFilters => {
                    let changed = self.clear_thread_picker_filters();
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status(if changed {
                            "thread filters cleared".to_string()
                        } else {
                            "thread filters already clear".to_string()
                        });
                    }
                    self.replace_top_command_palette(build_root_palette(self));
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
                    let modes = config.mode_catalog.modes;
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
                PaletteCommand::PickAllowedTools => {
                    self.overlays.pop();
                    self.input = "/allowed-tools ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                }
                PaletteCommand::PickExecpolicyRules => {
                    self.overlays.pop();
                    self.input = "/execpolicy-rules ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
                        })
                        .await
                    {
                        self.set_status(format!("set mode error: {err}"));
                    } else {
                        self.set_status(format!("mode={mode}"));
                        if let Err(err) = self.refresh_header(app, thread_id).await {
                            self.set_status(format!("refresh header error: {err}"));
                        }
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
        role: None,
                            model: Some(model.clone()),
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
                        })
                        .await
                    {
                        self.set_status(format!("set model error: {err}"));
                    } else {
                        self.set_status(format!("model={model}"));
                        if let Err(err) = self.refresh_header(app, thread_id).await {
                            self.set_status(format!("refresh header error: {err}"));
                        }
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
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
                PaletteCommand::SetAllowedTools(tool) => {
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: Some(vec![tool.clone()]),
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
                        })
                        .await
                    {
                        self.set_status(format!("set allowed_tools error: {err}"));
                    } else {
                        self.set_status(format!("allowed_tools=[{tool}]"));
                        self.header_needs_refresh = true;
                    }
                }
                PaletteCommand::ClearAllowedTools => {
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: true,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
                        })
                        .await
                    {
                        self.set_status(format!("clear allowed_tools error: {err}"));
                    } else {
                        self.set_status("allowed_tools=<cleared>".to_string());
                        self.header_needs_refresh = true;
                    }
                }
                PaletteCommand::ClearExecpolicyRules => {
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
        role: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: true,
                        })
                        .await
                    {
                        self.set_status(format!("clear execpolicy_rules error: {err}"));
                    } else {
                        self.set_status("execpolicy_rules=<cleared>".to_string());
                        self.header_needs_refresh = true;
                    }
                }
                PaletteCommand::InsertSkill(_) => {}
                PaletteCommand::ApprovalsCycleFilter
                | PaletteCommand::ApprovalsNextFailed
                | PaletteCommand::ApprovalsPrevFailed
                | PaletteCommand::ApprovalsRefresh
                | PaletteCommand::ApprovalsSelectPrev
                | PaletteCommand::ApprovalsSelectNext
                | PaletteCommand::ApprovalsApprove
                | PaletteCommand::ApprovalsDeny
                | PaletteCommand::ApprovalsToggleRemember
                | PaletteCommand::ApprovalsOpenDetails
                | PaletteCommand::ProcessesRefresh
                | PaletteCommand::ProcessesSelectPrev
                | PaletteCommand::ProcessesSelectNext
                | PaletteCommand::ProcessesInspect
                | PaletteCommand::ProcessesKill
                | PaletteCommand::ProcessesInterrupt
                | PaletteCommand::ArtifactsRefresh
                | PaletteCommand::ArtifactsSelectPrev
                | PaletteCommand::ArtifactsSelectNext
                | PaletteCommand::ArtifactsRead
                | PaletteCommand::ArtifactsLoadVersions
                | PaletteCommand::ArtifactsReloadVersions
                | PaletteCommand::ArtifactsPrevVersion
                | PaletteCommand::ArtifactsNextVersion
                | PaletteCommand::ArtifactsLatestVersion => {}
            }

            Ok(false)
        }

        fn apply_overlay_palette_local_key(&mut self, key: KeyEvent) -> bool {
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                self.overlays.pop();
            }
            let mut status = None::<String>;
            let Some(overlay) = self.overlays.last_mut() else {
                return false;
            };
            let handled = match overlay {
                Overlay::Approvals(view) => {
                    match handle_local_approvals_key(view, key.code) {
                        ApprovalsLocalKeyResult::Handled(local_status) => {
                            status = local_status;
                            true
                        }
                        ApprovalsLocalKeyResult::Unhandled => false,
                    }
                }
                Overlay::Processes(view) => {
                    matches!(
                        handle_local_processes_key(view, key.code),
                        ProcessesLocalKeyResult::Handled
                    )
                }
                Overlay::Artifacts(view) => {
                    match handle_local_artifacts_key(view, key.code) {
                        ArtifactsLocalKeyResult::Handled(local_status) => {
                            status = local_status;
                            true
                        }
                        ArtifactsLocalKeyResult::Unhandled => false,
                    }
                }
                Overlay::Text(_) | Overlay::CommandPalette(_) => false,
            };
            if let Some(msg) = status {
                self.set_status(msg);
            }
            handled
        }

        async fn forward_palette_key_to_underlying_overlay(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<()> {
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                self.overlays.pop();
            }
            if self.overlays.is_empty() {
                return Ok(());
            }

            let mut status = None::<String>;
            let mut set_pending_action = None::<PendingAction>;
            let op;

            {
                let Some(overlay) = self.overlays.last_mut() else {
                    return Ok(());
                };
                match overlay {
                    Overlay::Approvals(view) => {
                        if let ApprovalsLocalKeyResult::Handled(local_status) =
                            handle_local_approvals_key(view, key.code)
                        {
                            status = local_status;
                        }
                        op = OverlayOp::None;
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
                    Overlay::CommandPalette(_) => {
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
            if let OverlayOp::Push(overlay) = op {
                self.overlays.push(overlay);
            }

            Ok(())
        }

        fn transcript_page(&self) -> u16 {
            self.transcript_viewport_height.saturating_sub(1).max(1)
        }
    }

    fn overlay_palette_command_key(command: &PaletteCommand) -> Option<KeyEvent> {
        let code = match command {
            PaletteCommand::ApprovalsCycleFilter => KeyCode::Char('t'),
            PaletteCommand::ApprovalsNextFailed => KeyCode::Char('f'),
            PaletteCommand::ApprovalsPrevFailed => KeyCode::Char('F'),
            PaletteCommand::ApprovalsRefresh => KeyCode::Char('r'),
            PaletteCommand::ApprovalsSelectPrev => KeyCode::Up,
            PaletteCommand::ApprovalsSelectNext => KeyCode::Down,
            PaletteCommand::ApprovalsApprove => KeyCode::Char('y'),
            PaletteCommand::ApprovalsDeny => KeyCode::Char('n'),
            PaletteCommand::ApprovalsToggleRemember => KeyCode::Char('m'),
            PaletteCommand::ApprovalsOpenDetails => KeyCode::Enter,
            PaletteCommand::ProcessesRefresh => KeyCode::Char('r'),
            PaletteCommand::ProcessesSelectPrev => KeyCode::Up,
            PaletteCommand::ProcessesSelectNext => KeyCode::Down,
            PaletteCommand::ProcessesInspect => KeyCode::Enter,
            PaletteCommand::ProcessesKill => KeyCode::Char('k'),
            PaletteCommand::ProcessesInterrupt => KeyCode::Char('x'),
            PaletteCommand::ArtifactsRefresh => KeyCode::Char('r'),
            PaletteCommand::ArtifactsSelectPrev => KeyCode::Up,
            PaletteCommand::ArtifactsSelectNext => KeyCode::Down,
            PaletteCommand::ArtifactsRead => KeyCode::Enter,
            PaletteCommand::ArtifactsLoadVersions => KeyCode::Char('v'),
            PaletteCommand::ArtifactsReloadVersions => KeyCode::Char('R'),
            PaletteCommand::ArtifactsPrevVersion => KeyCode::Char('['),
            PaletteCommand::ArtifactsNextVersion => KeyCode::Char(']'),
            PaletteCommand::ArtifactsLatestVersion => KeyCode::Char('0'),
            _ => return None,
        };
        Some(KeyEvent::new(code, KeyModifiers::NONE))
    }
