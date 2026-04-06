    #[cfg(test)]
    mod tests {
        use std::str::FromStr;

        use omne_protocol::{ArtifactReadMetadataFallbackReason, ArtifactReadMetadataSource};
        use ratatui::backend::TestBackend;

        use super::*;

        fn render_to_string(
            state: &mut UiState,
            width: u16,
            height: u16,
        ) -> anyhow::Result<String> {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend)?;
            terminal.draw(|f| draw_ui(f, state))?;
            let buffer = terminal.backend().buffer();

            let mut out = String::new();
            for y in 0..height {
                for x in 0..width {
                    out.push_str(buffer[(x, y)].symbol());
                }
                if y + 1 < height {
                    out.push('\n');
                }
            }
            Ok(out)
        }

        #[test]
        fn renders_thread_list_snapshot() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            state.header.model_context_window = Some(100_000);
            state.current_context_tokens_estimate = Some(39_280);
            state.total_tokens_used = 39_280;
            state.threads = vec![
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                    cwd: Some("/repo".to_string()),
                    attention_state: "running".to_string(),
                    has_plan_ready: false,
                    has_diff_ready: false,
                    has_fan_out_linkage_issue: false,
                    has_fan_out_auto_apply_error: false,
                    has_fan_in_dependency_blocked: false,
                    pending_subagent_proxy_approvals: 0,
                    has_test_failed: false,
                    created_at: None,
                    updated_at: None,
                    title: Some("First".to_string()),
                    first_message: Some("hello".to_string()),
                },
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000002")?,
                    cwd: Some("/repo".to_string()),
                    attention_state: "failed".to_string(),
                    has_plan_ready: false,
                    has_diff_ready: false,
                    has_fan_out_linkage_issue: true,
                    has_fan_out_auto_apply_error: false,
                    has_fan_in_dependency_blocked: false,
                    pending_subagent_proxy_approvals: 0,
                    has_test_failed: false,
                    created_at: None,
                    updated_at: None,
                    title: Some("Second".to_string()),
                    first_message: Some("world".to_string()),
                },
            ];
            state.selected_thread = 1;

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"threads [all] archived=off (↑↓ Enter n h l a b s c r q)         
Updated  Attn   Title   CWD    Message                          
  -        run    First   /repo  hello                          
▶ -        link!  Second  /repo  world                          
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
69% context left  threads f=all (Ctrl-K=commands)               "#;
            assert_eq!(actual, expected);
            Ok(())
        }

        #[test]
        fn renders_fan_in_dependency_blocked_badge_in_thread_list() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            state.threads = vec![ThreadMeta {
                thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                cwd: Some("/repo".to_string()),
                attention_state: "running".to_string(),
                has_plan_ready: false,
                has_diff_ready: false,
                has_fan_out_linkage_issue: false,
                has_fan_out_auto_apply_error: false,
                has_fan_in_dependency_blocked: true,
                pending_subagent_proxy_approvals: 0,
                has_test_failed: false,
                created_at: None,
                updated_at: None,
                title: Some("First".to_string()),
                first_message: Some("hello".to_string()),
            }];

            let actual = render_to_string(&mut state, 64, 6)?;
            assert!(actual.contains("fanin!"));
            Ok(())
        }

        #[test]
        fn renders_subagent_pending_badge_in_thread_list() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            state.threads = vec![ThreadMeta {
                thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                cwd: Some("/repo".to_string()),
                attention_state: "need_approval".to_string(),
                has_plan_ready: false,
                has_diff_ready: false,
                has_fan_out_linkage_issue: false,
                has_fan_out_auto_apply_error: false,
                has_fan_in_dependency_blocked: false,
                pending_subagent_proxy_approvals: 2,
                has_test_failed: false,
                created_at: None,
                updated_at: None,
                title: Some("First".to_string()),
                first_message: Some("hello".to_string()),
            }];

            let actual = render_to_string(&mut state, 64, 6)?;
            assert!(actual.contains("sub2"));
            Ok(())
        }

        #[test]
        fn renders_subagent_pending_badge_caps_large_counts() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            state.threads = vec![ThreadMeta {
                thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                cwd: Some("/repo".to_string()),
                attention_state: "need_approval".to_string(),
                has_plan_ready: false,
                has_diff_ready: false,
                has_fan_out_linkage_issue: false,
                has_fan_out_auto_apply_error: false,
                has_fan_in_dependency_blocked: false,
                pending_subagent_proxy_approvals: 1200,
                has_test_failed: false,
                created_at: None,
                updated_at: None,
                title: Some("First".to_string()),
                first_message: Some("hello".to_string()),
            }];

            let actual = render_to_string(&mut state, 64, 6)?;
            assert!(actual.contains("sub999+"));
            Ok(())
        }

        #[test]
        fn renders_thread_view_snapshot() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.header.mode = Some("coder".to_string());
            state.header.model = Some("gpt-4.1".to_string());
            state.last_seq = 12;
            state.push_transcript(TranscriptEntry {
                role: TranscriptRole::System,
                text: "[model] gpt-4.1 (global_default)".to_string(),
            });
            state.push_transcript(TranscriptEntry {
                role: TranscriptRole::User,
                text: "Hello".to_string(),
            });
            state.push_transcript(TranscriptEntry {
                role: TranscriptRole::Assistant,
                text: "Hi!".to_string(),
            });
            state.streaming = Some(StreamingState {
                turn_id,
                output_text: "Streaming...".to_string(),
                thinking: String::new(),
            });
            state.input = "next".to_string();
            state.header.model_context_window = Some(100_000);
            state.current_context_tokens_estimate = Some(39_280);
            state.total_tokens_used = 39_280;

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"                                                                
system: [model] gpt-4.1 (global_default)                        
user: Hello                                                     
assistant: Hi!                                                  
assistant: Streaming...                                         
                                                                
› next                                                          
                                                                
69% context left  th=00000000 m=coder md=gpt-4.1 g=*/0 (Ctrl-K) 
                                                                
                                                                
                                                                "#;
            assert_eq!(actual, expected);
            Ok(())
        }

        #[test]
        fn thread_view_footer_shows_subagent_pending_total_on_narrow_width() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.header.mode = Some("coder".to_string());
            state.header.model = Some("gpt-4.1".to_string());
            state.subagent_pending_summary = Some(SubagentPendingSummary {
                total: 3,
                states: std::collections::BTreeMap::from([
                    ("failed".to_string(), 1usize),
                    ("running".to_string(), 2usize),
                ]),
            });

            let actual = render_to_string(&mut state, 64, 8)?;
            assert!(actual.contains("sub=3"));
            Ok(())
        }

        #[test]
        fn thread_view_footer_shows_subagent_pending_state_breakdown_on_wide_width() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.header.mode = Some("coder".to_string());
            state.header.model = Some("gpt-4.1".to_string());
            state.subagent_pending_summary = Some(SubagentPendingSummary {
                total: 3,
                states: std::collections::BTreeMap::from([
                    ("failed".to_string(), 1usize),
                    ("running".to_string(), 2usize),
                ]),
            });

            let actual = render_to_string(&mut state, 140, 8)?;
            assert!(actual.contains("sub=3("));
            assert!(actual.contains("failed:1"));
            assert!(actual.contains("running:2"));
            Ok(())
        }

        #[test]
        fn parse_inline_context_allows_trailing_space_for_slash_commands() {
            let ctx = parse_inline_context("/model ").expect("context");
            assert!(matches!(ctx.kind, InlinePaletteKind::Model));
            assert_eq!(ctx.query, "");
        }

        #[test]
        fn parse_inline_context_supports_allowed_tools_and_execpolicy_rules() {
            let ctx = parse_inline_context("/allowed-tools file/read").expect("context");
            assert!(matches!(ctx.kind, InlinePaletteKind::AllowedTools));
            assert_eq!(ctx.query, "file/read");

            let ctx = parse_inline_context("/execpolicy-rules rules/a.rules").expect("context");
            assert!(matches!(ctx.kind, InlinePaletteKind::ExecpolicyRules));
            assert_eq!(ctx.query, "rules/a.rules");
        }

        #[test]
        fn parse_inline_list_command_supports_clear_and_list() {
            let cmd = parse_inline_list_command("/allowed-tools file/read, file/glob").expect("cmd");
            assert!(matches!(cmd.kind, InlineListCommandKind::AllowedTools));
            assert_eq!(
                cmd.setting,
                InlineListCommandSetting::Set(vec![
                    "file/read".to_string(),
                    "file/glob".to_string()
                ])
            );

            let cmd = parse_inline_list_command("/execpolicy-rules clear").expect("cmd");
            assert!(matches!(cmd.kind, InlineListCommandKind::ExecpolicyRules));
            assert_eq!(cmd.setting, InlineListCommandSetting::Clear);
        }

        #[test]
        fn root_palette_shows_current_thread_gate_values() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.header.allowed_tools_count = Some(3);
            state.header.execpolicy_rules_count = 2;

            let palette = build_root_palette(&state);
            let labels = palette
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "allowed-tools=3"));
            assert!(labels.iter().any(|label| label == "execpolicy-rules=2"));
            Ok(())
        }

        #[test]
        fn root_palette_shows_linkage_filter_state() {
            let mut state = UiState::new(false);
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "linkage-filter=off"));
            assert!(labels.iter().any(|label| label == "auto-apply-filter=off"));
            assert!(labels.iter().any(|label| label == "fan-in-filter=off"));
            assert!(labels.iter().any(|label| label == "subagent-filter=off"));
            assert!(labels.iter().any(|label| label == "clear-filters"));
            assert!(labels.iter().any(|label| label == "archived=off"));

            state.only_fan_out_linkage_issue = true;
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "linkage-filter=on"));
            assert!(labels.iter().any(|label| label == "auto-apply-filter=off"));
            assert!(labels.iter().any(|label| label == "fan-in-filter=off"));
            assert!(labels.iter().any(|label| label == "subagent-filter=off"));
            assert!(labels.iter().any(|label| label == "clear-filters"));
            assert!(labels.iter().any(|label| label == "archived=off"));

            state.only_fan_out_auto_apply_error = true;
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "linkage-filter=on"));
            assert!(labels.iter().any(|label| label == "auto-apply-filter=on"));
            assert!(labels.iter().any(|label| label == "fan-in-filter=off"));
            assert!(labels.iter().any(|label| label == "subagent-filter=off"));
            assert!(labels.iter().any(|label| label == "clear-filters"));
            assert!(labels.iter().any(|label| label == "archived=off"));

            state.only_fan_in_dependency_blocked = true;
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "linkage-filter=on"));
            assert!(labels.iter().any(|label| label == "auto-apply-filter=on"));
            assert!(labels.iter().any(|label| label == "fan-in-filter=on"));
            assert!(labels.iter().any(|label| label == "subagent-filter=off"));
            assert!(labels.iter().any(|label| label == "clear-filters"));
            assert!(labels.iter().any(|label| label == "archived=off"));

            state.only_subagent_proxy_approval = true;
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "linkage-filter=on"));
            assert!(labels.iter().any(|label| label == "auto-apply-filter=on"));
            assert!(labels.iter().any(|label| label == "fan-in-filter=on"));
            assert!(labels.iter().any(|label| label == "subagent-filter=on"));
            assert!(labels.iter().any(|label| label == "clear-filters"));
            assert!(labels.iter().any(|label| label == "archived=off"));

            state.include_archived = true;
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "archived=on"));
        }

        #[test]
        fn approvals_overlay_palette_shows_filter_and_failed_navigation_actions() {
            let failed = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some("failed".to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };
            let running = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some("running".to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-02T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id: ThreadId::new(),
                approvals: vec![failed.clone()],
                all_approvals: vec![failed, running],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::FailedSubagent,
                subagent_pending_summary: None,
            }));

            let palette = build_overlay_palette(&state).expect("approvals palette");
            assert_eq!(palette.title, "approvals");
            let labels = palette
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "back"));
            assert!(labels.iter().any(|label| label == "refresh"));
            assert!(labels.iter().any(|label| label == "select-prev"));
            assert!(labels.iter().any(|label| label == "select-next"));
            assert!(labels.iter().any(|label| label == "filter=failed (1/2)"));
            assert!(labels.iter().any(|label| label == "next-failed"));
            assert!(labels.iter().any(|label| label == "prev-failed"));
            assert!(labels.iter().any(|label| label == "approve"));
            assert!(labels.iter().any(|label| label == "deny"));
            assert!(labels.iter().any(|label| label == "remember=off"));
            assert!(labels.iter().any(|label| label == "details"));
        }

        #[test]
        fn processes_overlay_palette_shows_process_actions() {
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Processes(ProcessesOverlay {
                thread_id: ThreadId::new(),
                processes: vec![],
                selected: 0,
            }));

            let palette = build_overlay_palette(&state).expect("processes palette");
            assert_eq!(palette.title, "processes");
            let labels = palette
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "back"));
            assert!(labels.iter().any(|label| label == "refresh (0)"));
            assert!(labels.iter().any(|label| label == "select-prev"));
            assert!(labels.iter().any(|label| label == "select-next"));
            assert!(labels.iter().any(|label| label == "inspect"));
            assert!(labels.iter().any(|label| label == "kill"));
            assert!(labels.iter().any(|label| label == "interrupt"));
        }

        #[test]
        fn artifacts_overlay_palette_shows_artifact_actions() {
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                thread_id: ThreadId::new(),
                artifacts: vec![],
                selected: 0,
                versions_for: None,
                versions: vec![],
                selected_version: 0,
                version_cache: HashMap::new(),
                selected_version_cache: HashMap::new(),
            }));

            let palette = build_overlay_palette(&state).expect("artifacts palette");
            assert_eq!(palette.title, "artifacts");
            let labels = palette
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "back"));
            assert!(labels.iter().any(|label| label == "refresh (0)"));
            assert!(labels.iter().any(|label| label == "select-prev"));
            assert!(labels.iter().any(|label| label == "select-next"));
            assert!(labels.iter().any(|label| label == "read"));
            assert!(labels.iter().any(|label| label == "versions"));
            assert!(labels.iter().any(|label| label == "versions-reload"));
            assert!(labels.iter().any(|label| label == "version-prev"));
            assert!(labels.iter().any(|label| label == "version-next"));
            assert!(labels.iter().any(|label| label == "version-latest"));
        }

        #[test]
        fn overlay_palette_command_key_maps_overlay_actions_to_expected_keys() {
            let approvals = [
                (PaletteCommand::ApprovalsCycleFilter, KeyCode::Char('t')),
                (PaletteCommand::ApprovalsNextFailed, KeyCode::Char('f')),
                (PaletteCommand::ApprovalsPrevFailed, KeyCode::Char('F')),
                (PaletteCommand::ApprovalsRefresh, KeyCode::Char('r')),
                (PaletteCommand::ApprovalsSelectPrev, KeyCode::Up),
                (PaletteCommand::ApprovalsSelectNext, KeyCode::Down),
                (PaletteCommand::ApprovalsApprove, KeyCode::Char('y')),
                (PaletteCommand::ApprovalsDeny, KeyCode::Char('n')),
                (PaletteCommand::ApprovalsToggleRemember, KeyCode::Char('m')),
                (PaletteCommand::ApprovalsOpenDetails, KeyCode::Enter),
            ];
            for (command, expected_code) in approvals {
                let key = overlay_palette_command_key(&command).expect("approvals key");
                assert_eq!(key.code, expected_code);
                assert_eq!(key.modifiers, KeyModifiers::NONE);
            }

            let processes = [
                (PaletteCommand::ProcessesRefresh, KeyCode::Char('r')),
                (PaletteCommand::ProcessesSelectPrev, KeyCode::Up),
                (PaletteCommand::ProcessesSelectNext, KeyCode::Down),
                (PaletteCommand::ProcessesInspect, KeyCode::Enter),
                (PaletteCommand::ProcessesKill, KeyCode::Char('k')),
                (PaletteCommand::ProcessesInterrupt, KeyCode::Char('x')),
            ];
            for (command, expected_code) in processes {
                let key = overlay_palette_command_key(&command).expect("processes key");
                assert_eq!(key.code, expected_code);
                assert_eq!(key.modifiers, KeyModifiers::NONE);
            }

            let artifacts = [
                (PaletteCommand::ArtifactsRefresh, KeyCode::Char('r')),
                (PaletteCommand::ArtifactsSelectPrev, KeyCode::Up),
                (PaletteCommand::ArtifactsSelectNext, KeyCode::Down),
                (PaletteCommand::ArtifactsRead, KeyCode::Enter),
                (PaletteCommand::ArtifactsLoadVersions, KeyCode::Char('v')),
                (PaletteCommand::ArtifactsReloadVersions, KeyCode::Char('R')),
                (PaletteCommand::ArtifactsPrevVersion, KeyCode::Char('[')),
                (PaletteCommand::ArtifactsNextVersion, KeyCode::Char(']')),
                (PaletteCommand::ArtifactsLatestVersion, KeyCode::Char('0')),
            ];
            for (command, expected_code) in artifacts {
                let key = overlay_palette_command_key(&command).expect("artifacts key");
                assert_eq!(key.code, expected_code);
                assert_eq!(key.modifiers, KeyModifiers::NONE);
            }
        }

        #[test]
        fn overlay_palette_command_key_ignores_non_overlay_actions() {
            assert!(overlay_palette_command_key(&PaletteCommand::Help).is_none());
            assert!(overlay_palette_command_key(&PaletteCommand::Quit).is_none());
            assert!(overlay_palette_command_key(&PaletteCommand::OpenApprovals).is_none());
        }

        #[test]
        fn command_palette_context_label_defaults_and_normalizes() {
            let empty = CommandPaletteOverlay::new("   ", vec![]);
            assert_eq!(empty.context_label(), "commands");

            let approvals = CommandPaletteOverlay::new("Approvals", vec![]);
            assert_eq!(approvals.context_label(), "approvals");
        }

        #[test]
        fn overlay_palettes_actions_are_close_or_key_mapped() {
            let thread_id = ThreadId::new();
            let approvals_palette = {
                let mut state = UiState::new(false);
                state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                    thread_id,
                    approvals: vec![],
                    all_approvals: vec![],
                    selected: 0,
                    remember: false,
                    filter: ApprovalsFilter::All,
                    subagent_pending_summary: None,
                }));
                build_overlay_palette(&state).expect("approvals palette")
            };
            let processes_palette = {
                let mut state = UiState::new(false);
                state.overlays.push(Overlay::Processes(ProcessesOverlay {
                    thread_id,
                    processes: vec![],
                    selected: 0,
                }));
                build_overlay_palette(&state).expect("processes palette")
            };
            let artifacts_palette = {
                let mut state = UiState::new(false);
                state.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                    thread_id,
                    artifacts: vec![],
                    selected: 0,
                    versions_for: None,
                    versions: vec![],
                    selected_version: 0,
                    version_cache: HashMap::new(),
                    selected_version_cache: HashMap::new(),
                }));
                build_overlay_palette(&state).expect("artifacts palette")
            };

            for palette in [approvals_palette, processes_palette, artifacts_palette] {
                for item in palette.items {
                    match item.action {
                        PaletteCommand::ClosePalette => {}
                        action => assert!(
                            overlay_palette_command_key(&action).is_some(),
                            "palette '{}' has unmapped action '{}'",
                            palette.title,
                            item.label
                        ),
                    }
                }
            }
        }

        #[test]
        fn overlay_palette_mapped_keys_drive_local_overlay_state_changes() -> anyhow::Result<()> {
            let mk_approval = |action: &str| ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: action.to_string(),
                    action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                    params: serde_json::json!({}),
                    summary: None,
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };
            let mut approvals = ApprovalsOverlay {
                thread_id: ThreadId::new(),
                approvals: vec![mk_approval("process/start"), mk_approval("process/kill")],
                all_approvals: vec![mk_approval("process/start"), mk_approval("process/kill")],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: None,
            };

            let select_next_key =
                overlay_palette_command_key(&PaletteCommand::ApprovalsSelectNext).expect("key");
            match handle_local_approvals_key(&mut approvals, select_next_key.code) {
                ApprovalsLocalKeyResult::Handled(_) => {}
                ApprovalsLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(approvals.selected, 1);

            let remember_key = overlay_palette_command_key(
                &PaletteCommand::ApprovalsToggleRemember,
            )
            .expect("key");
            match handle_local_approvals_key(&mut approvals, remember_key.code) {
                ApprovalsLocalKeyResult::Handled(_) => {}
                ApprovalsLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert!(approvals.remember);

            let mut processes = ProcessesOverlay {
                thread_id: ThreadId::new(),
                processes: vec![
                    ProcessInfo {
                        process_id: ProcessId::new(),
                        thread_id: ThreadId::new(),
                        turn_id: None,
                        argv: vec!["sleep".to_string(), "1".to_string()],
                        cwd: "/tmp".to_string(),
                        started_at: "2026-01-01T00:00:00Z".to_string(),
                        status: ProcessStatus::Running,
                        exit_code: None,
                        stdout_path: "/tmp/stdout.log".to_string(),
                        stderr_path: "/tmp/stderr.log".to_string(),
                        last_update_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    ProcessInfo {
                        process_id: ProcessId::new(),
                        thread_id: ThreadId::new(),
                        turn_id: None,
                        argv: vec!["echo".to_string(), "ok".to_string()],
                        cwd: "/tmp".to_string(),
                        started_at: "2026-01-01T00:00:01Z".to_string(),
                        status: ProcessStatus::Running,
                        exit_code: None,
                        stdout_path: "/tmp/stdout2.log".to_string(),
                        stderr_path: "/tmp/stderr2.log".to_string(),
                        last_update_at: "2026-01-01T00:00:01Z".to_string(),
                    },
                ],
                selected: 0,
            };
            let process_next_key =
                overlay_palette_command_key(&PaletteCommand::ProcessesSelectNext).expect("key");
            match handle_local_processes_key(&mut processes, process_next_key.code) {
                ProcessesLocalKeyResult::Handled => {}
                ProcessesLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(processes.selected, 1);

            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let mut artifacts = ArtifactsOverlay {
                thread_id: ThreadId::new(),
                artifacts: vec![ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "summary".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 4,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 100,
                    provenance: None,
                }],
                selected: 0,
                versions_for: Some(artifact_id),
                versions: vec![4, 3, 2],
                selected_version: 1,
                version_cache: HashMap::from([(artifact_id, vec![4, 3, 2])]),
                selected_version_cache: HashMap::from([(artifact_id, 1)]),
            };

            let next_version_key =
                overlay_palette_command_key(&PaletteCommand::ArtifactsNextVersion).expect("key");
            match handle_local_artifacts_key(&mut artifacts, next_version_key.code) {
                ArtifactsLocalKeyResult::Handled(_) => {}
                ArtifactsLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(artifacts.selected_version, 0);

            let prev_version_key =
                overlay_palette_command_key(&PaletteCommand::ArtifactsPrevVersion).expect("key");
            match handle_local_artifacts_key(&mut artifacts, prev_version_key.code) {
                ArtifactsLocalKeyResult::Handled(_) => {}
                ArtifactsLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(artifacts.selected_version, 1);

            Ok(())
        }

        #[test]
        fn toggle_overlay_command_palette_opens_with_context_status_and_closes() {
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id: ThreadId::new(),
                approvals: vec![],
                all_approvals: vec![],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: None,
            }));

            assert!(state.toggle_overlay_command_palette());
            let opened = state.overlays.last().expect("palette opened");
            match opened {
                Overlay::CommandPalette(view) => assert_eq!(view.title, "approvals"),
                _ => panic!("expected command palette overlay"),
            }
            assert_eq!(
                state.status.as_deref(),
                Some("overlay commands: approvals")
            );
            assert!(state.status_expires_at.is_some());

            assert!(state.toggle_overlay_command_palette());
            let closed = state.overlays.last().expect("approvals remains");
            match closed {
                Overlay::Approvals(_) => {}
                _ => panic!("expected approvals overlay after close"),
            }
        }

        #[test]
        fn toggle_overlay_command_palette_ignores_non_overlay_types() {
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Text(TextOverlay {
                title: "Help".to_string(),
                text: "body".to_string(),
                scroll: 0,
            }));
            assert!(!state.toggle_overlay_command_palette());
            let top = state.overlays.last().expect("text overlay");
            match top {
                Overlay::Text(view) => assert_eq!(view.title, "Help"),
                _ => panic!("expected text overlay"),
            }
            assert_eq!(
                state.status.as_deref(),
                Some("overlay commands unavailable")
            );
            assert!(state.status_expires_at.is_some());
        }

        #[test]
        fn temporary_status_expires_after_ttl() {
            let mut state = UiState::new(false);
            state.set_temporary_status("overlay commands unavailable".to_string(), Duration::ZERO);
            assert_eq!(
                state.status.as_deref(),
                Some("overlay commands unavailable")
            );

            assert!(state.expire_status_if_needed(Instant::now()));
            assert!(state.status.is_none());
        }

        #[test]
        fn regular_status_does_not_expire_without_ttl() {
            let mut state = UiState::new(false);
            state.set_status("refreshed".to_string());
            assert_eq!(state.status.as_deref(), Some("refreshed"));

            assert!(!state.expire_status_if_needed(
                Instant::now() + Duration::from_secs(10)
            ));
            assert_eq!(state.status.as_deref(), Some("refreshed"));
        }

        fn select_overlay_palette_command_by_query(
            view: &mut CommandPaletteOverlay,
            query: &str,
        ) -> PaletteCommand {
            for ch in query.chars() {
                assert!(handle_key_command_palette(
                    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
                    view
                )
                .is_none());
            }
            handle_key_command_palette(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                view,
            )
            .expect("selected action")
        }

        #[test]
        fn overlay_palette_key_flow_applies_selected_command_to_underlying_approvals() {
            let make_approval = |action: &str| ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: action.to_string(),
                    action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                    params: serde_json::json!({}),
                    summary: None,
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id: ThreadId::new(),
                approvals: vec![make_approval("process/start"), make_approval("process/kill")],
                all_approvals: vec![make_approval("process/start"), make_approval("process/kill")],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: None,
            }));

            assert!(state.toggle_overlay_command_palette());

            let command = {
                let view = match state.overlays.last_mut() {
                    Some(Overlay::CommandPalette(view)) => view,
                    _ => panic!("expected command palette overlay"),
                };
                select_overlay_palette_command_by_query(view, "select-next")
            };
            assert!(matches!(command, PaletteCommand::ApprovalsSelectNext));

            let mapped_key = overlay_palette_command_key(&command).expect("mapped key");
            assert!(state.apply_overlay_palette_local_key(mapped_key));

            let overlay = state.overlays.last().expect("approvals overlay");
            match overlay {
                Overlay::Approvals(view) => {
                    assert_eq!(view.selected, 1);
                    assert!(!view.remember);
                }
                _ => panic!("expected approvals overlay"),
            }
        }

        #[test]
        fn overlay_palette_key_flow_applies_selected_command_to_underlying_processes() {
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Processes(ProcessesOverlay {
                thread_id: ThreadId::new(),
                processes: vec![
                    ProcessInfo {
                        process_id: ProcessId::new(),
                        thread_id: ThreadId::new(),
                        turn_id: None,
                        argv: vec!["sleep".to_string(), "1".to_string()],
                        cwd: "/tmp".to_string(),
                        started_at: "2026-01-01T00:00:00Z".to_string(),
                        status: ProcessStatus::Running,
                        exit_code: None,
                        stdout_path: "/tmp/stdout.log".to_string(),
                        stderr_path: "/tmp/stderr.log".to_string(),
                        last_update_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    ProcessInfo {
                        process_id: ProcessId::new(),
                        thread_id: ThreadId::new(),
                        turn_id: None,
                        argv: vec!["echo".to_string(), "ok".to_string()],
                        cwd: "/tmp".to_string(),
                        started_at: "2026-01-01T00:00:01Z".to_string(),
                        status: ProcessStatus::Running,
                        exit_code: None,
                        stdout_path: "/tmp/stdout2.log".to_string(),
                        stderr_path: "/tmp/stderr2.log".to_string(),
                        last_update_at: "2026-01-01T00:00:01Z".to_string(),
                    },
                ],
                selected: 0,
            }));

            assert!(state.toggle_overlay_command_palette());
            let command = {
                let view = match state.overlays.last_mut() {
                    Some(Overlay::CommandPalette(view)) => view,
                    _ => panic!("expected command palette overlay"),
                };
                select_overlay_palette_command_by_query(view, "select-next")
            };
            assert!(matches!(command, PaletteCommand::ProcessesSelectNext));

            let mapped_key = overlay_palette_command_key(&command).expect("mapped key");
            assert!(state.apply_overlay_palette_local_key(mapped_key));

            let overlay = state.overlays.last().expect("processes overlay");
            match overlay {
                Overlay::Processes(view) => assert_eq!(view.selected, 1),
                _ => panic!("expected processes overlay"),
            }
        }

        #[test]
        fn overlay_palette_key_flow_applies_selected_command_to_underlying_artifacts() -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                thread_id: ThreadId::new(),
                artifacts: vec![ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "summary".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 4,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 100,
                    provenance: None,
                }],
                selected: 0,
                versions_for: Some(artifact_id),
                versions: vec![4, 3, 2],
                selected_version: 1,
                version_cache: HashMap::from([(artifact_id, vec![4, 3, 2])]),
                selected_version_cache: HashMap::from([(artifact_id, 1)]),
            }));

            assert!(state.toggle_overlay_command_palette());
            let command = {
                let view = match state.overlays.last_mut() {
                    Some(Overlay::CommandPalette(view)) => view,
                    _ => panic!("expected command palette overlay"),
                };
                select_overlay_palette_command_by_query(view, "version-next")
            };
            assert!(matches!(command, PaletteCommand::ArtifactsNextVersion));

            let mapped_key = overlay_palette_command_key(&command).expect("mapped key");
            assert!(state.apply_overlay_palette_local_key(mapped_key));

            let overlay = state.overlays.last().expect("artifacts overlay");
            match overlay {
                Overlay::Artifacts(view) => {
                    assert_eq!(view.selected_version, 0);
                    assert_eq!(view.selected_version_cache.get(&artifact_id).copied(), Some(0));
                }
                _ => panic!("expected artifacts overlay"),
            }
            Ok(())
        }

        #[test]
        fn thread_view_renders_overlay_local_command_palette() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let approval = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "process/start".to_string(),
                    action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                    params: serde_json::json!({}),
                    summary: None,
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id,
                approvals: vec![approval.clone()],
                all_approvals: vec![approval],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: None,
            }));

            let palette = build_overlay_palette(&state).expect("approvals palette");
            state.overlays.push(Overlay::CommandPalette(palette));

            let actual = render_to_string(&mut state, 120, 18)?;
            assert!(actual.contains("approvals: (type to search)"));
            assert!(actual.contains("filter=all"));
            Ok(())
        }

        #[test]
        fn clear_thread_picker_filters_resets_all_attention_flags() {
            let mut state = UiState::new(false);
            assert!(!state.clear_thread_picker_filters());

            state.only_fan_out_linkage_issue = true;
            state.only_fan_out_auto_apply_error = true;
            state.only_fan_in_dependency_blocked = true;
            state.only_subagent_proxy_approval = true;
            assert!(state.clear_thread_picker_filters());
            assert!(!state.only_fan_out_linkage_issue);
            assert!(!state.only_fan_out_auto_apply_error);
            assert!(!state.only_fan_in_dependency_blocked);
            assert!(!state.only_subagent_proxy_approval);
        }

        #[test]
        fn apply_thread_picker_filters_can_focus_linkage_issue_threads() -> anyhow::Result<()> {
            let t1 = ThreadMeta {
                thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                cwd: Some("/repo".to_string()),
                attention_state: "running".to_string(),
                has_plan_ready: false,
                has_diff_ready: false,
                has_fan_out_linkage_issue: false,
                has_fan_out_auto_apply_error: false,
                has_fan_in_dependency_blocked: false,
                pending_subagent_proxy_approvals: 0,
                has_test_failed: false,
                created_at: None,
                updated_at: None,
                title: Some("First".to_string()),
                first_message: Some("hello".to_string()),
            };
            let t2 = ThreadMeta {
                thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000002")?,
                cwd: Some("/repo".to_string()),
                attention_state: "failed".to_string(),
                has_plan_ready: false,
                has_diff_ready: false,
                has_fan_out_linkage_issue: true,
                has_fan_out_auto_apply_error: true,
                has_fan_in_dependency_blocked: true,
                pending_subagent_proxy_approvals: 2,
                has_test_failed: false,
                created_at: None,
                updated_at: None,
                title: Some("Second".to_string()),
                first_message: Some("world".to_string()),
            };
            let threads = vec![t1.clone(), t2.clone()];

            let mut state = UiState::new(false);
            let all = state.apply_thread_picker_filters(threads.clone());
            assert_eq!(all.len(), 2);

            state.only_fan_out_linkage_issue = true;
            let linkage_only = state.apply_thread_picker_filters(threads.clone());
            assert_eq!(linkage_only.len(), 1);
            assert_eq!(linkage_only[0].thread_id, t2.thread_id);

            state.only_fan_out_linkage_issue = false;
            state.only_fan_out_auto_apply_error = true;
            let auto_only = state.apply_thread_picker_filters(threads.clone());
            assert_eq!(auto_only.len(), 1);
            assert_eq!(auto_only[0].thread_id, t2.thread_id);

            state.only_fan_out_linkage_issue = true;
            state.only_fan_out_auto_apply_error = true;
            let both = state.apply_thread_picker_filters(threads);
            assert_eq!(both.len(), 1);
            assert_eq!(both[0].thread_id, t2.thread_id);

            state.only_fan_out_linkage_issue = false;
            state.only_fan_out_auto_apply_error = false;
            state.only_fan_in_dependency_blocked = true;
            let fanin_only = state.apply_thread_picker_filters(vec![t1.clone(), t2.clone()]);
            assert_eq!(fanin_only.len(), 1);
            assert_eq!(
                fanin_only[0].thread_id,
                ThreadId::from_str("00000000-0000-0000-0000-000000000002")?
            );

            state.only_fan_in_dependency_blocked = false;
            state.only_subagent_proxy_approval = true;
            let subagent_only = state.apply_thread_picker_filters(vec![t1, t2]);
            assert_eq!(subagent_only.len(), 1);
            assert_eq!(
                subagent_only[0].thread_id,
                ThreadId::from_str("00000000-0000-0000-0000-000000000002")?
            );
            Ok(())
        }

        #[test]
        fn thread_list_header_shows_linkage_filter_state() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            let off = render_to_string(&mut state, 64, 4)?;
            assert!(off.contains("threads [all]"));

            state.only_fan_out_linkage_issue = true;
            let on = render_to_string(&mut state, 64, 4)?;
            assert!(on.contains("threads [link]"));

            state.only_fan_out_linkage_issue = false;
            state.only_fan_out_auto_apply_error = true;
            let auto = render_to_string(&mut state, 64, 4)?;
            assert!(auto.contains("threads [auto]"));

            state.only_fan_out_linkage_issue = true;
            let both = render_to_string(&mut state, 64, 4)?;
            assert!(both.contains("threads [link+auto]"));

            state.only_fan_out_linkage_issue = false;
            state.only_fan_out_auto_apply_error = false;
            state.only_fan_in_dependency_blocked = true;
            let fanin = render_to_string(&mut state, 64, 4)?;
            assert!(fanin.contains("threads [fanin]"));

            state.only_fan_out_linkage_issue = true;
            let link_fanin = render_to_string(&mut state, 64, 4)?;
            assert!(link_fanin.contains("threads [link+fanin]"));

            state.only_fan_out_linkage_issue = false;
            state.only_fan_in_dependency_blocked = false;
            state.only_subagent_proxy_approval = true;
            let subagent = render_to_string(&mut state, 64, 4)?;
            assert!(subagent.contains("threads [subagent]"));
            Ok(())
        }

        #[test]
        fn thread_picker_footer_shows_linkage_filter_state() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            let off = render_to_string(&mut state, 64, 12)?;
            assert!(off.contains("threads f=all (Ctrl-K=commands)"));

            state.only_fan_out_linkage_issue = true;
            let on = render_to_string(&mut state, 64, 12)?;
            assert!(on.contains("threads f=link (Ctrl-K=commands)"));

            state.only_fan_out_linkage_issue = false;
            state.only_fan_out_auto_apply_error = true;
            let auto = render_to_string(&mut state, 64, 12)?;
            assert!(auto.contains("threads f=auto (Ctrl-K=commands)"));

            state.only_fan_out_linkage_issue = true;
            let both = render_to_string(&mut state, 64, 12)?;
            assert!(both.contains("threads f=link+auto (Ctrl-K=commands)"));

            state.only_fan_out_linkage_issue = false;
            state.only_fan_out_auto_apply_error = false;
            state.only_fan_in_dependency_blocked = true;
            let fanin = render_to_string(&mut state, 64, 12)?;
            assert!(fanin.contains("threads f=fanin (Ctrl-K=commands)"));

            state.only_fan_out_linkage_issue = true;
            let link_fanin = render_to_string(&mut state, 64, 12)?;
            assert!(link_fanin.contains("threads f=link+fanin (Ctrl-K=commands)"));

            state.only_fan_out_linkage_issue = false;
            state.only_fan_in_dependency_blocked = false;
            state.only_subagent_proxy_approval = true;
            let subagent = render_to_string(&mut state, 64, 12)?;
            assert!(subagent.contains("threads f=subagent (Ctrl-K=commands)"));
            Ok(())
        }

        #[test]
        fn artifacts_overlay_shows_version_selection_details() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                thread_id,
                artifacts: vec![ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "weekly".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 4,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 128,
                    provenance: None,
                }],
                selected: 0,
                versions_for: Some(artifact_id),
                versions: vec![4, 3, 2],
                selected_version: 1,
                version_cache: HashMap::from([(artifact_id, vec![4, 3, 2])]),
                selected_version_cache: HashMap::from([(artifact_id, 1)]),
            }));

            let actual = render_to_string(&mut state, 120, 20)?;
            assert!(actual.contains("v=versions"));
            assert!(actual.contains("R=reload"));
            assert!(actual.contains("0=latest"));
            assert!(actual.contains("selected_version: 3"));
            assert!(actual.contains("available_versions: 4, 3, 2"));
            assert!(actual.contains("selected_state: historical"));
            Ok(())
        }

        #[test]
        fn artifact_versions_cache_restores_when_reselecting_artifact() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let artifact_a = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let artifact_b = ArtifactId::from_str("22222222-0000-0000-0000-000000000002")?;
            let mut view = ArtifactsOverlay {
                thread_id,
                artifacts: vec![
                    ArtifactMetadata {
                        artifact_id: artifact_a,
                        artifact_type: "report".to_string(),
                        summary: "a".to_string(),
                        preview: None,
                        created_at: OffsetDateTime::UNIX_EPOCH,
                        updated_at: OffsetDateTime::UNIX_EPOCH,
                        version: 4,
                        content_path: "/tmp/a.md".to_string(),
                        size_bytes: 100,
                        provenance: None,
                    },
                    ArtifactMetadata {
                        artifact_id: artifact_b,
                        artifact_type: "report".to_string(),
                        summary: "b".to_string(),
                        preview: None,
                        created_at: OffsetDateTime::UNIX_EPOCH,
                        updated_at: OffsetDateTime::UNIX_EPOCH,
                        version: 1,
                        content_path: "/tmp/b.md".to_string(),
                        size_bytes: 20,
                        provenance: None,
                    },
                ],
                selected: 0,
                versions_for: None,
                versions: Vec::new(),
                selected_version: 0,
                version_cache: HashMap::new(),
                selected_version_cache: HashMap::new(),
            };

            apply_versions_to_artifacts_overlay(
                &mut view,
                artifact_a,
                &ArtifactVersionsResponse {
                    tool_id: ToolId::new(),
                    artifact_id: artifact_a,
                    latest_version: 4,
                    versions: vec![4, 3, 2],
                    history_versions: vec![3, 2],
                },
            );
            view.selected_version = 2;
            view.selected_version_cache.insert(artifact_a, 2);

            view.selected = 1;
            sync_versions_for_selected_artifact(&mut view);
            assert_eq!(view.versions_for, None);

            view.selected = 0;
            sync_versions_for_selected_artifact(&mut view);
            assert_eq!(view.versions_for, Some(artifact_a));
            assert_eq!(view.selected_version, 2);
            assert_eq!(view.versions, vec![4, 3, 2]);
            Ok(())
        }

        #[test]
        fn handle_local_artifacts_key_supports_version_navigation_and_latest_reset() -> anyhow::Result<()>
        {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let mut view = ArtifactsOverlay {
                thread_id,
                artifacts: vec![ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "a".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 4,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 100,
                    provenance: None,
                }],
                selected: 0,
                versions_for: Some(artifact_id),
                versions: vec![4, 3, 2],
                selected_version: 0,
                version_cache: HashMap::from([(artifact_id, vec![4, 3, 2])]),
                selected_version_cache: HashMap::from([(artifact_id, 0)]),
            };

            match handle_local_artifacts_key(&mut view, KeyCode::Char('[')) {
                ArtifactsLocalKeyResult::Handled(_) => {}
                ArtifactsLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(view.selected_version, 1);
            assert_eq!(view.selected_version_cache.get(&artifact_id), Some(&1));

            match handle_local_artifacts_key(&mut view, KeyCode::Char(']')) {
                ArtifactsLocalKeyResult::Handled(_) => {}
                ArtifactsLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(view.selected_version, 0);
            assert_eq!(view.selected_version_cache.get(&artifact_id), Some(&0));

            view.selected_version = 2;
            view.selected_version_cache.insert(artifact_id, 2);
            let status = match handle_local_artifacts_key(&mut view, KeyCode::Char('0')) {
                ArtifactsLocalKeyResult::Handled(status) => status,
                ArtifactsLocalKeyResult::Unhandled => panic!("expected handled key"),
            };
            assert_eq!(view.selected_version, 0);
            assert_eq!(view.selected_version_cache.get(&artifact_id), Some(&0));
            assert!(
                status
                    .as_deref()
                    .is_some_and(|msg| msg.contains("artifact version reset to latest: 4"))
            );
            Ok(())
        }

        #[test]
        fn stale_artifact_versions_cache_is_invalidated() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let mut view = ArtifactsOverlay {
                thread_id,
                artifacts: vec![ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "a".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 5,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 100,
                    provenance: None,
                }],
                selected: 0,
                versions_for: None,
                versions: Vec::new(),
                selected_version: 0,
                version_cache: HashMap::from([(artifact_id, vec![4, 3, 2])]),
                selected_version_cache: HashMap::from([(artifact_id, 1)]),
            };

            let activated = activate_cached_versions_for_artifact(&mut view, artifact_id, 5);
            assert!(!activated);
            assert!(!view.version_cache.contains_key(&artifact_id));
            assert!(!view.selected_version_cache.contains_key(&artifact_id));
            Ok(())
        }

        #[test]
        fn count_missing_versions_detects_possible_pruned_history() {
            assert_eq!(count_missing_versions(&[4, 3, 2]), 0);
            assert_eq!(count_missing_versions(&[10, 9, 7, 6]), 1);
            assert_eq!(count_missing_versions(&[]), 0);
        }

        #[test]
        fn artifact_read_error_hint_guides_latest_fallback() {
            let retained = anyhow::anyhow!(
                "rpc error: artifact version not retained: requested=2, latest=4"
            );
            let hint = artifact_read_error_hint(&retained).expect("hint");
            assert!(hint.contains("press 0"));

            let missing = anyhow::anyhow!(
                "rpc error: artifact version not found: requested=9, latest=4"
            );
            let hint = artifact_read_error_hint(&missing).expect("hint");
            assert!(hint.contains("selected version"));
        }

        #[test]
        fn artifact_read_error_hint_ignores_other_errors() {
            let other = anyhow::anyhow!("rpc error: permission denied");
            assert!(artifact_read_error_hint(&other).is_none());
        }

        #[test]
        fn artifact_read_text_includes_version_metadata() -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let resp = ArtifactReadResponse {
                tool_id: ToolId::new(),
                metadata: ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "weekly".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 4,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 128,
                    provenance: None,
                },
                text: "hello".to_string(),
                truncated: false,
                bytes: 5,
                version: 2,
                latest_version: 4,
                historical: true,
                metadata_source: ArtifactReadMetadataSource::LatestFallback,
                metadata_fallback_reason: Some(
                    ArtifactReadMetadataFallbackReason::HistoryMetadataMissing,
                ),
                prune_report: None,
                fan_in_summary: None,
                fan_out_linkage_issue: None,
                fan_out_linkage_issue_clear: None,
                fan_out_result: None,
            };
            let text = build_artifact_read_text(&resp);
            assert!(text.contains("version: 2"));
            assert!(text.contains("latest_version: 4"));
            assert!(text.contains("historical: true"));
            assert!(text.contains("metadata_source: latest_fallback"));
            assert!(text.contains(
                "metadata_fallback_reason: history_metadata_missing"
            ));
            Ok(())
        }

        #[test]
        fn artifact_read_text_skips_fallback_reason_when_absent() -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let resp = ArtifactReadResponse {
                tool_id: ToolId::new(),
                metadata: ArtifactMetadata {
                    artifact_id,
                    artifact_type: "report".to_string(),
                    summary: "weekly".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 4,
                    content_path: "/tmp/a.md".to_string(),
                    size_bytes: 128,
                    provenance: None,
                },
                text: "hello".to_string(),
                truncated: false,
                bytes: 5,
                version: 4,
                latest_version: 4,
                historical: false,
                metadata_source: ArtifactReadMetadataSource::Latest,
                metadata_fallback_reason: None,
                prune_report: None,
                fan_in_summary: None,
                fan_out_linkage_issue: None,
                fan_out_linkage_issue_clear: None,
                fan_out_result: None,
            };
            let text = build_artifact_read_text(&resp);
            assert!(text.contains("metadata_source: latest"));
            assert!(!text.contains("metadata_fallback_reason:"));
            Ok(())
        }

        #[test]
        fn artifact_read_text_includes_fan_in_summary_structured_section() -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let resp = ArtifactReadResponse {
                tool_id: ToolId::new(),
                metadata: ArtifactMetadata {
                    artifact_id,
                    artifact_type: "fan_in_summary".to_string(),
                    summary: "workflow summary".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 3,
                    content_path: "/tmp/fan-in.md".to_string(),
                    size_bytes: 256,
                    provenance: None,
                },
                text: "raw markdown".to_string(),
                truncated: false,
                bytes: 12,
                version: 3,
                latest_version: 3,
                historical: false,
                metadata_source: ArtifactReadMetadataSource::Latest,
                metadata_fallback_reason: None,
                prune_report: None,
                fan_in_summary: Some(omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
                    schema_version: "fan_in_summary.v1".to_string(),
                    thread_id: "thread-1".to_string(),
                    task_count: 2,
                    scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                        env_max_concurrent_subagents: 4,
                        effective_concurrency_limit: 2,
                        priority_aging_rounds: 3,
                    },
                    tasks: vec![
                        omne_app_server_protocol::ArtifactFanInSummaryTask {
                            task_id: "task_a".to_string(),
                            title: "do work".to_string(),
                            thread_id: None,
                            turn_id: None,
                            status: "NeedUserInput".to_string(),
                            reason: Some("awaiting approval".to_string()),
                            dependency_blocked: false,
                            dependency_blocker_task_id: None,
                            dependency_blocker_status: None,
                            result_artifact_id: None,
                            result_artifact_error: None,
                            result_artifact_structured_error: None,
                            result_artifact_error_id: None,
                            result_artifact_diagnostics: None,
                            pending_approval: Some(
                                omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                                    approval_id: "approval-1".to_string(),
                                    action: "artifact/read".to_string(),
                                    summary: None,
                                    approve_cmd: Some(
                                        "omne approval decide thread-1 approval-1 --approve"
                                            .to_string(),
                                    ),
                                    deny_cmd: Some(
                                        "omne approval decide thread-1 approval-1 --deny"
                                            .to_string(),
                                    ),
                                },
                            ),
                        },
                        omne_app_server_protocol::ArtifactFanInSummaryTask {
                            task_id: "task_b".to_string(),
                            title: "follow-up".to_string(),
                            thread_id: None,
                            turn_id: None,
                            status: "Failed".to_string(),
                            reason: Some(
                                "blocked by dependency: task_a status=NeedUserInput".to_string(),
                            ),
                            dependency_blocked: true,
                            dependency_blocker_task_id: Some("task_a".to_string()),
                            dependency_blocker_status: Some("NeedUserInput".to_string()),
                            result_artifact_id: None,
                            result_artifact_error: None,
                            result_artifact_structured_error: None,
                            result_artifact_error_id: None,
                            result_artifact_diagnostics: None,
                            pending_approval: None,
                        },
                    ],
                }),
                fan_out_linkage_issue: None,
                fan_out_linkage_issue_clear: None,
                fan_out_result: None,
            };
            let text = build_artifact_read_text(&resp);
            assert!(text.contains("# Fan-in Summary (structured)"));
            assert!(text.contains("schema_version: fan_in_summary.v1"));
            assert!(text.contains("dependency_blocked: 1"));
            assert!(text.contains("pending_approval: action=artifact/read approval_id=approval-1"));
            assert!(text.contains("approve_cmd: omne approval decide thread-1 approval-1 --approve"));
            assert!(text.contains("deny_cmd: omne approval decide thread-1 approval-1 --deny"));
            assert!(text.contains("dependency_blocker_task_id: task_a"));
            assert!(text.contains("dependency_blocker_status: NeedUserInput"));
            Ok(())
        }

        #[test]
        fn artifact_read_text_includes_fan_out_linkage_issue_structured_section() -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let resp = ArtifactReadResponse {
                tool_id: ToolId::new(),
                metadata: ArtifactMetadata {
                    artifact_id,
                    artifact_type: "fan_out_linkage_issue".to_string(),
                    summary: "linkage".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 1,
                    content_path: "/tmp/linkage.md".to_string(),
                    size_bytes: 128,
                    provenance: None,
                },
                text: "raw markdown".to_string(),
                truncated: false,
                bytes: 12,
                version: 1,
                latest_version: 1,
                historical: false,
                metadata_source: ArtifactReadMetadataSource::Latest,
                metadata_fallback_reason: None,
                prune_report: None,
                fan_in_summary: None,
                fan_out_linkage_issue: Some(
                    omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
                        schema_version: "fan_out_linkage_issue.v1".to_string(),
                        fan_in_summary_artifact_id: "artifact-1".to_string(),
                        issue: "fan-out linkage issue: blocked".to_string(),
                        issue_truncated: true,
                    },
                ),
                fan_out_linkage_issue_clear: None,
                fan_out_result: None,
            };
            let text = build_artifact_read_text(&resp);
            assert!(text.contains("# Fan-out Linkage Issue (structured)"));
            assert!(text.contains("schema_version: fan_out_linkage_issue.v1"));
            assert!(text.contains("fan_in_summary_artifact_id: artifact-1"));
            assert!(text.contains("issue_truncated: true"));
            assert!(text.contains("summary: fan-out linkage issue: blocked fan_in_summary_artifact_id=artifact-1 issue_truncated=true"));
            assert!(text.contains("fan_out_linkage_issue artifact_id=11111111-0000-0000-0000-000000000001"));
            assert!(text.contains("- fan-out linkage issue: blocked"));
            Ok(())
        }

        #[test]
        fn artifact_read_text_includes_fan_out_linkage_issue_clear_structured_section()
        -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let resp = ArtifactReadResponse {
                tool_id: ToolId::new(),
                metadata: ArtifactMetadata {
                    artifact_id,
                    artifact_type: "fan_out_linkage_issue_clear".to_string(),
                    summary: "linkage clear".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 1,
                    content_path: "/tmp/linkage-clear.md".to_string(),
                    size_bytes: 128,
                    provenance: None,
                },
                text: "raw markdown".to_string(),
                truncated: false,
                bytes: 12,
                version: 1,
                latest_version: 1,
                historical: false,
                metadata_source: ArtifactReadMetadataSource::Latest,
                metadata_fallback_reason: None,
                prune_report: None,
                fan_in_summary: None,
                fan_out_linkage_issue: None,
                fan_out_linkage_issue_clear: Some(
                    omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
                        schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
                        fan_in_summary_artifact_id: "artifact-1".to_string(),
                    },
                ),
                fan_out_result: None,
            };
            let text = build_artifact_read_text(&resp);
            assert!(text.contains("# Fan-out Linkage Issue Clear (structured)"));
            assert!(text.contains("schema_version: fan_out_linkage_issue_clear.v1"));
            assert!(text.contains("fan_in_summary_artifact_id: artifact-1"));
            assert!(text.contains("summary: fan-out linkage issue cleared fan_in_summary_artifact_id=artifact-1"));
            assert!(text.contains(
                "fan_out_linkage_issue_clear artifact_id=11111111-0000-0000-0000-000000000001"
            ));
            Ok(())
        }

        #[test]
        fn artifact_read_text_includes_fan_out_result_structured_section()
        -> anyhow::Result<()> {
            let artifact_id = ArtifactId::from_str("11111111-0000-0000-0000-000000000001")?;
            let resp = ArtifactReadResponse {
                tool_id: ToolId::new(),
                metadata: ArtifactMetadata {
                    artifact_id,
                    artifact_type: "fan_out_result".to_string(),
                    summary: "fan-out result".to_string(),
                    preview: None,
                    created_at: OffsetDateTime::UNIX_EPOCH,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                    version: 1,
                    content_path: "/tmp/fan-out-result.md".to_string(),
                    size_bytes: 128,
                    provenance: None,
                },
                text: "raw markdown".to_string(),
                truncated: false,
                bytes: 12,
                version: 1,
                latest_version: 1,
                historical: false,
                metadata_source: ArtifactReadMetadataSource::Latest,
                metadata_fallback_reason: None,
                prune_report: None,
                fan_in_summary: None,
                fan_out_linkage_issue: None,
                fan_out_linkage_issue_clear: None,
                fan_out_result: Some(omne_app_server_protocol::ArtifactFanOutResultStructuredData {
                    schema_version: omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
                    task_id: "t-isolated".to_string(),
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    workspace_mode: "isolated_write".to_string(),
                    workspace_cwd: Some("/tmp/subagent/repo".to_string()),
                    isolated_write_patch: Some(
                        omne_app_server_protocol::ArtifactFanOutResultIsolatedWritePatchStructuredData {
                            artifact_type: Some("patch".to_string()),
                            artifact_id: Some("artifact-1".to_string()),
                            truncated: Some(true),
                            read_cmd: Some("omne artifact read thread-1 artifact-1".to_string()),
                            workspace_cwd: None,
                            error: None,
                            structured_error: None,
                        },
                    ),
                    isolated_write_handoff: Some(
                        omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteHandoffStructuredData {
                            workspace_cwd: Some("/tmp/subagent/repo".to_string()),
                            status_argv: vec![
                                "git".to_string(),
                                "-C".to_string(),
                                "/tmp/subagent/repo".to_string(),
                                "status".to_string(),
                                "--short".to_string(),
                                "--".to_string(),
                            ],
                            diff_argv: vec![
                                "git".to_string(),
                                "-C".to_string(),
                                "/tmp/subagent/repo".to_string(),
                                "diff".to_string(),
                                "--binary".to_string(),
                                "--".to_string(),
                            ],
                            apply_patch_hint: Some(
                                "capture diff output and apply in target workspace with git apply"
                                    .to_string(),
                            ),
                            patch: Some(
                                omne_app_server_protocol::ArtifactFanOutResultIsolatedWritePatchStructuredData {
                                    artifact_type: Some("patch".to_string()),
                                    artifact_id: Some("artifact-1".to_string()),
                                    truncated: Some(true),
                                    read_cmd: None,
                                    workspace_cwd: None,
                                    error: None,
                                    structured_error: None,
                                },
                            ),
                        },
                    ),
                    isolated_write_auto_apply: Some(
                        omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                            enabled: true,
                            attempted: true,
                            applied: true,
                            workspace_cwd: Some("/tmp/subagent/repo".to_string()),
                            target_workspace_cwd: Some("/tmp/parent/repo".to_string()),
                            check_argv: vec![
                                "git".to_string(),
                                "-C".to_string(),
                                "/tmp/parent/repo".to_string(),
                                "apply".to_string(),
                                "--check".to_string(),
                                "--whitespace=nowarn".to_string(),
                                "-".to_string(),
                            ],
                            apply_argv: vec![
                                "git".to_string(),
                                "-C".to_string(),
                                "/tmp/parent/repo".to_string(),
                                "apply".to_string(),
                                "--whitespace=nowarn".to_string(),
                                "-".to_string(),
                            ],
                            patch_artifact_id: Some("artifact-1".to_string()),
                            patch_read_cmd: Some(
                                "omne artifact read thread-1 artifact-1".to_string(),
                            ),
                            failure_stage: None,
                            recovery_hint: None,
                            recovery_commands: vec![
                                omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData {
                                    label: "read_patch_artifact".to_string(),
                                    argv: vec![
                                        "omne".to_string(),
                                        "artifact".to_string(),
                                        "read".to_string(),
                                        "thread-1".to_string(),
                                        "artifact-1".to_string(),
                                    ],
                                },
                            ],
                            error: None,
                            structured_error: None,
                        },
                    ),
                    status: "completed".to_string(),
                    reason: None,
                }),
            };

            let text = build_artifact_read_text(&resp);
            assert!(text.contains("# Fan-out Result (structured)"));
            assert!(text.contains("schema_version: fan_out_result.v1"));
            assert!(text.contains("task_id: t-isolated"));
            assert!(text.contains("workspace_mode: isolated_write"));
            assert!(text.contains("## isolated_write_patch"));
            assert!(text.contains("- artifact_type: patch"));
            assert!(text.contains("- artifact_id: artifact-1"));
            assert!(text.contains("- truncated: true"));
            assert!(text.contains("## isolated_write_handoff"));
            assert!(text.contains("- status_argv: git -C /tmp/subagent/repo status --short --"));
            assert!(text.contains("- patch_artifact_id: artifact-1"));
            assert!(text.contains("## isolated_write_auto_apply"));
            assert!(text.contains("- applied: true"));
            assert!(text.contains("- target_workspace_cwd: /tmp/parent/repo"));
            assert!(text.contains("- patch_artifact_id: artifact-1"));
            assert!(text.contains("- patch_read_cmd: omne artifact read thread-1 artifact-1"));
            assert!(text.contains("- recovery_commands:"));
            assert!(text.contains(
                "  - read_patch_artifact: omne artifact read thread-1 artifact-1"
            ));
            Ok(())
        }

        #[test]
        fn parse_artifact_tui_outcome_ok_is_not_misclassified_as_denied() -> anyhow::Result<()> {
            let response = omne_app_server_protocol::ArtifactListResponse {
                tool_id: ToolId::new(),
                artifacts: Vec::new(),
                errors: Vec::new(),
            };
            let value = serde_json::to_value(response)?;
            let parsed: RpcActionOutcome<omne_app_server_protocol::ArtifactListResponse> =
                parse_artifact_tui_outcome("artifact/list", value)?;
            assert!(matches!(parsed, RpcActionOutcome::Ok(_)));
            Ok(())
        }

        #[test]
        fn parse_artifact_tui_outcome_denied_stays_denied() -> anyhow::Result<()> {
            let denied = omne_app_server_protocol::ArtifactDeniedResponse {
                tool_id: ToolId::new(),
                denied: true,
                structured_error: None,
                error_code: None,
                remembered: None,
            };
            let value = serde_json::to_value(denied)?;
            let parsed: RpcActionOutcome<omne_app_server_protocol::ArtifactListResponse> =
                parse_artifact_tui_outcome("artifact/list", value)?;
            assert!(matches!(parsed, RpcActionOutcome::Denied { .. }));
            Ok(())
        }

        #[test]
        fn parse_artifact_tui_outcome_needs_approval_is_exposed() -> anyhow::Result<()> {
            let thread_id = ThreadId::new();
            let approval_id = ApprovalId::new();
            let value = serde_json::to_value(omne_app_server_protocol::ArtifactNeedsApprovalResponse {
                needs_approval: true,
                thread_id,
                approval_id,
            })?;
            let parsed: RpcActionOutcome<omne_app_server_protocol::ArtifactListResponse> =
                parse_artifact_tui_outcome("artifact/list", value)?;
            assert!(matches!(
                parsed,
                RpcActionOutcome::NeedsApproval {
                    thread_id: t,
                    approval_id: a
                } if t == thread_id && a == approval_id
            ));
            Ok(())
        }

        #[test]
        fn parse_process_tui_outcome_ok_is_not_misclassified_as_denied() -> anyhow::Result<()> {
            let response = omne_app_server_protocol::ProcessSignalResponse {
                ok: true,
                accepted: true,
                process_id: ProcessId::new(),
                delivery: omne_app_server_protocol::ProcessSignalDelivery::Queued,
            };
            let value = serde_json::to_value(response)?;
            let parsed: RpcActionOutcome<omne_app_server_protocol::ProcessSignalResponse> =
                parse_process_tui_outcome("process/kill", value)?;
            assert!(matches!(parsed, RpcActionOutcome::Ok(_)));
            Ok(())
        }

        #[test]
        fn parse_process_tui_outcome_denied_stays_denied() -> anyhow::Result<()> {
            let denied = omne_app_server_protocol::ProcessDeniedResponse {
                tool_id: ToolId::new(),
                denied: true,
                thread_id: ThreadId::new(),
                remembered: None,
                structured_error: None,
                error_code: None,
            };
            let value = serde_json::to_value(denied)?;
            let parsed: RpcActionOutcome<omne_app_server_protocol::ProcessSignalResponse> =
                parse_process_tui_outcome("process/kill", value)?;
            assert!(matches!(parsed, RpcActionOutcome::Denied { .. }));
            Ok(())
        }

        #[test]
        fn parse_process_tui_outcome_needs_approval_is_exposed() -> anyhow::Result<()> {
            let thread_id = ThreadId::new();
            let approval_id = ApprovalId::new();
            let value = serde_json::to_value(omne_app_server_protocol::ProcessNeedsApprovalResponse {
                needs_approval: true,
                thread_id,
                approval_id,
            })?;
            let parsed: RpcActionOutcome<omne_app_server_protocol::ProcessSignalResponse> =
                parse_process_tui_outcome("process/kill", value)?;
            assert!(matches!(
                parsed,
                RpcActionOutcome::NeedsApproval {
                    thread_id: t,
                    approval_id: a
                } if t == thread_id && a == approval_id
            ));
            Ok(())
        }

        #[test]
        fn approval_action_label_prefers_action_id() {
            let request = omne_app_server_protocol::ApprovalRequestInfo {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: "legacy/action".to_string(),
                action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                params: serde_json::json!({}),
                summary: None,
                requested_at: "2026-01-01T00:00:00Z".to_string(),
            };
            assert_eq!(approval_action_label(&request), "process/start");
        }

        #[test]
        fn approval_subagent_state_color_maps_running_and_failed() {
            let request = omne_app_server_protocol::ApprovalRequestInfo {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval),
                params: serde_json::json!({}),
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    requirement: None,
                    argv: None,
                    cwd: None,
                    process_id: None,
                    artifact_type: None,
                    path: None,
                    server: None,
                    tool: Some("process/start".to_string()),
                    hook: None,
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: Some(ApprovalId::new()),
                    child_attention_state: Some("running".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                }),
                requested_at: "2026-01-01T00:00:00Z".to_string(),
            };
            assert_eq!(
                approval_subagent_state_color(&request),
                Some(ratatui::style::Color::Yellow)
            );

            let request_failed = omne_app_server_protocol::ApprovalRequestInfo {
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    child_attention_state: Some("FAILED".to_string()),
                    ..request.summary.clone().expect("summary")
                }),
                ..request
            };
            assert_eq!(
                approval_subagent_state_color(&request_failed),
                Some(ratatui::style::Color::LightRed)
            );
        }

        #[test]
        fn approval_subagent_state_color_ignores_non_subagent_action() {
            let request = omne_app_server_protocol::ApprovalRequestInfo {
                approval_id: ApprovalId::new(),
                turn_id: None,
                action: "process/start".to_string(),
                action_id: Some(omne_app_server_protocol::ThreadApprovalActionId::ProcessStart),
                params: serde_json::json!({}),
                summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                    requirement: None,
                    argv: None,
                    cwd: None,
                    process_id: None,
                    artifact_type: None,
                    path: None,
                    server: None,
                    tool: Some("process/start".to_string()),
                    hook: None,
                    child_thread_id: Some(ThreadId::new()),
                    child_turn_id: None,
                    child_approval_id: Some(ApprovalId::new()),
                    child_attention_state: Some("FAILED".to_string()),
                    child_last_turn_status: None,
                    approve_cmd: None,
                    deny_cmd: None,
                }),
                requested_at: "2026-01-01T00:00:00Z".to_string(),
            };
            assert_eq!(approval_subagent_state_color(&request), None);
        }

        #[test]
        fn sort_approvals_for_overlay_prioritizes_subagent_attention_state() {
            let mk = |action: &str,
                      action_id: omne_app_server_protocol::ThreadApprovalActionId,
                      state: Option<&str>,
                      requested_at: &str| {
                ApprovalItem {
                    request: omne_app_server_protocol::ApprovalRequestInfo {
                        approval_id: ApprovalId::new(),
                        turn_id: None,
                        action: action.to_string(),
                        action_id: Some(action_id),
                        params: serde_json::json!({}),
                        summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                            requirement: None,
                            argv: None,
                            cwd: None,
                            process_id: None,
                            artifact_type: None,
                            path: None,
                            server: None,
                            tool: Some("process/start".to_string()),
                            hook: None,
                            child_thread_id: Some(ThreadId::new()),
                            child_turn_id: None,
                            child_approval_id: Some(ApprovalId::new()),
                            child_attention_state: state.map(ToString::to_string),
                            child_last_turn_status: None,
                            approve_cmd: None,
                            deny_cmd: None,
                        }),
                        requested_at: requested_at.to_string(),
                    },
                    decision: None,
                }
            };

            let mut items = vec![
                mk(
                    "process/start",
                    omne_app_server_protocol::ThreadApprovalActionId::ProcessStart,
                    Some("failed"),
                    "2026-01-01T00:00:00Z",
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("running"),
                    "2026-01-02T00:00:00Z",
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("FAILED"),
                    "2026-01-03T00:00:00Z",
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("idle"),
                    "2026-01-04T00:00:00Z",
                ),
            ];

            sort_approvals_for_overlay(items.as_mut_slice());
            let ordered = items
                .iter()
                .map(|it| {
                    (
                        it.request.action.clone(),
                        it.request
                            .summary
                            .as_ref()
                            .and_then(|s| s.child_attention_state.clone()),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                ordered,
                vec![
                    ("subagent/proxy_approval".to_string(), Some("FAILED".to_string())),
                    ("subagent/proxy_approval".to_string(), Some("running".to_string())),
                    ("subagent/proxy_approval".to_string(), Some("idle".to_string())),
                    ("process/start".to_string(), Some("failed".to_string())),
                ]
            );
        }

        #[test]
        fn sort_approvals_for_overlay_keeps_requested_at_within_same_priority() {
            let mut items = vec![
                ApprovalItem {
                    request: omne_app_server_protocol::ApprovalRequestInfo {
                        approval_id: ApprovalId::new(),
                        turn_id: None,
                        action: "subagent/proxy_approval".to_string(),
                        action_id: Some(
                            omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                        ),
                        params: serde_json::json!({}),
                        summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                            requirement: None,
                            argv: None,
                            cwd: None,
                            process_id: None,
                            artifact_type: None,
                            path: None,
                            server: None,
                            tool: None,
                            hook: None,
                            child_thread_id: Some(ThreadId::new()),
                            child_turn_id: None,
                            child_approval_id: Some(ApprovalId::new()),
                            child_attention_state: Some("running".to_string()),
                            child_last_turn_status: None,
                            approve_cmd: None,
                            deny_cmd: None,
                        }),
                        requested_at: "2026-01-03T00:00:00Z".to_string(),
                    },
                    decision: None,
                },
                ApprovalItem {
                    request: omne_app_server_protocol::ApprovalRequestInfo {
                        approval_id: ApprovalId::new(),
                        turn_id: None,
                        action: "subagent/proxy_approval".to_string(),
                        action_id: Some(
                            omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                        ),
                        params: serde_json::json!({}),
                        summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                            requirement: None,
                            argv: None,
                            cwd: None,
                            process_id: None,
                            artifact_type: None,
                            path: None,
                            server: None,
                            tool: None,
                            hook: None,
                            child_thread_id: Some(ThreadId::new()),
                            child_turn_id: None,
                            child_approval_id: Some(ApprovalId::new()),
                            child_attention_state: Some("running".to_string()),
                            child_last_turn_status: None,
                            approve_cmd: None,
                            deny_cmd: None,
                        }),
                        requested_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    decision: None,
                },
            ];

            sort_approvals_for_overlay(items.as_mut_slice());
            assert!(items[0].request.requested_at < items[1].request.requested_at);
        }

        #[test]
        fn next_failed_subagent_approval_index_wraps_to_next_match() {
            let mk = |action: &str,
                      action_id: omne_app_server_protocol::ThreadApprovalActionId,
                      state: Option<&str>| {
                ApprovalItem {
                    request: omne_app_server_protocol::ApprovalRequestInfo {
                        approval_id: ApprovalId::new(),
                        turn_id: None,
                        action: action.to_string(),
                        action_id: Some(action_id),
                        params: serde_json::json!({}),
                        summary: Some(
                            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                                requirement: None,
                                argv: None,
                                cwd: None,
                                process_id: None,
                                artifact_type: None,
                                path: None,
                                server: None,
                                tool: None,
                                hook: None,
                                child_thread_id: Some(ThreadId::new()),
                                child_turn_id: None,
                                child_approval_id: Some(ApprovalId::new()),
                                child_attention_state: state.map(ToString::to_string),
                                child_last_turn_status: None,
                                approve_cmd: None,
                                deny_cmd: None,
                            },
                        ),
                        requested_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    decision: None,
                }
            };

            let items = vec![
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("running"),
                ),
                mk(
                    "process/start",
                    omne_app_server_protocol::ThreadApprovalActionId::ProcessStart,
                    Some("failed"),
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("FAILED"),
                ),
            ];

            assert_eq!(
                next_failed_subagent_approval_index(items.as_slice(), 0),
                Some(2)
            );
            assert_eq!(
                next_failed_subagent_approval_index(items.as_slice(), 2),
                Some(2)
            );
        }

        #[test]
        fn next_failed_subagent_approval_index_returns_none_without_match() {
            let items = vec![ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some("running".to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            }];

            assert_eq!(
                next_failed_subagent_approval_index(items.as_slice(), 0),
                None
            );
        }

        #[test]
        fn prev_failed_subagent_approval_index_wraps_to_previous_match() {
            let mk = |action: &str,
                      action_id: omne_app_server_protocol::ThreadApprovalActionId,
                      state: Option<&str>| {
                ApprovalItem {
                    request: omne_app_server_protocol::ApprovalRequestInfo {
                        approval_id: ApprovalId::new(),
                        turn_id: None,
                        action: action.to_string(),
                        action_id: Some(action_id),
                        params: serde_json::json!({}),
                        summary: Some(
                            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                                requirement: None,
                                argv: None,
                                cwd: None,
                                process_id: None,
                                artifact_type: None,
                                path: None,
                                server: None,
                                tool: None,
                                hook: None,
                                child_thread_id: Some(ThreadId::new()),
                                child_turn_id: None,
                                child_approval_id: Some(ApprovalId::new()),
                                child_attention_state: state.map(ToString::to_string),
                                child_last_turn_status: None,
                                approve_cmd: None,
                                deny_cmd: None,
                            },
                        ),
                        requested_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    decision: None,
                }
            };

            let items = vec![
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("FAILED"),
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("running"),
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("error"),
                ),
            ];

            assert_eq!(
                prev_failed_subagent_approval_index(items.as_slice(), 1),
                Some(0)
            );
            assert_eq!(
                prev_failed_subagent_approval_index(items.as_slice(), 0),
                Some(2)
            );
        }

        #[test]
        fn prev_failed_subagent_approval_index_returns_none_without_match() {
            let items = vec![ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some("running".to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            }];

            assert_eq!(
                prev_failed_subagent_approval_index(items.as_slice(), 0),
                None
            );
        }

        #[test]
        fn failed_subagent_approval_count_counts_only_failed_subagent_items() {
            let mk = |action: &str,
                      action_id: omne_app_server_protocol::ThreadApprovalActionId,
                      state: Option<&str>| {
                ApprovalItem {
                    request: omne_app_server_protocol::ApprovalRequestInfo {
                        approval_id: ApprovalId::new(),
                        turn_id: None,
                        action: action.to_string(),
                        action_id: Some(action_id),
                        params: serde_json::json!({}),
                        summary: Some(
                            omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                                requirement: None,
                                argv: None,
                                cwd: None,
                                process_id: None,
                                artifact_type: None,
                                path: None,
                                server: None,
                                tool: None,
                                hook: None,
                                child_thread_id: Some(ThreadId::new()),
                                child_turn_id: None,
                                child_approval_id: Some(ApprovalId::new()),
                                child_attention_state: state.map(ToString::to_string),
                                child_last_turn_status: None,
                                approve_cmd: None,
                                deny_cmd: None,
                            },
                        ),
                        requested_at: "2026-01-01T00:00:00Z".to_string(),
                    },
                    decision: None,
                }
            };

            let items = vec![
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("FAILED"),
                ),
                mk(
                    "subagent/proxy_approval",
                    omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    Some("running"),
                ),
                mk(
                    "process/start",
                    omne_app_server_protocol::ThreadApprovalActionId::ProcessStart,
                    Some("failed"),
                ),
            ];
            assert_eq!(failed_subagent_approval_count(items.as_slice()), 1);
        }

        #[test]
        fn approval_filter_label_and_cycle_cover_all_variants() {
            assert_eq!(approval_filter_label(ApprovalsFilter::All), "all");
            assert_eq!(approval_filter_label(ApprovalsFilter::FailedSubagent), "failed");
            assert_eq!(approval_filter_label(ApprovalsFilter::RunningSubagent), "running");

            assert_eq!(
                next_approvals_filter(ApprovalsFilter::All),
                ApprovalsFilter::FailedSubagent
            );
            assert_eq!(
                next_approvals_filter(ApprovalsFilter::FailedSubagent),
                ApprovalsFilter::RunningSubagent
            );
            assert_eq!(
                next_approvals_filter(ApprovalsFilter::RunningSubagent),
                ApprovalsFilter::All
            );
        }

        #[test]
        fn rebuild_filtered_approvals_keeps_selection_by_approval_id() {
            let failed_id = ApprovalId::new();
            let running_id = ApprovalId::new();
            let mk = |approval_id: ApprovalId, state: &str| ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id,
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some(state.to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut overlay = ApprovalsOverlay {
                thread_id: ThreadId::new(),
                approvals: vec![],
                all_approvals: vec![mk(running_id, "running"), mk(failed_id, "failed")],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::FailedSubagent,
                subagent_pending_summary: None,
            };

            rebuild_filtered_approvals(&mut overlay, Some(failed_id));
            assert_eq!(overlay.approvals.len(), 1);
            assert_eq!(overlay.approvals[0].request.approval_id, failed_id);
            assert_eq!(overlay.selected, 0);
        }

        #[test]
        fn handle_local_approvals_key_supports_failed_navigation_and_filter_cycle() {
            let failed_id = ApprovalId::new();
            let failed_id_2 = ApprovalId::new();
            let running_id = ApprovalId::new();
            let running_id_2 = ApprovalId::new();
            let mk = |approval_id: ApprovalId, state: &str| ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id,
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some(state.to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut overlay = new_approvals_overlay(
                ThreadId::new(),
                vec![
                    mk(running_id, "running"),
                    mk(failed_id, "failed"),
                    mk(running_id_2, "running"),
                    mk(failed_id_2, "error"),
                ],
                0,
                None,
            );

            let status = match handle_local_approvals_key(&mut overlay, KeyCode::Char('f')) {
                ApprovalsLocalKeyResult::Handled(status) => status,
                ApprovalsLocalKeyResult::Unhandled => panic!("expected handled key"),
            };
            assert_eq!(overlay.approvals[overlay.selected].request.approval_id, failed_id);
            assert!(
                status
                    .as_deref()
                    .is_some_and(|msg| msg.contains(&failed_id.to_string()))
            );

            let status = match handle_local_approvals_key(&mut overlay, KeyCode::Char('F')) {
                ApprovalsLocalKeyResult::Handled(status) => status,
                ApprovalsLocalKeyResult::Unhandled => panic!("expected handled key"),
            };
            assert_eq!(
                overlay.approvals[overlay.selected].request.approval_id,
                failed_id_2
            );
            assert!(
                status
                    .as_deref()
                    .is_some_and(|msg| msg.contains(&failed_id_2.to_string()))
            );

            let status = match handle_local_approvals_key(&mut overlay, KeyCode::Char('t')) {
                ApprovalsLocalKeyResult::Handled(status) => status,
                ApprovalsLocalKeyResult::Unhandled => panic!("expected handled key"),
            };
            assert_eq!(overlay.filter, ApprovalsFilter::FailedSubagent);
            assert_eq!(overlay.approvals.len(), 2);
            assert!(
                status
                    .as_deref()
                    .is_some_and(|msg| msg.contains("approvals filter=failed"))
            );

            let status = match handle_local_approvals_key(&mut overlay, KeyCode::Char('m')) {
                ApprovalsLocalKeyResult::Handled(status) => status,
                ApprovalsLocalKeyResult::Unhandled => panic!("expected handled key"),
            };
            assert!(overlay.remember);
            assert!(
                status
                    .as_deref()
                    .is_some_and(|msg| msg.contains("remember=true"))
            );
        }

        #[test]
        fn handle_local_processes_key_supports_selection_navigation() {
            let process_a = ProcessInfo {
                process_id: ProcessId::new(),
                thread_id: ThreadId::new(),
                turn_id: None,
                argv: vec!["sleep".to_string(), "1".to_string()],
                cwd: "/tmp".to_string(),
                started_at: "2026-01-01T00:00:00Z".to_string(),
                status: ProcessStatus::Running,
                exit_code: None,
                stdout_path: "/tmp/stdout.log".to_string(),
                stderr_path: "/tmp/stderr.log".to_string(),
                last_update_at: "2026-01-01T00:00:00Z".to_string(),
            };
            let process_b = ProcessInfo {
                process_id: ProcessId::new(),
                thread_id: ThreadId::new(),
                turn_id: None,
                argv: vec!["echo".to_string(), "ok".to_string()],
                cwd: "/tmp".to_string(),
                started_at: "2026-01-01T00:00:01Z".to_string(),
                status: ProcessStatus::Running,
                exit_code: None,
                stdout_path: "/tmp/stdout2.log".to_string(),
                stderr_path: "/tmp/stderr2.log".to_string(),
                last_update_at: "2026-01-01T00:00:01Z".to_string(),
            };
            let mut overlay = ProcessesOverlay {
                thread_id: ThreadId::new(),
                processes: vec![process_a, process_b],
                selected: 0,
            };

            match handle_local_processes_key(&mut overlay, KeyCode::Down) {
                ProcessesLocalKeyResult::Handled => {}
                ProcessesLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(overlay.selected, 1);

            match handle_local_processes_key(&mut overlay, KeyCode::Down) {
                ProcessesLocalKeyResult::Handled => {}
                ProcessesLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(overlay.selected, 1);

            match handle_local_processes_key(&mut overlay, KeyCode::Up) {
                ProcessesLocalKeyResult::Handled => {}
                ProcessesLocalKeyResult::Unhandled => panic!("expected handled key"),
            }
            assert_eq!(overlay.selected, 0);
        }

        #[test]
        fn approval_summary_hint_uses_path_when_present() {
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: Some("/tmp/project/src/main.rs".to_string()),
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            };
            let hint = approval_summary_hint(&summary).expect("summary hint");
            assert!(hint.starts_with("path="));
            assert!(hint.contains("src/main.rs"));
        }

        #[test]
        fn approval_subagent_link_surfaces_child_identifiers() {
            let child_thread_id = ThreadId::new();
            let child_turn_id = TurnId::new();
            let child_approval_id = ApprovalId::new();
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: Some("process/start".to_string()),
                hook: None,
                child_thread_id: Some(child_thread_id),
                child_turn_id: Some(child_turn_id),
                child_approval_id: Some(child_approval_id),
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            };
            let link = approval_subagent_link(&summary).expect("subagent link");
            assert!(link.contains("child_thread_id="));
            assert!(link.contains(&child_thread_id.to_string()));
            assert!(link.contains("child_turn_id="));
            assert!(link.contains(&child_turn_id.to_string()));
            assert!(link.contains("child_approval_id="));
            assert!(link.contains(&child_approval_id.to_string()));
        }

        #[test]
        fn approval_summary_hint_prefers_context_when_subagent_ids_exist() {
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: Some("/tmp/ws/src/main.rs".to_string()),
                server: None,
                tool: Some("file/write".to_string()),
                hook: None,
                child_thread_id: Some(ThreadId::new()),
                child_turn_id: Some(TurnId::new()),
                child_approval_id: Some(ApprovalId::new()),
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            };
            let hint = approval_summary_hint(&summary).expect("summary hint");
            assert!(hint.contains("path=/tmp/ws/src/main.rs"));
        }

        #[test]
        fn approval_summary_lines_include_approve_cmd() {
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some("omne approval decide t1 a1 --approve".to_string()),
                deny_cmd: Some("omne approval decide t1 a1 --deny".to_string()),
            };
            let lines = approval_summary_lines(&summary);
            assert!(lines.iter().any(|line| line.contains("approve_cmd: ")));
            assert!(lines.iter().any(|line| line.contains("--approve")));
        }

        #[test]
        fn approval_summary_hint_falls_back_to_approve_cmd() {
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: None,
                hook: None,
                child_thread_id: None,
                child_turn_id: None,
                child_approval_id: None,
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some("omne approval decide t1 a1 --approve".to_string()),
                deny_cmd: Some("omne approval decide t1 a1 --deny".to_string()),
            };
            let hint = approval_summary_hint(&summary).expect("summary hint");
            assert!(hint.contains("approve_cmd="));
            assert!(hint.contains("--approve"));
        }

        #[test]
        fn approval_details_include_quick_command_section() {
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: Some("process/start".to_string()),
                hook: None,
                child_thread_id: Some(ThreadId::new()),
                child_turn_id: None,
                child_approval_id: Some(ApprovalId::new()),
                child_attention_state: None,
                child_last_turn_status: None,
                approve_cmd: Some("omne approval decide t1 a1 --approve".to_string()),
                deny_cmd: Some("omne approval decide t1 a1 --deny".to_string()),
            };
            let item = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({ "k": "v" }),
                    summary: Some(summary),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let panel = build_approval_details(&item);
            assert!(panel.contains("quick_command:"));
            assert!(panel.contains("approve: "));
            assert!(panel.contains("--approve"));
            assert!(panel.contains("deny: "));
            assert!(panel.contains("--deny"));

            let text = build_approval_details_text(&item);
            assert!(text.contains("# Quick Command"));
            assert!(text.contains("approve: "));
            assert!(text.contains("--approve"));
            assert!(text.contains("deny: "));
            assert!(text.contains("--deny"));
        }

        #[test]
        fn approvals_overlay_row_shows_compact_subagent_state_hint() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let summary = omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                requirement: None,
                argv: None,
                cwd: None,
                process_id: None,
                artifact_type: None,
                path: None,
                server: None,
                tool: Some("process/start".to_string()),
                hook: None,
                child_thread_id: Some(ThreadId::new()),
                child_turn_id: Some(TurnId::new()),
                child_approval_id: Some(ApprovalId::new()),
                child_attention_state: Some("FAILED".to_string()),
                child_last_turn_status: None,
                approve_cmd: None,
                deny_cmd: None,
            };
            let approval = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(summary),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id,
                approvals: vec![approval.clone()],
                all_approvals: vec![approval],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: None,
            }));

            let actual = render_to_string(&mut state, 300, 20)?;
            assert!(actual.contains("subagent/proxy_approval (failed |"));
            assert!(!actual.contains("subagent/proxy_approval (child_attention_state="));
            Ok(())
        }

        #[test]
        fn approvals_overlay_title_includes_subagent_pending_summary() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let approval = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: None,
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id,
                approvals: vec![approval.clone()],
                all_approvals: vec![approval],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: Some(SubagentPendingSummary {
                    total: 3,
                    states: std::collections::BTreeMap::from([
                        ("running".to_string(), 2usize),
                        ("failed".to_string(), 1usize),
                    ]),
                }),
            }));

            let actual = render_to_string(&mut state, 180, 20)?;
            assert!(actual.contains("sub=3("));
            assert!(actual.contains("running:2"));
            assert!(actual.contains("failed:1"));
            Ok(())
        }

        #[test]
        fn approvals_overlay_title_includes_failed_count() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let approval = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some("failed".to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id,
                approvals: vec![approval.clone()],
                all_approvals: vec![approval],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::All,
                subagent_pending_summary: None,
            }));

            let actual = render_to_string(&mut state, 240, 20)?;
            assert!(actual.contains("failed=1"));
            Ok(())
        }

        #[test]
        fn approvals_overlay_title_includes_filter_label() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let approval = ApprovalItem {
                request: omne_app_server_protocol::ApprovalRequestInfo {
                    approval_id: ApprovalId::new(),
                    turn_id: None,
                    action: "subagent/proxy_approval".to_string(),
                    action_id: Some(
                        omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval,
                    ),
                    params: serde_json::json!({}),
                    summary: Some(omne_app_server_protocol::ThreadAttentionPendingApprovalSummary {
                        requirement: None,
                        argv: None,
                        cwd: None,
                        process_id: None,
                        artifact_type: None,
                        path: None,
                        server: None,
                        tool: None,
                        hook: None,
                        child_thread_id: Some(ThreadId::new()),
                        child_turn_id: None,
                        child_approval_id: Some(ApprovalId::new()),
                        child_attention_state: Some("failed".to_string()),
                        child_last_turn_status: None,
                        approve_cmd: None,
                        deny_cmd: None,
                    }),
                    requested_at: "2026-01-01T00:00:00Z".to_string(),
                },
                decision: None,
            };

            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id,
                approvals: vec![approval.clone()],
                all_approvals: vec![approval],
                selected: 0,
                remember: false,
                filter: ApprovalsFilter::FailedSubagent,
                subagent_pending_summary: None,
            }));

            let actual = render_to_string(&mut state, 260, 20)?;
            assert!(actual.contains("filter=failed"));
            Ok(())
        }
    }
