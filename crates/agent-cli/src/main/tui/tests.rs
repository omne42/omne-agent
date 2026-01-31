    #[cfg(test)]
    mod tests {
        use std::str::FromStr;

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
            state.total_input_tokens_used = 20_000;
            state.total_cache_input_tokens_used = 1_234;
            state.total_output_tokens_used = 19_280;
            state.total_tokens_used = 39_280;
            state.last_tokens_in_context_window = Some(39_280);
            state.threads = vec![
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                    cwd: Some("/repo".to_string()),
                    created_at: None,
                    updated_at: None,
                    title: Some("First".to_string()),
                    first_message: Some("hello".to_string()),
                },
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000002")?,
                    cwd: Some("/repo".to_string()),
                    created_at: None,
                    updated_at: None,
                    title: Some("Second".to_string()),
                    first_message: Some("world".to_string()),
                },
            ];
            state.selected_thread = 1;

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"threads (↑↓ Enter=open n=new r=refresh q/Ctrl-C=quit)           
Updated  Title   CWD    Message                                 
  -        First   /repo  hello                                 
▶ -        Second  /repo  world                                 
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
69% ctx  input: 20000(1234), output: 19280  threads (Ctrl-K=comm"#;
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
            state.total_input_tokens_used = 20_000;
            state.total_cache_input_tokens_used = 1_234;
            state.total_output_tokens_used = 19_280;
            state.total_tokens_used = 39_280;
            state.last_tokens_in_context_window = Some(39_280);

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"                                                                
system: [model] gpt-4.1 (global_default)                        
user: Hello                                                     
assistant: Hi!                                                  
assistant: Streaming...                                         
                                                                
                                                                
                                                                
                                                                
› next                                                          
                                                                
69% ctx  input: 20000(1234), output: 19280  th=00000000 mode=cod"#;
            assert_eq!(actual, expected);
            Ok(())
        }

        #[test]
        fn scrollback_mode_renders_transcript_and_prompt() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;

            let mut state = UiState::new(false);
            state.scrollback_enabled = true;
            state.active_thread = Some(thread_id);
            state.push_transcript(TranscriptEntry {
                role: TranscriptRole::Assistant,
                text: "Hi!".to_string(),
            });
            state.input = "next".to_string();

            let rendered = render_to_string(&mut state, 64, 12)?;
            assert!(rendered.contains("assistant: Hi!"));
            assert!(rendered.contains("› next"));
            Ok(())
        }

        #[test]
        fn mouse_wheel_scrolls_transcript_in_thread_view() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
            state.transcript_max_scroll = 10;
            state.transcript_scroll = 5;
            state.transcript_follow = false;

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 2);
            assert!(!state.transcript_follow);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 0);
            assert!(!state.transcript_follow);

            assert!(!state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 0);
            assert!(!state.transcript_follow);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 3);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 6);
            assert!(!state.transcript_follow);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 9);
            assert!(!state.transcript_follow);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 10);
            assert!(state.transcript_follow);

            assert!(!state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.transcript_scroll, 10);
            assert!(state.transcript_follow);

            Ok(())
        }

        #[test]
        fn mouse_wheel_scrolls_text_overlay() {
            let mut state = UiState::new(false);
            state.overlays.push(Overlay::Text(TextOverlay {
                title: "overlay".to_string(),
                text: "hello\nworld".to_string(),
                scroll: 0,
            }));

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));

            let scroll = match state.overlays.last() {
                Some(Overlay::Text(view)) => view.scroll,
                other => panic!("expected text overlay, got {other:?}"),
            };
            assert_eq!(scroll, 3);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));

            let scroll = match state.overlays.last() {
                Some(Overlay::Text(view)) => view.scroll,
                other => panic!("expected text overlay, got {other:?}"),
            };
            assert_eq!(scroll, 0);
        }

        #[test]
        fn mouse_wheel_moves_thread_picker_selection() -> anyhow::Result<()> {
            let mut state = UiState::new(false);
            state.threads = vec![
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                    cwd: None,
                    created_at: None,
                    updated_at: None,
                    title: Some("First".to_string()),
                    first_message: None,
                },
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000002")?,
                    cwd: None,
                    created_at: None,
                    updated_at: None,
                    title: Some("Second".to_string()),
                    first_message: None,
                },
            ];
            state.selected_thread = 0;

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.selected_thread, 1);

            assert!(!state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.selected_thread, 1);

            assert!(state.handle_mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::empty(),
            }));
            assert_eq!(state.selected_thread, 0);

            Ok(())
        }

        #[test]
        fn parse_inline_context_allows_trailing_space_for_slash_commands() {
            let ctx = parse_inline_context("/model ").expect("context");
            assert!(matches!(ctx.kind, InlinePaletteKind::Model));
            assert_eq!(ctx.query, "");
        }

        #[test]
        fn input_cursor_supports_midline_editing() {
            let mut state = UiState::new(false);

            state.insert_input_char('a');
            state.insert_input_char('b');
            state.insert_input_char('c');
            assert_eq!(state.input, "abc");
            assert_eq!(state.input_cursor, 3);

            state.move_input_cursor_left();
            state.move_input_cursor_left();
            assert_eq!(state.input_cursor, 1);

            state.insert_input_char('X');
            assert_eq!(state.input, "aXbc");
            assert_eq!(state.input_cursor, 2);

            state.input_backspace();
            assert_eq!(state.input, "abc");
            assert_eq!(state.input_cursor, 1);

            state.input_delete();
            assert_eq!(state.input, "ac");
            assert_eq!(state.input_cursor, 1);

            state.clear_input();
            state.insert_input_char('你');
            state.insert_input_char('好');
            assert_eq!(state.input, "你好");
            assert_eq!(state.input_cursor, "你好".len());

            state.move_input_cursor_left();
            assert_eq!(state.input_cursor, "你".len());

            state.input_backspace();
            assert_eq!(state.input, "好");
            assert_eq!(state.input_cursor, 0);
        }

        #[test]
        fn build_input_lines_positions_cursor_across_wrap() {
            // prompt is 2 columns ("› "), so width=9 => available=7.
            // "abcdefg" fills the first segment, "h" wraps to the next.
            let render = build_input_lines("abcdefgh", 8, 9);
            assert_eq!(render.cursor_line, 2); // padding + wrapped line index 1
            assert_eq!(render.cursor_col, 3); // prompt + 1 char on wrapped line
        }

        #[test]
        fn scrub_wide_symbol_placeholders_removes_extra_spaces() {
            let area = ratatui::layout::Rect::new(0, 0, 4, 1);
            let mut buf = ratatui::buffer::Buffer::empty(area);
            buf.set_string(0, 0, "你好", ratatui::style::Style::default());
            assert_eq!(buf.content[1].symbol(), " ");
            assert_eq!(buf.content[3].symbol(), " ");

            scrub_wide_symbol_placeholders(&mut buf);
            assert_eq!(buf.content[1].symbol(), "");
            assert_eq!(buf.content[3].symbol(), "");
        }

        #[test]
        fn apply_live_event_dedupes_by_seq() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);

            let event = ThreadEvent {
                seq: pm_protocol::EventSeq(1),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::TurnStarted {
                    turn_id,
                    input: "执行ls -a".to_string(),
                    context_refs: None,
                    attachments: None,
                    priority: pm_protocol::TurnPriority::Foreground,
                },
            };

            assert!(state.apply_live_event(&event));
            assert_eq!(state.transcript.len(), 1);

            assert!(!state.apply_live_event(&event));
            assert_eq!(state.transcript.len(), 1);
            Ok(())
        }

        #[test]
        fn agent_step_tool_results_render_output_text() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);

            let tool_output = serde_json::json!({
                "stdout": "a\nb\n",
                "stderr": "",
                "exit_code": 0,
            });

            let event = ThreadEvent {
                seq: pm_protocol::EventSeq(1),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AgentStep {
                    turn_id,
                    step: 1,
                    model: "gpt-test".to_string(),
                    response_id: "resp_1".to_string(),
                    text: None,
                    tool_calls: vec![pm_protocol::AgentStepToolCall {
                        name: "exec_command".to_string(),
                        call_id: "call_1".to_string(),
                        arguments: "{}".to_string(),
                    }],
                    tool_results: vec![pm_protocol::AgentStepToolResult {
                        call_id: "call_1".to_string(),
                        output: serde_json::to_string(&tool_output)?,
                    }],
                    token_usage: None,
                    warnings_count: None,
                },
            };

            state.apply_event(&event);
            let last = state.transcript.back().expect("transcript entry");
            assert!(matches!(last.role, TranscriptRole::Tool));
            assert!(last.text.contains("exec/command"));
            assert!(last.text.contains("a"));
            assert!(last.text.contains("b"));
            Ok(())
        }

        #[test]
        fn process_tool_results_render_stdout_tail_and_hide_paths() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;
            let process_id = ProcessId::from_str("00000000-0000-0000-0000-0000000000bb")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(1),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::ProcessStarted {
                    process_id,
                    turn_id: Some(turn_id),
                    argv: vec!["bash".to_string(), "-lc".to_string(), "ls -a".to_string()],
                    cwd: "/repo".to_string(),
                    stdout_path: "/tmp/stdout.log".to_string(),
                    stderr_path: "/tmp/stderr.log".to_string(),
                },
            });

            let start_output = serde_json::json!({
                "process_id": process_id,
                "stdout_path": "/tmp/stdout.log",
                "stderr_path": "/tmp/stderr.log",
            });

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(2),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AgentStep {
                    turn_id,
                    step: 0,
                    model: "gpt-test".to_string(),
                    response_id: "resp_1".to_string(),
                    text: None,
                    tool_calls: vec![pm_protocol::AgentStepToolCall {
                        name: "process_start".to_string(),
                        call_id: "call_start".to_string(),
                        arguments: serde_json::to_string(&serde_json::json!({
                            "argv": ["bash", "-lc", "ls -a"],
                            "cwd": "/repo",
                        }))?,
                    }],
                    tool_results: vec![pm_protocol::AgentStepToolResult {
                        call_id: "call_start".to_string(),
                        output: serde_json::to_string(&start_output)?,
                    }],
                    token_usage: None,
                    warnings_count: None,
                },
            });

            let inspect_output = serde_json::json!({
                "process": {
                    "argv": ["bash", "-lc", "ls -a"],
                    "cwd": "/repo",
                    "exit_code": 0,
                },
                "stdout_tail": ".\n..\nfile\n",
                "stderr_tail": "",
            });

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(3),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AgentStep {
                    turn_id,
                    step: 1,
                    model: "gpt-test".to_string(),
                    response_id: "resp_2".to_string(),
                    text: None,
                    tool_calls: vec![pm_protocol::AgentStepToolCall {
                        name: "process_inspect".to_string(),
                        call_id: "call_inspect".to_string(),
                        arguments: serde_json::to_string(&serde_json::json!({
                            "process_id": process_id,
                            "max_lines": 200,
                        }))?,
                    }],
                    tool_results: vec![pm_protocol::AgentStepToolResult {
                        call_id: "call_inspect".to_string(),
                        output: serde_json::to_string(&inspect_output)?,
                    }],
                    token_usage: None,
                    warnings_count: None,
                },
            });

            let joined = state
                .transcript
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            assert!(!joined.contains("stdout_path"));
            assert!(!joined.contains("stderr_path"));
            assert!(!joined.contains("process/start"));
            assert!(joined.contains("$ ls -a"));
            assert!(joined.contains("file"));
            Ok(())
        }

        #[test]
        fn process_tail_tool_output_merges_into_started_line() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;
            let process_id = ProcessId::from_str("00000000-0000-0000-0000-0000000000bb")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(1),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::ProcessStarted {
                    process_id,
                    turn_id: Some(turn_id),
                    argv: vec!["bash".to_string(), "-lc".to_string(), "pwd".to_string()],
                    cwd: "/repo".to_string(),
                    stdout_path: "/tmp/stdout.log".to_string(),
                    stderr_path: "/tmp/stderr.log".to_string(),
                },
            });

            let tail_output = serde_json::json!({
                "text": "/repo\n",
            });

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(2),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AgentStep {
                    turn_id,
                    step: 0,
                    model: "gpt-test".to_string(),
                    response_id: "resp_1".to_string(),
                    text: None,
                    tool_calls: vec![pm_protocol::AgentStepToolCall {
                        name: "process_tail".to_string(),
                        call_id: "call_tail".to_string(),
                        arguments: serde_json::to_string(&serde_json::json!({
                            "process_id": process_id,
                            "stream": "stdout",
                            "max_lines": 200,
                        }))?,
                    }],
                    tool_results: vec![pm_protocol::AgentStepToolResult {
                        call_id: "call_tail".to_string(),
                        output: serde_json::to_string(&tail_output)?,
                    }],
                    token_usage: None,
                    warnings_count: None,
                },
            });

            let joined = state
                .transcript
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            assert!(!joined.contains("process/tail"));
            assert!(joined.contains("$ pwd"));
            assert!(joined.contains("/repo"));
            Ok(())
        }

        #[test]
        fn token_usage_counts_final_numeric_when_agent_step_is_redacted() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(1),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AgentStep {
                    turn_id,
                    step: 0,
                    model: "gpt-test".to_string(),
                    response_id: "resp_1".to_string(),
                    text: None,
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    token_usage: Some(serde_json::json!({
                        "input_tokens": "<REDACTED>",
                        "output_tokens": "<REDACTED>",
                        "total_tokens": "<REDACTED>",
                    })),
                    warnings_count: None,
                },
            });
            assert_eq!(state.total_input_tokens_used, 0);
            assert_eq!(state.total_output_tokens_used, 0);
            assert!(!state.token_usage_by_response.contains_key("resp_1"));

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(2),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AssistantMessage {
                    turn_id: Some(turn_id),
                    text: "hello".to_string(),
                    model: Some("gpt-test".to_string()),
                    response_id: Some("resp_1".to_string()),
                    token_usage: Some(serde_json::json!({
                        "input_tokens": 10,
                        "output_tokens": 5,
                        "total_tokens": 15,
                    })),
                },
            });
            assert_eq!(state.total_input_tokens_used, 10);
            assert_eq!(state.total_output_tokens_used, 5);
            assert_eq!(state.total_tokens_used, 15);
            Ok(())
        }

        #[test]
        fn token_usage_patches_cached_tokens_when_details_arrive_late() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(1),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AgentStep {
                    turn_id,
                    step: 0,
                    model: "gpt-test".to_string(),
                    response_id: "resp_1".to_string(),
                    text: None,
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    token_usage: Some(serde_json::json!({
                        "input_tokens": 100,
                        "output_tokens": 10,
                        "total_tokens": 110,
                    })),
                    warnings_count: None,
                },
            });
            assert_eq!(state.total_input_tokens_used, 100);
            assert_eq!(state.total_cache_input_tokens_used, 0);
            assert_eq!(state.total_output_tokens_used, 10);
            assert_eq!(state.total_tokens_used, 110);

            state.apply_event(&ThreadEvent {
                seq: pm_protocol::EventSeq(2),
                timestamp: time::OffsetDateTime::now_utc(),
                thread_id,
                kind: ThreadEventKind::AssistantMessage {
                    turn_id: Some(turn_id),
                    text: "hello".to_string(),
                    model: Some("gpt-test".to_string()),
                    response_id: Some("resp_1".to_string()),
                    token_usage: Some(serde_json::json!({
                        "input_tokens": 100,
                        "input_tokens_details": { "cached_tokens": 80 },
                        "output_tokens": 10,
                        "total_tokens": 110,
                    })),
                },
            });

            assert_eq!(state.total_input_tokens_used, 100);
            assert_eq!(state.total_cache_input_tokens_used, 80);
            assert_eq!(state.total_output_tokens_used, 10);
            assert_eq!(state.total_tokens_used, 110);
            Ok(())
        }
    }
