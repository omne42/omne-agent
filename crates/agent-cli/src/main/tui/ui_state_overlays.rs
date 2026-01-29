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
