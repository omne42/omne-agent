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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TranscriptRole {
        User,
        Assistant,
        Thinking,
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
        scrollback_enabled: bool,
        threads: Vec<ThreadMeta>,
        selected_thread: usize,
        active_thread: Option<ThreadId>,
        header: HeaderState,
        header_needs_refresh: bool,
        overlays: Vec<Overlay>,
        inline_palette: Option<InlinePalette>,
        last_seq: u64,
        transcript: VecDeque<TranscriptEntry>,
        transcript_flushed: usize,
        transcript_flushed_line_offset: usize,
        transcript_scroll: u16,
        transcript_follow: bool,
        transcript_max_scroll: u16,
        transcript_viewport_height: u16,
        tool_events: HashMap<ToolId, String>,
        process_started_lines: HashMap<ProcessId, String>,
        streaming: Option<StreamingState>,
        streaming_entry_active: bool,
        thinking_turn_id: Option<TurnId>,
        active_turn_id: Option<TurnId>,
        turn_inflight_started_at: Option<Instant>,
        turn_inflight_id: Option<TurnId>,
        input: String,
        input_cursor: usize,
        status: Option<String>,
        total_input_tokens_used: u64,
        total_cache_input_tokens_used: u64,
        total_output_tokens_used: u64,
        total_tokens_used: u64,
        token_usage_by_response: HashMap<String, SeenTokenUsage>,
        last_tokens_in_context_window: Option<u64>,
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

    #[derive(Debug, Clone, Default)]
    struct SeenTokenUsage {
        input_tokens: Option<u64>,
        cache_input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
    }
