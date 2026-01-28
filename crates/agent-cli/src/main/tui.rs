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
    use pm_jsonrpc::ClientHandle;
    use pm_protocol::{
        ApprovalDecision, ApprovalId, ArtifactId, ArtifactMetadata, ModelRoutingRuleSource,
        ProcessId, ThreadEvent, ThreadEventKind, ThreadId, TurnId, TurnStatus,
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
    use unicode_width::UnicodeWidthStr;

    enum PollOutcome {
        Response(SubscribeResponse),
        Timeout,
    }

    struct ModelFetchInFlight {
        thread_id: ThreadId,
        handle: tokio::task::JoinHandle<anyhow::Result<Vec<String>>>,
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

    async fn handle_tick(
        state: &mut UiState,
        app: &mut super::App,
        header_timeout: Duration,
        last_threads_refresh: &mut Instant,
    ) {
        if let Some(thread_id) = state.active_thread {
            if state.header_needs_refresh {
                match tokio::time::timeout(header_timeout, state.refresh_header(app, thread_id))
                    .await
                {
                    Ok(Ok(())) => state.header_needs_refresh = false,
                    Ok(Err(err)) => {
                        state.set_status(format!("header refresh error: {err}"));
                    }
                    Err(_) => {}
                }
            }
        }

        if state.active_thread.is_none() && last_threads_refresh.elapsed() >= Duration::from_secs(2)
        {
            if let Err(err) = state.refresh_threads(app).await {
                state.set_status(format!("refresh error: {err}"));
            } else {
                *last_threads_refresh = Instant::now();
            }
        }
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

        loop {
            terminal.draw(|f| draw_ui(f, &mut state))?;

            if state.active_thread != last_active_thread {
                last_active_thread = state.active_thread;
                if let Some(inflight) = poll_inflight.take() {
                    inflight.handle.abort();
                }
                state.cancel_model_fetch();
                last_poll = Instant::now() - poll_interval;
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
                        handle_tick(&mut state, app, header_timeout, &mut last_threads_refresh).await;
                    }
                }
            } else {
                tokio::select! {
                    Some(note) = notifications.recv() => {
                        if let Err(err) = state.handle_notification(note) {
                            state.set_status(format!("notification error: {err}"));
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
                        handle_tick(&mut state, app, header_timeout, &mut last_threads_refresh).await;
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
                            for event in resp.events {
                                state.apply_event(&event);
                            }
                            if resp.has_more {
                                last_poll = Instant::now() - poll_interval;
                            }
                        }
                    }
                    Ok(Ok(PollOutcome::Timeout)) => {}
                    Ok(Err(err)) => {
                        state.set_status(format!("poll error: {err}"));
                    }
                    Err(err) => {
                        state.set_status(format!("poll task error: {err}"));
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
                    }
                    Ok(Err(err)) => {
                        state.model_fetch_pending = false;
                        state.set_status(format!("thread/models error: {err}"));
                        let show_error_palette = matches!(
                            state.overlays.last(),
                            Some(Overlay::CommandPalette(view)) if view.title == "Set model"
                        );
                        if show_error_palette {
                            state.replace_top_command_palette(build_model_error_palette(
                                &err.to_string(),
                            ));
                        }
                    }
                    Err(err) => {
                        state.model_fetch_pending = false;
                        state.set_status(format!("thread/models task error: {err}"));
                        let show_error_palette = matches!(
                            state.overlays.last(),
                            Some(Overlay::CommandPalette(view)) if view.title == "Set model"
                        );
                        if show_error_palette {
                            state.replace_top_command_palette(build_model_error_palette(
                                &err.to_string(),
                            ));
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
            }
            None => {
                let started = tokio::time::timeout(timeout, app.thread_start(None))
                    .await
                    .context("thread/start timeout")??;
                let thread_id: ThreadId = serde_json::from_value(started["thread_id"].clone())
                    .context("thread_id missing")?;
                let last_seq = started["last_seq"].as_u64().unwrap_or(0);
                state.activate_thread(thread_id, last_seq);
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
        Error,
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
        Noop,
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
                        let hay = item.label.to_ascii_lowercase();
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

        items.push(PaletteItem {
            label: "New conversation".to_string(),
            action: PaletteCommand::NewThread,
        });
        items.push(PaletteItem {
            label: "Thread picker".to_string(),
            action: PaletteCommand::ThreadPicker,
        });
        items.push(PaletteItem {
            label: "Refresh threads".to_string(),
            action: PaletteCommand::RefreshThreads,
        });

        if state.active_thread.is_some() {
            let mode = state.header.mode.as_deref().unwrap_or("-");
            let model = state.header.model.as_deref().unwrap_or("-");

            items.push(PaletteItem {
                label: "Approvals".to_string(),
                action: PaletteCommand::OpenApprovals,
            });
            items.push(PaletteItem {
                label: "Processes".to_string(),
                action: PaletteCommand::OpenProcesses,
            });
            items.push(PaletteItem {
                label: "Artifacts".to_string(),
                action: PaletteCommand::OpenArtifacts,
            });
            items.push(PaletteItem {
                label: format!("Set agent/mode (current: {mode})"),
                action: PaletteCommand::PickMode,
            });
            items.push(PaletteItem {
                label: format!("Set model (current: {model})"),
                action: PaletteCommand::PickModel,
            });
            items.push(PaletteItem {
                label: "Set approval policy".to_string(),
                action: PaletteCommand::PickApprovalPolicy,
            });
            items.push(PaletteItem {
                label: "Set sandbox policy".to_string(),
                action: PaletteCommand::PickSandboxPolicy,
            });
            items.push(PaletteItem {
                label: "Set sandbox network access".to_string(),
                action: PaletteCommand::PickSandboxNetworkAccess,
            });
        }

        items.push(PaletteItem {
            label: "Help".to_string(),
            action: PaletteCommand::Help,
        });
        items.push(PaletteItem {
            label: "Quit".to_string(),
            action: PaletteCommand::Quit,
        });

        CommandPaletteOverlay::new("Commands", items)
    }

    fn build_mode_palette(modes: Vec<String>, current: Option<&str>) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem {
            label: "← Back".to_string(),
            action: PaletteCommand::OpenRoot,
        });
        for mode in modes {
            let is_current = current.is_some_and(|c| c == mode);
            let label = if is_current {
                format!("{mode} (current)")
            } else {
                mode.clone()
            };
            items.push(PaletteItem {
                label,
                action: PaletteCommand::SetMode(mode),
            });
        }
        CommandPaletteOverlay::new("Set agent/mode", items)
    }

    fn build_model_palette(mut models: Vec<String>, current: Option<&str>) -> CommandPaletteOverlay {
        models.sort();
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem {
            label: "← Back".to_string(),
            action: PaletteCommand::OpenRoot,
        });
        for model in models {
            let is_current = current.is_some_and(|c| c == model);
            let label = if is_current {
                format!("{model} (current)")
            } else {
                model.clone()
            };
            items.push(PaletteItem {
                label,
                action: PaletteCommand::SetModel(model),
            });
        }
        CommandPaletteOverlay::new("Set model", items)
    }

    fn build_model_loading_palette() -> CommandPaletteOverlay {
        let items = vec![
            PaletteItem {
                label: "loading models...".to_string(),
                action: PaletteCommand::Noop,
            },
            PaletteItem {
                label: "type model and press Enter".to_string(),
                action: PaletteCommand::Noop,
            },
        ];
        CommandPaletteOverlay::new("Set model", items)
    }

    fn build_model_error_palette(error: &str) -> CommandPaletteOverlay {
        let items = vec![
            PaletteItem {
                label: format!("error: {error}"),
                action: PaletteCommand::Noop,
            },
            PaletteItem {
                label: "retry list".to_string(),
                action: PaletteCommand::PickModel,
            },
            PaletteItem {
                label: "type model and press Enter".to_string(),
                action: PaletteCommand::Noop,
            },
            PaletteItem {
                label: "← Back".to_string(),
                action: PaletteCommand::OpenRoot,
            },
        ];
        CommandPaletteOverlay::new("Set model", items)
    }

    fn build_approval_policy_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem {
            label: "← Back".to_string(),
            action: PaletteCommand::OpenRoot,
        });

        let policies = [
            super::CliApprovalPolicy::AutoApprove,
            super::CliApprovalPolicy::OnRequest,
            super::CliApprovalPolicy::Manual,
            super::CliApprovalPolicy::UnlessTrusted,
            super::CliApprovalPolicy::AutoDeny,
        ];
        for policy in policies {
            items.push(PaletteItem {
                label: format!("approval_policy={}", approval_policy_label(policy)),
                action: PaletteCommand::SetApprovalPolicy(policy),
            });
        }
        CommandPaletteOverlay::new("Set approval policy", items)
    }

    fn build_sandbox_policy_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem {
            label: "← Back".to_string(),
            action: PaletteCommand::OpenRoot,
        });

        let policies = [
            super::CliSandboxPolicy::ReadOnly,
            super::CliSandboxPolicy::WorkspaceWrite,
            super::CliSandboxPolicy::DangerFullAccess,
        ];
        for policy in policies {
            items.push(PaletteItem {
                label: format!("sandbox_policy={}", sandbox_policy_label(policy)),
                action: PaletteCommand::SetSandboxPolicy(policy),
            });
        }
        CommandPaletteOverlay::new("Set sandbox policy", items)
    }

    fn build_sandbox_network_access_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem {
            label: "← Back".to_string(),
            action: PaletteCommand::OpenRoot,
        });

        let values = [
            super::CliSandboxNetworkAccess::Deny,
            super::CliSandboxNetworkAccess::Allow,
        ];
        for value in values {
            items.push(PaletteItem {
                label: format!(
                    "sandbox_network_access={}",
                    sandbox_network_access_label(value)
                ),
                action: PaletteCommand::SetSandboxNetworkAccess(value),
            });
        }
        CommandPaletteOverlay::new("Set sandbox network access", items)
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

    fn tui_help_text() -> String {
        let mut out = String::new();
        out.push_str("keys:\n\n");
        out.push_str("  Ctrl-K           command palette\n");
        out.push_str("  /                command palette\n");
        out.push_str("  Ctrl-Q           quit\n");
        out.push_str("  Ctrl-C           interrupt active turn / quit when idle\n\n");
        out.push_str("thread view:\n\n");
        out.push_str("  Enter            send input\n");
        out.push_str("  Esc              back to thread picker\n");
        out.push_str("  ↑/↓              scroll transcript\n");
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
        last_seq: u64,
        transcript: VecDeque<TranscriptEntry>,
        transcript_scroll: u16,
        transcript_follow: bool,
        transcript_max_scroll: u16,
        transcript_viewport_height: u16,
        streaming: Option<StreamingState>,
        active_turn_id: Option<TurnId>,
        input: String,
        status: Option<String>,
        pending_action: Option<PendingAction>,
        model_fetch: Option<ModelFetchInFlight>,
        model_fetch_pending: bool,
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
                last_seq: 0,
                transcript: VecDeque::new(),
                transcript_scroll: 0,
                transcript_follow: true,
                transcript_max_scroll: 0,
                transcript_viewport_height: 0,
                streaming: None,
                active_turn_id: None,
                input: String::new(),
                status: None,
                pending_action: None,
                model_fetch: None,
                model_fetch_pending: false,
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
            self.last_seq = last_seq;
            self.transcript.clear();
            self.transcript_scroll = 0;
            self.transcript_follow = true;
            self.transcript_max_scroll = 0;
            self.transcript_viewport_height = 0;
            self.streaming = None;
            self.active_turn_id = None;
            self.pending_action = None;
            self.cancel_model_fetch();
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
            self.header.thinking = self
                .header
                .thinking
                .clone()
                .filter(|s| !s.trim().is_empty());

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
                ThreadEventKind::ThreadConfigUpdated { mode, model, .. } => {
                    if let Some(mode) = mode.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.mode = Some(mode.to_string());
                    }
                    if let Some(model) = model.as_deref().filter(|s| !s.trim().is_empty()) {
                        self.header.model = Some(model.to_string());
                    }
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

        fn apply_model_list(&mut self, models: Vec<String>) {
            if !self.model_fetch_pending {
                return;
            }
            self.model_fetch_pending = false;
            if models.is_empty() {
                self.set_status("thread/models error: empty model list".to_string());
                return;
            }
            let palette = build_model_palette(models, self.header.model.as_deref());
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
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
            self.status = None;
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
                self.toggle_command_palette();
                return Ok(false);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                if let (Some(thread_id), Some(turn_id)) = (self.active_thread, self.active_turn_id)
                {
                    app.turn_interrupt(thread_id, turn_id, Some("tui ctrl-c".to_string()))
                        .await?;
                    self.status = Some(format!("interrupt requested: {turn_id}"));
                    return Ok(false);
                }
                return Ok(true);
            }
            if self.overlays.is_empty()
                && !key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('/')
                && self.input.trim().is_empty()
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

            match key.code {
                KeyCode::Up => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self.transcript_scroll.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.transcript_follow = false;
                    self.transcript_scroll = self.transcript_scroll.saturating_add(1);
                    if self.transcript_scroll >= self.transcript_max_scroll {
                        self.transcript_scroll = self.transcript_max_scroll;
                        self.transcript_follow = true;
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
                    self.active_thread = None;
                    self.header = HeaderState::default();
                    self.header_needs_refresh = false;
                    self.overlays.clear();
                    self.input.clear();
                    self.streaming = None;
                    self.active_turn_id = None;
                    self.pending_action = None;
                    self.transcript_scroll = 0;
                    self.transcript_follow = true;
                    self.transcript_max_scroll = 0;
                    self.transcript_viewport_height = 0;
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
                    let turn_id = app.turn_start(thread_id, input, None).await?;
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

        fn toggle_command_palette(&mut self) {
            if matches!(self.overlays.last(), Some(Overlay::CommandPalette(_))) {
                self.cancel_model_fetch();
                self.overlays.pop();
                return;
            }

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
                    if self.active_thread.is_none() {
                        self.overlays.pop();
                        self.refresh_threads(app).await?;
                        return Ok(false);
                    }

                    self.active_thread = None;
                    self.header = HeaderState::default();
                    self.header_needs_refresh = false;
                    self.overlays.clear();
                    self.input.clear();
                    self.streaming = None;
                    self.active_turn_id = None;
                    self.pending_action = None;
                    self.transcript_scroll = 0;
                    self.transcript_follow = true;
                    self.transcript_max_scroll = 0;
                    self.transcript_viewport_height = 0;
                    self.refresh_threads(app).await?;
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
                        && view.title == "Set model"
                        && !view.query.trim().is_empty()
                    {
                        return Some(PaletteCommand::SetModel(
                            view.query.trim().to_string(),
                        ));
                    }
                    return Some(action);
                }
                if view.title == "Set model" {
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
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        match state.active_thread {
            None => draw_thread_list(f, state, layout[0]),
            Some(_) => draw_thread_view(f, state, layout[0]),
        }

        draw_input(f, state, layout[1]);
        draw_footer(f, state, layout[2]);

        if let Some(overlay) = state.overlays.last() {
            draw_overlay(f, overlay);
        }
    }

    fn draw_footer(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let msg = match state.active_thread {
            Some(thread_id) => {
                let short = super::thread_id_short(thread_id);
                let mode = state.header.mode.as_deref().unwrap_or("-");
                let provider = state.header.provider.as_deref().unwrap_or("-");
                let model = state.header.model.as_deref().unwrap_or("-");
                let thinking = state.header.thinking.as_deref().unwrap_or("-");
                let mcp = if state.header.mcp_enabled { "on" } else { "off" };

                if area.width < 80 {
                    format!("th={short} mode={mode} model={model} (Ctrl-K)")
                } else {
                    format!(
                        "thread={short} agent={mode} provider={provider} model={model} thinking={thinking} mcp={mcp} (Ctrl-K=commands)"
                    )
                }
            }
            None => "threads (Ctrl-K=commands)".to_string(),
        };

        let style = Style::default().fg(Color::Gray);
        let paragraph = match state.status.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(status) => {
                let status_style = if UiState::is_error_message(status) {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default()
                };
                Paragraph::new(Line::from(vec![
                    Span::styled(msg, style),
                    Span::styled(" | ", style),
                    Span::styled(status, status_style),
                ]))
            }
            None => Paragraph::new(msg).style(style),
        };

        f.render_widget(paragraph, area);
    }

    fn draw_thread_list(f: &mut ratatui::Frame, state: &UiState, area: ratatui::layout::Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        let header = "threads (↑↓ Enter=open n=new r=refresh q/Ctrl-C=quit)";
        f.render_widget(
            Paragraph::new(header).style(Style::default().fg(Color::Gray)),
            chunks[0],
        );

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
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");

        let selected = if state.threads.is_empty() {
            None
        } else {
            Some(state.selected_thread)
        };
        f.render_stateful_widget(list, chunks[1], &mut list_state(selected));
    }

    fn list_state(selected: Option<usize>) -> ratatui::widgets::ListState {
        let mut state = ratatui::widgets::ListState::default();
        state.select(selected);
        state
    }

    fn draw_thread_view(
        f: &mut ratatui::Frame,
        state: &mut UiState,
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
        let width = area.width.max(1);
        let base_paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
        let line_count: u16 = base_paragraph
            .line_count(width)
            .try_into()
            .unwrap_or(u16::MAX);
        let max_scroll = line_count.saturating_sub(area.height);
        state.transcript_max_scroll = max_scroll;
        state.transcript_viewport_height = area.height;

        let scroll = if state.transcript_follow {
            max_scroll
        } else {
            state.transcript_scroll.min(max_scroll)
        };
        state.transcript_scroll = scroll;

        f.render_widget(base_paragraph.scroll((scroll, 0)), area);
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
            TranscriptRole::Error => Line::from(vec![
                Span::styled("error: ", Style::default().fg(Color::Red)),
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
        let prompt = if state.active_thread.is_some() { "› " } else { "" };
        let line = format!("{prompt}{}", state.input);
        let input = Paragraph::new(line).wrap(Wrap { trim: false });
        f.render_widget(input, area);

        if state.active_thread.is_some() && state.overlays.is_empty() {
            let prompt_width = UnicodeWidthStr::width(prompt);
            let input_width = UnicodeWidthStr::width(state.input.as_str());
            let cursor_offset = prompt_width.saturating_add(input_width);
            let cursor_offset = u16::try_from(cursor_offset).unwrap_or(u16::MAX);
            let x = area.x.saturating_add(cursor_offset);
            let y = area.y;
            let max_x = area.x.saturating_add(area.width.saturating_sub(1));
            f.set_cursor_position((x.min(max_x), y));
        }
    }

    fn draw_overlay(f: &mut ratatui::Frame, overlay: &Overlay) {
        let area = match overlay {
            Overlay::CommandPalette(_) => centered_rect(80, 60, f.area()),
            _ => centered_rect(90, 80, f.area()),
        };
        f.render_widget(Clear, area);

        match overlay {
            Overlay::Approvals(view) => draw_approvals_overlay(f, area, view),
            Overlay::Processes(view) => draw_processes_overlay(f, area, view),
            Overlay::Artifacts(view) => draw_artifacts_overlay(f, area, view),
            Overlay::Text(view) => draw_text_overlay(f, area, view),
            Overlay::CommandPalette(view) => draw_command_palette_overlay(f, area, view),
        }
    }

    fn draw_command_palette_overlay(
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        view: &CommandPaletteOverlay,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("{} (Esc=close Enter=run)", view.title.as_str()));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);

        let query = if view.query.trim().is_empty() {
            "filter: (type to search)".to_string()
        } else {
            format!("filter: {}", view.query)
        };
        f.render_widget(
            Paragraph::new(query).style(Style::default().fg(Color::Gray)),
            chunks[0],
        );

        let items = view
            .filtered
            .iter()
            .filter_map(|idx| view.items.get(*idx))
            .map(|item| ListItem::new(item.label.as_str()))
            .collect::<Vec<_>>();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Commands"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");

        let selected = if view.filtered.is_empty() {
            None
        } else {
            Some(view.selected)
        };
        f.render_stateful_widget(list, chunks[1], &mut list_state(selected));
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

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"threads (↑↓ Enter=open n=new r=refresh q/Ctrl-C=quit)           
  [idle] 00000000-0000-0000-0000-000000000001  cwd=/repo  model=
▶ [running] 00000000-0000-0000-0000-000000000002  cwd=/repo  mod
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
threads (Ctrl-K=commands)                                       "#;
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

            let actual = render_to_string(&mut state, 64, 12)?;
            let expected = r#"system: [model] gpt-4.1 (global_default)                        
user: Hello                                                     
assistant: Hi!                                                  
assistant: Streaming...                                         
                                                                
                                                                
                                                                
                                                                
                                                                
                                                                
› next                                                          
th=00000000 mode=coder model=gpt-4.1 (Ctrl-K)                   "#;
            assert_eq!(actual, expected);
            Ok(())
        }
    }
}

async fn run_tui(app: &mut App, args: TuiArgs) -> anyhow::Result<()> {
    tui::run_tui(app, args).await
}
