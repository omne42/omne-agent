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
            state.total_tokens_used = 39_280;
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
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
69% context left  threads (Ctrl-K=commands)                     "#;
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
                                                                
69% context left  th=00000000 mode=coder model=gpt-4.1 (Ctrl-K) 
                                                                
                                                                
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
    }
