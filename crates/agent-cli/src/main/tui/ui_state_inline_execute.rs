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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
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
        role: None,
                        model: Some(model.clone()),
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
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
                PaletteCommand::SetAllowedTools(tool) => {
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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: Some(vec![tool.clone()]),
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
                    })
                    .await?;
                    self.set_status(format!("allowed_tools=[{tool}]"));
                    self.clear_inline_line();
                    self.inline_palette = None;
                    self.header_needs_refresh = true;
                    Ok(false)
                }
                PaletteCommand::ClearAllowedTools => {
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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: true,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: false,
                    })
                    .await?;
                    self.set_status("allowed_tools=<cleared>".to_string());
                    self.clear_inline_line();
                    self.inline_palette = None;
                    self.header_needs_refresh = true;
                    Ok(false)
                }
                PaletteCommand::ClearExecpolicyRules => {
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
        role: None,
                        model: None,
                        clear_model: false,
                        openai_base_url: None,
                        clear_openai_base_url: false,
                        thinking: None,
                        clear_thinking: false,
                        show_thinking: None,
                        clear_show_thinking: false,
                        allowed_tools: None,
                        clear_allowed_tools: false,
                        execpolicy_rules: None,
                        clear_execpolicy_rules: true,
                    })
                    .await?;
                    self.set_status("execpolicy_rules=<cleared>".to_string());
                    self.clear_inline_line();
                    self.inline_palette = None;
                    self.header_needs_refresh = true;
                    Ok(false)
                }
                PaletteCommand::PickMode => {
                    self.input = "/mode ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickModel => {
                    self.input = "/model ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickApprovalPolicy => {
                    self.input = "/approval-policy ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickSandboxPolicy => {
                    self.input = "/sandbox-policy ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickSandboxNetworkAccess => {
                    self.input = "/sandbox-network ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickAllowedTools => {
                    self.input = "/allowed-tools ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::PickExecpolicyRules => {
                    self.input = "/execpolicy-rules ".to_string();
                    self.inline_palette = None;
                    self.update_inline_palette(app).await?;
                    Ok(false)
                }
                PaletteCommand::OpenRoot => {
                    self.input = "/".to_string();
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

        async fn execute_inline_list_command_from_query(
            &mut self,
            app: &mut super::App,
        ) -> anyhow::Result<bool> {
            let Some(command) = parse_inline_list_command(&self.input) else {
                return Ok(false);
            };
            let Some(thread_id) = self.active_thread else {
                self.set_status("no active thread".to_string());
                return Ok(true);
            };
            match command.kind {
                InlineListCommandKind::AllowedTools => match command.setting {
                    InlineListCommandSetting::Missing => {
                        self.set_status(
                            "usage: /allowed-tools <a,b> | /allowed-tools clear".to_string(),
                        );
                        return Ok(true);
                    }
                    InlineListCommandSetting::Clear => {
                        app.thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
        role: None,
                            model: None,
                            clear_model: false,
                            openai_base_url: None,
                            clear_openai_base_url: false,
                            thinking: None,
                            clear_thinking: false,
                            show_thinking: None,
                            clear_show_thinking: false,
                            allowed_tools: None,
                            clear_allowed_tools: true,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
                        })
                        .await?;
                        self.set_status("allowed_tools=<cleared>".to_string());
                    }
                    InlineListCommandSetting::Set(values) => {
                        app.thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
        role: None,
                            model: None,
                            clear_model: false,
                            openai_base_url: None,
                            clear_openai_base_url: false,
                            thinking: None,
                            clear_thinking: false,
                            show_thinking: None,
                            clear_show_thinking: false,
                            allowed_tools: Some(values.clone()),
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: false,
                        })
                        .await?;
                        self.set_status(format!(
                            "allowed_tools={}",
                            serde_json::to_string(&values).unwrap_or_else(|_| "[...]".to_string())
                        ));
                    }
                },
                InlineListCommandKind::ExecpolicyRules => match command.setting {
                    InlineListCommandSetting::Missing => {
                        self.set_status(
                            "usage: /execpolicy-rules <a,b> | /execpolicy-rules clear"
                                .to_string(),
                        );
                        return Ok(true);
                    }
                    InlineListCommandSetting::Clear => {
                        app.thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
        role: None,
                            model: None,
                            clear_model: false,
                            openai_base_url: None,
                            clear_openai_base_url: false,
                            thinking: None,
                            clear_thinking: false,
                            show_thinking: None,
                            clear_show_thinking: false,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: None,
                            clear_execpolicy_rules: true,
                        })
                        .await?;
                        self.set_status("execpolicy_rules=<cleared>".to_string());
                    }
                    InlineListCommandSetting::Set(values) => {
                        app.thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
        role: None,
                            model: None,
                            clear_model: false,
                            openai_base_url: None,
                            clear_openai_base_url: false,
                            thinking: None,
                            clear_thinking: false,
                            show_thinking: None,
                            clear_show_thinking: false,
                            allowed_tools: None,
                            clear_allowed_tools: false,
                            execpolicy_rules: Some(values.clone()),
                            clear_execpolicy_rules: false,
                        })
                        .await?;
                        self.set_status(format!(
                            "execpolicy_rules={}",
                            serde_json::to_string(&values).unwrap_or_else(|_| "[...]".to_string())
                        ));
                    }
                },
            }
            self.clear_inline_line();
            self.inline_palette = None;
            self.header_needs_refresh = true;
            Ok(true)
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
        role: None,
                model: None,
                clear_model: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                thinking: Some(next.to_string()),
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                allowed_tools: None,
                clear_allowed_tools: false,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
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

        fn handle_notification(&mut self, note: omne_jsonrpc::Notification) -> anyhow::Result<()> {
            match note.method.as_str() {
                "item/delta" => {
                    let params = note
                        .params
                        .as_ref()
                        .and_then(Value::as_object)
                        .context("delta params is not object")?;
                    let Some(kind) = params.get("kind").and_then(|v| v.as_str()) else {
                        return Ok(());
                    };
                    let Some(delta) = params.get("delta").and_then(|v| v.as_str()) else {
                        return Ok(());
                    };
                    if delta.is_empty() {
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

                    match kind {
                        "output_text" | "thinking" => {
                            match &mut self.streaming {
                                Some(streaming) if streaming.turn_id == turn_id => {
                                    if kind == "output_text" {
                                        streaming.output_text.push_str(delta);
                                    } else {
                                        streaming.thinking.push_str(delta);
                                    }
                                }
                                _ => {
                                    let mut streaming = StreamingState {
                                        turn_id,
                                        output_text: String::new(),
                                        thinking: String::new(),
                                    };
                                    if kind == "output_text" {
                                        streaming.output_text = delta.to_string();
                                    } else {
                                        streaming.thinking = delta.to_string();
                                    }
                                    self.streaming = Some(streaming);
                                }
                            }
                        }
                        "warning" => {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::System,
                                text: format!("warning: {delta}"),
                            });
                        }
                        _ => {}
                    }
                }
                "thread/event"
                | "turn/started"
                | "turn/completed"
                | "item/started"
                | "item/completed" => {
                    let event = serde_json::from_value::<ThreadEvent>(
                        note.params.context("ThreadEvent params missing")?,
                    )
                        .context("parse ThreadEvent notification")?;
                    if self.active_thread == Some(event.thread_id) && event.seq.0 > self.last_seq {
                        self.last_seq = event.seq.0;
                        self.apply_event(&event);
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }
