    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io::{Stdout, Write};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    use futures_util::StreamExt;
    use omne_jsonrpc::ClientHandle;
    use omne_protocol::{
        ApprovalDecision, ApprovalId, ArtifactId, ArtifactMetadata, ModelRoutingRuleSource,
        ProcessId, ThreadEvent, ThreadEventKind, ThreadId, ToolId, ToolStatus, TurnId, TurnStatus,
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
    use ratatui::Terminal;
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
                let params = serde_json::to_value(omne_app_server_protocol::ThreadSubscribeParams {
                    thread_id,
                    since_seq,
                    max_events: Some(10_000),
                    kinds: None,
                    wait_ms: Some(0),
                })
                .context("serialize thread/subscribe params")?;
                let value = handle
                    .request("thread/subscribe", params)
                    .await?;
                serde_json::from_value::<SubscribeResponse>(value)
                    .context("parse thread/subscribe response")
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
                let params = serde_json::to_value(omne_app_server_protocol::ThreadModelsParams {
                    thread_id,
                })
                .context("serialize thread/models params")?;
                let value = handle
                    .request("thread/models", params)
                    .await?;
                let response: omne_app_server_protocol::ThreadModelsResponse =
                    serde_json::from_value(value).context("parse thread/models response")?;
                Ok(response.models)
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
        priority: Option<omne_protocol::TurnPriority>,
    ) -> anyhow::Result<TurnStartInFlight> {
        let (input, context_refs, attachments, directives) = super::split_special_directives(&input)?;
        let input_for_request = input.clone();
        let task = tokio::spawn(async move {
            let params = serde_json::to_value(omne_app_server_protocol::TurnStartParams {
                thread_id,
                input: input_for_request,
                context_refs: Some(context_refs),
                attachments: Some(attachments),
                directives: Some(directives),
                priority,
            })
            .context("serialize turn/start params")?;
            let value = handle
                .request("turn/start", params)
                .await?;
            let response: omne_app_server_protocol::TurnStartResponse =
                serde_json::from_value(value).context("parse turn/start response")?;
            Ok(response.turn_id)
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
        if state.expire_status_if_needed(Instant::now()) {
            changed = true;
        }
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

            if state.subagent_pending_summary_needs_refresh {
                if let Ok(Ok(())) = tokio::time::timeout(
                    header_timeout,
                    state.refresh_subagent_pending_summary(app, thread_id),
                )
                .await
                {
                    changed = true;
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

        let mut terminal = setup_terminal().context("setup terminal")?;
        let _guard = TerminalGuard::enter().context("enter terminal mode")?;
        terminal.clear().context("clear terminal")?;

        let mut state = UiState::new(args.include_archived);
        state.set_status("connecting...".to_string());
        terminal.draw(|f| draw_ui(f, &mut state))?;

        let startup_timeout = std::env::var("OMNE_TUI_STARTUP_TIMEOUT_MS")
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
            state.clear_status();
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
                                if key.kind != crossterm::event::KeyEventKind::Press {
                                    continue;
                                }
                                if state.handle_key(app, key).await? {
                                    return Ok(());
                                }
                                needs_draw = true;
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
                                if key.kind != crossterm::event::KeyEventKind::Press {
                                    continue;
                                }
                                if state.handle_key(app, key).await? {
                                    return Ok(());
                                }
                                needs_draw = true;
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
                            state.last_seq = resp.last_seq;
                            let has_events = !resp.events.is_empty();
                            for event in resp.events {
                                state.apply_event(&event);
                            }
                            if has_events {
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
                let Some(fetch) = state.model_fetch.take() else {
                    continue;
                };
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
                let Some(pending) = state.turn_start.take() else {
                    continue;
                };
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
                            state.input = pending.input;
                        }
                        state.set_status(format!("turn/start error: {err}"));
                        needs_draw = true;
                    }
                    Err(err) => {
                        if state.active_thread == Some(pending.thread_id)
                            && state.input.trim().is_empty()
                        {
                            state.input = pending.input;
                        }
                        state.set_status(format!("turn/start task error: {err}"));
                        needs_draw = true;
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
            stdout.flush().ok();
            Ok(Self)
        }
    }

    impl Drop for TerminalGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let mut stdout = std::io::stdout();
            let _ = stdout.flush();
        }
    }

    fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
        let stdout = std::io::stdout();
        let backend = CrosstermBackend::new(stdout);
        Ok(Terminal::new(backend)?)
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
                let last_seq = resume.last_seq;
                state.activate_thread(thread_id, last_seq);
                if let Ok(value) = app.thread_state(thread_id).await {
                    state.thread_cwd = value
                        .cwd
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string);
                    state.total_tokens_used = value.total_tokens_used;
                }
            }
            None => {
                let started = tokio::time::timeout(timeout, app.thread_start(None))
                    .await
                    .context("thread/start timeout")??;
                let thread_id = started.thread_id;
                let last_seq = started.last_seq;
                state.activate_thread(thread_id, last_seq);
                if let Ok(value) = app.thread_state(thread_id).await {
                    state.thread_cwd = value
                        .cwd
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string);
                    state.total_tokens_used = value.total_tokens_used;
                }
            }
        }
        Ok(())
    }
