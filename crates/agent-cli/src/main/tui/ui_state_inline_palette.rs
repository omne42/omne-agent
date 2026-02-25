    impl UiState {
        fn set_inline_palette(&mut self, kind: InlinePaletteKind, view: CommandPaletteOverlay) {
            self.inline_palette = Some(InlinePalette { kind, view });
        }

        fn update_inline_query(&mut self, query: &str) {
            if let Some(inline) = self.inline_palette.as_mut() {
                if inline.view.query != query {
                    inline.view.query = query.to_string();
                    inline.view.rebuild_filter();
                }
            }
        }

        fn inline_selected_action(&self) -> Option<PaletteCommand> {
            self.inline_palette
                .as_ref()
                .and_then(|inline| inline.view.selected_action())
        }

        fn insert_command_trigger(&mut self) {
            if self.input.is_empty()
                || self
                    .input
                    .chars()
                    .last()
                    .is_some_and(char::is_whitespace)
            {
                self.input.push('/');
            } else {
                self.input.push(' ');
                self.input.push('/');
            }
            self.transcript_follow = true;
        }

        fn move_inline_selection(&mut self, delta: i32) {
            let Some(inline) = self.inline_palette.as_mut() else {
                return;
            };
            if inline.view.filtered.is_empty() {
                inline.view.selected = 0;
                return;
            }
            if delta < 0 {
                inline.view.selected = inline.view.selected.saturating_sub(1);
            } else if delta > 0 {
                inline.view.selected =
                    (inline.view.selected + 1).min(inline.view.filtered.len() - 1);
            }
        }

        fn clear_inline_line(&mut self) {
            let (line_start, _) = last_line_bounds(&self.input);
            self.input.truncate(line_start);
        }

        fn replace_inline_token(&mut self, trigger: char, replacement: &str, trailing_space: bool) {
            if let Some((start, end)) = inline_token_span(&self.input, trigger) {
                let mut value = replacement.to_string();
                if trailing_space {
                    value.push(' ');
                }
                self.input.replace_range(start..end, &value);
            }
        }

        fn cancel_turn_start(&mut self) {
            if let Some(pending) = self.turn_start.take() {
                pending.handle.abort();
            }
        }

        async fn refresh_mode_catalog(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(thread_id) = self.active_thread else {
                return Ok(());
            };
            let config = app.thread_config_explain(thread_id).await?;
            let modes = config
                .mode_catalog
                .modes
                .into_iter()
                .map(|mode| mode.trim().to_string())
                .filter(|mode| !mode.is_empty())
                .collect::<Vec<_>>();
            self.mode_catalog = modes;
            self.mode_catalog_loaded = true;
            Ok(())
        }

        async fn refresh_skill_catalog(&mut self) -> anyhow::Result<()> {
            fn home_dir() -> Option<PathBuf> {
                std::env::var_os("HOME")
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .or_else(|| {
                        std::env::var_os("USERPROFILE")
                            .filter(|s| !s.is_empty())
                            .map(PathBuf::from)
                    })
            }

            let mut roots = Vec::<PathBuf>::new();
            if let Ok(dir) = std::env::var("OMNE_SKILLS_DIR") {
                let dir = dir.trim();
                if !dir.is_empty() {
                    roots.push(PathBuf::from(dir));
                }
            }
            if let Some(thread_cwd) = self.thread_cwd.as_deref() {
                let root = PathBuf::from(thread_cwd);
                roots.push(root.join(".omne_data").join("spec").join("skills"));
                roots.push(root.join(".codex").join("skills"));
            }
            if let Some(home) = home_dir() {
                roots.push(home.join(".omne_data").join("spec").join("skills"));
            }

            let mut names = std::collections::BTreeSet::<String>::new();
            for root in roots {
                let mut dir = match tokio::fs::read_dir(&root).await {
                    Ok(dir) => dir,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(err) => return Err(err).with_context(|| format!("read {}", root.display())),
                };
                while let Some(entry) = dir.next_entry().await? {
                    let file_type = entry.file_type().await?;
                    if !file_type.is_dir() {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    let path = entry.path().join("SKILL.md");
                    if tokio::fs::metadata(&path).await.is_ok() {
                        names.insert(name);
                    }
                }
            }

            self.skill_catalog = names.into_iter().collect();
            self.skill_catalog_loaded = true;
            Ok(())
        }

        async fn update_inline_palette(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(context) = parse_inline_context(&self.input) else {
                self.inline_palette = None;
                return Ok(());
            };

            match context.kind {
                InlinePaletteKind::Command => {
                    if self
                        .inline_palette
                        .as_ref()
                        .is_none_or(|inline| inline.kind != InlinePaletteKind::Command)
                    {
                        let palette =
                            build_inline_command_palette(self.active_thread.is_some());
                        self.set_inline_palette(InlinePaletteKind::Command, palette);
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::Role => {
                    if !self.mode_catalog_loaded {
                        self.refresh_mode_catalog(app).await?;
                    }
                    if self
                        .inline_palette
                        .as_ref()
                        .is_none_or(|inline| inline.kind != InlinePaletteKind::Role)
                    {
                        let palette = build_inline_role_palette(
                            self.mode_catalog.clone(),
                            self.header.mode.as_deref(),
                        );
                        self.set_inline_palette(InlinePaletteKind::Role, palette);
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::Skill => {
                    if !self.skill_catalog_loaded {
                        self.refresh_skill_catalog().await?;
                    }
                    if self
                        .inline_palette
                        .as_ref()
                        .is_none_or(|inline| inline.kind != InlinePaletteKind::Skill)
                    {
                        let palette = build_inline_skill_palette(self.skill_catalog.clone());
                        self.set_inline_palette(InlinePaletteKind::Skill, palette);
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::Model => {
                    if self
                        .inline_palette
                        .as_ref()
                        .is_none_or(|inline| inline.kind != InlinePaletteKind::Model)
                    {
                        let palette = if self.model_list_loaded {
                            build_inline_model_palette(
                                self.model_list.clone(),
                                self.header.model.as_deref(),
                            )
                        } else {
                            build_model_loading_palette()
                        };
                        self.set_inline_palette(InlinePaletteKind::Model, palette);
                    }
                    if !self.model_list_loaded && !self.model_fetch_pending {
                        if let Some(thread_id) = self.active_thread {
                            self.model_fetch_pending = true;
                            self.model_fetch = Some(spawn_model_fetch(
                                app.rpc_handle(),
                                thread_id,
                                Duration::from_secs(5),
                            ));
                        }
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::ApprovalPolicy => {
                    if self.inline_palette.as_ref().is_none_or(|inline| {
                        inline.kind != InlinePaletteKind::ApprovalPolicy
                    }) {
                        self.set_inline_palette(
                            InlinePaletteKind::ApprovalPolicy,
                            build_inline_approval_policy_palette(),
                        );
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::SandboxPolicy => {
                    if self.inline_palette.as_ref().is_none_or(|inline| {
                        inline.kind != InlinePaletteKind::SandboxPolicy
                    }) {
                        self.set_inline_palette(
                            InlinePaletteKind::SandboxPolicy,
                            build_inline_sandbox_policy_palette(),
                        );
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::SandboxNetworkAccess => {
                    if self.inline_palette.as_ref().is_none_or(|inline| {
                        inline.kind != InlinePaletteKind::SandboxNetworkAccess
                    }) {
                        self.set_inline_palette(
                            InlinePaletteKind::SandboxNetworkAccess,
                            build_inline_sandbox_network_access_palette(),
                        );
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::AllowedTools => {
                    if self.inline_palette.as_ref().is_none_or(|inline| {
                        inline.kind != InlinePaletteKind::AllowedTools
                    }) {
                        self.set_inline_palette(
                            InlinePaletteKind::AllowedTools,
                            build_inline_allowed_tools_palette(),
                        );
                    }
                    self.update_inline_query(context.query.trim());
                }
                InlinePaletteKind::ExecpolicyRules => {
                    if self.inline_palette.as_ref().is_none_or(|inline| {
                        inline.kind != InlinePaletteKind::ExecpolicyRules
                    }) {
                        self.set_inline_palette(
                            InlinePaletteKind::ExecpolicyRules,
                            build_inline_execpolicy_rules_palette(),
                        );
                    }
                    self.update_inline_query(context.query.trim());
                }
            }

            Ok(())
        }
    }
