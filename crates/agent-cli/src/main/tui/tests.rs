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
            state.total_tokens_used = 39_280;
            state.threads = vec![
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                    cwd: Some("/repo".to_string()),
                    attention_state: "running".to_string(),
                    has_plan_ready: false,
                    has_diff_ready: false,
                    has_fan_out_linkage_issue: false,
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
                    has_test_failed: false,
                    created_at: None,
                    updated_at: None,
                    title: Some("Second".to_string()),
                    first_message: Some("world".to_string()),
                },
            ];
            state.selected_thread = 1;

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"threads [all] (↑↓ Enter n=new l=filter r=refresh q=quit)        
Updated  Attn   Title   CWD    Message                          
  -        run    First   /repo  hello                          
▶ -        link!  Second  /repo  world                          
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
69% context left  threads f=all (Ctrl-K=commands)               "#;
            assert_eq!(actual, expected);
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
                text: "Streaming...".to_string(),
            });
            state.input = "next".to_string();
            state.header.model_context_window = Some(100_000);
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

            state.only_fan_out_linkage_issue = true;
            let labels = build_root_palette(&state)
                .items
                .iter()
                .map(|item| item.label.clone())
                .collect::<Vec<_>>();
            assert!(labels.iter().any(|label| label == "linkage-filter=on"));
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
            let linkage_only = state.apply_thread_picker_filters(threads);
            assert_eq!(linkage_only.len(), 1);
            assert_eq!(linkage_only[0].thread_id, t2.thread_id);
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
                    task_count: 1,
                    scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                        env_max_concurrent_subagents: 4,
                        effective_concurrency_limit: 2,
                        priority_aging_rounds: 3,
                    },
                    tasks: vec![omne_app_server_protocol::ArtifactFanInSummaryTask {
                        task_id: "task_a".to_string(),
                        title: "do work".to_string(),
                        thread_id: None,
                        turn_id: None,
                        status: "NeedUserInput".to_string(),
                        reason: Some("awaiting approval".to_string()),
                        dependency_blocked: false,
                        result_artifact_id: None,
                        result_artifact_error: None,
                        result_artifact_error_id: None,
                        pending_approval: Some(
                            omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                                approval_id: "approval-1".to_string(),
                                action: "artifact/read".to_string(),
                                summary: None,
                                approve_cmd: Some(
                                    "omne approval decide thread-1 approval-1 --approve".to_string(),
                                ),
                                deny_cmd: Some(
                                    "omne approval decide thread-1 approval-1 --deny".to_string(),
                                ),
                            },
                        ),
                    }],
                }),
                fan_out_linkage_issue: None,
                fan_out_linkage_issue_clear: None,
                fan_out_result: None,
            };
            let text = build_artifact_read_text(&resp);
            assert!(text.contains("# Fan-in Summary (structured)"));
            assert!(text.contains("schema_version: fan_in_summary.v1"));
            assert!(text.contains("pending_approval: action=artifact/read approval_id=approval-1"));
            assert!(text.contains("approve_cmd: omne approval decide thread-1 approval-1 --approve"));
            assert!(text.contains("deny_cmd: omne approval decide thread-1 approval-1 --deny"));
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
                                },
                            ),
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
            let response = omne_app_server_protocol::ProcessSignalResponse { ok: true };
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
    }
