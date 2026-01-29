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
