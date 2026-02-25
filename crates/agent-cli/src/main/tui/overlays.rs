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

    fn approval_action_label(request: &ApprovalRequestInfo) -> String {
        super::approval_action_label_from_parts(request.action_id, Some(request.action.as_str()))
    }

    fn is_subagent_proxy_approval_request(request: &ApprovalRequestInfo) -> bool {
        request.action_id
            == Some(omne_app_server_protocol::ThreadApprovalActionId::SubagentProxyApproval)
            || request.action.eq_ignore_ascii_case("subagent/proxy_approval")
    }

    fn approval_subagent_state_hint(request: &ApprovalRequestInfo) -> Option<String> {
        if !is_subagent_proxy_approval_request(request) {
            return None;
        }
        request
            .summary
            .as_ref()
            .and_then(|summary| summary.child_attention_state.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
    }

    fn approval_subagent_state_color(
        request: &ApprovalRequestInfo,
    ) -> Option<ratatui::style::Color> {
        let state = approval_subagent_state_hint(request)?;
        match state.as_str() {
            "failed" | "error" => Some(ratatui::style::Color::LightRed),
            "running" => Some(ratatui::style::Color::Yellow),
            _ => None,
        }
    }

    fn is_failed_subagent_proxy_approval_request(request: &ApprovalRequestInfo) -> bool {
        matches!(
            approval_subagent_state_hint(request).as_deref(),
            Some("failed" | "error")
        )
    }

    fn next_failed_subagent_approval_index(items: &[ApprovalItem], current: usize) -> Option<usize> {
        if items.is_empty() {
            return None;
        }
        for offset in 1..=items.len() {
            let idx = (current + offset) % items.len();
            if is_failed_subagent_proxy_approval_request(&items[idx].request) {
                return Some(idx);
            }
        }
        None
    }

    fn prev_failed_subagent_approval_index(items: &[ApprovalItem], current: usize) -> Option<usize> {
        if items.is_empty() {
            return None;
        }
        for offset in 1..=items.len() {
            let idx = (current + items.len() - (offset % items.len())) % items.len();
            if is_failed_subagent_proxy_approval_request(&items[idx].request) {
                return Some(idx);
            }
        }
        None
    }

    fn failed_subagent_approval_count(items: &[ApprovalItem]) -> usize {
        items
            .iter()
            .filter(|item| is_failed_subagent_proxy_approval_request(&item.request))
            .count()
    }

    fn approval_matches_filter(item: &ApprovalItem, filter: ApprovalsFilter) -> bool {
        match filter {
            ApprovalsFilter::All => true,
            ApprovalsFilter::FailedSubagent => {
                is_failed_subagent_proxy_approval_request(&item.request)
            }
            ApprovalsFilter::RunningSubagent => matches!(
                approval_subagent_state_hint(&item.request).as_deref(),
                Some("running")
            ),
        }
    }

    fn approval_filter_label(filter: ApprovalsFilter) -> &'static str {
        match filter {
            ApprovalsFilter::All => "all",
            ApprovalsFilter::FailedSubagent => "failed",
            ApprovalsFilter::RunningSubagent => "running",
        }
    }

    fn next_approvals_filter(filter: ApprovalsFilter) -> ApprovalsFilter {
        match filter {
            ApprovalsFilter::All => ApprovalsFilter::FailedSubagent,
            ApprovalsFilter::FailedSubagent => ApprovalsFilter::RunningSubagent,
            ApprovalsFilter::RunningSubagent => ApprovalsFilter::All,
        }
    }

    fn rebuild_filtered_approvals(
        view: &mut ApprovalsOverlay,
        preferred_approval: Option<ApprovalId>,
    ) {
        view.approvals = view
            .all_approvals
            .iter()
            .filter(|item| approval_matches_filter(item, view.filter))
            .cloned()
            .collect();
        view.selected = select_approval(view.approvals.as_slice(), preferred_approval);
    }

    fn new_approvals_overlay(
        thread_id: ThreadId,
        approvals: Vec<ApprovalItem>,
        selected: usize,
        subagent_pending_summary: Option<SubagentPendingSummary>,
    ) -> ApprovalsOverlay {
        let selected_approval = approvals.get(selected).map(|item| item.request.approval_id);
        let mut overlay = ApprovalsOverlay {
            thread_id,
            approvals: Vec::new(),
            all_approvals: approvals,
            selected: 0,
            remember: false,
            filter: ApprovalsFilter::All,
            subagent_pending_summary,
        };
        rebuild_filtered_approvals(&mut overlay, selected_approval);
        overlay
    }

    fn approval_sort_priority(request: &ApprovalRequestInfo) -> u8 {
        if let Some(state) = approval_subagent_state_hint(request) {
            return match state.as_str() {
                "failed" | "error" => 0,
                "running" => 1,
                _ => 2,
            };
        }
        3
    }

    fn sort_approvals_for_overlay(items: &mut [ApprovalItem]) {
        items.sort_by(|a, b| {
            approval_sort_priority(&a.request)
                .cmp(&approval_sort_priority(&b.request))
                .then_with(|| a.request.requested_at.cmp(&b.request.requested_at))
                .then_with(|| {
                    a.request
                        .approval_id
                        .to_string()
                        .cmp(&b.request.approval_id.to_string())
                })
        });
    }

    fn approval_summary_hint(
        summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
    ) -> Option<String> {
        let hint = super::approval_summary_context_hint_from_summary(summary).or_else(|| {
            summary
                .approve_cmd
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("approve_cmd={value}"))
        })?;
        let Some((key, value)) = hint.split_once('=') else {
            return Some(hint);
        };
        match key {
            "path" | "requirement" | "argv" | "cwd" => Some(format!("{key}={}", right_elide(value, 48))),
            "approve_cmd" => Some(format!("{key}={}", right_elide(value, 64))),
            _ => Some(hint),
        }
    }

    fn approval_subagent_link(
        summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
    ) -> Option<String> {
        super::approval_subagent_link_from_summary(summary)
    }

    fn approval_summary_lines(
        summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
    ) -> Vec<String> {
        super::approval_summary_lines_from_summary(summary)
    }

    fn approval_approve_cmd(
        summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
    ) -> Option<String> {
        super::approval_approve_cmd_from_summary(summary)
    }

    fn approval_deny_cmd(
        summary: &omne_app_server_protocol::ThreadAttentionPendingApprovalSummary,
    ) -> Option<String> {
        super::approval_deny_cmd_from_summary(summary)
    }

    async fn load_approvals(
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<ApprovalItem>> {
        let parsed: ApprovalListResponse = app.approval_list(thread_id, false).await?;
        let mut approvals = parsed.approvals;
        sort_approvals_for_overlay(approvals.as_mut_slice());
        Ok(approvals)
    }

    async fn load_processes(
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<Vec<ProcessInfo>> {
        let parsed: ProcessListResponse = app.process_list(Some(thread_id)).await?;
        Ok(parsed.processes)
    }

    async fn load_artifact_versions(
        app: &mut super::App,
        thread_id: ThreadId,
        artifact_id: ArtifactId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<RpcActionOutcome<ArtifactVersionsResponse>> {
        rpc_artifact_versions_tui_outcome(
            app,
            omne_app_server_protocol::ArtifactVersionsParams {
                thread_id,
                turn_id: None,
                approval_id,
                artifact_id,
            },
        )
        .await
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

    enum ApprovalsLocalKeyResult {
        Unhandled,
        Handled(Option<String>),
    }

    enum ProcessesLocalKeyResult {
        Unhandled,
        Handled,
    }

    enum ArtifactsLocalKeyResult {
        Unhandled,
        Handled(Option<String>),
    }

    fn handle_local_approvals_key(
        view: &mut ApprovalsOverlay,
        code: KeyCode,
    ) -> ApprovalsLocalKeyResult {
        match code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
                ApprovalsLocalKeyResult::Handled(None)
            }
            KeyCode::Down => {
                if !view.approvals.is_empty() {
                    view.selected = (view.selected + 1).min(view.approvals.len() - 1);
                }
                ApprovalsLocalKeyResult::Handled(None)
            }
            KeyCode::Char('f') => {
                if let Some(next) =
                    next_failed_subagent_approval_index(view.approvals.as_slice(), view.selected)
                {
                    view.selected = next;
                    let approval_id = view
                        .approvals
                        .get(next)
                        .map(|item| item.request.approval_id.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    ApprovalsLocalKeyResult::Handled(Some(format!(
                        "approvals: jumped to failed subagent approval {approval_id}"
                    )))
                } else {
                    ApprovalsLocalKeyResult::Handled(Some(
                        "approvals: no failed subagent approvals".to_string(),
                    ))
                }
            }
            KeyCode::Char('F') => {
                if let Some(prev) =
                    prev_failed_subagent_approval_index(view.approvals.as_slice(), view.selected)
                {
                    view.selected = prev;
                    let approval_id = view
                        .approvals
                        .get(prev)
                        .map(|item| item.request.approval_id.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    ApprovalsLocalKeyResult::Handled(Some(format!(
                        "approvals: jumped to previous failed subagent approval {approval_id}"
                    )))
                } else {
                    ApprovalsLocalKeyResult::Handled(Some(
                        "approvals: no failed subagent approvals".to_string(),
                    ))
                }
            }
            KeyCode::Char('t') => {
                let current = view
                    .approvals
                    .get(view.selected)
                    .map(|item| item.request.approval_id);
                view.filter = next_approvals_filter(view.filter);
                rebuild_filtered_approvals(view, current);
                ApprovalsLocalKeyResult::Handled(Some(format!(
                    "approvals filter={}",
                    approval_filter_label(view.filter)
                )))
            }
            KeyCode::Char('m') => {
                view.remember = !view.remember;
                ApprovalsLocalKeyResult::Handled(Some(format!("remember={}", view.remember)))
            }
            _ => ApprovalsLocalKeyResult::Unhandled,
        }
    }

    fn handle_local_processes_key(
        view: &mut ProcessesOverlay,
        code: KeyCode,
    ) -> ProcessesLocalKeyResult {
        match code {
            KeyCode::Up => {
                view.selected = view.selected.saturating_sub(1);
                ProcessesLocalKeyResult::Handled
            }
            KeyCode::Down => {
                if !view.processes.is_empty() {
                    view.selected = (view.selected + 1).min(view.processes.len() - 1);
                }
                ProcessesLocalKeyResult::Handled
            }
            _ => ProcessesLocalKeyResult::Unhandled,
        }
    }

    fn handle_local_artifacts_key(
        view: &mut ArtifactsOverlay,
        code: KeyCode,
    ) -> ArtifactsLocalKeyResult {
        match code {
            KeyCode::Up => {
                let current = view
                    .artifacts
                    .get(view.selected)
                    .map(|artifact| artifact.artifact_id);
                view.selected = view.selected.saturating_sub(1);
                sync_versions_if_selected_artifact_changed(view, current);
                ArtifactsLocalKeyResult::Handled(None)
            }
            KeyCode::Down => {
                let current = view
                    .artifacts
                    .get(view.selected)
                    .map(|artifact| artifact.artifact_id);
                if !view.artifacts.is_empty() {
                    view.selected = (view.selected + 1).min(view.artifacts.len() - 1);
                }
                sync_versions_if_selected_artifact_changed(view, current);
                ArtifactsLocalKeyResult::Handled(None)
            }
            KeyCode::Left | KeyCode::Char('[') => {
                if view.versions_for.is_some() && !view.versions.is_empty() {
                    view.selected_version =
                        (view.selected_version + 1).min(view.versions.len() - 1);
                    if let Some(artifact_id) = view.versions_for {
                        view.selected_version_cache
                            .insert(artifact_id, view.selected_version);
                    }
                }
                ArtifactsLocalKeyResult::Handled(None)
            }
            KeyCode::Right | KeyCode::Char(']') => {
                if view.versions_for.is_some() && !view.versions.is_empty() {
                    view.selected_version = view.selected_version.saturating_sub(1);
                    if let Some(artifact_id) = view.versions_for {
                        view.selected_version_cache
                            .insert(artifact_id, view.selected_version);
                    }
                }
                ArtifactsLocalKeyResult::Handled(None)
            }
            KeyCode::Char('0') => {
                let Some(meta) = view.artifacts.get(view.selected).cloned() else {
                    return ArtifactsLocalKeyResult::Handled(None);
                };
                if activate_cached_versions_for_artifact(view, meta.artifact_id, meta.version)
                    || (view.versions_for == Some(meta.artifact_id) && !view.versions.is_empty())
                {
                    view.selected_version =
                        view.versions.iter().position(|v| *v == meta.version).unwrap_or(0);
                    view.selected_version_cache
                        .insert(meta.artifact_id, view.selected_version);
                    ArtifactsLocalKeyResult::Handled(Some(format!(
                        "artifact version reset to latest: {}",
                        meta.version
                    )))
                } else {
                    ArtifactsLocalKeyResult::Handled(Some(
                        "artifact read will use latest version".to_string(),
                    ))
                }
            }
            _ => ArtifactsLocalKeyResult::Unhandled,
        }
    }

    async fn handle_key_approvals_overlay(
        app: &mut super::App,
        key: KeyEvent,
        view: &mut ApprovalsOverlay,
    ) -> anyhow::Result<(OverlayOp, Option<String>, Option<ApprovalId>)> {
        let mut status = None::<String>;
        let mut decided = None::<ApprovalId>;

        if let ApprovalsLocalKeyResult::Handled(local_status) =
            handle_local_approvals_key(view, key.code)
        {
            return Ok((OverlayOp::None, local_status, None));
        }

        match key.code {
            KeyCode::Char('r') => {
                let current = view
                    .approvals
                    .get(view.selected)
                    .map(|item| item.request.approval_id);
                view.all_approvals = load_approvals(app, view.thread_id).await?;
                rebuild_filtered_approvals(view, current);
                refresh_approvals_overlay_subagent_summary(app, view).await;
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
                view.all_approvals = load_approvals(app, view.thread_id).await?;
                rebuild_filtered_approvals(view, Some(approval_id));
                refresh_approvals_overlay_subagent_summary(app, view).await;
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
                let text = build_approval_details_text(item);
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

    async fn refresh_approvals_overlay_subagent_summary(
        app: &mut super::App,
        view: &mut ApprovalsOverlay,
    ) {
        let summary = app
            .thread_attention(view.thread_id)
            .await
            .ok()
            .and_then(|attention| {
                summarize_subagent_pending_approvals_for_overlay(
                    attention.pending_approvals.as_slice(),
                )
            });
        view.subagent_pending_summary = summary;
    }

    fn summarize_subagent_pending_approvals_for_overlay(
        approvals: &[omne_app_server_protocol::ThreadAttentionPendingApproval],
    ) -> Option<SubagentPendingSummary> {
        let mut total = 0usize;
        let mut states = std::collections::BTreeMap::<String, usize>::new();

        for pending in approvals {
            if !super::is_subagent_proxy_pending_approval(pending) {
                continue;
            }
            total = total.saturating_add(1);
            let state = pending
                .summary
                .as_ref()
                .and_then(|summary| summary.child_attention_state.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_else(|| "unknown".to_string());
            *states.entry(state).or_default() += 1;
        }

        (total > 0).then_some(SubagentPendingSummary { total, states })
    }

    fn build_approval_details_text(item: &ApprovalItem) -> String {
        let mut text = String::new();
        text.push_str(&format!(
            "approval_id: {}\nrequested_at: {}\naction: {}\n",
            item.request.approval_id,
            item.request.requested_at,
            item.request.action
        ));
        text.push_str(&format!("action_label: {}\n", approval_action_label(&item.request)));
        if let Some(action_id) = item.request.action_id {
            if let Ok(raw) = serde_json::to_string(&action_id) {
                text.push_str(&format!("action_id: {}\n", raw.trim_matches('"')));
            }
        }
        if let Some(turn_id) = item.request.turn_id {
            text.push_str(&format!("turn_id: {turn_id}\n"));
        }
        if let Some(summary) = item.request.summary.as_ref() {
            if let Some(subagent_link) = approval_subagent_link(summary) {
                text.push_str(&format!("subagent_proxy: {subagent_link}\n"));
            }
            if let Some(approve_cmd) = approval_approve_cmd(summary) {
                text.push_str("\n# Quick Command\n\n");
                text.push_str("approve: ");
                text.push_str(&approve_cmd);
                text.push('\n');
                if let Some(deny_cmd) = approval_deny_cmd(summary) {
                    text.push_str("deny: ");
                    text.push_str(&deny_cmd);
                    text.push('\n');
                }
            }
            text.push_str("\n# Summary\n\n");
            let lines = approval_summary_lines(summary);
            if lines.is_empty() {
                text.push_str("(empty)\n");
            } else {
                for line in lines {
                    text.push_str(&line);
                    text.push('\n');
                }
            }
        }
        if let Some(decision) = &item.decision {
            text.push_str(&format!(
                "\n# Decision\n\ndecision: {}\ndecided_at: {}\nremember: {}\n",
                approval_decision_str(decision.decision),
                decision.decided_at,
                decision.remember
            ));
            if let Some(reason) = decision.reason.as_deref().filter(|s| !s.trim().is_empty()) {
                text.push_str(&format!("reason: {reason}\n"));
            }
        }
        text.push_str("\n# Params\n\n");
        text.push_str(
            &serde_json::to_string_pretty(&item.request.params)
                .unwrap_or_else(|_| item.request.params.to_string()),
        );
        text
    }

    async fn handle_key_processes_overlay(
        app: &mut super::App,
        key: KeyEvent,
        view: &mut ProcessesOverlay,
    ) -> anyhow::Result<(OverlayOp, Option<String>, Option<PendingAction>)> {
        if let ProcessesLocalKeyResult::Handled = handle_local_processes_key(view, key.code) {
            return Ok((OverlayOp::None, None, None));
        }

        let mut status = None::<String>;

        match key.code {
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
                let outcome = rpc_process_inspect_tui_outcome(
                    app,
                    omne_app_server_protocol::ProcessInspectParams {
                        process_id: info.process_id,
                        turn_id: None,
                        approval_id: None,
                        max_lines: Some(max_lines),
                    },
                )
                .await?;
                match outcome {
                    RpcActionOutcome::NeedsApproval {
                        thread_id,
                        approval_id,
                    } => {
                        let approvals = load_approvals(app, thread_id).await?;
                        let selected = select_approval(&approvals, Some(approval_id));
                        let overlay = Overlay::Approvals(new_approvals_overlay(
                            thread_id,
                            approvals,
                            selected,
                            None,
                        ));
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
                    RpcActionOutcome::Denied { summary } => {
                        status = Some(format!("process/inspect denied: {summary}"));
                        return Ok((OverlayOp::None, status, None));
                    }
                    RpcActionOutcome::Ok(parsed) => {
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
                }
            }
            KeyCode::Char('k') => {
                let Some(info) = view.processes.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let outcome = rpc_process_kill_tui_outcome(
                    app,
                    omne_app_server_protocol::ProcessKillParams {
                        process_id: info.process_id,
                        turn_id: None,
                        approval_id: None,
                        reason: Some("tui kill".to_string()),
                    },
                )
                .await?;
                match outcome {
                    RpcActionOutcome::NeedsApproval {
                        thread_id,
                        approval_id,
                    } => {
                        let approvals = load_approvals(app, thread_id).await?;
                        let selected = select_approval(&approvals, Some(approval_id));
                        let overlay = Overlay::Approvals(new_approvals_overlay(
                            thread_id,
                            approvals,
                            selected,
                            None,
                        ));
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
                    RpcActionOutcome::Denied { summary } => {
                        status = Some(format!("process/kill denied: {summary}"));
                        return Ok((OverlayOp::None, status, None));
                    }
                    RpcActionOutcome::Ok(_) => {
                        status = Some(format!("kill requested: {}", info.process_id));
                        view.processes = load_processes(app, view.thread_id).await?;
                        view.selected = select_process(&view.processes, Some(info.process_id));
                    }
                }
            }
            KeyCode::Char('x') => {
                let Some(info) = view.processes.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let outcome = rpc_process_interrupt_tui_outcome(
                    app,
                    omne_app_server_protocol::ProcessInterruptParams {
                        process_id: info.process_id,
                        turn_id: None,
                        approval_id: None,
                        reason: Some("tui interrupt".to_string()),
                    },
                )
                .await?;
                match outcome {
                    RpcActionOutcome::NeedsApproval {
                        thread_id,
                        approval_id,
                    } => {
                        let approvals = load_approvals(app, thread_id).await?;
                        let selected = select_approval(&approvals, Some(approval_id));
                        let overlay = Overlay::Approvals(new_approvals_overlay(
                            thread_id,
                            approvals,
                            selected,
                            None,
                        ));
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
                    RpcActionOutcome::Denied { summary } => {
                        status = Some(format!("process/interrupt denied: {summary}"));
                        return Ok((OverlayOp::None, status, None));
                    }
                    RpcActionOutcome::Ok(_) => {
                        status = Some(format!("interrupt requested: {}", info.process_id));
                        view.processes = load_processes(app, view.thread_id).await?;
                        view.selected = select_process(&view.processes, Some(info.process_id));
                    }
                }
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
        if let ArtifactsLocalKeyResult::Handled(status) =
            handle_local_artifacts_key(view, key.code)
        {
            return Ok((OverlayOp::None, status, None));
        }

        let mut status = None::<String>;

        match key.code {
            KeyCode::Char('r') => {
                let current = view
                    .artifacts
                    .get(view.selected)
                    .map(|artifact| artifact.artifact_id);
                let outcome = rpc_artifact_list_tui_outcome(
                    app,
                    omne_app_server_protocol::ArtifactListParams {
                        thread_id: view.thread_id,
                        turn_id: None,
                        approval_id: None,
                    },
                )
                .await?;
                match outcome {
                    RpcActionOutcome::NeedsApproval {
                        thread_id,
                        approval_id,
                    } => {
                        let approvals = load_approvals(app, thread_id).await?;
                        let selected = select_approval(&approvals, Some(approval_id));
                        let overlay = Overlay::Approvals(new_approvals_overlay(
                            thread_id,
                            approvals,
                            selected,
                            None,
                        ));
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
                    RpcActionOutcome::Denied { summary } => {
                        status = Some(format!("artifact/list denied: {summary}"));
                        return Ok((OverlayOp::None, status, None));
                    }
                    RpcActionOutcome::Ok(parsed) => {
                        if !parsed.errors.is_empty() {
                            status = Some(format!("artifact/list errors: {}", parsed.errors.len()));
                        }
                        view.artifacts = parsed.artifacts;
                        view.selected = select_artifact(&view.artifacts, current);
                        let existing = view
                            .artifacts
                            .iter()
                            .map(|artifact| artifact.artifact_id)
                            .collect::<HashSet<_>>();
                        view.version_cache.retain(|artifact_id, _| existing.contains(artifact_id));
                        view.selected_version_cache
                            .retain(|artifact_id, _| existing.contains(artifact_id));
                        sync_versions_for_selected_artifact(view);
                    }
                }
            }
            KeyCode::Char('v') | KeyCode::Char('R') => {
                let force_reload = matches!(key.code, KeyCode::Char('R'));
                let Some(meta) = view.artifacts.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                if !force_reload
                    && activate_cached_versions_for_artifact(view, meta.artifact_id, meta.version)
                {
                    let selected_version = selected_artifact_version(view, &meta);
                    status = Some(format!(
                        "artifact versions cached: {} (latest={}, selected={selected_version})",
                        view.versions.len(),
                        meta.version,
                        selected_version = selected_version.unwrap_or(meta.version)
                    ));
                    return Ok((OverlayOp::None, status, None));
                }
                let outcome =
                    load_artifact_versions(app, view.thread_id, meta.artifact_id, None).await?;
                match outcome {
                    RpcActionOutcome::NeedsApproval {
                        thread_id,
                        approval_id,
                    } => {
                        let approvals = load_approvals(app, thread_id).await?;
                        let selected = select_approval(&approvals, Some(approval_id));
                        let overlay = Overlay::Approvals(new_approvals_overlay(
                            thread_id,
                            approvals,
                            selected,
                            None,
                        ));
                        status = Some(format!("artifact/versions needs approval: {approval_id}"));
                        return Ok((
                            OverlayOp::Push(overlay),
                            status,
                            Some(PendingAction::ArtifactVersions {
                                thread_id,
                                artifact_id: meta.artifact_id,
                                approval_id,
                            }),
                        ));
                    }
                    RpcActionOutcome::Denied { summary } => {
                        status = Some(format!("artifact/versions denied: {summary}"));
                        return Ok((OverlayOp::None, status, None));
                    }
                    RpcActionOutcome::Ok(parsed) => {
                        apply_versions_to_artifacts_overlay(view, meta.artifact_id, &parsed);
                        let selected_version = selected_artifact_version(view, &meta);
                        let verb = if force_reload { "reloaded" } else { "loaded" };
                        status = Some(format!(
                            "artifact versions {verb}: {} (latest={}, selected={selected_version})",
                            parsed.versions.len(),
                            parsed.latest_version,
                            selected_version = selected_version.unwrap_or(meta.version)
                        ));
                    }
                }
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                let Some(meta) = view.artifacts.get(view.selected).cloned() else {
                    return Ok((OverlayOp::None, None, None));
                };
                let version = selected_artifact_version(view, &meta);
                let max_bytes = 256 * 1024u64;
                let outcome = match rpc_artifact_read_tui_outcome(
                    app,
                    omne_app_server_protocol::ArtifactReadParams {
                        thread_id: view.thread_id,
                        turn_id: None,
                        approval_id: None,
                        artifact_id: meta.artifact_id,
                        version,
                        max_bytes: Some(max_bytes),
                    },
                )
                .await
                {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        if let Some(hint) = artifact_read_error_hint(&err) {
                            status = Some(hint);
                            return Ok((OverlayOp::None, status, None));
                        }
                        return Err(err);
                    }
                };
                match outcome {
                    RpcActionOutcome::NeedsApproval {
                        thread_id,
                        approval_id,
                    } => {
                        let approvals = load_approvals(app, thread_id).await?;
                        let selected = select_approval(&approvals, Some(approval_id));
                        let overlay = Overlay::Approvals(new_approvals_overlay(
                            thread_id,
                            approvals,
                            selected,
                            None,
                        ));
                        status = Some(format!("artifact/read needs approval: {approval_id}"));
                        return Ok((
                            OverlayOp::Push(overlay),
                            status,
                            Some(PendingAction::ArtifactRead {
                                thread_id,
                                artifact_id: meta.artifact_id,
                                max_bytes,
                                version,
                                approval_id,
                            }),
                        ));
                    }
                    RpcActionOutcome::Denied { summary } => {
                        status = Some(format!("artifact/read denied: {summary}"));
                        return Ok((OverlayOp::None, status, None));
                    }
                    RpcActionOutcome::Ok(parsed) => {
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
                }
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

    fn sync_versions_if_selected_artifact_changed(
        view: &mut ArtifactsOverlay,
        previous: Option<ArtifactId>,
    ) {
        let current = view
            .artifacts
            .get(view.selected)
            .map(|artifact| artifact.artifact_id);
        if current != previous {
            sync_versions_for_selected_artifact(view);
        }
    }

    fn sync_versions_for_selected_artifact(view: &mut ArtifactsOverlay) {
        let Some((artifact_id, latest_version)) = view
            .artifacts
            .get(view.selected)
            .map(|artifact| (artifact.artifact_id, artifact.version))
        else {
            view.versions_for = None;
            view.versions.clear();
            view.selected_version = 0;
            return;
        };

        if view.versions_for == Some(artifact_id) {
            if versions_match_latest(&view.versions, latest_version) {
                if view.selected_version >= view.versions.len() {
                    view.selected_version = 0;
                }
                view.selected_version_cache
                    .insert(artifact_id, view.selected_version);
                return;
            }
        }

        if activate_cached_versions_for_artifact(view, artifact_id, latest_version) {
            return;
        }

        view.versions_for = None;
        view.versions.clear();
        view.selected_version = 0;
    }

    fn versions_match_latest(versions: &[u32], latest: u32) -> bool {
        if versions.is_empty() {
            return false;
        }
        versions.first().copied() == Some(latest) && versions.contains(&latest)
    }

    fn activate_cached_versions_for_artifact(
        view: &mut ArtifactsOverlay,
        artifact_id: ArtifactId,
        latest_version: u32,
    ) -> bool {
        let Some(cached) = view.version_cache.get(&artifact_id).cloned() else {
            return false;
        };

        if !versions_match_latest(&cached, latest_version) {
            view.version_cache.remove(&artifact_id);
            view.selected_version_cache.remove(&artifact_id);
            return false;
        }

        view.versions_for = Some(artifact_id);
        view.versions = cached;
        let default_selected = view
            .versions
            .iter()
            .position(|v| *v == latest_version)
            .unwrap_or(0);
        let selected = view
            .selected_version_cache
            .get(&artifact_id)
            .copied()
            .unwrap_or(default_selected)
            .min(view.versions.len() - 1);
        view.selected_version = selected;
        view.selected_version_cache.insert(artifact_id, selected);
        true
    }

    fn selected_artifact_version(
        view: &ArtifactsOverlay,
        meta: &ArtifactMetadata,
    ) -> Option<u32> {
        if view.versions_for == Some(meta.artifact_id) {
            if let Some(version) = view.versions.get(view.selected_version).copied() {
                return Some(version);
            }
        }
        None
    }

    fn apply_versions_to_artifacts_overlay(
        view: &mut ArtifactsOverlay,
        artifact_id: ArtifactId,
        parsed: &ArtifactVersionsResponse,
    ) {
        view.version_cache
            .insert(artifact_id, parsed.versions.clone());
        view.versions_for = Some(artifact_id);
        view.versions = parsed.versions.clone();
        view.selected_version = view
            .versions
            .iter()
            .position(|v| *v == parsed.latest_version)
            .unwrap_or(0);
        if view.selected_version >= view.versions.len() {
            view.selected_version = 0;
        }
        view.selected_version_cache
            .insert(artifact_id, view.selected_version);
    }

    fn artifact_read_error_hint(err: &anyhow::Error) -> Option<String> {
        let message = err.to_string();
        if message.contains("artifact version not retained")
            || message.contains("artifact version not found")
        {
            return Some(
                "artifact/read failed for selected version; press 0 to read latest".to_string(),
            );
        }
        None
    }
