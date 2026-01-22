mod tui {
    use std::collections::VecDeque;
    use std::io::{Stdout, Write};
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use crossterm::{execute, ExecutableCommand};
    use futures_util::StreamExt;
    use pm_protocol::{ModelRoutingRuleSource, ThreadEvent, ThreadEventKind, ThreadId, TurnId, TurnStatus};
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
    use ratatui::Terminal;
    use serde::Deserialize;

    pub(super) async fn run_tui(
        app: &mut super::App,
        args: super::TuiArgs,
    ) -> anyhow::Result<()> {
        let mut notifications = app
            .take_notifications()
            .context("notifications already taken")?;

        let mut terminal = setup_terminal().context("setup terminal")?;
        let _guard = TerminalGuard::enter().context("enter terminal mode")?;

        let mut state = UiState::new(args.include_archived);
        state.refresh_threads(app).await?;

        if let Some(thread_id) = args.thread_id {
            state.open_thread(app, thread_id).await?;
        }

        let mut event_stream = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut last_threads_refresh = Instant::now();

        loop {
            terminal.draw(|f| draw_ui(f, &state))?;

            tokio::select! {
                Some(note) = notifications.recv() => {
                    if let Err(err) = state.handle_notification(note) {
                        state.status = Some(format!("notification error: {err}"));
                    }
                }
                Some(Ok(event)) = event_stream.next() => {
                    match event {
                        Event::Key(key) => {
                            if key.kind != crossterm::event::KeyEventKind::Press {
                                continue;
                            }
                            if state.handle_key(app, key).await? {
                                return Ok(());
                            }
                        }
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                }
                _ = tick.tick() => {
                    if state.active_thread.is_some() {
                        if let Err(err) = state.poll_thread(app).await {
                            state.status = Some(format!("poll error: {err}"));
                        }
                    }
                    if state.active_thread.is_none() && last_threads_refresh.elapsed() >= Duration::from_secs(2) {
                        if let Err(err) = state.refresh_threads(app).await {
                            state.status = Some(format!("refresh error: {err}"));
                        } else {
                            last_threads_refresh = Instant::now();
                        }
                    }
                }
            }
        }
    }

    struct TerminalGuard;

    impl TerminalGuard {
        fn enter() -> anyhow::Result<Self> {
            enable_raw_mode()?;
            let mut stdout = std::io::stdout();
            execute!(stdout, EnterAlternateScreen)?;
            stdout.flush().ok();
            Ok(Self)
        }
    }

    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let mut stdout = std::io::stdout();
            let _ = stdout.execute(LeaveAlternateScreen);
            let _ = stdout.flush();
        }
    }

    fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
        let stdout = std::io::stdout();
        let backend = CrosstermBackend::new(stdout);
        Ok(Terminal::new(backend)?)
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ThreadListMetaResponse {
        #[serde(default)]
        threads: Vec<ThreadMeta>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ThreadMeta {
        thread_id: ThreadId,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        attention_state: String,
        #[serde(default)]
        last_seq: u64,
    }

    #[derive(Debug, Clone)]
    struct TranscriptEntry {
        role: TranscriptRole,
        text: String,
    }

    #[derive(Debug, Clone, Copy)]
    enum TranscriptRole {
        User,
        Assistant,
        System,
    }

    #[derive(Debug, Clone)]
    struct StreamingState {
        turn_id: TurnId,
        text: String,
    }

    struct UiState {
        include_archived: bool,
        threads: Vec<ThreadMeta>,
        selected_thread: usize,
        active_thread: Option<ThreadId>,
        last_seq: u64,
        transcript: VecDeque<TranscriptEntry>,
        streaming: Option<StreamingState>,
        active_turn_id: Option<TurnId>,
        input: String,
        status: Option<String>,
    }

    impl UiState {
        fn new(include_archived: bool) -> Self {
            Self {
                include_archived,
                threads: Vec::new(),
                selected_thread: 0,
                active_thread: None,
                last_seq: 0,
                transcript: VecDeque::new(),
                streaming: None,
                active_turn_id: None,
                input: String::new(),
                status: None,
            }
        }

        async fn refresh_threads(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let value = app.thread_list_meta(self.include_archived).await?;
            let parsed =
                serde_json::from_value::<ThreadListMetaResponse>(value).context("parse threads")?;
            self.threads = parsed.threads;
            if self.selected_thread >= self.threads.len() {
                self.selected_thread = self.threads.len().saturating_sub(1);
            }
            Ok(())
        }

        async fn open_thread(
            &mut self,
            app: &mut super::App,
            thread_id: ThreadId,
        ) -> anyhow::Result<()> {
            let resume = app.thread_resume(thread_id).await?;
            let last_seq = resume["last_seq"].as_u64().unwrap_or(0);
            let since_seq = last_seq.saturating_sub(2_000);

            let resp = app
                .thread_subscribe(thread_id, since_seq, Some(2_000), Some(0))
                .await
                .context("subscribe initial events")?;

            self.active_thread = Some(thread_id);
            self.last_seq = resp.last_seq;
            self.transcript.clear();
            self.streaming = None;
            self.active_turn_id = None;
            for event in resp.events {
                self.apply_event(&event);
            }
            Ok(())
        }

        async fn poll_thread(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(thread_id) = self.active_thread else {
                return Ok(());
            };
            let resp = app
                .thread_subscribe(thread_id, self.last_seq, Some(10_000), Some(0))
                .await?;
            self.last_seq = resp.last_seq;
            for event in resp.events {
                self.apply_event(&event);
            }
            Ok(())
        }

        fn apply_event(&mut self, event: &ThreadEvent) {
            match &event.kind {
                ThreadEventKind::TurnStarted { turn_id, input } => {
                    self.active_turn_id = Some(*turn_id);
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::User,
                        text: input.clone(),
                    });
                }
                ThreadEventKind::AssistantMessage { turn_id, text, .. } => {
                    if let Some(turn_id) = turn_id
                        && self.streaming.as_ref().is_some_and(|s| s.turn_id == *turn_id)
                    {
                        self.streaming = None;
                    }
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::Assistant,
                        text: text.clone(),
                    });
                }
                ThreadEventKind::ModelRouted {
                    selected_model,
                    rule_source,
                    reason,
                    ..
                } => {
                    let mut line = format!(
                        "[model] {selected_model} ({})",
                        model_routing_rule_source_str(*rule_source)
                    );
                    if let Some(reason) = reason.as_deref().filter(|s| !s.trim().is_empty()) {
                        line.push_str(&format!(" - {reason}"));
                    }
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::System,
                        text: line,
                    });
                }
                ThreadEventKind::TurnCompleted { turn_id, status, .. } => {
                    if self.active_turn_id == Some(*turn_id) {
                        self.active_turn_id = None;
                    }
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::System,
                        text: format!("[turn] {turn_id} {}", turn_status_str(*status)),
                    });
                }
                _ => {}
            }
        }

        fn push_transcript(&mut self, entry: TranscriptEntry) {
            const MAX_TRANSCRIPT_ITEMS: usize = 500;
            if self.transcript.len() >= MAX_TRANSCRIPT_ITEMS {
                self.transcript.pop_front();
            }
            self.transcript.push_back(entry);
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
                    if self.active_thread == Some(event.thread_id) && event.seq.0 > self.last_seq {
                        self.last_seq = event.seq.0;
                        self.apply_event(&event);
                    }
                }
                _ => {}
            }
            Ok(())
        }

        async fn handle_key(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            self.status = None;
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                if let (Some(thread_id), Some(turn_id)) = (self.active_thread, self.active_turn_id)
                {
                    app.turn_interrupt(thread_id, turn_id, Some("tui ctrl-c".to_string()))
                        .await?;
                    self.status = Some(format!("interrupt requested: {turn_id}"));
                }
                return Ok(false);
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
            match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Esc => {
                    self.active_thread = None;
                    self.input.clear();
                    self.streaming = None;
                    self.active_turn_id = None;
                    self.refresh_threads(app).await?;
                }
                KeyCode::Enter => {
                    let input = self.input.trim().to_string();
                    if input.is_empty() {
                        return Ok(false);
                    }
                    let Some(thread_id) = self.active_thread else {
                        return Ok(false);
                    };
                    self.input.clear();
                    let turn_id = app.turn_start(thread_id, input).await?;
                    self.active_turn_id = Some(turn_id);
                }
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(c) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.input.push(c);
                    }
                }
                _ => {}
            }
            Ok(false)
        }
    }

    fn draw_ui(f: &mut ratatui::Frame, state: &UiState) {
        let outer = Block::default().borders(Borders::ALL).title("pm tui");
        let area = outer.inner(f.area());
        f.render_widget(outer, f.area());

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3), Constraint::Length(1)])
            .split(area);

        match state.active_thread {
            None => draw_thread_list(f, state, layout[0]),
            Some(thread_id) => draw_thread_view(f, state, thread_id, layout[0]),
        }

        draw_input(f, state, layout[1]);
        draw_status(f, state, layout[2]);
    }

    fn draw_thread_list(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let items = state
            .threads
            .iter()
            .map(|t| {
                let cwd = t
                    .cwd
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("-");
                let model = t
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("-");
                let line = format!(
                    "[{}] {}  cwd={}  model={}  last_seq={}",
                    t.attention_state.trim(),
                    t.thread_id,
                    cwd,
                    model,
                    t.last_seq
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Threads (↑↓ Enter=open n=new r=refresh q=quit)"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");

        let selected = if state.threads.is_empty() {
            None
        } else {
            Some(state.selected_thread)
        };
        f.render_stateful_widget(list, area, &mut list_state(selected));
    }

    fn list_state(selected: Option<usize>) -> ratatui::widgets::ListState {
        let mut state = ratatui::widgets::ListState::default();
        state.select(selected);
        state
    }

    fn draw_thread_view(
        f: &mut ratatui::Frame,
        state: &UiState,
        thread_id: ThreadId,
        area: ratatui::layout::Rect,
    ) {
        let mut lines = Vec::<Line>::new();
        for entry in &state.transcript {
            lines.push(format_transcript_entry(entry));
        }
        if let Some(streaming) = &state.streaming {
            lines.push(Line::from(vec![
                Span::styled("assistant: ", Style::default().fg(Color::Green)),
                Span::raw(streaming.text.as_str()),
            ]));
        }

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(
                        "Thread {thread_id} (Esc=back Ctrl-C=interrupt q=quit)"
                    )),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    fn format_transcript_entry(entry: &TranscriptEntry) -> Line<'_> {
        match entry.role {
            TranscriptRole::User => Line::from(vec![
                Span::styled("user: ", Style::default().fg(Color::Yellow)),
                Span::raw(entry.text.as_str()),
            ]),
            TranscriptRole::Assistant => Line::from(vec![
                Span::styled("assistant: ", Style::default().fg(Color::Green)),
                Span::raw(entry.text.as_str()),
            ]),
            TranscriptRole::System => Line::from(vec![
                Span::styled("system: ", Style::default().fg(Color::Cyan)),
                Span::raw(entry.text.as_str()),
            ]),
        }
    }

    fn model_routing_rule_source_str(value: ModelRoutingRuleSource) -> &'static str {
        match value {
            ModelRoutingRuleSource::Subagent => "subagent",
            ModelRoutingRuleSource::ProjectOverride => "project_override",
            ModelRoutingRuleSource::KeywordRule => "keyword_rule",
            ModelRoutingRuleSource::RoleDefault => "role_default",
            ModelRoutingRuleSource::GlobalDefault => "global_default",
        }
    }

    fn turn_status_str(value: TurnStatus) -> &'static str {
        match value {
            TurnStatus::Completed => "completed",
            TurnStatus::Interrupted => "interrupted",
            TurnStatus::Failed => "failed",
            TurnStatus::Cancelled => "cancelled",
            TurnStatus::Stuck => "stuck",
        }
    }

    fn draw_input(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let input = Paragraph::new(state.input.as_str())
            .block(Block::default().borders(Borders::ALL).title("Input"))
            .wrap(Wrap { trim: false });
        f.render_widget(input, area);

        if state.active_thread.is_some() {
            let x = area
                .x
                .saturating_add(1)
                .saturating_add(state.input.len() as u16);
            let y = area.y.saturating_add(1);
            f.set_cursor_position((x.min(area.x + area.width - 2), y));
        }
    }

    fn draw_status(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let msg = match (&state.active_thread, &state.status) {
            (_, Some(status)) => status.clone(),
            (Some(thread_id), None) => format!(
                "connected; thread={thread_id}; last_seq={}",
                state.last_seq
            ),
            (None, None) => "connected".to_string(),
        };
        let style = if state.status.is_some() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Gray)
        };
        let paragraph = Paragraph::new(msg).style(style);
        f.render_widget(paragraph, area);
    }

    #[cfg(test)]
    mod tests {
        use std::str::FromStr;

        use ratatui::backend::TestBackend;

        use super::*;

        fn render_to_string(state: &UiState, width: u16, height: u16) -> anyhow::Result<String> {
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
            state.threads = vec![
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000001")?,
                    cwd: Some("/repo".to_string()),
                    model: Some("gpt-4.1".to_string()),
                    attention_state: "idle".to_string(),
                    last_seq: 1,
                },
                ThreadMeta {
                    thread_id: ThreadId::from_str("00000000-0000-0000-0000-000000000002")?,
                    cwd: Some("/repo".to_string()),
                    model: Some("gpt-4.1-mini".to_string()),
                    attention_state: "running".to_string(),
                    last_seq: 42,
                },
            ];
            state.selected_thread = 1;

            let actual = render_to_string(&state, 64, 12)?;
            let expected = r#"┌pm tui────────────────────────────────────────────────────────┐
│┌Threads (↑↓ Enter=open n=new r=refresh q=quit)──────────────┐│
││  [idle] 00000000-0000-0000-0000-000000000001  cwd=/repo  mo││
││▶ [running] 00000000-0000-0000-0000-000000000002  cwd=/repo ││
││                                                            ││
││                                                            ││
│└────────────────────────────────────────────────────────────┘│
│┌Input───────────────────────────────────────────────────────┐│
││                                                            ││
│└────────────────────────────────────────────────────────────┘│
│connected                                                     │
└──────────────────────────────────────────────────────────────┘"#;
            assert_eq!(actual, expected);
            Ok(())
        }

        #[test]
        fn renders_thread_view_snapshot() -> anyhow::Result<()> {
            let thread_id = ThreadId::from_str("00000000-0000-0000-0000-000000000001")?;
            let turn_id = TurnId::from_str("00000000-0000-0000-0000-0000000000aa")?;

            let mut state = UiState::new(false);
            state.active_thread = Some(thread_id);
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

            let actual = render_to_string(&state, 64, 12)?;
            let expected = r#"┌pm tui────────────────────────────────────────────────────────┐
│┌Thread 00000000-0000-0000-0000-000000000001 (Esc=back Ctrl-C┐│
││system: [model] gpt-4.1 (global_default)                    ││
││user: Hello                                                 ││
││assistant: Hi!                                              ││
││assistant: Streaming...                                     ││
│└────────────────────────────────────────────────────────────┘│
│┌Input───────────────────────────────────────────────────────┐│
││next                                                        ││
│└────────────────────────────────────────────────────────────┘│
│connected; thread=00000000-0000-0000-0000-000000000001; last_s│
└──────────────────────────────────────────────────────────────┘"#;
            assert_eq!(actual, expected);
            Ok(())
        }
    }
}

async fn run_tui(app: &mut App, args: TuiArgs) -> anyhow::Result<()> {
    tui::run_tui(app, args).await
}
