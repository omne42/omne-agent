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

    #[derive(Debug, Clone)]
    enum Overlay {
        Approvals(ApprovalsOverlay),
        Processes(ProcessesOverlay),
        Artifacts(ArtifactsOverlay),
        Text(TextOverlay),
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

    struct UiState {
        include_archived: bool,
        threads: Vec<ThreadMeta>,
        selected_thread: usize,
        active_thread: Option<ThreadId>,
        overlays: Vec<Overlay>,
        last_seq: u64,
        transcript: VecDeque<TranscriptEntry>,
        streaming: Option<StreamingState>,
        active_turn_id: Option<TurnId>,
        input: String,
        status: Option<String>,
        pending_action: Option<PendingAction>,
    }

    impl UiState {
        fn new(include_archived: bool) -> Self {
            Self {
                include_archived,
                threads: Vec::new(),
                selected_thread: 0,
                active_thread: None,
                overlays: Vec::new(),
                last_seq: 0,
                transcript: VecDeque::new(),
                streaming: None,
                active_turn_id: None,
                input: String::new(),
                status: None,
                pending_action: None,
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
            self.overlays.clear();
            self.last_seq = resp.last_seq;
            self.transcript.clear();
            self.streaming = None;
            self.active_turn_id = None;
            self.pending_action = None;
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

        async fn handle_key_overlay(
            &mut self,
            app: &mut super::App,
            key: KeyEvent,
        ) -> anyhow::Result<bool> {
            let mut status = None::<String>;
            let mut decided = None::<ApprovalId>;
            let mut set_pending_action = None::<PendingAction>;
            let op;

            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
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
                }
            }

            if let Some(msg) = status {
                self.status = Some(msg);
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
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                if let (Some(thread_id), Some(turn_id)) = (self.active_thread, self.active_turn_id)
                {
                    app.turn_interrupt(thread_id, turn_id, Some("tui ctrl-c".to_string()))
                        .await?;
                    self.status = Some(format!("interrupt requested: {turn_id}"));
                }
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
                KeyCode::Esc => {
                    self.active_thread = None;
                    self.overlays.clear();
                    self.input.clear();
                    self.streaming = None;
                    self.active_turn_id = None;
                    self.pending_action = None;
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
                self.status = Some(format!("artifact/list needs approval: {approval_id}"));
                return Ok(());
            }

            if is_denied(&value) {
                self.status = Some(format!("artifact/list denied: {}", summarize_json(&value)));
                return Ok(());
            }

            let parsed = serde_json::from_value::<ArtifactListResponse>(value)
                .context("parse artifact/list response")?;
            if !parsed.errors.is_empty() {
                self.status = Some(format!("artifact/list errors: {}", parsed.errors.len()));
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
                        self.status = Some("process/inspect still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.status = Some(format!(
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
                        self.status = Some("process/kill still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.status = Some(format!(
                            "process/kill denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }
                    self.status = Some(format!("kill requested: {process_id}"));
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
                        self.status =
                            Some("process/interrupt still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.status = Some(format!(
                            "process/interrupt denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }
                    self.status = Some(format!("interrupt requested: {process_id}"));
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
                        self.status = Some("artifact/list still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.status = Some(format!(
                            "artifact/list denied: {}",
                            summarize_json(&value)
                        ));
                        return Ok(());
                    }

                    let parsed = serde_json::from_value::<ArtifactListResponse>(value)
                        .context("parse artifact/list response")?;
                    if !parsed.errors.is_empty() {
                        self.status = Some(format!("artifact/list errors: {}", parsed.errors.len()));
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
                        self.status = Some("artifact/read still needs approval".to_string());
                        return Ok(());
                    }
                    if is_denied(&value) {
                        self.status = Some(format!(
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

        if let Some(overlay) = state.overlays.last() {
            draw_overlay(f, overlay);
        }
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
                        "Thread {thread_id} (Esc=back Ctrl-Q=quit Ctrl-A=approvals Ctrl-P=processes Ctrl-O=artifacts Ctrl-C=interrupt)"
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

        if state.active_thread.is_some() && state.overlays.is_empty() {
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

    fn draw_overlay(f: &mut ratatui::Frame, overlay: &Overlay) {
        let area = centered_rect(90, 80, f.area());
        f.render_widget(Clear, area);

        match overlay {
            Overlay::Approvals(view) => draw_approvals_overlay(f, area, view),
            Overlay::Processes(view) => draw_processes_overlay(f, area, view),
            Overlay::Artifacts(view) => draw_artifacts_overlay(f, area, view),
            Overlay::Text(view) => draw_text_overlay(f, area, view),
        }
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
│┌Thread 00000000-0000-0000-0000-000000000001 (Esc=back Ctrl-Q┐│
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
