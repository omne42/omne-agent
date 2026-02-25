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
        attention_state: String,
        #[serde(default)]
        has_plan_ready: bool,
        #[serde(default)]
        has_diff_ready: bool,
        #[serde(default)]
        has_fan_out_linkage_issue: bool,
        #[serde(default)]
        has_fan_out_auto_apply_error: bool,
        #[serde(default)]
        has_fan_in_dependency_blocked: bool,
        #[serde(default)]
        pending_subagent_proxy_approvals: usize,
        #[serde(default)]
        has_test_failed: bool,
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SubagentPendingSummary {
        total: usize,
        states: std::collections::BTreeMap<String, usize>,
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
        AllowedTools,
        ExecpolicyRules,
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
        all_approvals: Vec<ApprovalItem>,
        selected: usize,
        remember: bool,
        filter: ApprovalsFilter,
        subagent_pending_summary: Option<SubagentPendingSummary>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ApprovalsFilter {
        All,
        FailedSubagent,
        RunningSubagent,
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
        versions_for: Option<ArtifactId>,
        versions: Vec<u32>,
        selected_version: usize,
        version_cache: HashMap<ArtifactId, Vec<u32>>,
        selected_version_cache: HashMap<ArtifactId, usize>,
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
        ClosePalette,
        OpenRoot,
        Help,
        NewThread,
        ThreadPicker,
        RefreshThreads,
        ToggleIncludeArchived,
        ToggleLinkageFilter,
        ToggleAutoApplyErrorFilter,
        ToggleFanInDependencyBlockedFilter,
        ToggleSubagentProxyApprovalFilter,
        ClearThreadFilters,
        OpenApprovals,
        OpenProcesses,
        OpenArtifacts,
        PickMode,
        PickModel,
        PickApprovalPolicy,
        PickSandboxPolicy,
        PickSandboxNetworkAccess,
        PickAllowedTools,
        PickExecpolicyRules,
        SetMode(String),
        SetModel(String),
        SetApprovalPolicy(super::CliApprovalPolicy),
        SetSandboxPolicy(super::CliSandboxPolicy),
        SetSandboxNetworkAccess(super::CliSandboxNetworkAccess),
        SetAllowedTools(String),
        ClearAllowedTools,
        ClearExecpolicyRules,
        ApprovalsCycleFilter,
        ApprovalsNextFailed,
        ApprovalsPrevFailed,
        ApprovalsRefresh,
        ApprovalsSelectPrev,
        ApprovalsSelectNext,
        ApprovalsApprove,
        ApprovalsDeny,
        ApprovalsToggleRemember,
        ApprovalsOpenDetails,
        ProcessesRefresh,
        ProcessesSelectPrev,
        ProcessesSelectNext,
        ProcessesInspect,
        ProcessesKill,
        ProcessesInterrupt,
        ArtifactsRefresh,
        ArtifactsSelectPrev,
        ArtifactsSelectNext,
        ArtifactsRead,
        ArtifactsLoadVersions,
        ArtifactsReloadVersions,
        ArtifactsPrevVersion,
        ArtifactsNextVersion,
        ArtifactsLatestVersion,
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

        fn context_label(&self) -> String {
            let title = self.title.trim();
            if title.is_empty() {
                "commands".to_string()
            } else {
                title.to_ascii_lowercase()
            }
        }
    }

    fn build_root_palette(state: &UiState) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        let include_archived = if state.include_archived { "on" } else { "off" };
        let linkage_filter = if state.only_fan_out_linkage_issue {
            "on"
        } else {
            "off"
        };
        let auto_apply_filter = if state.only_fan_out_auto_apply_error {
            "on"
        } else {
            "off"
        };
        let fan_in_filter = if state.only_fan_in_dependency_blocked {
            "on"
        } else {
            "off"
        };
        let subagent_filter = if state.only_subagent_proxy_approval {
            "on"
        } else {
            "off"
        };

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
        items.push(PaletteItem::with_detail(
            format!("archived={include_archived}"),
            "include archived threads",
            PaletteCommand::ToggleIncludeArchived,
        ));
        items.push(PaletteItem::with_detail(
            format!("linkage-filter={linkage_filter}"),
            "thread picker filter",
            PaletteCommand::ToggleLinkageFilter,
        ));
        items.push(PaletteItem::with_detail(
            format!("auto-apply-filter={auto_apply_filter}"),
            "thread picker filter",
            PaletteCommand::ToggleAutoApplyErrorFilter,
        ));
        items.push(PaletteItem::with_detail(
            format!("fan-in-filter={fan_in_filter}"),
            "thread picker filter",
            PaletteCommand::ToggleFanInDependencyBlockedFilter,
        ));
        items.push(PaletteItem::with_detail(
            format!("subagent-filter={subagent_filter}"),
            "thread picker filter",
            PaletteCommand::ToggleSubagentProxyApprovalFilter,
        ));
        items.push(PaletteItem::with_detail(
            "clear-filters",
            "reset thread picker filters",
            PaletteCommand::ClearThreadFilters,
        ));

        if state.active_thread.is_some() {
            let mode = state.header.mode.as_deref().unwrap_or("-");
            let model = state.header.model.as_deref().unwrap_or("-");
            let allowed_tools = state
                .header
                .allowed_tools_count
                .map(|count| count.to_string())
                .unwrap_or_else(|| "*".to_string());
            let execpolicy_rules = state.header.execpolicy_rules_count;

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
            items.push(PaletteItem::with_detail(
                format!("allowed-tools={allowed_tools}"),
                "thread tool allowlist",
                PaletteCommand::PickAllowedTools,
            ));
            items.push(PaletteItem::with_detail(
                format!("execpolicy-rules={execpolicy_rules}"),
                "thread execpolicy rules",
                PaletteCommand::PickExecpolicyRules,
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

    fn build_overlay_palette(state: &UiState) -> Option<CommandPaletteOverlay> {
        match state.overlays.last() {
            Some(Overlay::Approvals(view)) => Some(build_approvals_overlay_palette(view)),
            Some(Overlay::Processes(view)) => Some(build_processes_overlay_palette(view)),
            Some(Overlay::Artifacts(view)) => Some(build_artifacts_overlay_palette(view)),
            Some(Overlay::Text(_))
            | Some(Overlay::CommandPalette(_))
            | None => None,
        }
    }

    fn build_approvals_overlay_palette(view: &ApprovalsOverlay) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::ClosePalette));
        items.push(PaletteItem::with_detail(
            "refresh",
            "reload approvals (r)",
            PaletteCommand::ApprovalsRefresh,
        ));
        items.push(PaletteItem::with_detail(
            "select-prev",
            "select previous approval (↑)",
            PaletteCommand::ApprovalsSelectPrev,
        ));
        items.push(PaletteItem::with_detail(
            "select-next",
            "select next approval (↓)",
            PaletteCommand::ApprovalsSelectNext,
        ));
        items.push(PaletteItem::with_detail(
            format!(
                "filter={} ({}/{})",
                approval_filter_label(view.filter),
                view.approvals.len(),
                view.all_approvals.len()
            ),
            "cycle approvals filter (t)",
            PaletteCommand::ApprovalsCycleFilter,
        ));
        items.push(PaletteItem::with_detail(
            "next-failed",
            "jump to next failed/error subagent approval (f)",
            PaletteCommand::ApprovalsNextFailed,
        ));
        items.push(PaletteItem::with_detail(
            "prev-failed",
            "jump to previous failed/error subagent approval (F)",
            PaletteCommand::ApprovalsPrevFailed,
        ));
        items.push(PaletteItem::with_detail(
            "approve",
            "approve selected item (y)",
            PaletteCommand::ApprovalsApprove,
        ));
        items.push(PaletteItem::with_detail(
            "deny",
            "deny selected item (n)",
            PaletteCommand::ApprovalsDeny,
        ));
        items.push(PaletteItem::with_detail(
            format!("remember={}", if view.remember { "on" } else { "off" }),
            "toggle remember (m)",
            PaletteCommand::ApprovalsToggleRemember,
        ));
        items.push(PaletteItem::with_detail(
            "details",
            "open selected approval details (Enter)",
            PaletteCommand::ApprovalsOpenDetails,
        ));
        CommandPaletteOverlay::new("approvals", items)
    }

    fn build_processes_overlay_palette(view: &ProcessesOverlay) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::ClosePalette));
        items.push(PaletteItem::with_detail(
            format!("refresh ({})", view.processes.len()),
            "reload process list (r)",
            PaletteCommand::ProcessesRefresh,
        ));
        items.push(PaletteItem::with_detail(
            "select-prev",
            "select previous process (↑)",
            PaletteCommand::ProcessesSelectPrev,
        ));
        items.push(PaletteItem::with_detail(
            "select-next",
            "select next process (↓)",
            PaletteCommand::ProcessesSelectNext,
        ));
        items.push(PaletteItem::with_detail(
            "inspect",
            "inspect selected process (Enter/i)",
            PaletteCommand::ProcessesInspect,
        ));
        items.push(PaletteItem::with_detail(
            "kill",
            "kill selected process (k)",
            PaletteCommand::ProcessesKill,
        ));
        items.push(PaletteItem::with_detail(
            "interrupt",
            "interrupt selected process (x)",
            PaletteCommand::ProcessesInterrupt,
        ));
        CommandPaletteOverlay::new("processes", items)
    }

    fn build_artifacts_overlay_palette(view: &ArtifactsOverlay) -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("back", PaletteCommand::ClosePalette));
        items.push(PaletteItem::with_detail(
            format!("refresh ({})", view.artifacts.len()),
            "reload artifact list (r)",
            PaletteCommand::ArtifactsRefresh,
        ));
        items.push(PaletteItem::with_detail(
            "select-prev",
            "select previous artifact (↑)",
            PaletteCommand::ArtifactsSelectPrev,
        ));
        items.push(PaletteItem::with_detail(
            "select-next",
            "select next artifact (↓)",
            PaletteCommand::ArtifactsSelectNext,
        ));
        items.push(PaletteItem::with_detail(
            "read",
            "read selected artifact (Enter/i)",
            PaletteCommand::ArtifactsRead,
        ));
        items.push(PaletteItem::with_detail(
            "versions",
            "load versions for selected artifact (v)",
            PaletteCommand::ArtifactsLoadVersions,
        ));
        items.push(PaletteItem::with_detail(
            "versions-reload",
            "force reload versions (R)",
            PaletteCommand::ArtifactsReloadVersions,
        ));
        items.push(PaletteItem::with_detail(
            "version-prev",
            "older version ([)",
            PaletteCommand::ArtifactsPrevVersion,
        ));
        items.push(PaletteItem::with_detail(
            "version-next",
            "newer version (])",
            PaletteCommand::ArtifactsNextVersion,
        ));
        items.push(PaletteItem::with_detail(
            "version-latest",
            "jump to latest version (0)",
            PaletteCommand::ArtifactsLatestVersion,
        ));
        CommandPaletteOverlay::new("artifacts", items)
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
            items.push(PaletteItem::with_detail(
                "/allowed-tools",
                "thread tool allowlist",
                PaletteCommand::PickAllowedTools,
            ));
            items.push(PaletteItem::with_detail(
                "/execpolicy-rules",
                "thread execpolicy rules",
                PaletteCommand::PickExecpolicyRules,
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

    const KNOWN_ALLOWED_TOOLS: &[&str] = &[
        "file/read",
        "file/glob",
        "file/grep",
        "file/write",
        "file/patch",
        "file/edit",
        "file/delete",
        "fs/mkdir",
        "repo/search",
        "repo/index",
        "repo/symbols",
        "mcp/list_servers",
        "mcp/list_tools",
        "mcp/list_resources",
        "mcp/call",
        "artifact/write",
        "artifact/list",
        "artifact/read",
        "artifact/versions",
        "artifact/delete",
        "process/start",
        "process/list",
        "process/inspect",
        "process/kill",
        "process/interrupt",
        "process/tail",
        "process/follow",
    ];

    fn build_inline_allowed_tools_palette() -> CommandPaletteOverlay {
        let mut items = Vec::<PaletteItem>::new();
        items.push(PaletteItem::new("clear", PaletteCommand::ClearAllowedTools));
        for tool in KNOWN_ALLOWED_TOOLS {
            items.push(PaletteItem::new(
                (*tool).to_string(),
                PaletteCommand::SetAllowedTools((*tool).to_string()),
            ));
        }
        CommandPaletteOverlay::new("allowed-tools", items)
    }

    fn build_inline_execpolicy_rules_palette() -> CommandPaletteOverlay {
        let items = vec![
            PaletteItem::new("clear", PaletteCommand::ClearExecpolicyRules),
            PaletteItem::new("type comma-separated paths", PaletteCommand::Noop),
        ];
        CommandPaletteOverlay::new("execpolicy-rules", items)
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
        out.push_str("  Ctrl-K (overlay) overlay command palette\n\n");
        out.push_str("slash commands:\n\n");
        out.push_str("  /allowed-tools <a,b>      set thread allowed_tools\n");
        out.push_str("  /allowed-tools clear      clear thread allowed_tools\n");
        out.push_str("  /execpolicy-rules <a,b>   set thread execpolicy_rules\n");
        out.push_str("  /execpolicy-rules clear   clear thread execpolicy_rules\n\n");
        out.push_str("thread picker:\n\n");
        out.push_str("  n                new thread\n");
        out.push_str("  h                toggle include archived\n");
        out.push_str("  l                toggle linkage filter\n");
        out.push_str("  a                toggle auto-apply-error filter\n");
        out.push_str("  b                toggle fan-in-blocked filter\n");
        out.push_str("  s                toggle subagent-approval filter\n");
        out.push_str("  c                clear thread picker filters\n");
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
        ArtifactVersions {
            thread_id: ThreadId,
            artifact_id: ArtifactId,
            approval_id: ApprovalId,
        },
        ArtifactRead {
            thread_id: ThreadId,
            artifact_id: ArtifactId,
            max_bytes: u64,
            version: Option<u32>,
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
                Self::ArtifactVersions { approval_id, .. } => *approval_id,
                Self::ArtifactRead { approval_id, .. } => *approval_id,
                Self::ProcessInspect { approval_id, .. } => *approval_id,
                Self::ProcessKill { approval_id, .. } => *approval_id,
                Self::ProcessInterrupt { approval_id, .. } => *approval_id,
            }
        }
    }

    type ApprovalListResponse = omne_app_server_protocol::ApprovalListResponse;

    type ApprovalItem = omne_app_server_protocol::ApprovalListItem;

    type ApprovalRequestInfo = omne_app_server_protocol::ApprovalRequestInfo;

    type ProcessListResponse = omne_app_server_protocol::ProcessListResponse;

    type ProcessStatus = omne_app_server_protocol::ProcessStatus;

    type ProcessInfo = omne_app_server_protocol::ProcessInfo;

    type ProcessInspectResponse = omne_app_server_protocol::ProcessInspectResponse;

    type ArtifactReadResponse = omne_app_server_protocol::ArtifactReadResponse;

    type ArtifactVersionsResponse = omne_app_server_protocol::ArtifactVersionsResponse;

    #[derive(Debug, Clone, Default)]
    struct HeaderState {
        mode: Option<String>,
        provider: Option<String>,
        model: Option<String>,
        thinking: Option<String>,
        mcp_enabled: bool,
        model_context_window: Option<u64>,
        allowed_tools_count: Option<usize>,
        execpolicy_rules_count: usize,
    }

    fn env_truthy(key: &str) -> bool {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
    }

    struct UiState {
        include_archived: bool,
        only_fan_out_linkage_issue: bool,
        only_fan_out_auto_apply_error: bool,
        only_fan_in_dependency_blocked: bool,
        only_subagent_proxy_approval: bool,
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
        subagent_pending_summary: Option<SubagentPendingSummary>,
        subagent_pending_summary_needs_refresh: bool,
        input: String,
        status: Option<String>,
        status_expires_at: Option<Instant>,
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

    fn thread_picker_filter_label(
        only_fan_out_linkage_issue: bool,
        only_fan_out_auto_apply_error: bool,
        only_fan_in_dependency_blocked: bool,
        only_subagent_proxy_approval: bool,
    ) -> String {
        let mut labels = Vec::<&str>::new();
        if only_fan_out_linkage_issue {
            labels.push("link");
        }
        if only_fan_out_auto_apply_error {
            labels.push("auto");
        }
        if only_fan_in_dependency_blocked {
            labels.push("fanin");
        }
        if only_subagent_proxy_approval {
            labels.push("subagent");
        }
        if labels.is_empty() {
            "all".to_string()
        } else {
            labels.join("+")
        }
    }
