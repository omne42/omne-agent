    impl UiState {
        async fn execute_inline_command(
            &mut self,
            app: &mut super::App,
            command: PaletteCommand,
        ) -> anyhow::Result<bool> {
            match command {
                PaletteCommand::InsertSkill(skill) => {
                    self.replace_inline_token('$', &format!("${skill}"), true);
                    self.inline_palette = None;
                    Ok(false)
                }
                PaletteCommand::SetMode(mode) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    app.thread_configure(super::ThreadConfigureArgs {
                        thread_id,
                        approval_policy: None,
                        sandbox_policy: None,
                        sandbox_writable_roots: None,
                        sandbox_network_access: None,
                        mode: Some(mode.clone()),
                        openai_provider: None,
                        model: None,
                        openai_base_url: None,
                        thinking: None,
                    })
                    .await?;
                    self.header.mode = Some(mode.clone());
                    self.set_status(format!("mode={}", normalize_label(&mode)));
                    self.clear_inline_line();
                    self.inline_palette = None;
                    Ok(false)
                }
                PaletteCommand::SetModel(model) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    app.thread_configure(super::ThreadConfigureArgs {
                        thread_id,
                        approval_policy: None,
                        sandbox_policy: None,
                        sandbox_writable_roots: None,
                        sandbox_network_access: None,
                        mode: None,
                        openai_provider: None,
                        model: Some(model.clone()),
                        openai_base_url: None,
                        thinking: None,
                    })
                    .await?;
                    self.header.model = Some(model.clone());
                    self.set_status(format!("model={model}"));
                    self.clear_inline_line();
                    self.inline_palette = None;
                    Ok(false)
                }
                PaletteCommand::SetApprovalPolicy(policy) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    app.thread_configure(super::ThreadConfigureArgs {
                        thread_id,
                        approval_policy: Some(policy),
                        sandbox_policy: None,
                        sandbox_writable_roots: None,
                        sandbox_network_access: None,
                        mode: None,
                        openai_provider: None,
                        model: None,
                        openai_base_url: None,
                        thinking: None,
                    })
                    .await?;
                    self.set_status(format!(
                        "approval-policy={}",
                        approval_policy_label(policy)
                    ));
                    self.clear_inline_line();
                    self.inline_palette = None;
                    Ok(false)
                }
                PaletteCommand::SetSandboxPolicy(policy) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    app.thread_configure(super::ThreadConfigureArgs {
                        thread_id,
                        approval_policy: None,
                        sandbox_policy: Some(policy),
                        sandbox_writable_roots: None,
                        sandbox_network_access: None,
                        mode: None,
                        openai_provider: None,
                        model: None,
                        openai_base_url: None,
                        thinking: None,
                    })
                    .await?;
                    self.set_status(format!(
                        "sandbox-policy={}",
                        sandbox_policy_label(policy)
                    ));
                    self.clear_inline_line();
                    self.inline_palette = None;
                    Ok(false)
                }
                PaletteCommand::SetSandboxNetworkAccess(access) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    app.thread_configure(super::ThreadConfigureArgs {
                        thread_id,
                        approval_policy: None,
                        sandbox_policy: None,
                        sandbox_writable_roots: None,
                        sandbox_network_access: Some(access),
                        mode: None,
                        openai_provider: None,
                        model: None,
                        openai_base_url: None,
                        thinking: None,
                    })
                    .await?;
                    self.set_status(format!(
                        "sandbox-network={}",
                        sandbox_network_access_label(access)
                    ));
                    self.clear_inline_line();
                    self.inline_palette = None;
                    Ok(false)
                }
                PaletteCommand::PickMode => {
                    self.set_input("/mode ".to_string());
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickModel => {
                    self.set_input("/model ".to_string());
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickApprovalPolicy => {
                    self.set_input("/approval-policy ".to_string());
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickSandboxPolicy => {
                    self.set_input("/sandbox-policy ".to_string());
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickSandboxNetworkAccess => {
                    self.set_input("/sandbox-network ".to_string());
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::OpenRoot => {
                    self.set_input("/".to_string());
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::Noop => Ok(false),
                _ => {
                    let exit = self.execute_palette_command(app, command).await?;
                    self.clear_inline_line();
                    self.inline_palette = None;
                    Ok(exit)
                }
            }
        }

        async fn cycle_thinking(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(thread_id) = self.active_thread else {
                return Ok(());
            };
            let levels = ["small", "medium", "high", "xhigh"];
            let current = self
                .header
                .thinking
                .as_deref()
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "medium".to_string());
            let current = levels
                .iter()
                .position(|level| *level == current)
                .unwrap_or(1);
            let next = levels[(current + 1) % levels.len()];
            app.thread_configure(super::ThreadConfigureArgs {
                thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                openai_provider: None,
                model: None,
                openai_base_url: None,
                thinking: Some(next.to_string()),
            })
            .await?;
            self.header.thinking = Some(next.to_string());
            self.set_status(format!("thinking={next}"));
            Ok(())
        }

        fn apply_model_list(&mut self, models: Vec<String>) {
            if !self.model_fetch_pending {
                return;
            }
            self.model_fetch_pending = false;
            self.model_list = models;
            self.model_list_loaded = !self.model_list.is_empty();
            if self.model_list.is_empty() {
                self.set_status("thread/models error: empty model list".to_string());
            }
            if let Some(inline) = self.inline_palette.as_mut() {
                if inline.kind == InlinePaletteKind::Model {
                    inline.view = build_inline_model_palette(
                        self.model_list.clone(),
                        self.header.model.as_deref(),
                    );
                }
            }
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                let palette =
                    build_model_palette(self.model_list.clone(), self.header.model.as_deref());
                self.replace_top_command_palette(palette);
            }
        }

        fn is_error_message(msg: &str) -> bool {
            let lower = msg.to_ascii_lowercase();
            lower.contains("error")
                || lower.contains("timeout")
                || lower.contains("failed")
                || lower.contains("denied")
        }

        fn handle_notification(&mut self, note: pm_jsonrpc::Notification) -> anyhow::Result<()> {
            match note.method.as_str() {
                "item/delta" => {
                    let params = note.params.as_object().context("delta params is not object")?;
                    let Some(delta) = params.get("delta").and_then(|v| v.as_str()) else {
                        return Ok(());
                    };
                    if delta.is_empty() {
                        return Ok(());
                    }
                    if params.get("kind").and_then(|v| v.as_str()) != Some("output_text") {
                        return Ok(());
                    }
                    let thread_id = serde_json::from_value::<ThreadId>(
                        params
                            .get("thread_id")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    )
                    .context("parse delta thread_id")?;
                    if self.active_thread != Some(thread_id) {
                        return Ok(());
                    }
                    let turn_id = serde_json::from_value::<TurnId>(
                        params
                            .get("turn_id")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    )
                    .context("parse delta turn_id")?;
                    match &mut self.streaming {
                        Some(streaming) if streaming.turn_id == turn_id => {
                            streaming.text.push_str(delta);
                        }
                        _ => {
                            self.streaming = Some(StreamingState {
                                turn_id,
                                text: delta.to_string(),
                            });
                        }
                    }
                }
                "thread/event"
                | "turn/started"
                | "turn/completed"
                | "item/started"
                | "item/completed" => {
                    let event = serde_json::from_value::<ThreadEvent>(note.params)
                        .context("parse ThreadEvent notification")?;
                    let _ = self.apply_live_event(&event);
                }
                _ => {}
            }
            Ok(())
        }
    }
