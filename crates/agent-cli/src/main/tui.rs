mod tui {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io::{Stdout, Write};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
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

        let mut terminal = setup_terminal().context("setup terminal")?;
        let _guard = TerminalGuard::enter().context("enter terminal mode")?;
        terminal.clear().context("clear terminal")?;

        let mut state = UiState::new(args.include_archived);
        state.status = Some("connecting...".to_string());
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
                let last_seq = resume["last_seq"].as_u64().unwrap_or(0);
                state.activate_thread(thread_id, last_seq);
                if let Ok(value) = app.thread_state(thread_id).await {
                    state.thread_cwd = value
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    state.total_tokens_used = value
                        .get("total_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
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
                    state.total_tokens_used = value
                        .get("total_tokens_used")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                }
            }
        }
        Ok(())
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
        created_at: Option<String>,
        #[serde(default)]
        updated_at: Option<String>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        first_message: Option<String>,
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
        Error,
        Tool,
    }

    #[derive(Debug, Clone)]
    struct StreamingState {
        turn_id: TurnId,
        text: String,
    }

    #[derive(Debug, Clone)]
    enum Overlay {
        Approvals(ApprovalsOverlay),
        Processes(ProcessesOverlay),
        Artifacts(ArtifactsOverlay),
        Text(TextOverlay),
        CommandPalette(CommandPaletteOverlay),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InlinePaletteKind {
        Command,
        Role,
        Skill,
        Model,
        ApprovalPolicy,
        SandboxPolicy,
        SandboxNetworkAccess,
    }

    #[derive(Debug, Clone)]
    struct InlinePalette {
        kind: InlinePaletteKind,
        view: CommandPaletteOverlay,
    }

    #[derive(Debug, Clone)]
    struct ApprovalsOverlay {
        thread_id: ThreadId,
        approvals: Vec<ApprovalItem>,
        selected: usize,
        remember: bool,
    }

    #[derive(Debug, Clone)]
    struct ProcessesOverlay {
        thread_id: ThreadId,
        processes: Vec<ProcessInfo>,
        selected: usize,
    }

    #[derive(Debug, Clone)]
    struct ArtifactsOverlay {
        thread_id: ThreadId,
        artifacts: Vec<ArtifactMetadata>,
        selected: usize,
    }

    #[derive(Debug, Clone)]
    struct TextOverlay {
        title: String,
        text: String,
        scroll: u16,
    }

    #[derive(Debug, Clone)]
    struct CommandPaletteOverlay {
        title: String,
        query: String,
        items: Vec<PaletteItem>,
        filtered: Vec<usize>,
        selected: usize,
    }

    #[derive(Debug, Clone)]
    struct PaletteItem {
        label: String,
        detail: Option<String>,
        action: PaletteCommand,
    }

    #[derive(Debug, Clone)]
    enum PaletteCommand {
        Quit,
        OpenRoot,
        Help,
        NewThread,
        ThreadPicker,
        RefreshThreads,
        OpenApprovals,
        OpenProcesses,
        OpenArtifacts,
        PickMode,
        PickModel,
        PickApprovalPolicy,
        PickSandboxPolicy,
        PickSandboxNetworkAccess,
        SetMode(String),
        SetModel(String),
        SetApprovalPolicy(super::CliApprovalPolicy),
        SetSandboxPolicy(super::CliSandboxPolicy),
        SetSandboxNetworkAccess(super::CliSandboxNetworkAccess),
        InsertSkill(String),
        Noop,
    }

    impl PaletteItem {
        fn new(label: impl Into<String>, action: PaletteCommand) -> Self {
            Self {
                label: label.into(),
                detail: None,
                action,
            }
        }

        fn with_detail(
            label: impl Into<String>,
            detail: impl Into<String>,
            action: PaletteCommand,
        ) -> Self {
            Self {
                label: label.into(),
                detail: Some(detail.into()),
                action,
            }
        }
    }

    impl CommandPaletteOverlay {
        fn new(title: impl Into<String>, items: Vec<PaletteItem>) -> Self {
            let mut out = Self {
                title: title.into(),
                query: String::new(),
                items,
                filtered: Vec::new(),
                selected: 0,
            };
            out.rebuild_filter();
            out
        }

        fn rebuild_filter(&mut self) {
            let query = self.query.trim().to_ascii_lowercase();
            let tokens = query
                .split_whitespace()
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>();

            self.filtered = if tokens.is_empty() {
                (0..self.items.len()).collect::<Vec<_>>()
            } else {
                self.items
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, item)| {
                        let hay = match item.detail.as_deref() {
                            Some(detail) if !detail.trim().is_empty() => {
                                format!("{} {}", item.label, detail)
                            }
                            _ => item.label.clone(),
                        };
                        let hay = hay.to_ascii_lowercase();
                        if tokens.iter().all(|token| hay.contains(token)) {
                            Some(idx)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            };

            if self.filtered.is_empty() {
                self.selected = 0;
            } else if self.selected >= self.filtered.len() {
                self.selected = self.filtered.len() - 1;
            }
        }

        fn selected_action(&self) -> Option<PaletteCommand> {
            let idx = *self.filtered.get(self.selected)?;
            self.items.get(idx).map(|item| item.action.clone())
        }
    }

    fn build_root_palette(state: &UiState) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();

        items.push(PaletteItem::with_detail(
            "new",
            "new thread",
            PaletteCommand::NewThread,
        ));
        items.push(PaletteItem::with_detail(
            "threads",
            "thread picker",
            PaletteCommand::ThreadPicker,
        ));
        items.push(PaletteItem::with_detail(
            "refresh",
            "refresh threads",
            PaletteCommand::RefreshThreads,
        ));

        if state.active_thread.is_some() {
            let mode = state.header.mode.as_deref().unwrap_or("-");
            let model = state.header.model.as_deref().unwrap_or("-");

            items.push(PaletteItem::with_detail(
                "approvals",
                "approval list",
                PaletteCommand::OpenApprovals,
            ));
            items.push(PaletteItem::with_detail(
                "processes",
                "process list",
                PaletteCommand::OpenProcesses,
            ));
            items.push(PaletteItem::with_detail(
                "artifacts",
                "artifact list",
                PaletteCommand::OpenArtifacts,
            ));
            items.push(PaletteItem::with_detail(
                format!("mode={}", normalize_label(mode)),
                "select role",
                PaletteCommand::PickMode,
            ));
            items.push(PaletteItem::with_detail(
                format!("model={model}"),
                "select model",
                PaletteCommand::PickModel,
            ));
            items.push(PaletteItem::with_detail(
                "approval-policy",
                "approval policy",
                PaletteCommand::PickApprovalPolicy,
            ));
            items.push(PaletteItem::with_detail(
                "sandbox-policy",
                "sandbox policy",
                PaletteCommand::PickSandboxPolicy,
            ));
            items.push(PaletteItem::with_detail(
                "sandbox-network",
                "network access",
                PaletteCommand::PickSandboxNetworkAccess,
            ));
        }

        items.push(PaletteItem::with_detail(
            "help",
            "show help",
            PaletteCommand::Help,
        ));
        items.push(PaletteItem::with_detail("quit", "quit", PaletteCommand::Quit));

        CommandPaletteOverlay::new("commands", items)
    }

    fn build_mode_palette(modes: Vec<String>, current: Option<&str>) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::OpenRoot));
        for mode in modes {
            let is_current = current.is_some_and(|c| c == mode);
            let label = normalize_label(&mode);
            let label = if is_current { format!("{label}*") } else { label };
            items.push(PaletteItem::new(label, PaletteCommand::SetMode(mode)));
        }
        CommandPaletteOverlay::new("mode", items)
    }

    fn build_model_palette(mut models: Vec<String>, current: Option<&str>) -> CommandPaletteOverlay {
        models.sort();
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::OpenRoot));
        for model in models {
            let is_current = current.is_some_and(|c| c == model);
            let label = if is_current {
                format!("{model}*")
            } else {
                model.clone()
            };
            items.push(PaletteItem::new(label, PaletteCommand::SetModel(model)));
        }
        CommandPaletteOverlay::new("model", items)
    }

    fn build_model_loading_palette() -> CommandPaletteOverlay {
        let items = vec![
            PaletteItem::new("loading models...", PaletteCommand::Noop),
            PaletteItem::new("type model and press enter", PaletteCommand::Noop),
        ];
        CommandPaletteOverlay::new("model", items)
    }

    fn build_model_error_palette(error: &str) -> CommandPaletteOverlay {
        let items = vec![
            PaletteItem::new(format!("error: {error}"), PaletteCommand::Noop),
            PaletteItem::new("retry", PaletteCommand::PickModel),
            PaletteItem::new("type model and press enter", PaletteCommand::Noop),
            PaletteItem::new("back", PaletteCommand::OpenRoot),
        ];
        CommandPaletteOverlay::new("model", items)
    }

    fn build_approval_policy_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::OpenRoot));

        let policies = [
            super::CliApprovalPolicy::AutoApprove,
            super::CliApprovalPolicy::OnRequest,
            super::CliApprovalPolicy::Manual,
            super::CliApprovalPolicy::UnlessTrusted,
            super::CliApprovalPolicy::AutoDeny,
        ];
        for policy in policies {
            items.push(PaletteItem::new(
                format!("approval-policy={}", approval_policy_label(policy)),
                PaletteCommand::SetApprovalPolicy(policy),
            ));
        }
        CommandPaletteOverlay::new("approval-policy", items)
    }

    fn build_sandbox_policy_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::OpenRoot));

        let policies = [
            super::CliSandboxPolicy::ReadOnly,
            super::CliSandboxPolicy::WorkspaceWrite,
            super::CliSandboxPolicy::DangerFullAccess,
        ];
        for policy in policies {
            items.push(PaletteItem::new(
                format!("sandbox-policy={}", sandbox_policy_label(policy)),
                PaletteCommand::SetSandboxPolicy(policy),
            ));
        }
        CommandPaletteOverlay::new("sandbox-policy", items)
    }

    fn build_sandbox_network_access_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::OpenRoot));

        let values = [
            super::CliSandboxNetworkAccess::Deny,
            super::CliSandboxNetworkAccess::Allow,
        ];
        for value in values {
            items.push(PaletteItem::new(
                format!(
                    "sandbox-network={}",
                    sandbox_network_access_label(value)
                ),
                PaletteCommand::SetSandboxNetworkAccess(value),
            ));
        }
        CommandPaletteOverlay::new("sandbox-network", items)
    }

    fn build_inline_command_palette(active_thread: bool) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::with_detail(
            "/new",
            "new thread",
            PaletteCommand::NewThread,
        ));
        items.push(PaletteItem::with_detail(
            "/threads",
            "thread picker",
            PaletteCommand::ThreadPicker,
        ));
        items.push(PaletteItem::with_detail(
            "/refresh",
            "refresh threads",
            PaletteCommand::RefreshThreads,
        ));
        if active_thread {
            items.push(PaletteItem::with_detail(
                "/approvals",
                "approval list",
                PaletteCommand::OpenApprovals,
            ));
            items.push(PaletteItem::with_detail(
                "/processes",
                "process list",
                PaletteCommand::OpenProcesses,
            ));
            items.push(PaletteItem::with_detail(
                "/artifacts",
                "artifact list",
                PaletteCommand::OpenArtifacts,
            ));
            items.push(PaletteItem::with_detail(
                "/mode",
                "select role",
                PaletteCommand::PickMode,
            ));
            items.push(PaletteItem::with_detail(
                "/model",
                "select model",
                PaletteCommand::PickModel,
            ));
            items.push(PaletteItem::with_detail(
                "/approval-policy",
                "approval policy",
                PaletteCommand::PickApprovalPolicy,
            ));
            items.push(PaletteItem::with_detail(
                "/sandbox-policy",
                "sandbox policy",
                PaletteCommand::PickSandboxPolicy,
            ));
            items.push(PaletteItem::with_detail(
                "/sandbox-network",
                "network access",
                PaletteCommand::PickSandboxNetworkAccess,
            ));
        }
        items.push(PaletteItem::with_detail(
            "/help",
            "show help",
            PaletteCommand::Help,
        ));
        items.push(PaletteItem::with_detail("/quit", "quit", PaletteCommand::Quit));
        CommandPaletteOverlay::new("commands", items)
    }

    fn build_inline_role_palette(
        mut modes: Vec<String>,
        current: Option<&str>,
    ) -> CommandPaletteOverlay {
        modes.sort();
        let mut items = Vec::<PaletteItem>::new();
        for mode in modes {
            let is_current = current.is_some_and(|c| c == mode);
            let label = normalize_label(&mode);
            let label = if is_current { format!("{label}*") } else { label };
            items.push(PaletteItem::with_detail(
                format!("@{label}"),
                "role",
                PaletteCommand::SetMode(mode),
            ));
        }
        CommandPaletteOverlay::new("roles", items)
    }

    fn build_inline_skill_palette(mut skills: Vec<String>) -> CommandPaletteOverlay {
        skills.sort();
        let mut items = Vec::<PaletteItem>::new();
        for skill in skills {
            let label = normalize_label(&skill);
            items.push(PaletteItem::with_detail(
                format!("${label}"),
                "skill",
                PaletteCommand::InsertSkill(skill),
            ));
        }
        CommandPaletteOverlay::new("skills", items)
    }

    fn build_inline_model_palette(
        mut models: Vec<String>,
        current: Option<&str>,
    ) -> CommandPaletteOverlay {
        models.sort();
        let mut items = Vec::<PaletteItem>::new();
        for model in models {
            let is_current = current.is_some_and(|c| c == model);
            let label = if is_current {
                format!("{model}*")
            } else {
                model.clone()
            };
            items.push(PaletteItem::new(label, PaletteCommand::SetModel(model)));
        }
        CommandPaletteOverlay::new("model", items)
    }

    fn build_inline_approval_policy_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        let policies = [
            super::CliApprovalPolicy::AutoApprove,
            super::CliApprovalPolicy::OnRequest,
            super::CliApprovalPolicy::Manual,
            super::CliApprovalPolicy::UnlessTrusted,
            super::CliApprovalPolicy::AutoDeny,
        ];
        for policy in policies {
            items.push(PaletteItem::new(
                format!("approval-policy={}", approval_policy_label(policy)),
                PaletteCommand::SetApprovalPolicy(policy),
            ));
        }
        CommandPaletteOverlay::new("approval-policy", items)
    }

    fn build_inline_sandbox_policy_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        let policies = [
            super::CliSandboxPolicy::ReadOnly,
            super::CliSandboxPolicy::WorkspaceWrite,
            super::CliSandboxPolicy::DangerFullAccess,
        ];
        for policy in policies {
            items.push(PaletteItem::new(
                format!("sandbox-policy={}", sandbox_policy_label(policy)),
                PaletteCommand::SetSandboxPolicy(policy),
            ));
        }
        CommandPaletteOverlay::new("sandbox-policy", items)
    }

    fn build_inline_sandbox_network_access_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        let values = [
            super::CliSandboxNetworkAccess::Deny,
            super::CliSandboxNetworkAccess::Allow,
        ];
        for value in values {
            items.push(PaletteItem::new(
                format!("sandbox-network={}", sandbox_network_access_label(value)),
                PaletteCommand::SetSandboxNetworkAccess(value),
            ));
        }
        CommandPaletteOverlay::new("sandbox-network", items)
    }

    fn approval_policy_label(value: super::CliApprovalPolicy) -> &'static str {
        match value {
            super::CliApprovalPolicy::AutoApprove => "auto_approve",
            super::CliApprovalPolicy::OnRequest => "on_request",
            super::CliApprovalPolicy::Manual => "manual",
            super::CliApprovalPolicy::UnlessTrusted => "unless_trusted",
            super::CliApprovalPolicy::AutoDeny => "auto_deny",
        }
    }

    fn sandbox_policy_label(value: super::CliSandboxPolicy) -> &'static str {
        match value {
            super::CliSandboxPolicy::ReadOnly => "read_only",
            super::CliSandboxPolicy::WorkspaceWrite => "workspace_write",
            super::CliSandboxPolicy::DangerFullAccess => "danger_full_access",
        }
    }

    fn sandbox_network_access_label(value: super::CliSandboxNetworkAccess) -> &'static str {
        match value {
            super::CliSandboxNetworkAccess::Deny => "deny",
            super::CliSandboxNetworkAccess::Allow => "allow",
        }
    }

    fn normalize_label(value: &str) -> String {
        value
            .trim()
            .to_ascii_lowercase()
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    fn tui_help_text() -> String {
        let mut out = String::new();
        out.push_str("keys:\n\n");
        out.push_str("  Ctrl-K           command palette\n");
        out.push_str("  /                command palette\n");
        out.push_str("  Ctrl-Q           quit\n");
        out.push_str("  Esc/Ctrl-C       clear input / quit when empty\n\n");
        out.push_str("thread view:\n\n");
        out.push_str("  Enter            send input\n");
        out.push_str("  Ctrl/Cmd-Enter   newline\n");
        out.push_str("  Tab              cycle thinking intensity\n");
        out.push_str("  @                roles\n");
        out.push_str("  $                skills\n");
        out.push_str("  ↑/↓              scroll transcript (menu closed)\n");
        out.push_str("  Ctrl+↑/↓         scroll transcript (menu open)\n");
        out.push_str("  PageUp/PageDown  scroll transcript (page)\n");
        out.push_str("  Home/End         top/bottom + follow\n");
        out.push_str("  Ctrl-A           approvals overlay\n");
        out.push_str("  Ctrl-P           processes overlay\n");
        out.push_str("  Ctrl-O           artifacts overlay\n\n");
        out.push_str("thread picker:\n\n");
        out.push_str("  n                new thread\n");
        out.push_str("  r                refresh\n");
        out.push_str("  ↑/↓ Enter        open\n");
        out
    }

    #[derive(Debug, Clone)]
    enum PendingAction {
        ArtifactList {
            thread_id: ThreadId,
            approval_id: ApprovalId,
        },
        ArtifactRead {
            thread_id: ThreadId,
            artifact_id: ArtifactId,
            max_bytes: u64,
            approval_id: ApprovalId,
        },
        ProcessInspect {
            thread_id: ThreadId,
            process_id: ProcessId,
            max_lines: usize,
            approval_id: ApprovalId,
        },
        ProcessKill {
            thread_id: ThreadId,
            process_id: ProcessId,
            approval_id: ApprovalId,
        },
        ProcessInterrupt {
            thread_id: ThreadId,
            process_id: ProcessId,
            approval_id: ApprovalId,
        },
    }

    impl PendingAction {
        fn approval_id(&self) -> ApprovalId {
            match self {
                Self::ArtifactList { approval_id, .. } => *approval_id,
                Self::ArtifactRead { approval_id, .. } => *approval_id,
                Self::ProcessInspect { approval_id, .. } => *approval_id,
                Self::ProcessKill { approval_id, .. } => *approval_id,
                Self::ProcessInterrupt { approval_id, .. } => *approval_id,
            }
        }
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ApprovalListResponse {
        #[serde(default)]
        approvals: Vec<ApprovalItem>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ApprovalItem {
        request: ApprovalRequestInfo,
        #[serde(default)]
        decision: Option<ApprovalDecisionInfo>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ApprovalRequestInfo {
        approval_id: ApprovalId,
        #[serde(default)]
        turn_id: Option<TurnId>,
        action: String,
        #[serde(default)]
        params: Value,
        requested_at: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ApprovalDecisionInfo {
        decision: ApprovalDecision,
        #[serde(default)]
        remember: bool,
        #[serde(default)]
        reason: Option<String>,
        decided_at: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ProcessListResponse {
        #[serde(default)]
        processes: Vec<ProcessInfo>,
    }

    #[derive(Debug, Clone, Copy, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum ProcessStatus {
        Running,
        Exited,
        Abandoned,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ProcessInfo {
        process_id: ProcessId,
        thread_id: ThreadId,
        #[serde(default)]
        turn_id: Option<TurnId>,
        argv: Vec<String>,
        cwd: String,
        started_at: String,
        status: ProcessStatus,
        #[serde(default)]
        exit_code: Option<i32>,
        stdout_path: String,
        stderr_path: String,
        last_update_at: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ProcessInspectResponse {
        process: ProcessInfo,
        #[serde(default)]
        stdout_tail: String,
        #[serde(default)]
        stderr_tail: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ArtifactListResponse {
        #[serde(default)]
        artifacts: Vec<ArtifactMetadata>,
        #[serde(default)]
        errors: Vec<Value>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ArtifactReadResponse {
        metadata: ArtifactMetadata,
        #[serde(default)]
        text: String,
        #[serde(default)]
        truncated: bool,
        #[serde(default)]
        bytes: u64,
    }

    #[derive(Debug, Clone, Default)]
    struct HeaderState {
        mode: Option<String>,
        provider: Option<String>,
        model: Option<String>,
        thinking: Option<String>,
        mcp_enabled: bool,
        model_context_window: Option<u64>,
    }

    fn env_truthy(key: &str) -> bool {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
    }

    struct UiState {
        include_archived: bool,
        threads: Vec<ThreadMeta>,
        selected_thread: usize,
        active_thread: Option<ThreadId>,
        header: HeaderState,
        header_needs_refresh: bool,
        overlays: Vec<Overlay>,
        inline_palette: Option<InlinePalette>,
        last_seq: u64,
        transcript: VecDeque<TranscriptEntry>,
        transcript_scroll: u16,
        transcript_follow: bool,
        transcript_max_scroll: u16,
        transcript_viewport_height: u16,
        tool_events: HashMap<ToolId, String>,
        streaming: Option<StreamingState>,
        active_turn_id: Option<TurnId>,
        input: String,
        status: Option<String>,
        total_tokens_used: u64,
        counted_usage_responses: HashSet<String>,
        skip_token_usage_before_seq: Option<u64>,
        pending_action: Option<PendingAction>,
        model_fetch: Option<ModelFetchInFlight>,
        model_fetch_pending: bool,
        model_list: Vec<String>,
        model_list_loaded: bool,
        thread_cwd: Option<String>,
        mode_catalog: Vec<String>,
        mode_catalog_loaded: bool,
        skill_catalog: Vec<String>,
        skill_catalog_loaded: bool,
        turn_start: Option<TurnStartInFlight>,
    }

    impl UiState {
        fn new(include_archived: bool) -> Self {
            Self {
                include_archived,
                threads: Vec::new(),
                selected_thread: 0,
                active_thread: None,
                header: HeaderState::default(),
                header_needs_refresh: false,
                overlays: Vec::new(),
                inline_palette: None,
                last_seq: 0,
                transcript: VecDeque::new(),
                transcript_scroll: 0,
                transcript_follow: true,
                transcript_max_scroll: 0,
                transcript_viewport_height: 0,
                tool_events: HashMap::new(),
                streaming: None,
                active_turn_id: None,
                input: String::new(),
                status: None,
                total_tokens_used: 0,
                counted_usage_responses: HashSet::new(),
                skip_token_usage_before_seq: None,
                pending_action: None,
                model_fetch: None,
                model_fetch_pending: false,
                model_list: Vec::new(),
                model_list_loaded: false,
                thread_cwd: None,
                mode_catalog: Vec::new(),
                mode_catalog_loaded: false,
                skill_catalog: Vec::new(),
                skill_catalog_loaded: false,
                turn_start: None,
            }
        }

        fn activate_thread(&mut self, thread_id: ThreadId, last_seq: u64) {
            let since_seq = last_seq.saturating_sub(2_000);
            self.reset_thread_state(thread_id, since_seq);
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

            self.reset_thread_state(thread_id, resp.last_seq);
            if let Ok(state) = app.thread_state(thread_id).await {
                self.thread_cwd = state
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                if let Some(tokens) = state.get("total_tokens_used").and_then(|v| v.as_u64()) {
                    self.total_tokens_used = tokens;
                    self.skip_token_usage_before_seq = Some(resp.last_seq);
                }
            }
            for event in resp.events {
                self.apply_event(&event);
            }
            if let Err(err) = self.refresh_header(app, thread_id).await {
                self.set_status(format!("header refresh error: {err}"));
            } else {
                self.header_needs_refresh = false;
            }
            Ok(())
        }

        fn reset_thread_state(&mut self, thread_id: ThreadId, last_seq: u64) {
            self.active_thread = Some(thread_id);
            self.header = HeaderState::default();
            self.header_needs_refresh = true;
            self.overlays.clear();
            self.inline_palette = None;
            self.last_seq = last_seq;
            self.transcript.clear();
            self.transcript_scroll = 0;
            self.transcript_follow = true;
            self.transcript_max_scroll = 0;
            self.transcript_viewport_height = 0;
            self.tool_events.clear();
            self.streaming = None;
            self.active_turn_id = None;
            self.total_tokens_used = 0;
            self.counted_usage_responses.clear();
            self.skip_token_usage_before_seq = None;
            self.pending_action = None;
            self.cancel_model_fetch();
            self.cancel_turn_start();
            self.model_list.clear();
            self.model_list_loaded = false;
            self.thread_cwd = None;
            self.mode_catalog.clear();
            self.mode_catalog_loaded = false;
            self.skill_catalog.clear();
            self.skill_catalog_loaded = false;
        }

        async fn refresh_header(
            &mut self,
            app: &mut super::App,
            thread_id: ThreadId,
        ) -> anyhow::Result<()> {
            let config = app.thread_config_explain(thread_id).await?;

            self.header.mode = config
                .get("effective")
                .and_then(|v| v.get("mode"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            self.header.provider = Self::extract_openai_provider(&config);
            self.header.model = config
                .get("effective")
                .and_then(|v| v.get("model"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            self.header.thinking = config
                .get("effective")
                .and_then(|v| v.get("thinking"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            self.header.model_context_window = config
                .get("effective")
                .and_then(|v| v.get("model_context_window"))
                .and_then(|v| v.as_u64());

            self.header.mcp_enabled = env_truthy("CODE_PM_ENABLE_MCP");
            Ok(())
        }

        fn extract_openai_provider(config: &Value) -> Option<String> {
            let layers = config.get("layers")?.as_array()?;
            for layer in layers.iter().rev() {
                let provider = layer.get("openai_provider").and_then(|v| v.as_str());
                if let Some(provider) = provider.map(str::trim).filter(|s| !s.is_empty()) {
                    return Some(provider.to_string());
                }
            }
            None
        }

        fn apply_event(&mut self, event: &ThreadEvent) {
            match &event.kind {
                ThreadEventKind::TurnStarted { turn_id, input, .. } => {
                    self.cancel_turn_start();
                    self.active_turn_id = Some(*turn_id);
                    self.push_transcript(TranscriptEntry {
                        role: TranscriptRole::User,
                        text: input.clone(),
                    });
                }
                ThreadEventKind::AssistantMessage {
                    turn_id,
                    text,
                    response_id,
                    token_usage,
                    ..
                } => {
                    self.record_token_usage(
                        response_id.as_deref(),
                        token_usage.as_ref(),
                        event.seq.0,
                    );
                    let mut streamed = None::<String>;
                    if let Some(turn_id) = turn_id
                        && self.streaming.as_ref().is_some_and(|s| s.turn_id == *turn_id)
                    {
                        streamed = self.streaming.as_ref().map(|s| s.text.clone());
                        self.streaming = None;
                    }
                    if let Some(streamed) = streamed {
                        let streamed_trim = streamed.trim();
                        let final_trim = text.trim();
                        if !streamed_trim.is_empty()
                            && (final_trim.is_empty() || !final_trim.starts_with(streamed_trim))
                        {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Assistant,
                                text: streamed,
                            });
                        }
                    }
                    if !text.trim().is_empty() {
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Assistant,
                            text: text.clone(),
                        });
                    }
                }
                ThreadEventKind::AgentStep {
                    response_id,
                    token_usage,
                    ..
                } => {
                    self.record_token_usage(
                        Some(response_id.as_str()),
                        token_usage.as_ref(),
                        event.seq.0,
                    );
                }
                ThreadEventKind::ModelRouted {
                    selected_model,
                    rule_source,
                    reason,
                    ..
                } => {
                    self.header.model = Some(selected_model.clone());
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
                ThreadEventKind::ToolStarted { tool_id, tool, params, .. } => {
                    self.tool_events.insert(*tool_id, tool.clone());
                    if !should_suppress_tool_started(tool) {
                        if let Some(line) = format_tool_started_line(tool, params.as_ref()) {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Tool,
                                text: line,
                            });
                        }
                    }
                }
                ThreadEventKind::ToolCompleted {
                    tool_id,
                    status,
                    error,
                    result,
                    ..
                } => {
                    let info = self.tool_events.remove(tool_id);
                    let name = info.as_deref().unwrap_or("tool");
                    if *status == ToolStatus::Completed {
                        if should_suppress_tool_completed(name)
                            || should_suppress_tool_started(name)
                        {
                            return;
                        }
                        if let Some(result) = result.as_ref().filter(|v| !v.is_null())
                            && let Some(line) = format_tool_result_line(name, result)
                        {
                            self.push_transcript(TranscriptEntry {
                                role: TranscriptRole::Tool,
                                text: line,
                            });
                        }
                    } else {
                        let mut line = format!("{name} {}", tool_status_str(*status));
                        if let Some(err) = error.as_deref().filter(|s| !s.trim().is_empty()) {
                            line.push_str(": ");
                            line.push_str(err);
                        }
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Error,
                            text: line,
                        });
                    }
                }
                ThreadEventKind::ProcessStarted { argv, cwd, .. } => {
                    if let Some(line) =
                        format_process_started_line(argv, cwd, self.thread_cwd.as_deref())
                    {
                        self.push_transcript(TranscriptEntry {
                            role: TranscriptRole::Tool,
                            text: line,
                        });
                    }
                }
                ThreadEventKind::ThreadConfigUpdated {
                    mode,
                    model,
                    thinking,
                    ..
                } => {
                    if let Some(mode) = mode.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.mode = Some(mode.to_string());
                    }
                    if let Some(model) = model.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.model = Some(model.to_string());
                    }
                    if let Some(thinking) = thinking.as_deref().filter(|s| !s.trim().is_empty())
                    {
                        self.header.thinking = Some(thinking.to_string());
                    }
                    self.header_needs_refresh = true;
                }
                _ => {}
            }
        }

        fn push_transcript(&mut self, entry: TranscriptEntry) {
            const MAX_TRANSCRIPT_ITEMS: usize = 5000;
            if self.transcript.len() >= MAX_TRANSCRIPT_ITEMS {
                self.transcript.pop_front();
            }
            self.transcript.push_back(entry);
        }

        fn record_token_usage(
            &mut self,
            response_id: Option<&str>,
            usage: Option<&Value>,
            event_seq: u64,
        ) {
            let Some(tokens) = usage.and_then(usage_total_tokens) else {
                return;
            };
            if let Some(skip_before) = self.skip_token_usage_before_seq {
                if event_seq <= skip_before {
                    return;
                }
            }
            if let Some(response_id) = response_id.filter(|s| !s.trim().is_empty()) {
                if !self.counted_usage_responses.insert(response_id.to_string()) {
                    return;
                }
            }
            self.total_tokens_used = self.total_tokens_used.saturating_add(tokens);
        }

        fn set_status(&mut self, msg: String) {
            if Self::is_error_message(&msg) {
                self.report_error(msg);
            } else {
                self.status = Some(msg);
            }
        }

        fn report_error(&mut self, msg: String) {
            self.status = Some(msg.clone());
            if self.active_thread.is_some() {
                self.push_transcript(TranscriptEntry {
                    role: TranscriptRole::Error,
                    text: msg,
                });
            }
        }

        fn cancel_model_fetch(&mut self) {
            if let Some(fetch) = self.model_fetch.take() {
                fetch.handle.abort();
            }
            self.model_fetch_pending = false;
        }

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
                .get("mode_catalog")
                .and_then(|v| v.get("modes"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
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
            if let Ok(dir) = std::env::var("CODE_PM_SKILLS_DIR") {
                let dir = dir.trim();
                if !dir.is_empty() {
                    roots.push(PathBuf::from(dir));
                }
            }
            if let Some(thread_cwd) = self.thread_cwd.as_deref() {
                let root = PathBuf::from(thread_cwd);
                roots.push(root.join(".codepm_data").join("spec").join("skills"));
                roots.push(root.join(".codex").join("skills"));
            }
            if let Some(home) = home_dir() {
                roots.push(home.join(".codepm_data").join("spec").join("skills"));
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
            }

            Ok(())
        }

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
                    if self.active_thread == Some(event.thread_id) && event.seq.0 > self.last_seq {
                        self.last_seq = event.seq.0;
                        self.apply_event(&event);
                    }
                }
                _ => {}
            }
            Ok(())
        }

        async fn handle_key_overlay(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            let mut status = None::<String>;
            let mut decided = None::<ApprovalId>;
            let mut set_pending_action = None::<PendingAction>;
            let mut palette_command = None::<PaletteCommand>;
            let op;

            let close_with_q =
                key.code == KeyCode::Char('q')
                    && !matches!(self.overlays.last(), Some(Overlay::CommandPalette(_)));
            let closing_palette = matches!(self.overlays.last(), Some(Overlay::CommandPalette(_)));
            if key.code == KeyCode::Esc || close_with_q {
                if closing_palette {
                    self.cancel_model_fetch();
                }
                self.overlays.pop();
                return Ok(false);
            }

            {
                let Some(overlay) = self.overlays.last_mut() else {
                    return Ok(false);
                };
                match overlay {
                    Overlay::Approvals(view) => {
                        (op, status, decided) =
                            handle_key_approvals_overlay(app, key, view).await?;
                    }
                    Overlay::Processes(view) => {
                        (op, status, set_pending_action) =
                            handle_key_processes_overlay(app, key, view).await?;
                    }
                    Overlay::Artifacts(view) => {
                        (op, status, set_pending_action) =
                            handle_key_artifacts_overlay(app, key, view).await?;
                    }
                    Overlay::Text(view) => {
                        op = handle_key_text_overlay(key, view);
                    }
                    Overlay::CommandPalette(view) => {
                        palette_command = handle_key_command_palette(key, view);
                        op = OverlayOp::None;
                    }
                }
            }

            if let Some(msg) = status {
                self.set_status(msg);
            }

            if let Some(pending) = set_pending_action {
                self.pending_action = Some(pending);
            }

            if let Some(approval_id) = decided {
                if self
                    .pending_action
                    .as_ref()
                    .is_some_and(|pending| pending.approval_id() == approval_id)
                {
                    self.resume_pending_action(app).await?;
                }
            }

            match op {
                OverlayOp::None => {}
                OverlayOp::Push(overlay) => {
                    self.overlays.push(overlay);
                }
            }

            if let Some(command) = palette_command {
                return self.execute_palette_command(app, command).await;
            }

            Ok(false)
        }

        async fn handle_key(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            if let Some(status) = self.status.take() {
                if Self::is_error_message(&status) {
                    self.status = Some(status);
                }
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                let mut cleared_input = false;
                if self.active_thread.is_some() && !self.input.trim().is_empty() {
                    self.input.clear();
                    self.transcript_follow = true;
                    self.update_inline_palette(app).await?;
                    cleared_input = true;
                }
                if !self.overlays.is_empty() {
                    let closing_palette =
                        matches!(self.overlays.last(), Some(Overlay::CommandPalette(_)));
                    if closing_palette {
                        self.cancel_model_fetch();
                    }
                    self.overlays.pop();
                    return Ok(false);
                }
                if self.active_thread.is_some() {
                    if cleared_input {
                        return Ok(false);
                    }
                    return Ok(true);
                }
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
                if self.active_thread.is_some() {
                    self.insert_command_trigger();
                    self.update_inline_palette(app).await?;
                } else {
                    self.toggle_command_palette();
                }
                return Ok(false);
            }
            if self.active_thread.is_none()
                && self.overlays.is_empty()
                && !key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('/')
            {
                self.toggle_command_palette();
                return Ok(false);
            }

            if !self.overlays.is_empty() {
                return self.handle_key_overlay(app, key).await;
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
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('a') => {
                        self.open_approvals_overlay(app).await?;
                        return Ok(false);
                    }
                    KeyCode::Char('p') => {
                        self.open_processes_overlay(app).await?;
                        return Ok(false);
                    }
                    KeyCode::Char('o') => {
                        self.open_artifacts_overlay(app).await?;
                        return Ok(false);
                    }
                    _ => {}
                }
            }

            let inline_active = self.inline_palette.is_some();
            let mut input_changed = false;

            match key.code {
                KeyCode::Tab if key.modifiers.is_empty() => {
                    self.cycle_thinking(app).await?;
                }
                KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                }
                KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                    if self.transcript_scroll >= self.transcript_max_scroll {
                        self.transcript_scroll = self.transcript_max_scroll;
                        self.transcript_follow = true;
                    }
                }
                KeyCode::Up => {
                    if inline_active {
                        self.move_inline_selection(-1);
                    } else {
                        self.transcript_follow = false;
                        self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if inline_active {
                        self.move_inline_selection(1);
                    } else {
                        self.transcript_follow = false;
                        self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                        if self.transcript_scroll >= self.transcript_max_scroll {
                            self.transcript_scroll = self.transcript_max_scroll;
                            self.transcript_follow = true;
                        }
                    }
                }
                KeyCode::PageUp => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self
                        .transcript_scroll
                        .saturating_sub(self.transcript_page());
                }
                KeyCode::PageDown => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self
                        .transcript_scroll
                        .saturating_add(self.transcript_page());
                    if self.transcript_scroll >= self.transcript_max_scroll {
                        self.transcript_scroll = self.transcript_max_scroll;
                        self.transcript_follow = true;
                    }
                }
                KeyCode::Home => {
                    self.transcript_follow = false;
                    self.transcript_scroll = 0;
                }
                KeyCode::End => {
                    self.transcript_scroll = self.transcript_max_scroll;
                    self.transcript_follow = true;
                }
                KeyCode::Esc => {
                    if !self.input.trim().is_empty() {
                        self.input.clear();
                        self.transcript_follow = true;
                        input_changed = true;
                    } else {
                        return Ok(true);
                    }
                }
                KeyCode::Enter
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    self.input.push('\n');
                    input_changed = true;
                }
                KeyCode::Enter => {
                    if inline_active {
                        if let Some(command) = self.inline_selected_action() {
                            return self.execute_inline_command(app, command).await;
                        }
                        if let Some(inline) = self.inline_palette.as_ref() {
                            if inline.kind == InlinePaletteKind::Model {
                                let query = inline.view.query.trim();
                                if !query.is_empty() {
                                    return self
                                        .execute_inline_command(
                                            app,
                                            PaletteCommand::SetModel(query.to_string()),
                                        )
                                        .await;
                                }
                            }
                        }
                        return Ok(false);
                    }
                    let input = self.input.clone();
                    if input.trim().is_empty() {
                        return Ok(false);
                    }
                    let Some(thread_id) = self.active_thread else {
                        return Ok(false);
                    };
                    if self.turn_start.is_some() {
                        self.set_status("turn/start pending".to_string());
                        return Ok(false);
                    }
                    let rpc_handle = app.rpc_handle();
                    let pending = match spawn_turn_start(rpc_handle, thread_id, input, None) {
                        Ok(pending) => pending,
                        Err(err) => {
                            self.set_status(format!("turn/start error: {err}"));
                            return Ok(false);
                        }
                    };
                    self.input.clear();
                    self.turn_start = Some(pending);
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    input_changed = true;
                }
                KeyCode::Char(c) => {
                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                        self.input.push(c);
                        input_changed = true;
                    }
                }
                _ => {}
            }
            if input_changed {
                self.transcript_follow = true;
                self.update_inline_palette(app).await?;
            }
            Ok(false)
        }

        async fn return_to_thread_picker(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            self.active_thread = None;
            self.header = HeaderState::default();
            self.header_needs_refresh = false;
            self.overlays.clear();
            self.inline_palette = None;
            self.input.clear();
            self.streaming = None;
            self.active_turn_id = None;
            self.total_tokens_used = 0;
            self.counted_usage_responses.clear();
            self.skip_token_usage_before_seq = None;
            self.pending_action = None;
            self.cancel_turn_start();
            self.transcript_scroll = 0;
            self.transcript_follow = true;
            self.transcript_max_scroll = 0;
            self.transcript_viewport_height = 0;
            self.model_list.clear();
            self.model_list_loaded = false;
            self.thread_cwd = None;
            self.mode_catalog.clear();
            self.mode_catalog_loaded = false;
            self.skill_catalog.clear();
            self.skill_catalog_loaded = false;
            self.refresh_threads(app).await?;
            Ok(())
        }

        fn toggle_command_palette(&mut self) {
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                self.cancel_model_fetch();
                self.overlays.pop();
                return;
            }

            self.transcript_follow = true;
            self.overlays
                .push(Overlay::CommandPalette(build_root_palette(self)));
        }

        fn replace_top_command_palette(&mut self, palette: CommandPaletteOverlay) {
            match self.overlays.last_mut() {
                Some(Overlay::CommandPalette(view)) => {
                    *view = palette;
                }
                _ => {
                    self.overlays.push(Overlay::CommandPalette(palette));
                }
            }
        }

        async fn execute_palette_command(
            &mut self,
            app: &mut super::App,
            command: PaletteCommand,
        ) -> anyhow::Result<bool> {
            match command {
                PaletteCommand::Quit => return Ok(true),
                PaletteCommand::Noop => {}
                PaletteCommand::OpenRoot => {
                    self.replace_top_command_palette(build_root_palette(self));
                }
                PaletteCommand::Help => {
                    self.overlays.pop();
                    self.overlays.push(Overlay::Text(TextOverlay {
                        title: "Help".to_string(),
                        text: tui_help_text(),
                        scroll: 0,
                    }));
                }
                PaletteCommand::NewThread => {
                    let started = match app.thread_start(None).await {
                        Ok(v) => v,
                        Err(err) => {
                            self.set_status(format!("thread/start error: {err}"));
                            return Ok(false);
                        }
                    };
                    let thread_id: ThreadId =
                        serde_json::from_value(started["thread_id"].clone()).context("thread_id missing")?;
                    self.open_thread(app, thread_id).await?;
                }
                PaletteCommand::ThreadPicker => {
                    self.return_to_thread_picker(app).await?;
                }
                PaletteCommand::RefreshThreads => {
                    if let Err(err) = self.refresh_threads(app).await {
                        self.set_status(format!("refresh error: {err}"));
                    } else {
                        self.set_status("refreshed".to_string());
                    }
                }
                PaletteCommand::OpenApprovals => {
                    self.overlays.pop();
                    self.open_approvals_overlay(app).await?;
                }
                PaletteCommand::OpenProcesses => {
                    self.overlays.pop();
                    self.open_processes_overlay(app).await?;
                }
                PaletteCommand::OpenArtifacts => {
                    self.overlays.pop();
                    self.open_artifacts_overlay(app).await?;
                }
                PaletteCommand::PickMode => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    let config = match app.thread_config_explain(thread_id).await {
                        Ok(v) => v,
                        Err(err) => {
                            self.set_status(format!("thread/config/explain error: {err}"));
                            return Ok(false);
                        }
                    };
                    let modes = config
                        .get("mode_catalog")
                        .and_then(|v| v.get("modes"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    self.replace_top_command_palette(build_mode_palette(
                        modes,
                        self.header.mode.as_deref(),
                    ));
                }
                PaletteCommand::PickModel => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    if self.model_fetch.is_some() {
                        self.set_status("model list already loading".to_string());
                        return Ok(false);
                    }
                    let rpc_handle = app.rpc_handle();
                    self.model_fetch_pending = true;
                    self.replace_top_command_palette(build_model_loading_palette());
                    self.model_fetch = Some(spawn_model_fetch(
                        rpc_handle,
                        thread_id,
                        Duration::from_secs(2),
                    ));
                    self.set_status("loading models...".to_string());
                }
                PaletteCommand::PickApprovalPolicy => {
                    self.replace_top_command_palette(build_approval_policy_palette());
                }
                PaletteCommand::PickSandboxPolicy => {
                    self.replace_top_command_palette(build_sandbox_policy_palette());
                }
                PaletteCommand::PickSandboxNetworkAccess => {
                    self.replace_top_command_palette(build_sandbox_network_access_palette());
                }
                PaletteCommand::SetMode(mode) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: Some(mode.clone()),
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set mode error: {err}"));
                    } else {
                        self.set_status(format!("mode={mode}"));
                        let _ = self.refresh_header(app, thread_id).await;
                    }
                }
                PaletteCommand::SetModel(model) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
                            model: Some(model.clone()),
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set model error: {err}"));
                    } else {
                        self.set_status(format!("model={model}"));
                        let _ = self.refresh_header(app, thread_id).await;
                    }
                }
                PaletteCommand::SetApprovalPolicy(approval_policy) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: Some(approval_policy),
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set approval_policy error: {err}"));
                    } else {
                        self.set_status(format!(
                            "approval_policy={}",
                            approval_policy_label(approval_policy)
                        ));
                    }
                }
                PaletteCommand::SetSandboxPolicy(sandbox_policy) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: Some(sandbox_policy),
                            sandbox_writable_roots: None,
                            sandbox_network_access: None,
                            mode: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set sandbox_policy error: {err}"));
                    } else {
                        self.set_status(format!(
                            "sandbox_policy={}",
                            sandbox_policy_label(sandbox_policy)
                        ));
                    }
                }
                PaletteCommand::SetSandboxNetworkAccess(sandbox_network_access) => {
                    let Some(thread_id) = self.active_thread else {
                        self.set_status("no active thread".to_string());
                        return Ok(false);
                    };
                    self.overlays.pop();
                    if let Err(err) = app
                        .thread_configure(super::ThreadConfigureArgs {
                            thread_id,
                            approval_policy: None,
                            sandbox_policy: None,
                            sandbox_writable_roots: None,
                            sandbox_network_access: Some(sandbox_network_access),
                            mode: None,
                            model: None,
                            openai_base_url: None,
                            thinking: None,
                        })
                        .await
                    {
                        self.set_status(format!("set sandbox_network_access error: {err}"));
                    } else {
                        self.set_status(format!(
                            "sandbox_network_access={}",
                            sandbox_network_access_label(sandbox_network_access)
                        ));
                    }
                }
                PaletteCommand::InsertSkill(_) => {}
            }

            Ok(false)
        }

        fn transcript_page(&self) -> u16 {
            self.transcript_viewport_height.saturating_sub(1).max(1)
        }
    }

    #[derive(Debug)]
    enum OverlayOp {
        None,
        Push(Overlay),
    }

    fn approval_decision_str(value: ApprovalDecision) -> &'static str {
        match value {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Denied => "denied",
        }
    }

    async fn load_approvals(
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<ApprovalItem>> {
        let value = app.approval_list(thread_id, false).await?;
        let parsed = serde_json::from_value::<ApprovalListResponse>(value)
            .context("parse approval/list response")?;
        Ok(parsed.approvals)
    }

    async fn load_processes(
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<ProcessInfo>> {
        let value = app.process_list(Some(thread_id)).await?;
        let parsed = serde_json::from_value::<ProcessListResponse>(value)
            .context("parse process/list response")?;
        Ok(parsed.processes)
    }

    fn select_approval(items: &[ApprovalItem], current: Option<ApprovalId>) -> usize {
        let Some(current) = current else {
            return 0;
        };
        items
            .iter()
            .position(|item| item.request.approval_id == current)
            .unwrap_or(0)
    }

    async fn handle_key_approvals_overlay(
        app: &mut super::App,
        key: KeyEvent,
        view: &mut ApprovalsOverlay,
    ) -> anyhow::Result<(OverlayOp, Option<String>, Option<ApprovalId>)> {
        let mut status = None::<String>;
        let mut decided = None::<ApprovalId>;

        match key.code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if !view.approvals.is_empty() {
                    view.selected = (view.selected + 1).min(view.approvals.len() - 1);
                }
            }
            KeyCode::Char('r') => {
                let current = view
                    .approvals
                    .get(view.selected)
                    .map(|item| item.request.approval_id);
                view.approvals = load_approvals(app, view.thread_id).await?;
                view.selected = select_approval(&view.approvals, current);
            }
            KeyCode::Char('m') => {
                view.remember = !view.remember;
                status = Some(format!("remember={}", view.remember));
            }
            KeyCode::Char('y') | KeyCode::Char('n') => {
                let Some(item) = view.approvals.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                if item.decision.is_some() {
                    return Ok((OverlayOp::None, None, None));
                }
                let approval_id = item.request.approval_id;
                let decision = if key.code == KeyCode::Char('y') {
                    ApprovalDecision::Approved
                } else {
                    ApprovalDecision::Denied
                };
                app.approval_decide(view.thread_id, approval_id, decision, view.remember, None)
                    .await?;
                view.approvals = load_approvals(app, view.thread_id).await?;
                view.selected = select_approval(&view.approvals, Some(approval_id));
                status = Some(format!(
                    "approval decided: {approval_id} {}",
                    approval_decision_str(decision)
                ));
                decided = Some(approval_id);
            }
            KeyCode::Enter => {
                let Some(item) = view.approvals.get(view.selected) else {
                    return Ok((OverlayOp::None, None, None));
                };
                let mut text = String::new();
                text.push_str(&format!(
                    "approval_id: {}\nrequested_at: {}\naction: {}\n",
                    item.request.approval_id, item.request.requested_at, item.request.action
                ));
                if let Some(turn_id) = item.request.turn_id {
                    text.push_str(&format!("turn_id: {turn_id}\n"));
                }
                if let Some(decision) = &item.decision {
                    text.push_str(&format!(
                        "\n# Decision\n\ndecision: {}\ndecided_at: {}\nremember: {}\n",
                        approval_decision_str(decision.decision),
                        decision.decided_at,
                        decision.remember
                    ));
                    if let Some(reason) = decision.reason.as_deref().filter(|s| !s.trim().is_empty())
                    {
                        text.push_str(&format!("reason: {reason}\n"));
                    }
                }
                text.push_str("\n# Params\n\n");
                text.push_str(
                    &serde_json::to_string_pretty(&item.request.params)
                        .unwrap_or_else(|_| item.request.params.to_string()),
                );
                return Ok((
                    OverlayOp::Push(Overlay::Text(TextOverlay {
                        title: "Approval details".to_string(),
                        text,
                        scroll: 0,
                    })),
                    None,
                    None,
                ));
            }
            _ => {}
        }

        Ok((OverlayOp::None, status, decided))
    }

    async fn handle_key_processes_overlay(
        app: &mut super::App,
        key: KeyEvent,
        view: &mut ProcessesOverlay,
    ) -> anyhow::Result<(OverlayOp, Option<String>, Option<PendingAction>)> {
        let mut status = None::<String>;

        match key.code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if !view.processes.is_empty() {
                    view.selected = (view.selected + 1).min(view.processes.len() - 1);
                }
            }
            KeyCode::Char('r') => {
                let current = view
                    .processes
                    .get(view.selected)
                    .map(|process| process.process_id);
                view.processes = load_processes(app, view.thread_id).await?;
                view.selected = select_process(&view.processes, current);
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                let Some(info) = view.processes.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let max_lines = 200usize;
                let value = app
                    .rpc(
                        "process/inspect",
                        serde_json::json!({
                            "process_id": info.process_id,
                            "max_lines": max_lines,
                            "approval_id": null,
                        }),
                    )
                    .await?;

                if let Some((thread_id, approval_id)) = parse_needs_approval(&value) {
                    let approvals = load_approvals(app, thread_id).await?;
                    let selected = select_approval(&approvals, Some(approval_id));
                    let overlay = Overlay::Approvals(ApprovalsOverlay {
                        thread_id,
                        approvals,
                        selected,
                        remember: false,
                    });
                    status = Some(format!("process/inspect needs approval: {approval_id}"));
                    return Ok((
                        OverlayOp::Push(overlay),
                        status,
                        Some(PendingAction::ProcessInspect {
                            thread_id,
                            process_id: info.process_id,
                            max_lines,
                            approval_id,
                        }),
                    ));
                }

                if is_denied(&value) {
                    status = Some(format!(
                        "process/inspect denied: {}",
                        summarize_json(&value)
                    ));
                    return Ok((OverlayOp::None, status, None));
                }

                let parsed = serde_json::from_value::<ProcessInspectResponse>(value)
                    .context("parse process/inspect response")?;
                let text = build_process_inspect_text(&parsed);
                return Ok((
                    OverlayOp::Push(Overlay::Text(TextOverlay {
                        title: format!("Process {}", parsed.process.process_id),
                        text,
                        scroll: 0,
                    })),
                    None,
                    None,
                ));
            }
            KeyCode::Char('k') => {
                let Some(info) = view.processes.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let value = app
                    .rpc(
                        "process/kill",
                        serde_json::json!({
                            "process_id": info.process_id,
                            "reason": "tui kill",
                            "approval_id": null,
                        }),
                    )
                    .await?;

                if let Some((thread_id, approval_id)) = parse_needs_approval(&value) {
                    let approvals = load_approvals(app, thread_id).await?;
                    let selected = select_approval(&approvals, Some(approval_id));
                    let overlay = Overlay::Approvals(ApprovalsOverlay {
                        thread_id,
                        approvals,
                        selected,
                        remember: false,
                    });
                    status = Some(format!("process/kill needs approval: {approval_id}"));
                    return Ok((
                        OverlayOp::Push(overlay),
                        status,
                        Some(PendingAction::ProcessKill {
                            thread_id,
                            process_id: info.process_id,
                            approval_id,
                        }),
                    ));
                }

                if is_denied(&value) {
                    status =
                        Some(format!("process/kill denied: {}", summarize_json(&value)));
                    return Ok((OverlayOp::None, status, None));
                }

                status = Some(format!("kill requested: {}", info.process_id));
                view.processes = load_processes(app, view.thread_id).await?;
                view.selected = select_process(&view.processes, Some(info.process_id));
            }
            KeyCode::Char('x') => {
                let Some(info) = view.processes.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let value = app
                    .rpc(
                        "process/interrupt",
                        serde_json::json!({
                            "process_id": info.process_id,
                            "reason": "tui interrupt",
                            "approval_id": null,
                        }),
                    )
                    .await?;

                if let Some((thread_id, approval_id)) = parse_needs_approval(&value) {
                    let approvals = load_approvals(app, thread_id).await?;
                    let selected = select_approval(&approvals, Some(approval_id));
                    let overlay = Overlay::Approvals(ApprovalsOverlay {
                        thread_id,
                        approvals,
                        selected,
                        remember: false,
                    });
                    status = Some(format!(
                        "process/interrupt needs approval: {approval_id}"
                    ));
                    return Ok((
                        OverlayOp::Push(overlay),
                        status,
                        Some(PendingAction::ProcessInterrupt {
                            thread_id,
                            process_id: info.process_id,
                            approval_id,
                        }),
                    ));
                }

                if is_denied(&value) {
                    status = Some(format!(
                        "process/interrupt denied: {}",
                        summarize_json(&value)
                    ));
                    return Ok((OverlayOp::None, status, None));
                }

                status = Some(format!("interrupt requested: {}", info.process_id));
                view.processes = load_processes(app, view.thread_id).await?;
                view.selected = select_process(&view.processes, Some(info.process_id));
            }
            _ => {}
        }

        Ok((OverlayOp::None, status, None))
    }

    async fn handle_key_artifacts_overlay(
        app: &mut super::App,
        key: KeyEvent,
        view: &mut ArtifactsOverlay,
    ) -> anyhow::Result<(OverlayOp, Option<String>, Option<PendingAction>)> {
        let mut status = None::<String>;

        match key.code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if !view.artifacts.is_empty() {
                    view.selected = (view.selected + 1).min(view.artifacts.len() - 1);
                }
            }
            KeyCode::Char('r') => {
                let current = view
                    .artifacts
                    .get(view.selected)
                    .map(|artifact| artifact.artifact_id);

                let value = app
                    .rpc(
                        "artifact/list",
                        serde_json::json!({
                            "thread_id": view.thread_id,
                            "approval_id": null,
                        }),
                    )
                    .await?;

                if let Some((thread_id, approval_id)) = parse_needs_approval(&value) {
                    let approvals = load_approvals(app, thread_id).await?;
                    let selected = select_approval(&approvals, Some(approval_id));
                    let overlay = Overlay::Approvals(ApprovalsOverlay {
                        thread_id,
                        approvals,
                        selected,
                        remember: false,
                    });
                    status = Some(format!("artifact/list needs approval: {approval_id}"));
                    return Ok((
                        OverlayOp::Push(overlay),
                        status,
                        Some(PendingAction::ArtifactList {
                            thread_id,
                            approval_id,
                        }),
                    ));
                }

                if is_denied(&value) {
                    status = Some(format!("artifact/list denied: {}", summarize_json(&value)));
                    return Ok((OverlayOp::None, status, None));
                }

                let parsed = serde_json::from_value::<ArtifactListResponse>(value)
                    .context("parse artifact/list response")?;
                if !parsed.errors.is_empty() {
                    status = Some(format!("artifact/list errors: {}", parsed.errors.len()));
                }
                view.artifacts = parsed.artifacts;
                view.selected = select_artifact(&view.artifacts, current);
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                let Some(meta) = view.artifacts.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let max_bytes = 256 * 1024u64;
                let value = app
                    .rpc(
                        "artifact/read",
                        serde_json::json!({
                            "thread_id": view.thread_id,
                            "artifact_id": meta.artifact_id,
                            "max_bytes": max_bytes,
                            "approval_id": null,
                        }),
                    )
                    .await?;

                if let Some((thread_id, approval_id)) = parse_needs_approval(&value) {
                    let approvals = load_approvals(app, thread_id).await?;
                    let selected = select_approval(&approvals, Some(approval_id));
                    let overlay = Overlay::Approvals(ApprovalsOverlay {
                        thread_id,
                        approvals,
                        selected,
                        remember: false,
                    });
                    status = Some(format!("artifact/read needs approval: {approval_id}"));
                    return Ok((
                        OverlayOp::Push(overlay),
                        status,
                        Some(PendingAction::ArtifactRead {
                            thread_id,
                            artifact_id: meta.artifact_id,
                            max_bytes,
                            approval_id,
                        }),
                    ));
                }

                if is_denied(&value) {
                    status = Some(format!("artifact/read denied: {}", summarize_json(&value)));
                    return Ok((OverlayOp::None, status, None));
                }

                let parsed = serde_json::from_value::<ArtifactReadResponse>(value)
                    .context("parse artifact/read response")?;
                let text = build_artifact_read_text(&parsed);
                return Ok((
                    OverlayOp::Push(Overlay::Text(TextOverlay {
                        title: format!("Artifact {}", parsed.metadata.artifact_id),
                        text,
                        scroll: 0,
                    })),
                    None,
                    None,
                ));
            }
            _ => {}
        }

        Ok((OverlayOp::None, status, None))
    }

    fn select_process(items: &[ProcessInfo], current: Option<ProcessId>) -> usize {
        let Some(current) = current else {
            return 0;
        };
        items
            .iter()
            .position(|item| item.process_id == current)
            .unwrap_or(0)
    }

    fn select_artifact(items: &[ArtifactMetadata], current: Option<ArtifactId>) -> usize {
        let Some(current) = current else {
            return 0;
        };
        items
            .iter()
            .position(|item| item.artifact_id == current)
            .unwrap_or(0)
    }

    fn parse_needs_approval(value: &Value) -> Option<(ThreadId, ApprovalId)> {
        if !value
            .get("needs_approval")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return None;
        }
        let thread_id = serde_json::from_value::<ThreadId>(value.get("thread_id")?.clone()).ok()?;
        let approval_id =
            serde_json::from_value::<ApprovalId>(value.get("approval_id")?.clone()).ok()?;
        Some((thread_id, approval_id))
    }

    fn is_denied(value: &Value) -> bool {
        value.get("denied").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    fn summarize_json(value: &Value) -> String {
        let rendered = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
        let rendered = rendered.trim().to_string();
        let mut chars = rendered.chars();
        let prefix: String = chars.by_ref().take(280).collect();
        if chars.next().is_some() {
            format!("{prefix}…")
        } else {
            prefix
        }
    }

    fn summarize_tool_kv(value: &Value) -> String {
        const MAX_ITEMS: usize = 6;
        match value {
            Value::Object(map) => {
                let mut parts = Vec::new();
                for (key, value) in map {
                    if let Some(summary) = summarize_tool_value(value) {
                        if !summary.trim().is_empty() {
                            parts.push(format!("{key}={summary}"));
                        }
                    }
                    if parts.len() >= MAX_ITEMS {
                        break;
                    }
                }
                if parts.is_empty() {
                    String::new()
                } else {
                    format!("[{}]", parts.join(", "))
                }
            }
            _ => summarize_tool_value(value).unwrap_or_default(),
        }
    }

    fn summarize_tool_value(value: &Value) -> Option<String> {
        match value {
            Value::Null => None,
            Value::Bool(value) => Some(value.to_string()),
            Value::Number(value) => Some(value.to_string()),
            Value::String(value) => {
                let normalized = normalize_single_line(value);
                if normalized.is_empty() {
                    None
                } else {
                    Some(truncate_tool_text(&normalized))
                }
            }
            Value::Array(value) => Some(format!("[{}]", value.len())),
            Value::Object(value) => Some(format!("{{{}}}", value.len())),
        }
    }

    fn truncate_tool_text(value: &str) -> String {
        const MAX_CHARS: usize = 120;
        if value.chars().count() <= MAX_CHARS {
            return value.to_string();
        }
        let mut out: String = value.chars().take(MAX_CHARS).collect();
        out.push('…');
        out
    }

    fn should_suppress_tool_started(tool: &str) -> bool {
        matches!(
            tool,
            "process/start" | "process/inspect" | "process/tail" | "process/follow"
        )
    }

    fn should_suppress_tool_completed(tool: &str) -> bool {
        matches!(tool, "process/inspect" | "process/tail" | "process/follow")
    }

    fn format_tool_started_line(tool: &str, params: Option<&Value>) -> Option<String> {
        let line = match tool {
            "file/read" => format_file_action("read", params),
            "file/write" => format_file_action("write", params),
            "file/edit" => format_file_action("edit", params),
            "file/patch" => format_file_action("patch", params),
            "file/delete" => format_file_action("delete", params),
            "file/glob" => format_file_glob(params),
            "file/grep" => format_file_grep(params),
            "repo/search" => format_repo_search(params),
            "repo/index" => format_repo_index(params),
            "repo/symbols" => format_repo_symbols(params),
            "fs/mkdir" => format_fs_mkdir(params),
            "process/kill" => format_process_kill(params),
            "process/interrupt" => format_process_interrupt(params),
            "artifact/list" => Some("artifact list".to_string()),
            "artifact/read" => format_artifact_read(params),
            "artifact/write" => format_artifact_write(params),
            "artifact/delete" => format_artifact_delete(params),
            "mcp/list_servers" => Some("mcp list servers".to_string()),
            "mcp/list_tools" => format_mcp_list_tools(params),
            "mcp/list_resources" => format_mcp_list_resources(params),
            "mcp/call" => format_mcp_call(params),
            "subagent/spawn" => Some("subagent spawn".to_string()),
            _ => None,
        };
        if let Some(line) = line {
            return Some(line);
        }
        let summary = params.map(summarize_tool_kv).unwrap_or_default();
        if summary.trim().is_empty() {
            Some(tool.to_string())
        } else {
            Some(format!("{tool} {summary}"))
        }
    }

    fn format_tool_result_line(tool: &str, result: &Value) -> Option<String> {
        if should_suppress_tool_completed(tool) || tool == "process/start" {
            return None;
        }
        let summary = summarize_tool_kv(result);
        if summary.trim().is_empty() {
            None
        } else {
            Some(format!("{tool} → {summary}"))
        }
    }

    fn format_process_started_line(
        argv: &[String],
        cwd: &str,
        thread_cwd: Option<&str>,
    ) -> Option<String> {
        let cmd = extract_shell_command(argv).unwrap_or_else(|| format_argv(argv));
        if cmd.is_empty() {
            return None;
        }
        let mut line = format!("$ {cmd}");
        if let Some(cwd_display) = format_cwd_display(cwd, thread_cwd) {
            line.push_str(&format!(" (cwd={cwd_display})"));
        }
        Some(line)
    }

    fn format_argv(argv: &[String]) -> String {
        argv.iter()
            .map(|arg| format_shell_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn extract_shell_command(argv: &[String]) -> Option<String> {
        if argv.len() >= 3 && is_shell_wrapper(&argv[0]) && argv[1] == "-lc" {
            let cmd = argv[2].trim();
            return if cmd.is_empty() {
                None
            } else {
                Some(cmd.to_string())
            };
        }
        if argv.len() >= 4
            && argv[0] == "/usr/bin/env"
            && is_shell_wrapper(&argv[1])
            && argv[2] == "-lc"
        {
            let cmd = argv[3].trim();
            return if cmd.is_empty() {
                None
            } else {
                Some(cmd.to_string())
            };
        }
        None
    }

    fn is_shell_wrapper(cmd: &str) -> bool {
        matches!(
            cmd,
            "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/sh" | "/bin/zsh"
        )
    }

    fn format_shell_arg(value: &str) -> String {
        if value.is_empty() {
            return "\"\"".to_string();
        }
        let safe = value.chars().all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '/' | ':' ));
        if safe {
            value.to_string()
        } else {
            format!("{value:?}")
        }
    }

    fn format_cwd_display(cwd: &str, thread_cwd: Option<&str>) -> Option<String> {
        let trimmed = cwd.trim();
        if trimmed.is_empty() || trimmed == "." {
            return None;
        }
        if let Some(thread_cwd) = thread_cwd.map(str::trim).filter(|s| !s.is_empty()) {
            if trimmed == thread_cwd {
                return None;
            }
        }
        Some(trimmed.to_string())
    }

    fn format_file_action(action: &str, params: Option<&Value>) -> Option<String> {
        let path = param_str(params, "path")?;
        let root = param_str(params, "root");
        let path = format_path_with_root(root, path);
        Some(format!("{action} {path}"))
    }

    fn format_file_glob(params: Option<&Value>) -> Option<String> {
        let pattern = param_str(params, "pattern")?;
        let root = root_tag(param_str(params, "root"));
        let mut line = format!("glob \"{pattern}\"");
        if let Some(root) = root {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_file_grep(params: Option<&Value>) -> Option<String> {
        let query = param_str(params, "query")?;
        let mut line = format!("grep \"{query}\"");
        if let Some(include) = param_str(params, "include_glob") {
            line.push_str(&format!(" in {include}"));
        }
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_repo_search(params: Option<&Value>) -> Option<String> {
        let query = param_str(params, "query")?;
        let mut line = format!("search \"{query}\"");
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_repo_index(params: Option<&Value>) -> Option<String> {
        let mut line = "repo index".to_string();
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_repo_symbols(params: Option<&Value>) -> Option<String> {
        let mut line = "repo symbols".to_string();
        if let Some(root) = root_tag(param_str(params, "root")) {
            line.push_str(&format!(" ({root})"));
        }
        Some(line)
    }

    fn format_fs_mkdir(params: Option<&Value>) -> Option<String> {
        let path = param_str(params, "path")?;
        let recursive = param_bool(params, "recursive").unwrap_or(false);
        if recursive {
            Some(format!("mkdir -p {path}"))
        } else {
            Some(format!("mkdir {path}"))
        }
    }

    fn format_process_kill(params: Option<&Value>) -> Option<String> {
        let process_id = param_str(params, "process_id")?;
        Some(format!("kill {process_id}"))
    }

    fn format_process_interrupt(params: Option<&Value>) -> Option<String> {
        let process_id = param_str(params, "process_id")?;
        Some(format!("interrupt {process_id}"))
    }

    fn format_artifact_read(params: Option<&Value>) -> Option<String> {
        let artifact_id = param_str(params, "artifact_id")?;
        Some(format!("artifact read {artifact_id}"))
    }

    fn format_artifact_write(params: Option<&Value>) -> Option<String> {
        let mut line = "artifact write".to_string();
        if let Some(artifact_type) = param_str(params, "artifact_type") {
            line.push(' ');
            line.push_str(artifact_type);
        }
        if let Some(summary) = param_str(params, "summary") {
            let summary = truncate_tool_text(summary);
            if !summary.is_empty() {
                line.push_str(&format!(" \"{summary}\""));
            }
        }
        Some(line)
    }

    fn format_artifact_delete(params: Option<&Value>) -> Option<String> {
        let artifact_id = param_str(params, "artifact_id")?;
        Some(format!("artifact delete {artifact_id}"))
    }

    fn format_mcp_list_tools(params: Option<&Value>) -> Option<String> {
        let server = param_str(params, "server")?;
        Some(format!("mcp list_tools {server}"))
    }

    fn format_mcp_list_resources(params: Option<&Value>) -> Option<String> {
        let server = param_str(params, "server")?;
        Some(format!("mcp list_resources {server}"))
    }

    fn format_mcp_call(params: Option<&Value>) -> Option<String> {
        let server = param_str(params, "server").unwrap_or("-");
        let tool = param_str(params, "tool").unwrap_or("-");
        Some(format!("mcp {server}.{tool}"))
    }

    fn param_str<'a>(params: Option<&'a Value>, key: &str) -> Option<&'a str> {
        params
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn param_bool(params: Option<&Value>, key: &str) -> Option<bool> {
        params.and_then(|value| value.get(key)).and_then(Value::as_bool)
    }

    fn root_tag(root: Option<&str>) -> Option<&'static str> {
        if matches!(root, Some("reference")) {
            Some("ref")
        } else {
            None
        }
    }

    fn format_path_with_root(root: Option<&str>, path: &str) -> String {
        if matches!(root, Some("reference")) {
            format!("ref:{path}")
        } else {
            path.to_string()
        }
    }

    struct InlineContext {
        kind: InlinePaletteKind,
        query: String,
    }

    fn last_line_bounds(input: &str) -> (usize, &str) {
        match input.rfind('\n') {
            Some(idx) => (idx + 1, &input[idx + 1..]),
            None => (0, input),
        }
    }

    fn parse_inline_context(input: &str) -> Option<InlineContext> {
        let ends_with_whitespace = input.chars().last().is_some_and(char::is_whitespace);
        let (_line_start, line) = last_line_bounds(input);
        let line = line.trim_end();
        if line.is_empty() {
            return None;
        }

        if line.starts_with('/') {
            let body = line.trim_start_matches('/');
            let body = body.trim_start();
            if body.is_empty() {
                return Some(InlineContext {
                    kind: InlinePaletteKind::Command,
                    query: String::new(),
                });
            }
            let mut parts = body.splitn(2, char::is_whitespace);
            let token = parts.next().unwrap_or("").trim();
            let rest = parts.next().unwrap_or("").trim_start();
            if token.is_empty() {
                return Some(InlineContext {
                    kind: InlinePaletteKind::Command,
                    query: String::new(),
                });
            }
            let kind = match token {
                "mode" => InlinePaletteKind::Role,
                "model" => InlinePaletteKind::Model,
                "approval-policy" => InlinePaletteKind::ApprovalPolicy,
                "sandbox-policy" => InlinePaletteKind::SandboxPolicy,
                "sandbox-network" => InlinePaletteKind::SandboxNetworkAccess,
                _ => InlinePaletteKind::Command,
            };
            let query = match kind {
                InlinePaletteKind::Command => token.to_string(),
                _ => rest.to_string(),
            };
            return Some(InlineContext { kind, query });
        }

        if ends_with_whitespace {
            return None;
        }

        let token = line
            .split_whitespace()
            .last()
            .unwrap_or("")
            .trim_end_matches('\n');
        let mut token_chars = token.chars();
        let prefix = token_chars.next()?;
        let query: String = token_chars.collect();
        let kind = match prefix {
            '@' => InlinePaletteKind::Role,
            '$' => InlinePaletteKind::Skill,
            _ => return None,
        };
        Some(InlineContext { kind, query })
    }

    fn inline_token_span(input: &str, trigger: char) -> Option<(usize, usize)> {
        let (line_start, line) = last_line_bounds(input);
        let line_trimmed = line.trim_end();
        if line_trimmed.is_empty() {
            return None;
        }
        let token_start = line_trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let token = &line_trimmed[token_start..];
        if !token.starts_with(trigger) {
            return None;
        }
        Some((line_start + token_start, line_start + line_trimmed.len()))
    }

    fn build_process_inspect_text(resp: &ProcessInspectResponse) -> String {
        let mut out = String::new();
        out.push_str(&format!("process_id: {}\n", resp.process.process_id));
        out.push_str(&format!("thread_id: {}\n", resp.process.thread_id));
        out.push_str(&format!(
            "status: {}\n",
            process_status_str(resp.process.status)
        ));
        if let Some(turn_id) = resp.process.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str(&format!("started_at: {}\n", resp.process.started_at));
        out.push_str(&format!("last_update_at: {}\n", resp.process.last_update_at));
        if let Some(exit_code) = resp.process.exit_code {
            out.push_str(&format!("exit_code: {exit_code}\n"));
        }
        out.push_str(&format!("cwd: {}\n", resp.process.cwd));
        out.push_str(&format!("argv: {}\n", resp.process.argv.join(" ")));
        out.push_str(&format!("stdout_path: {}\n", resp.process.stdout_path));
        out.push_str(&format!("stderr_path: {}\n", resp.process.stderr_path));

        out.push_str("\n# stdout\n\n");
        out.push_str(resp.stdout_tail.trim_end());
        out.push_str("\n\n# stderr\n\n");
        out.push_str(resp.stderr_tail.trim_end());
        out
    }

    fn build_artifact_read_text(resp: &ArtifactReadResponse) -> String {
        let mut out = String::new();
        out.push_str(&format!("artifact_id: {}\n", resp.metadata.artifact_id));
        out.push_str(&format!("artifact_type: {}\n", resp.metadata.artifact_type));
        out.push_str(&format!("summary: {}\n", resp.metadata.summary));
        out.push_str(&format!("version: {}\n", resp.metadata.version));
        out.push_str(&format!("bytes: {}\n", resp.bytes));
        out.push_str(&format!("truncated: {}\n", resp.truncated));
        out.push_str("\n# Content\n\n");
        out.push_str(resp.text.trim_end());
        out
    }

    fn process_status_str(value: ProcessStatus) -> &'static str {
        match value {
            ProcessStatus::Running => "running",
            ProcessStatus::Exited => "exited",
            ProcessStatus::Abandoned => "abandoned",
        }
    }

    fn handle_key_text_overlay(key: KeyEvent, view: &mut TextOverlay) -> OverlayOp {
        match key.code {
            KeyCode::Up => {
                view.scroll = view.scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                view.scroll = view.scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                view.scroll = view.scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                view.scroll = view.scroll.saturating_add(10);
            }
            _ => {}
        }
        OverlayOp::None
    }

    fn handle_key_command_palette(
        key: KeyEvent,
        view: &mut CommandPaletteOverlay,
    ) -> Option<PaletteCommand> {
        match key.code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                if !view.filtered.is_empty() {
                    view.selected = (view.selected + 1).min(view.filtered.len() - 1);
                }
            }
            KeyCode::Enter => {
                let selected = view.selected_action();
                if let Some(action) = selected {
                    if matches!(action, PaletteCommand::Noop)
                        && view.title == "model"
                        && !view.query.trim().is_empty()
                    {
                        return Some(PaletteCommand::SetModel(
                            view.query.trim().to_string(),
                        ));
                    }
                    return Some(action);
                }
                if view.title == "model" {
                    let query = view.query.trim();
                    if !query.is_empty() {
                        return Some(PaletteCommand::SetModel(query.to_string()));
                    }
                }
                return None;
            }
            KeyCode::Backspace => {
                view.query.pop();
                view.selected = 0;
                view.rebuild_filter();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                view.query.clear();
                view.selected = 0;
                view.rebuild_filter();
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    view.query.push(c);
                    view.selected = 0;
                    view.rebuild_filter();
                }
            }
            _ => {}
        }

        None
    }

    impl UiState {
        async fn open_approvals_overlay(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(thread_id) = self.active_thread else {
                return Ok(());
            };

            if matches!(self.overlays.last(), Some(Overlay::Approvals(_))) {
                self.overlays.pop();
                return Ok(());
            }

            let approvals = load_approvals(app, thread_id).await?;
            let selected = self
                .pending_action
                .as_ref()
                .map(|p| p.approval_id())
                .map(|id| select_approval(&approvals, Some(id)))
                .unwrap_or(0);
            self.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                thread_id,
                approvals,
                selected,
                remember: false,
            }));
            Ok(())
        }

        async fn open_processes_overlay(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(thread_id) = self.active_thread else {
                return Ok(());
            };

            if matches!(self.overlays.last(), Some(Overlay::Processes(_))) {
                self.overlays.pop();
                return Ok(());
            }

            let processes = load_processes(app, thread_id).await?;
            self.overlays.push(Overlay::Processes(ProcessesOverlay {
                thread_id,
                processes,
                selected: 0,
            }));
            Ok(())
        }

        async fn open_artifacts_overlay(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(thread_id) = self.active_thread else {
                return Ok(());
            };

            if matches!(self.overlays.last(), Some(Overlay::Artifacts(_))) {
                self.overlays.pop();
                return Ok(());
            }

            let value = app
                .rpc(
                    "artifact/list",
                    serde_json::json!({
                        "thread_id": thread_id,
                        "approval_id": null,
                    }),
                )
                .await?;

            if let Some((thread_id, approval_id)) = parse_needs_approval(&value) {
                let approvals = load_approvals(app, thread_id).await?;
                let selected = select_approval(&approvals, Some(approval_id));
                self.overlays.push(Overlay::Approvals(ApprovalsOverlay {
                    thread_id,
                    approvals,
                    selected,
                    remember: false,
                }));
                self.pending_action = Some(PendingAction::ArtifactList {
                    thread_id,
                    approval_id,
                });
                self.set_status(format!("artifact/list needs approval: {approval_id}"));
                return Ok(());
            }

            if is_denied(&value) {
                self.set_status(format!("artifact/list denied: {}", summarize_json(&value)));
                return Ok(());
            }

            let parsed = serde_json::from_value::<ArtifactListResponse>(value)
                .context("parse artifact/list response")?;
            if !parsed.errors.is_empty() {
                self.set_status(format!("artifact/list errors: {}", parsed.errors.len()));
            }
            self.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                thread_id,
                artifacts: parsed.artifacts,
                selected: 0,
            }));
            Ok(())
        }

        async fn resume_pending_action(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(pending) = self.pending_action.take() else {
                return Ok(());
            };

            match pending {
                PendingAction::ProcessInspect {
                    thread_id,
                    process_id,
                    max_lines,
                    approval_id,
                } => {
                    let value = app
                        .rpc(
                            "process/inspect",
                            serde_json::json!({
                                "process_id": process_id,
                                "max_lines": max_lines,
                                "approval_id": approval_id,
                            }),
                        )
                        .await?;
                    if parse_needs_approval(&value).is_some() {
                        self.pending_action = Some(PendingAction::ProcessInspect {
                            thread_id,
                            process_id,
                            max_lines,
                            approval_id,
                        });
                        self.set_status("process/inspect still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.set_status(format!(
                            "process/inspect denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }
                    let parsed = serde_json::from_value::<ProcessInspectResponse>(value)
                        .context("parse process/inspect response")?;
                    let text = build_process_inspect_text(&parsed);
                    self.overlays.push(Overlay::Text(TextOverlay {
                        title: format!("Process {}", parsed.process.process_id),
                        text,
                        scroll: 0,
                    }));
                }
                PendingAction::ProcessKill {
                    thread_id,
                    process_id,
                    approval_id,
                } => {
                    let value = app
                        .rpc(
                            "process/kill",
                            serde_json::json!({
                                "process_id": process_id,
                                "reason": "tui kill",
                                "approval_id": approval_id,
                            }),
                        )
                        .await?;
                    if parse_needs_approval(&value).is_some() {
                        self.pending_action = Some(PendingAction::ProcessKill {
                            thread_id,
                            process_id,
                            approval_id,
                        });
                        self.set_status("process/kill still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.set_status(format!(
                            "process/kill denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }
                    self.set_status(format!("kill requested: {process_id}"));
                    self.refresh_processes_overlay(app, thread_id).await?;
                }
                PendingAction::ProcessInterrupt {
                    thread_id,
                    process_id,
                    approval_id,
                } => {
                    let value = app
                        .rpc(
                            "process/interrupt",
                            serde_json::json!({
                                "process_id": process_id,
                                "reason": "tui interrupt",
                                "approval_id": approval_id,
                            }),
                        )
                        .await?;
                    if parse_needs_approval(&value).is_some() {
                        self.pending_action = Some(PendingAction::ProcessInterrupt {
                            thread_id,
                            process_id,
                            approval_id,
                        });
                        self.set_status("process/interrupt still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.set_status(format!(
                            "process/interrupt denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }
                    self.set_status(format!("interrupt requested: {process_id}"));
                    self.refresh_processes_overlay(app, thread_id).await?;
                }
                PendingAction::ArtifactList {
                    thread_id,
                    approval_id,
                } => {
                    let value = app
                        .rpc(
                            "artifact/list",
                            serde_json::json!({
                                "thread_id": thread_id,
                                "approval_id": approval_id,
                            }),
                        )
                        .await?;
                    if parse_needs_approval(&value).is_some() {
                        self.pending_action = Some(PendingAction::ArtifactList {
                            thread_id,
                            approval_id,
                        });
                        self.set_status("artifact/list still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.set_status(format!(
                            "artifact/list denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }

                    let parsed = serde_json::from_value::<ArtifactListResponse>(value)
                        .context("parse artifact/list response")?;
                    if !parsed.errors.is_empty() {
                        self.set_status(format!("artifact/list errors: {}", parsed.errors.len()));
                    }
                    self.refresh_artifacts_overlay(thread_id, parsed.artifacts);
                }
                PendingAction::ArtifactRead {
                    thread_id,
                    artifact_id,
                    max_bytes,
                    approval_id,
                } => {
                    let value = app
                        .rpc(
                            "artifact/read",
                            serde_json::json!({
                                "thread_id": thread_id,
                                "artifact_id": artifact_id,
                                "max_bytes": max_bytes,
                                "approval_id": approval_id,
                            }),
                        )
                        .await?;
                    if parse_needs_approval(&value).is_some() {
                        self.pending_action = Some(PendingAction::ArtifactRead {
                            thread_id,
                            artifact_id,
                            max_bytes,
                            approval_id,
                        });
                        self.set_status("artifact/read still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.set_status(format!(
                            "artifact/read denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }
                    let parsed = serde_json::from_value::<ArtifactReadResponse>(value)
                        .context("parse artifact/read response")?;
                    let text = build_artifact_read_text(&parsed);
                    self.overlays.push(Overlay::Text(TextOverlay {
                        title: format!("Artifact {}", parsed.metadata.artifact_id),
                        text,
                        scroll: 0,
                    }));
                }
            }

            Ok(())
        }

        async fn refresh_processes_overlay(
            &mut self,
            app: &mut super::App,
            thread_id: ThreadId,
        ) -> anyhow::Result<()> {
            let processes = load_processes(app, thread_id).await?;
            for overlay in &mut self.overlays {
                if let Overlay::Processes(view) = overlay {
                    if view.thread_id == thread_id {
                        let current = view
                            .processes
                            .get(view.selected)
                            .map(|process| process.process_id);
                        view.processes = processes.clone();
                        view.selected = select_process(&view.processes, current);
                    }
                }
            }
            Ok(())
        }

        fn refresh_artifacts_overlay(&mut self, thread_id: ThreadId, artifacts: Vec<ArtifactMetadata>) {
            let mut updated = false;
            for overlay in &mut self.overlays {
                if let Overlay::Artifacts(view) = overlay {
                    if view.thread_id == thread_id {
                        let current = view
                            .artifacts
                            .get(view.selected)
                            .map(|artifact| artifact.artifact_id);
                        view.artifacts = artifacts.clone();
                        view.selected = select_artifact(&view.artifacts, current);
                        updated = true;
                    }
                }
            }
            if !updated {
                self.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                    thread_id,
                    artifacts,
                    selected: 0,
                }));
            }
        }
    }

    fn draw_ui(f: &mut ratatui::Frame, state: &mut UiState) {
        let area = f.area();
        f.render_widget(Clear, area);
        match state.active_thread {
            None => {
                let show_footer = !matches!(state.overlays.last(), Some(Overlay::CommandPalette(_)));
                if show_footer {
                    let layout = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(1), Constraint::Length(1)])
                        .split(area);
                    draw_thread_list(f, state, layout[0]);
                    draw_footer(f, state, layout[1]);
                } else {
                    draw_thread_list(f, state, area);
                }
            }
            Some(_) => draw_thread_view(f, state, area),
        }

        if let Some(overlay) = state.overlays.last() {
            match overlay {
                Overlay::CommandPalette(_) if state.active_thread.is_none() => {
                    draw_overlay(f, overlay);
                }
                Overlay::CommandPalette(_) => {}
                _ => draw_overlay(f, overlay),
            }
        }
    }

    const BASELINE_TOKENS: u64 = 12_000;

    fn percent_of_context_window_remaining(total_tokens_used: u64, context_window: u64) -> u64 {
        if context_window <= BASELINE_TOKENS {
            return 0;
        }
        let effective_window = context_window.saturating_sub(BASELINE_TOKENS);
        let used = total_tokens_used.saturating_sub(BASELINE_TOKENS);
        let remaining = effective_window.saturating_sub(used);
        (remaining as f64 / effective_window as f64 * 100.0)
            .clamp(0.0, 100.0)
            .round() as u64
    }

    fn build_footer_line(state: &UiState, width: u16) -> Line<'static> {
        let context_left = match state.header.model_context_window {
            Some(window) => {
                let pct = percent_of_context_window_remaining(state.total_tokens_used, window);
                format!("{pct}% context left")
            }
            None => {
                if state.total_tokens_used > 0 {
                    format!("{} used", state.total_tokens_used)
                } else {
                    "100% context left".to_string()
                }
            }
        };

        let msg = match state.active_thread {
            Some(thread_id) => {
                let short = super::thread_id_short(thread_id);
                let mode = state.header.mode.as_deref().unwrap_or("-");
                let provider = state.header.provider.as_deref().unwrap_or("-");
                let model = state.header.model.as_deref().unwrap_or("-");
                let thinking = state.header.thinking.as_deref().unwrap_or("-");
                let mcp = if state.header.mcp_enabled { "on" } else { "off" };

                if width < 80 {
                    format!("{context_left}  th={short} mode={mode} model={model} (Ctrl-K)")
                } else {
                    format!(
                        "{context_left}  thread={short} agent={mode} provider={provider} model={model} thinking={thinking} mcp={mcp} (Ctrl-K=commands)"
                    )
                }
            }
            None => format!("{context_left}  threads (Ctrl-K=commands)"),
        };

        let style = Style::default().fg(Color::Gray);
        match state.status.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(status) => {
                let status_style = if UiState::is_error_message(status) {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Gray)
                };
                Line::from(vec![
                    Span::styled(msg, style),
                    Span::styled(" | ", style),
                    Span::styled(status.to_string(), status_style),
                ])
            }
            None => Line::from(Span::styled(msg, style)),
        }
    }

    fn draw_footer(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let paragraph = Paragraph::new(build_footer_line(state, area.width));
        f.render_widget(paragraph, area);
    }

    fn parse_rfc3339_timestamp(value: &str) -> Option<OffsetDateTime> {
        OffsetDateTime::parse(value, &Rfc3339).ok()
    }

    fn human_time_ago(ts: OffsetDateTime) -> String {
        let now = OffsetDateTime::now_utc();
        let delta = now - ts;
        let secs = delta.whole_seconds().max(0);
        if secs < 60 {
            if secs == 1 {
                format!("{secs} second ago")
            } else {
                format!("{secs} seconds ago")
            }
        } else if secs < 60 * 60 {
            let mins = secs / 60;
            if mins == 1 {
                format!("{mins} minute ago")
            } else {
                format!("{mins} minutes ago")
            }
        } else if secs < 60 * 60 * 24 {
            let hours = secs / 3600;
            if hours == 1 {
                format!("{hours} hour ago")
            } else {
                format!("{hours} hours ago")
            }
        } else {
            let days = secs / (60 * 60 * 24);
            if days == 1 {
                format!("{days} day ago")
            } else {
                format!("{days} days ago")
            }
        }
    }

    fn format_updated_label(meta: &ThreadMeta) -> String {
        let updated = meta
            .updated_at
            .as_deref()
            .and_then(parse_rfc3339_timestamp)
            .or_else(|| meta.created_at.as_deref().and_then(parse_rfc3339_timestamp));
        updated.map(human_time_ago).unwrap_or_else(|| "-".to_string())
    }

    fn normalize_single_line(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return "-".to_string();
        }
        trimmed
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn right_elide(value: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }
        if UnicodeWidthStr::width(value) <= max_width {
            return value.to_string();
        }
        if max_width <= 1 {
            return "…".to_string();
        }
        let tail_len = max_width.saturating_sub(1);
        let mut out = String::new();
        out.push('…');
        let mut tail = String::new();
        for ch in value.chars().rev() {
            if UnicodeWidthStr::width(tail.as_str()) >= tail_len {
                break;
            }
            tail.push(ch);
        }
        out.push_str(&tail.chars().rev().collect::<String>());
        out
    }

    fn pad_to_width(value: &str, width: usize) -> String {
        let mut out = value.to_string();
        let pad = width.saturating_sub(UnicodeWidthStr::width(value));
        if pad > 0 {
            out.push_str(&" ".repeat(pad));
        }
        out
    }

    fn draw_thread_list(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        const MAX_TITLE_WIDTH: usize = 24;
        const MAX_CWD_WIDTH: usize = 24;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let header = "threads (↑↓ Enter=open n=new r=refresh q/Ctrl-C=quit)";
        f.render_widget(
            Paragraph::new(header).style(Style::default().fg(Color::Gray)),
            chunks[0],
        );

        let mut rows = Vec::with_capacity(state.threads.len());
        let mut max_updated_width = UnicodeWidthStr::width("Updated");
        let mut max_title_width = UnicodeWidthStr::width("Title");
        let mut max_cwd_width = UnicodeWidthStr::width("CWD");

        for thread in &state.threads {
            let updated = format_updated_label(thread);
            let title = thread
                .title
                .as_deref()
                .map(normalize_single_line)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".to_string());
            let title = right_elide(&title, MAX_TITLE_WIDTH);
            let cwd = thread
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| right_elide(s, MAX_CWD_WIDTH))
                .unwrap_or_else(|| "-".to_string());
            let message = thread
                .first_message
                .as_deref()
                .map(normalize_single_line)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".to_string());

            max_updated_width = max_updated_width.max(UnicodeWidthStr::width(updated.as_str()));
            max_title_width = max_title_width.max(UnicodeWidthStr::width(title.as_str()));
            max_cwd_width = max_cwd_width.max(UnicodeWidthStr::width(cwd.as_str()));

            rows.push((updated, title, cwd, message));
        }

        let column_header = format!(
            "{}  {}  {}  Message",
            pad_to_width("Updated", max_updated_width),
            pad_to_width("Title", max_title_width),
            pad_to_width("CWD", max_cwd_width),
        );
        let header_line = Paragraph::new(column_header).style(Style::default().fg(Color::Gray));
        let header_area = ratatui::layout::Rect {
            x: chunks[1].x,
            y: chunks[1].y,
            width: chunks[1].width,
            height: 1,
        };
        f.render_widget(header_line, header_area);

        let list_area = ratatui::layout::Rect {
            x: chunks[1].x,
            y: chunks[1].y + 1,
            width: chunks[1].width,
            height: chunks[1].height.saturating_sub(1),
        };

        let items = rows
            .into_iter()
            .map(|(updated, title, cwd, message)| {
                let line = format!(
                    "{}  {}  {}  {}",
                    pad_to_width(&updated, max_updated_width),
                    pad_to_width(&title, max_title_width),
                    pad_to_width(&cwd, max_cwd_width),
                    message
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");

        let selected = if state.threads.is_empty() {
            None
        } else {
            Some(state.selected_thread)
        };
        f.render_stateful_widget(list, list_area, &mut list_state(selected));
    }

    fn list_state(selected: Option<usize>) -> ratatui::widgets::ListState {
        let mut state = ratatui::widgets::ListState::default();
        state.select(selected);
        state
    }

    struct InputRender {
        lines: Vec<Line<'static>>,
        cursor_line: usize,
        cursor_col: usize,
    }

    struct PaletteRender {
        lines: Vec<Line<'static>>,
    }

    fn draw_thread_view(
        f: &mut ratatui::Frame,
        state: &mut UiState,
        area: ratatui::layout::Rect,
    ) {
        let width = area.width.max(1) as usize;
        let mut transcript_lines = Vec::<Line>::new();
        for entry in &state.transcript {
            transcript_lines.extend(format_transcript_entry_lines(entry, width));
        }
        if let Some(streaming) = &state.streaming {
            transcript_lines.extend(format_role_lines(
                TranscriptRole::Assistant,
                streaming.text.as_str(),
                width,
            ));
        }
        // 顶部固定留一行空白，避免首条输出把输入框顶线盖住。
        transcript_lines.insert(0, Line::from(Span::raw("")));

        const MAX_INLINE_PALETTE_ITEMS: usize = 12;
        const INLINE_PALETTE_MIN_LINES: usize = 4;

        let input_render = build_input_lines(&state.input, width);
        let input_lines = input_render.lines.len();
        let total_height = area.height as usize;

        let inline_view = state.inline_palette.as_ref().map(|inline| &inline.view);
        let footer_lines = if inline_view.is_some() { 0 } else { 1 };
        let reserved_bottom = input_lines.saturating_add(footer_lines);
        let palette_target_lines = inline_view
            .map(|view| {
                let items = if view.filtered.is_empty() {
                    1
                } else {
                    view.filtered.len().min(MAX_INLINE_PALETTE_ITEMS)
                };
                1 + items
            })
            .unwrap_or(0);
        let min_palette_lines = if inline_view.is_some() {
            INLINE_PALETTE_MIN_LINES
        } else {
            0
        };

        let max_bottom_pct = if inline_view.is_some() { 70 } else { 35 };
        let mut max_bottom = ((total_height * max_bottom_pct) / 100)
            .max(4)
            .min(total_height);
        let desired_bottom = reserved_bottom.saturating_add(palette_target_lines);
        let min_bottom_needed = reserved_bottom.saturating_add(min_palette_lines);
        max_bottom = max_bottom
            .max(desired_bottom)
            .max(min_bottom_needed)
            .min(total_height);

        let available_palette_lines = max_bottom.saturating_sub(reserved_bottom);
        let max_palette_lines = if inline_view.is_some() {
            available_palette_lines
                .min(palette_target_lines)
                .max(min_palette_lines.min(available_palette_lines))
        } else {
            0
        };

        let palette_render = inline_view
            .map(|view| build_command_palette_lines(view, width, max_palette_lines));

        let mut bottom_lines = Vec::<Line>::new();
        let input_start = bottom_lines.len();
        bottom_lines.extend(input_render.lines);
        let input_cursor = (
            input_start + input_render.cursor_line,
            input_render.cursor_col,
        );
        if let Some(palette) = palette_render {
            bottom_lines.extend(palette.lines);
        }
        if footer_lines > 0 {
            bottom_lines.push(build_footer_line(state, area.width));
        }

        let required_bottom = bottom_lines.len().max(1);
        let bottom_height = required_bottom.min(max_bottom).min(total_height);
        let max_top = total_height.saturating_sub(bottom_height);
        let top_height = transcript_lines.len().min(max_top);

        let transcript_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: top_height as u16,
        };
        let bottom_area = ratatui::layout::Rect {
            x: area.x,
            y: area.y.saturating_add(top_height as u16),
            width: area.width,
            height: bottom_height as u16,
        };

        let viewport_height = transcript_area.height as usize;
        let max_scroll = transcript_lines.len().saturating_sub(viewport_height);
        state.transcript_max_scroll = u16::try_from(max_scroll).unwrap_or(u16::MAX);
        state.transcript_viewport_height = transcript_area.height;

        let scroll = if state.transcript_follow {
            max_scroll
        } else {
            usize::from(state.transcript_scroll).min(max_scroll)
        };
        state.transcript_scroll = u16::try_from(scroll).unwrap_or(u16::MAX);

        if transcript_area.height > 0 {
            let paragraph = Paragraph::new(Text::from(transcript_lines));
            f.render_widget(
                paragraph.scroll((state.transcript_scroll, 0)),
                transcript_area,
            );
        }

        if bottom_area.height > 0 {
            let bottom_paragraph = Paragraph::new(Text::from(bottom_lines));
            f.render_widget(bottom_paragraph, bottom_area);

            if state.active_thread.is_some() {
                let cursor_target = match state.overlays.last() {
                    Some(_) => None,
                    None => Some(input_cursor),
                };
                if let Some((cursor_line, cursor_col)) = cursor_target {
                    if cursor_line < bottom_height {
                        let y = bottom_area.y.saturating_add(cursor_line as u16);
                        let x = bottom_area.x.saturating_add(cursor_col as u16);
                        let max_x = bottom_area
                            .x
                            .saturating_add(bottom_area.width.saturating_sub(1));
                        f.set_cursor_position((x.min(max_x), y));
                    }
                }
            }
        }
    }

    fn format_transcript_entry_lines(entry: &TranscriptEntry, width: usize) -> Vec<Line<'static>> {
        format_role_lines(entry.role, entry.text.as_str(), width)
    }

    fn format_role_lines(
        role: TranscriptRole,
        text: &str,
        width: usize,
    ) -> Vec<Line<'static>> {
        let (prefix, prefix_style, content_style) = match role {
            TranscriptRole::User => ("user: ", Style::default().fg(Color::Yellow), None),
            TranscriptRole::Assistant => ("assistant: ", Style::default().fg(Color::Green), None),
            TranscriptRole::System => ("system: ", Style::default().fg(Color::Cyan), None),
            TranscriptRole::Error => ("error: ", Style::default().fg(Color::Red), None),
            TranscriptRole::Tool => ("tool: ", Style::default().fg(Color::Blue), Some(Style::default().fg(Color::Blue))),
        };
        wrap_prefixed_lines(prefix, prefix_style, content_style, text, width)
    }

    fn wrap_prefixed_lines(
        prefix: &str,
        prefix_style: Style,
        content_style: Option<Style>,
        text: &str,
        width: usize,
    ) -> Vec<Line<'static>> {
        let width = width.max(1);
        let prefix_width = UnicodeWidthStr::width(prefix);
        let available_first = width.saturating_sub(prefix_width).max(1);
        let available_other = width;
        let mut out = Vec::<Line>::new();

        let mut first = true;
        for raw_line in text.split('\n') {
            let max_width = if first {
                available_first
            } else {
                available_other
            };
            let mut segments = wrap_plain_text(raw_line, max_width);
            if segments.is_empty() {
                segments.push(String::new());
            }
            for segment in segments {
                let lead = if first { prefix } else { "" };
                let lead_style = prefix_style;
                let body_style = content_style.unwrap_or_default();
                out.push(Line::from(vec![
                    Span::styled(lead.to_string(), lead_style),
                    Span::styled(segment, body_style),
                ]));
                first = false;
            }
            if first {
                out.push(Line::from(vec![Span::styled(
                    prefix.to_string(),
                    prefix_style,
                )]));
                first = false;
            }
        }

        if out.is_empty() {
            out.push(Line::from(vec![Span::styled(
                prefix.to_string(),
                prefix_style,
            )]));
        }
        out
    }

    fn wrap_plain_text(text: &str, max_width: usize) -> Vec<String> {
        let max_width = max_width.max(1);
        let mut out = Vec::new();
        let mut current = String::new();
        let mut current_width = 0usize;
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > max_width && !current.is_empty() {
                out.push(current);
                current = String::new();
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        out.push(current);
        out
    }

    fn truncate_to_width(text: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }
        let mut out = String::new();
        let mut width = 0usize;
        let mut truncated = false;
        for ch in text.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if width + ch_width > max_width {
                truncated = true;
                break;
            }
            out.push(ch);
            width += ch_width;
        }
        if truncated && max_width > 1 {
            if UnicodeWidthStr::width(out.as_str()) >= max_width {
                out.pop();
            }
            out.push('…');
        }
        out
    }

    fn build_palette_item_line(item: &PaletteItem, selected: bool, width: usize) -> Line<'static> {
        let width = width.max(1);
        let marker = if selected { ">" } else { " " };
        let mut base_style = Style::default();
        let mut detail_style = Style::default().fg(Color::DarkGray);
        if selected {
            base_style = base_style.add_modifier(Modifier::REVERSED);
            detail_style = detail_style.add_modifier(Modifier::REVERSED);
        }

        let mut spans = Vec::new();
        let marker_text = format!("{marker} ");
        let mut used = UnicodeWidthStr::width(marker_text.as_str());
        spans.push(Span::styled(marker_text, base_style));

        let mut remaining = width.saturating_sub(used);
        let label = truncate_to_width(item.label.as_str(), remaining);
        let label_width = UnicodeWidthStr::width(label.as_str());
        spans.push(Span::styled(label, base_style));
        used = used.saturating_add(label_width);
        remaining = width.saturating_sub(used);

        if let Some(detail) = item.detail.as_deref().filter(|d| !d.trim().is_empty()) {
            if remaining > 0 {
                let spacer = if remaining >= 2 { "  " } else { " " };
                let spacer_width = UnicodeWidthStr::width(spacer);
                if remaining >= spacer_width {
                    spans.push(Span::styled(spacer.to_string(), base_style));
                    used = used.saturating_add(spacer_width);
                    remaining = width.saturating_sub(used);
                    if remaining > 0 {
                        let detail = truncate_to_width(detail, remaining);
                        let detail_width = UnicodeWidthStr::width(detail.as_str());
                        spans.push(Span::styled(detail, detail_style));
                        used = used.saturating_add(detail_width);
                    }
                }
            }
        }

        let pad = width.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), base_style));
        }

        Line::from(spans)
    }

    fn build_command_palette_lines(
        view: &CommandPaletteOverlay,
        width: usize,
        max_lines: usize,
    ) -> PaletteRender {
        let mut lines = Vec::<Line>::new();
        if max_lines == 0 {
            return PaletteRender { lines };
        }
        let title = view.title.trim();
        let label = if title.is_empty() { "commands" } else { title };
        let label = format!("{}: ", label.to_ascii_lowercase());
        let query = view.query.as_str();
        let display_query = if query.trim().is_empty() {
            "(type to search)"
        } else {
            query
        };
        let header_text = truncate_to_width(&format!("{label}{display_query}"), width);
        let header_style = Style::default().fg(Color::Gray);
        let header_pad = width.saturating_sub(UnicodeWidthStr::width(header_text.as_str()));
        let mut header_spans = Vec::new();
        header_spans.push(Span::styled(header_text, header_style));
        if header_pad > 0 {
            header_spans.push(Span::styled(" ".repeat(header_pad), header_style));
        }
        lines.push(Line::from(header_spans));

        let available_items = max_lines.saturating_sub(1);
        if view.filtered.is_empty() {
            if available_items > 0 {
                let text = "  (no matches)";
                let style = Style::default().fg(Color::DarkGray);
                let pad = width.saturating_sub(UnicodeWidthStr::width(text));
                let mut spans = Vec::new();
                spans.push(Span::styled(text.to_string(), style));
                if pad > 0 {
                    spans.push(Span::styled(" ".repeat(pad), style));
                }
                lines.push(Line::from(spans));
            }
            return PaletteRender { lines };
        }

        if available_items == 0 {
            return PaletteRender { lines };
        }

        let visible = available_items.min(view.filtered.len()).max(1);
        let mut start = 0usize;
        if view.selected >= visible {
            start = view.selected + 1 - visible;
        }
        let end = (start + visible).min(view.filtered.len());

        for (offset, filtered_idx) in view.filtered[start..end].iter().enumerate() {
            if let Some(item) = view.items.get(*filtered_idx) {
                let selected = start + offset == view.selected;
                lines.push(build_palette_item_line(item, selected, width));
            }
        }

        PaletteRender { lines }
    }

    fn build_input_lines(input: &str, width: usize) -> InputRender {
        const PADDING_LINES: usize = 1;
        let width = width.max(1);
        let prompt = "› ";
        let prompt_width = UnicodeWidthStr::width(prompt);
        let indent = " ".repeat(prompt_width);
        let available = width.saturating_sub(prompt_width).max(1);
        let input_bg = Style::default().bg(Color::Rgb(60, 60, 60));
        let prompt_style = input_bg.fg(Color::Gray);
        let input_style = input_bg;

        let mut segments = Vec::<String>::new();
        for raw_line in input.split('\n') {
            let wrapped = wrap_plain_text(raw_line, available);
            if wrapped.is_empty() {
                segments.push(String::new());
            } else {
                segments.extend(wrapped);
            }
        }
        if segments.is_empty() {
            segments.push(String::new());
        }

        let cursor_line = segments.len().saturating_sub(1) + PADDING_LINES;
        let cursor_col = prompt_width
            .saturating_add(UnicodeWidthStr::width(
                segments
                    .last()
                    .map(|segment| segment.as_str())
                    .unwrap_or(""),
            ))
            .min(width.saturating_sub(1));

        let mut lines = Vec::<Line>::new();
        let padding = " ".repeat(width);
        for _ in 0..PADDING_LINES {
            lines.push(Line::from(Span::styled(padding.clone(), input_style)));
        }
        for (idx, segment) in segments.iter().enumerate() {
            let lead = if idx == 0 { prompt } else { indent.as_str() };
            let lead_style = prompt_style;
            let content_style = input_style;
            let used = UnicodeWidthStr::width(lead)
                .saturating_add(UnicodeWidthStr::width(segment.as_str()));
            let pad = width.saturating_sub(used);
            let mut spans = Vec::new();
            spans.push(Span::styled(lead.to_string(), lead_style));
            spans.push(Span::styled(segment.to_string(), content_style));
            if pad > 0 {
                spans.push(Span::styled(" ".repeat(pad), content_style));
            }
            lines.push(Line::from(spans));
        }
        for _ in 0..PADDING_LINES {
            lines.push(Line::from(Span::styled(padding.clone(), input_style)));
        }

        InputRender {
            lines,
            cursor_line,
            cursor_col,
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

    fn usage_total_tokens(usage: &Value) -> Option<u64> {
        let total_tokens = usage.get("total_tokens").and_then(Value::as_u64);
        let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
        let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
        total_tokens.or_else(|| match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        })
    }

    fn tool_status_str(value: ToolStatus) -> &'static str {
        match value {
            ToolStatus::Completed => "completed",
            ToolStatus::Failed => "failed",
            ToolStatus::Denied => "denied",
            ToolStatus::Cancelled => "cancelled",
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

    fn draw_overlay(f: &mut ratatui::Frame, overlay: &Overlay) {
        match overlay {
            Overlay::CommandPalette(view) => {
                draw_command_palette_overlay(f, f.area(), view);
            }
            _ => {
                let area = centered_rect(90, 80, f.area());
                f.render_widget(Clear, area);
                match overlay {
                    Overlay::Approvals(view) => draw_approvals_overlay(f, area, view),
                    Overlay::Processes(view) => draw_processes_overlay(f, area, view),
                    Overlay::Artifacts(view) => draw_artifacts_overlay(f, area, view),
                    Overlay::Text(view) => draw_text_overlay(f, area, view),
                    Overlay::CommandPalette(_) => {}
                }
            }
        }
    }

    fn draw_command_palette_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &CommandPaletteOverlay,
    ) {
        if area.height == 0 {
            return;
        }
        let width = area.width.max(1) as usize;
        let max_lines = ((area.height as usize) * 60 / 100)
            .max(3)
            .min(area.height as usize);
        let palette_render = build_command_palette_lines(view, width, max_lines);
        if palette_render.lines.is_empty() {
            return;
        }
        let height = palette_render.lines.len().min(max_lines).max(1) as u16;
        let rect = ratatui::layout::Rect {
            x: area.x,
            y: area.y.saturating_add(area.height.saturating_sub(height)),
            width: area.width,
            height,
        };
        f.render_widget(Clear, rect);
        let paragraph = Paragraph::new(Text::from(palette_render.lines));
        f.render_widget(paragraph, rect);
    }

    fn draw_approvals_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &ApprovalsOverlay,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                "Approvals (↑↓ select y=approve n=deny m=remember r=refresh Esc=close) remember={}",
                view.remember
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);

        let items = view
            .approvals
            .iter()
            .map(|item| {
                let line = format!(
                    "{} {}",
                    item.request.approval_id,
                    item.request.action.trim()
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Pending"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        let selected = if view.approvals.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[0], &mut list_state(selected));

        let details = view
            .approvals
            .get(view.selected)
            .map(build_approval_details)
            .unwrap_or_else(|| "no approvals".to_string());
        let paragraph = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[1]);
    }

    fn draw_processes_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &ProcessesOverlay,
    ) {
        let block = Block::default().borders(Borders::ALL).title(
            "Processes (↑↓ select Enter=inspect k=kill x=interrupt r=refresh Esc=close)",
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);

        let items = view
            .processes
            .iter()
            .map(|process| {
                let argv0 = process.argv.first().map(String::as_str).unwrap_or("");
                let line = format!(
                    "[{}] {} {}",
                    process_status_str(process.status),
                    process.process_id,
                    argv0
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Processes"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        let selected = if view.processes.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[0], &mut list_state(selected));

        let details = view
            .processes
            .get(view.selected)
            .map(build_process_details)
            .unwrap_or_else(|| "no processes".to_string());
        let paragraph = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[1]);
    }

    fn build_process_details(process: &ProcessInfo) -> String {
        let mut out = String::new();
        out.push_str(&format!("process_id: {}\n", process.process_id));
        out.push_str(&format!("thread_id: {}\n", process.thread_id));
        out.push_str(&format!(
            "status: {}\n",
            process_status_str(process.status)
        ));
        if let Some(turn_id) = process.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str(&format!("started_at: {}\n", process.started_at));
        out.push_str(&format!("last_update_at: {}\n", process.last_update_at));
        if let Some(exit_code) = process.exit_code {
            out.push_str(&format!("exit_code: {exit_code}\n"));
        }
        out.push_str(&format!("cwd: {}\n", process.cwd));
        out.push_str(&format!("argv: {}\n", process.argv.join(" ")));
        out.push_str(&format!("stdout_path: {}\n", process.stdout_path));
        out.push_str(&format!("stderr_path: {}\n", process.stderr_path));
        out
    }

    fn draw_artifacts_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &ArtifactsOverlay,
    ) {
        let block =
            Block::default()
                .borders(Borders::ALL)
                .title("Artifacts (↑↓ select Enter=read r=refresh Esc=close)");
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(inner);

        let items = view
            .artifacts
            .iter()
            .map(|meta| {
                let line = format!(
                    "[{}] {} {}",
                    meta.artifact_type.trim(),
                    meta.artifact_id,
                    meta.summary.trim()
                );
                ListItem::new(line)
            })
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Artifacts"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        let selected = if view.artifacts.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[0], &mut list_state(selected));

        let details = view
            .artifacts
            .get(view.selected)
            .map(build_artifact_details)
            .unwrap_or_else(|| "no artifacts".to_string());
        let paragraph = Paragraph::new(details)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, chunks[1]);
    }

    fn build_artifact_details(meta: &ArtifactMetadata) -> String {
        let mut out = String::new();
        out.push_str(&format!("artifact_id: {}\n", meta.artifact_id));
        out.push_str(&format!("artifact_type: {}\n", meta.artifact_type));
        out.push_str(&format!("summary: {}\n", meta.summary));
        out.push_str(&format!("version: {}\n", meta.version));
        out.push_str(&format!("size_bytes: {}\n", meta.size_bytes));
        out.push_str(&format!(
            "updated_at_unix: {}\n",
            meta.updated_at.unix_timestamp()
        ));
        out.push_str(&format!("content_path: {}\n", meta.content_path));
        out
    }

    fn build_approval_details(item: &ApprovalItem) -> String {
        let mut out = String::new();
        out.push_str(&format!("approval_id: {}\n", item.request.approval_id));
        out.push_str(&format!("requested_at: {}\n", item.request.requested_at));
        out.push_str(&format!("action: {}\n", item.request.action));
        if let Some(turn_id) = item.request.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str("\nparams:\n");
        out.push_str(
            &serde_json::to_string_pretty(&item.request.params)
                .unwrap_or_else(|_| item.request.params.to_string()),
        );
        out
    }

    fn draw_text_overlay(f: &mut ratatui::Frame, area: ratatui::layout::Rect, view: &TextOverlay) {
        let block = Block::default().borders(Borders::ALL).title(view.title.as_str());
        let paragraph = Paragraph::new(view.text.as_str())
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((view.scroll, 0));
        f.render_widget(paragraph, area);
    }

    fn centered_rect(percent_x: u16, percent_y: u16, r: ratatui::layout::Rect) -> ratatui::layout::Rect {
        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ])
            .split(r);

        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(popup_layout[1]);

        horizontal[1]
    }

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
}

async fn run_tui(app: &mut App, args: TuiArgs) -> anyhow::Result<()> {
    tui::run_tui(app, args).await
}
