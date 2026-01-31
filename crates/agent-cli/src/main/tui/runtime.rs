    use std::collections::{HashMap, VecDeque};
    use std::io::{IsTerminal, Stdout, Write};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use crossterm::event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyModifiers, MouseEvent, MouseEventKind,
    };
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    use futures_util::StreamExt;
    use pm_jsonrpc::ClientHandle;
    use pm_protocol::{
        ApprovalDecision, ApprovalId, ArtifactId, ArtifactMetadata, ModelRoutingRuleSource,
        ProcessId, ThreadEvent, ThreadEventKind, ThreadId, ToolId, ToolStatus, TurnId, TurnStatus,
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap, Widget};
    use ratatui::{Terminal, TerminalOptions, Viewport};
    use serde::Deserialize;
    use serde_json::Value;
    use super::SubscribeResponse;
    use time::format_description::well_known::Rfc3339;
    use time::OffsetDateTime;
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    enum PollOutcome {
        Response(SubscribeResponse),
        Timeout,
    }

    struct ModelFetchInFlight {
        thread_id: ThreadId,
        handle: tokio::task::JoinHandle<anyhow::Result<Vec<String>>>,
    }

    struct TurnStartInFlight {
        thread_id: ThreadId,
        input: String,
        handle: tokio::task::JoinHandle<anyhow::Result<TurnId>>,
    }

    struct PollInFlight {
        thread_id: ThreadId,
        handle: tokio::task::JoinHandle<anyhow::Result<PollOutcome>>,
    }

    fn spawn_poll(
        handle: ClientHandle,
        thread_id: ThreadId,
        since_seq: u64,
        poll_timeout: Duration,
    ) -> PollInFlight {
        let task = tokio::spawn(async move {
            let request = async {
                let value = handle
                    .request(
                        "thread/subscribe",
                        serde_json::json!({
                            "thread_id": thread_id,
                            "since_seq": since_seq,
                            "max_events": 10_000,
                            "wait_ms": 0,
                        }),
                    )
                    .await?;
                Ok(serde_json::from_value::<SubscribeResponse>(value)?)
            };

            match tokio::time::timeout(poll_timeout, request).await {
                Ok(Ok(resp)) => Ok(PollOutcome::Response(resp)),
                Ok(Err(err)) => Err(err),
                Err(_) => Ok(PollOutcome::Timeout),
            }
        });

        PollInFlight {
            thread_id,
            handle: task,
        }
    }

    fn spawn_model_fetch(
        handle: ClientHandle,
        thread_id: ThreadId,
        timeout: Duration,
    ) -> ModelFetchInFlight {
        let task = tokio::spawn(async move {
            let request = async {
                let value = handle
                    .request(
                        "thread/models",
                        serde_json::json!({
                            "thread_id": thread_id,
                        }),
                    )
                    .await?;
                let models = value
                    .get("models")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Ok(models)
            };
            match tokio::time::timeout(timeout, request).await {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("thread/models timeout")),
            }
        });

        ModelFetchInFlight { thread_id, handle: task }
    }

    fn spawn_turn_start(
        handle: ClientHandle,
        thread_id: ThreadId,
        input: String,
        priority: Option<pm_protocol::TurnPriority>,
    ) -> anyhow::Result<TurnStartInFlight> {
        let (input, context_refs, attachments) = super::split_special_directives(&input)?;
        let input_for_request = input.clone();
        let task = tokio::spawn(async move {
            let value = handle
                .request(
                    "turn/start",
                    serde_json::json!({
                        "thread_id": thread_id,
                        "input": input_for_request,
                        "context_refs": context_refs,
                        "attachments": attachments,
                        "priority": priority,
                    }),
                )
                .await?;
            serde_json::from_value(value["turn_id"].clone()).context("turn_id missing in result")
        });
        Ok(TurnStartInFlight {
            thread_id,
            input,
            handle: task,
        })
    }

    async fn handle_tick(
        state: &mut UiState,
        app: &mut super::App,
        header_timeout: Duration,
        last_threads_refresh: &mut Instant,
    ) -> bool {
        let mut changed = false;
        if let Some(thread_id) = state.active_thread {
            if state.header_needs_refresh {
                match tokio::time::timeout(header_timeout, state.refresh_header(app, thread_id))
                    .await
                {
                    Ok(Ok(())) => {
                        state.header_needs_refresh = false;
                        changed = true;
                    }
                    Ok(Err(err)) => {
                        state.set_status(format!("header refresh error: {err}"));
                        changed = true;
                    }
                    Err(_) => {}
                }
            }
        }

        if state.active_thread.is_none() && last_threads_refresh.elapsed() >= Duration::from_secs(2)
        {
            if let Err(err) = state.refresh_threads(app).await {
                state.set_status(format!("refresh error: {err}"));
                changed = true;
            } else {
                *last_threads_refresh = Instant::now();
                changed = true;
            }
        }
        changed
    }

    pub(super) async fn run_tui(
        app: &mut super::App,
        args: super::TuiArgs,
    ) -> anyhow::Result<()> {
        let mut notifications = app
            .take_notifications()
            .context("notifications already taken")?;

        let scrollback_enabled = tui_scrollback_enabled();
        let mouse_capture_enabled = tui_mouse_capture_enabled(scrollback_enabled);
        let _guard =
            TerminalGuard::enter(mouse_capture_enabled).context("enter terminal mode")?;
        let mut terminal = setup_terminal(scrollback_enabled).context("setup terminal")?;
        if !scrollback_enabled {
            terminal.clear().context("clear terminal")?;
        }

        let mut state = UiState::new(args.include_archived);
        state.scrollback_enabled = scrollback_enabled;
        state.status = Some("connecting...".to_string());
        if !mouse_capture_enabled {
            flush_transcript_to_scrollback(&mut terminal, &mut state)?;
        }
        terminal.draw(|f| draw_ui(f, &mut state))?;

        let startup_timeout = std::env::var("CODE_PM_TUI_STARTUP_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_secs(5))
            .min(Duration::from_secs(5));
        if let Err(err) =
            initialize_startup(&mut state, app, args.thread_id, startup_timeout).await
        {
            state.set_status(format!("startup error: {err}"));
        } else {
            state.status = None;
        }

        let rpc_handle = app.rpc_handle();
        let mut event_stream = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        let mut last_threads_refresh = Instant::now() - Duration::from_secs(2);
        let header_timeout = Duration::from_millis(500);
        let poll_timeout = Duration::from_secs(1);
        let poll_interval = Duration::from_millis(200);
        let mut last_poll = Instant::now() - poll_interval;
        let mut poll_inflight: Option<PollInFlight> = None;
        let mut last_active_thread: Option<ThreadId> = None;
        let mut needs_draw = true;

        loop {
            if needs_draw {
                if !mouse_capture_enabled {
                    flush_transcript_to_scrollback(&mut terminal, &mut state)?;
                }
                terminal.draw(|f| draw_ui(f, &mut state))?;
                needs_draw = false;
            }

            if state.active_thread != last_active_thread {
                last_active_thread = state.active_thread;
                if let Some(inflight) = poll_inflight.take() {
                    inflight.handle.abort();
                }
                state.cancel_model_fetch();
                last_poll = Instant::now() - poll_interval;
                needs_draw = true;
            }

            let mut poll_result = None::<(
                ThreadId,
                Result<anyhow::Result<PollOutcome>, tokio::task::JoinError>,
            )>;

            if let Some(inflight) = poll_inflight.as_mut() {
                let poll_thread_id = inflight.thread_id;
                tokio::select! {
                    result = &mut inflight.handle => {
                        poll_result = Some((poll_thread_id, result));
                    }
                    Some(note) = notifications.recv() => {
                        if let Err(err) = state.handle_notification(note) {
                            state.set_status(format!("notification error: {err}"));
                        }
                        needs_draw = true;
                    }
                    Some(Ok(event)) = event_stream.next() => {
                        match event {
                            Event::Key(key) => {
                                match key.kind {
                                    crossterm::event::KeyEventKind::Press => {}
                                    crossterm::event::KeyEventKind::Repeat => {
                                        if matches!(key.code, KeyCode::Enter) {
                                            continue;
                                        }
                                    }
                                    _ => continue,
                                }
                                if state.handle_key(app, key).await? {
                                    return Ok(());
                                }
                                needs_draw = true;
                            }
                            Event::Mouse(mouse) => {
                                if state.handle_mouse(mouse) {
                                    needs_draw = true;
                                }
                            }
                            Event::Resize(_, _) => {
                                needs_draw = true;
                            }
                            _ => {}
                        }
                    }
                    _ = tick.tick() => {
                        if handle_tick(&mut state, app, header_timeout, &mut last_threads_refresh).await
                        {
                            needs_draw = true;
                        }
                    }
                }
            } else {
                tokio::select! {
                    Some(note) = notifications.recv() => {
                        if let Err(err) = state.handle_notification(note) {
                            state.set_status(format!("notification error: {err}"));
                        }
                        needs_draw = true;
                    }
                    Some(Ok(event)) = event_stream.next() => {
                        match event {
                            Event::Key(key) => {
                                match key.kind {
                                    crossterm::event::KeyEventKind::Press => {}
                                    crossterm::event::KeyEventKind::Repeat => {
                                        if matches!(key.code, KeyCode::Enter) {
                                            continue;
                                        }
                                    }
                                    _ => continue,
                                }
                                if state.handle_key(app, key).await? {
                                    return Ok(());
                                }
                                needs_draw = true;
                            }
                            Event::Mouse(mouse) => {
                                if state.handle_mouse(mouse) {
                                    needs_draw = true;
                                }
                            }
                            Event::Resize(_, _) => {
                                needs_draw = true;
                            }
                            _ => {}
                        }
                    }
                    _ = tick.tick() => {
                        if handle_tick(&mut state, app, header_timeout, &mut last_threads_refresh).await
                        {
                            needs_draw = true;
                        }
                        if let Some(thread_id) = state.active_thread
                            && last_poll.elapsed() >= poll_interval
                        {
                            poll_inflight = Some(spawn_poll(
                                rpc_handle.clone(),
                                thread_id,
                                state.last_seq,
                                poll_timeout,
                            ));
                            last_poll = Instant::now();
                        }
                    }
                }
            }

            if let Some((thread_id, result)) = poll_result {
                poll_inflight = None;
                match result {
                    Ok(Ok(PollOutcome::Response(resp))) => {
                        if state.active_thread == Some(thread_id) {
                            let mut applied = false;
                            for event in resp.events {
                                if state.apply_live_event(&event) {
                                    applied = true;
                                }
                            }
                            state.last_seq = state.last_seq.max(resp.last_seq);
                            if applied {
                                needs_draw = true;
                            }
                            if resp.has_more {
                                last_poll = Instant::now() - poll_interval;
                            }
                        }
                    }
                    Ok(Ok(PollOutcome::Timeout)) => {}
                    Ok(Err(err)) => {
                        state.set_status(format!("poll error: {err}"));
                        needs_draw = true;
                    }
                    Err(err) => {
                        state.set_status(format!("poll task error: {err}"));
                        needs_draw = true;
                    }
                }
            }

            if state
                .model_fetch
                .as_ref()
                .is_some_and(|fetch| fetch.handle.is_finished())
            {
                let fetch = state.model_fetch.take().expect("checked model fetch");
                let result = fetch.handle.await;
                match result {
                    Ok(Ok(models)) => {
                        if state.active_thread == Some(fetch.thread_id) {
                            state.apply_model_list(models);
                        } else {
                            state.model_fetch_pending = false;
                        }
                        needs_draw = true;
                    }
                    Ok(Err(err)) => {
                        state.model_fetch_pending = false;
                        state.set_status(format!("thread/models error: {err}"));
                        let show_error_palette = matches!(
                            state.overlays.last(),
                            Some(Overlay::CommandPalette(view)) if view.title == "model"
                        );
                        if show_error_palette {
                            state.replace_top_command_palette(build_model_error_palette(
                                &err.to_string(),
                            ));
                        }
                        if let Some(inline) = state.inline_palette.as_mut() {
                            if inline.kind == InlinePaletteKind::Model {
                                inline.view = build_model_error_palette(&err.to_string());
                            }
                        }
                        needs_draw = true;
                    }
                    Err(err) => {
                        state.model_fetch_pending = false;
                        state.set_status(format!("thread/models task error: {err}"));
                        let show_error_palette = matches!(
                            state.overlays.last(),
                            Some(Overlay::CommandPalette(view)) if view.title == "model"
                        );
                        if show_error_palette {
                            state.replace_top_command_palette(build_model_error_palette(
                                &err.to_string(),
                            ));
                        }
                        if let Some(inline) = state.inline_palette.as_mut() {
                            if inline.kind == InlinePaletteKind::Model {
                                inline.view = build_model_error_palette(&err.to_string());
                            }
                        }
                        needs_draw = true;
                    }
                }
            }

            if state
                .turn_start
                .as_ref()
                .is_some_and(|pending| pending.handle.is_finished())
            {
                let pending = state.turn_start.take().expect("checked turn start");
                let result = pending.handle.await;
                match result {
                    Ok(Ok(turn_id)) => {
                        if state.active_thread == Some(pending.thread_id) {
                            state.active_turn_id = Some(turn_id);
                        }
                        needs_draw = true;
                    }
                    Ok(Err(err)) => {
                        if state.active_thread == Some(pending.thread_id)
                            && state.input.trim().is_empty()
                        {
                            state.set_input(pending.input);
                        }
                        state.set_status(format!("turn/start error: {err}"));
                        needs_draw = true;
                    }
                    Err(err) => {
                        if state.active_thread == Some(pending.thread_id)
                            && state.input.trim().is_empty()
                        {
                            state.set_input(pending.input);
                        }
                        state.set_status(format!("turn/start task error: {err}"));
                        needs_draw = true;
                    }
                }
            }
        }
    }

    struct TerminalGuard {
        mouse_capture: bool,
    }

    impl TerminalGuard {
        fn enter(mouse_capture: bool) -> anyhow::Result<Self> {
            enable_raw_mode()?;
            let mut stdout = std::io::stdout();
            if mouse_capture {
                if let Err(err) = crossterm::execute!(stdout, EnableMouseCapture) {
                    let _ = disable_raw_mode();
                    return Err(anyhow::Error::new(err).context("enable mouse capture"));
                }
            }
            stdout.flush().ok();
            Ok(Self { mouse_capture })
        }
    }

    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            let mut stdout = std::io::stdout();
            if self.mouse_capture {
                let _ = crossterm::execute!(stdout, DisableMouseCapture);
            }
            let _ = disable_raw_mode();
            let _ = stdout.flush();
        }
    }

    fn env_bool(key: &str) -> Option<bool> {
        let value = std::env::var(key).ok()?;
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    fn tui_scrollback_enabled() -> bool {
        env_bool("CODE_PM_TUI_SCROLLBACK").unwrap_or_else(|| std::io::stdout().is_terminal())
    }

    fn tui_mouse_capture_enabled(scrollback_enabled: bool) -> bool {
        // When we render with `Viewport::Inline`, prefer terminal-native scrollback by default.
        // Mouse capture blocks scrollback in terminals/tmux, so only enable it when explicitly
        // requested.
        let default = !scrollback_enabled;
        env_bool("CODE_PM_TUI_MOUSE_CAPTURE").unwrap_or(default)
    }

    fn tui_viewport_height(max_height: u16) -> u16 {
        let default = max_height.min(24);
        std::env::var("CODE_PM_TUI_VIEWPORT_HEIGHT")
            .ok()
            .and_then(|value| value.trim().parse::<u16>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(default)
            .min(max_height)
            .max(1)
    }

    fn setup_terminal(
        scrollback_enabled: bool,
    ) -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
        let stdout = std::io::stdout();
        let backend = CrosstermBackend::new(stdout);
        if !scrollback_enabled {
            return Ok(Terminal::new(backend)?);
        }

        let (_cols, rows) = crossterm::terminal::size()?;
        let max_height = rows.max(1);
        let height = tui_viewport_height(max_height);
        Ok(Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(height),
            },
        )?)
    }

    fn flush_transcript_to_scrollback(
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
        state: &mut UiState,
    ) -> anyhow::Result<()> {
        const KEEP_TAIL_ENTRIES: usize = 64;

        if !state.scrollback_enabled {
            return Ok(());
        }
        if state.active_thread.is_none() {
            return Ok(());
        }

        let flush_target = state
            .transcript
            .len()
            .saturating_sub(KEEP_TAIL_ENTRIES);
        if state.transcript_flushed >= flush_target {
            return Ok(());
        }

        let width = terminal.size().context("terminal size")?.width.max(1) as usize;
        let mut lines = Vec::<Line>::new();
        for entry in state
            .transcript
            .iter()
            .skip(state.transcript_flushed)
            .take(flush_target.saturating_sub(state.transcript_flushed))
        {
            lines.extend(format_transcript_entry_lines(entry, width));
        }
        state.transcript_flushed = flush_target;

        if lines.is_empty() {
            return Ok(());
        }

        let height = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let text = Text::from(lines);
        terminal.insert_before(height, move |buf| {
            Paragraph::new(text).render(buf.area, buf);
            scrub_wide_symbol_placeholders(buf);
        })?;
        Ok(())
    }

    fn scrub_wide_symbol_placeholders(buf: &mut ratatui::buffer::Buffer) {
        let width = buf.area.width.max(1) as usize;
        let len = buf.content.len();
        let mut idx = 0usize;
        while idx < len {
            let symbol_width = UnicodeWidthStr::width(buf.content[idx].symbol());
            if symbol_width > 1 {
                let col = idx % width;
                let remaining_in_row = width.saturating_sub(col);
                let to_blank = symbol_width.min(remaining_in_row);
                for offset in 1..to_blank {
                    if let Some(cell) = buf.content.get_mut(idx + offset) {
                        cell.set_symbol("");
                    }
                }
            }
            idx += 1;
        }
    }

    async fn initialize_startup(
        state: &mut UiState,
        app: &mut super::App,
        thread_id: Option<ThreadId>,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        match thread_id {
            Some(thread_id) => {
                let resume = tokio::time::timeout(timeout, app.thread_resume(thread_id))
                    .await
                    .context("thread/resume timeout")??;
                let last_seq = resume["last_seq"].as_u64().unwrap_or(0);
                state.activate_thread(thread_id, last_seq);
                if let Ok(value) = app.thread_state(thread_id).await {
                    state.thread_cwd = value
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    state.total_input_tokens_used = value
                        .get("input_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    state.total_cache_input_tokens_used = value
                        .get("cache_input_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    state.total_output_tokens_used = value
                        .get("output_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    state.total_tokens_used = value
                        .get("total_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if state.total_tokens_used > 0
                        || state.total_input_tokens_used > 0
                        || state.total_output_tokens_used > 0
                        || state.total_cache_input_tokens_used > 0
                    {
                        state.skip_token_usage_before_seq = Some(last_seq);
                    }
                }
            }
            None => {
                let started = tokio::time::timeout(timeout, app.thread_start(None))
                    .await
                    .context("thread/start timeout")??;
                let thread_id: ThreadId = serde_json::from_value(started["thread_id"].clone())
                    .context("thread_id missing")?;
                let last_seq = started["last_seq"].as_u64().unwrap_or(0);
                state.activate_thread(thread_id, last_seq);
                if let Ok(value) = app.thread_state(thread_id).await {
                    state.thread_cwd = value
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    state.total_input_tokens_used = value
                        .get("input_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    state.total_cache_input_tokens_used = value
                        .get("cache_input_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    state.total_output_tokens_used = value
                        .get("output_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    state.total_tokens_used = value
                        .get("total_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if state.total_tokens_used > 0
                        || state.total_input_tokens_used > 0
                        || state.total_output_tokens_used > 0
                        || state.total_cache_input_tokens_used > 0
                    {
                        state.skip_token_usage_before_seq = Some(last_seq);
                    }
                }
            }
        }
        Ok(())
    }
