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
            let subagent_pending_summary = app
                .thread_attention(thread_id)
                .await
                .ok()
                .and_then(|attention| {
                    summarize_subagent_pending_approvals_for_overlay(
                        attention.pending_approvals.as_slice(),
                    )
                })
                .or_else(|| self.subagent_pending_summary.clone());
            let selected = self
                .pending_action
                .as_ref()
                .map(|p| p.approval_id())
                .map(|id| select_approval(&approvals, Some(id)))
                .unwrap_or(0);
            self.overlays.push(Overlay::Approvals(new_approvals_overlay(
                thread_id,
                approvals,
                selected,
                subagent_pending_summary,
            )));
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

            let outcome = rpc_artifact_list_tui_outcome(
                app,
                omne_app_server_protocol::ArtifactListParams {
                    thread_id,
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
                    let subagent_pending_summary = app
                        .thread_attention(thread_id)
                        .await
                        .ok()
                        .and_then(|attention| {
                            summarize_subagent_pending_approvals_for_overlay(
                                attention.pending_approvals.as_slice(),
                            )
                        })
                        .or_else(|| self.subagent_pending_summary.clone());
                    let selected = select_approval(&approvals, Some(approval_id));
                    self.overlays.push(Overlay::Approvals(new_approvals_overlay(
                        thread_id,
                        approvals,
                        selected,
                        subagent_pending_summary,
                    )));
                    self.pending_action = Some(PendingAction::ArtifactList {
                        thread_id,
                        approval_id,
                    });
                    self.set_status(format!("artifact/list needs approval: {approval_id}"));
                    return Ok(());
                }
                RpcActionOutcome::Denied { summary } => {
                    self.set_status(format!("artifact/list denied: {summary}"));
                    return Ok(());
                }
                RpcActionOutcome::Ok(parsed) => {
                    if !parsed.errors.is_empty() {
                        self.set_status(format!("artifact/list errors: {}", parsed.errors.len()));
                    }
                    self.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                        thread_id,
                        artifacts: parsed.artifacts,
                        selected: 0,
                        versions_for: None,
                        versions: Vec::new(),
                        selected_version: 0,
                        version_cache: HashMap::new(),
                        selected_version_cache: HashMap::new(),
                    }));
                }
            }
            Ok(())
        }

        async fn resume_pending_action(&mut self, app: &mut super::App) -> anyhow::Result<()> {
            let Some(pending) = self.pending_action.take() else {
                return Ok(());
            };

            match pending {
                PendingAction::ProcessInspect {
                    thread_id: _thread_id,
                    process_id,
                    max_lines,
                    approval_id,
                } => {
                    let outcome = rpc_process_inspect_tui_outcome(
                        app,
                        omne_app_server_protocol::ProcessInspectParams {
                            process_id,
                            turn_id: None,
                            approval_id: Some(approval_id),
                            max_lines: Some(max_lines),
                        },
                    )
                    .await?;
                    match outcome {
                        RpcActionOutcome::NeedsApproval {
                            thread_id,
                            approval_id,
                        } => {
                            self.pending_action = Some(PendingAction::ProcessInspect {
                                thread_id,
                                process_id,
                                max_lines,
                                approval_id,
                            });
                            self.set_status("process/inspect still needs approval".to_string());
                            return Ok(());
                        }
                        RpcActionOutcome::Denied { summary } => {
                            self.set_status(format!("process/inspect denied: {summary}"));
                            return Ok(());
                        }
                        RpcActionOutcome::Ok(parsed) => {
                            let text = build_process_inspect_text(&parsed);
                            self.overlays.push(Overlay::Text(TextOverlay {
                                title: format!("Process {}", parsed.process.process_id),
                                text,
                                scroll: 0,
                            }));
                        }
                    }
                }
                PendingAction::ProcessKill {
                    thread_id,
                    process_id,
                    approval_id,
                } => {
                    let outcome = rpc_process_kill_tui_outcome(
                        app,
                        omne_app_server_protocol::ProcessKillParams {
                            process_id,
                            turn_id: None,
                            approval_id: Some(approval_id),
                            reason: Some("tui kill".to_string()),
                        },
                    )
                    .await?;
                    match outcome {
                        RpcActionOutcome::NeedsApproval {
                            thread_id,
                            approval_id,
                        } => {
                            self.pending_action = Some(PendingAction::ProcessKill {
                                thread_id,
                                process_id,
                                approval_id,
                            });
                            self.set_status("process/kill still needs approval".to_string());
                            return Ok(());
                        }
                        RpcActionOutcome::Denied { summary } => {
                            self.set_status(format!("process/kill denied: {summary}"));
                            return Ok(());
                        }
                        RpcActionOutcome::Ok(_) => {
                            self.set_status(format!("kill requested: {process_id}"));
                            self.refresh_processes_overlay(app, thread_id).await?;
                        }
                    }
                }
                PendingAction::ProcessInterrupt {
                    thread_id,
                    process_id,
                    approval_id,
                } => {
                    let outcome = rpc_process_interrupt_tui_outcome(
                        app,
                        omne_app_server_protocol::ProcessInterruptParams {
                            process_id,
                            turn_id: None,
                            approval_id: Some(approval_id),
                            reason: Some("tui interrupt".to_string()),
                        },
                    )
                    .await?;
                    match outcome {
                        RpcActionOutcome::NeedsApproval {
                            thread_id,
                            approval_id,
                        } => {
                            self.pending_action = Some(PendingAction::ProcessInterrupt {
                                thread_id,
                                process_id,
                                approval_id,
                            });
                            self.set_status("process/interrupt still needs approval".to_string());
                            return Ok(());
                        }
                        RpcActionOutcome::Denied { summary } => {
                            self.set_status(format!("process/interrupt denied: {summary}"));
                            return Ok(());
                        }
                        RpcActionOutcome::Ok(_) => {
                            self.set_status(format!("interrupt requested: {process_id}"));
                            self.refresh_processes_overlay(app, thread_id).await?;
                        }
                    }
                }
                PendingAction::ArtifactList {
                    thread_id,
                    approval_id,
                } => {
                    let outcome = rpc_artifact_list_tui_outcome(
                        app,
                        omne_app_server_protocol::ArtifactListParams {
                            thread_id,
                            turn_id: None,
                            approval_id: Some(approval_id),
                        },
                    )
                    .await?;
                    match outcome {
                        RpcActionOutcome::NeedsApproval {
                            thread_id,
                            approval_id,
                        } => {
                            self.pending_action = Some(PendingAction::ArtifactList {
                                thread_id,
                                approval_id,
                            });
                            self.set_status("artifact/list still needs approval".to_string());
                            return Ok(());
                        }
                        RpcActionOutcome::Denied { summary } => {
                            self.set_status(format!("artifact/list denied: {summary}"));
                            return Ok(());
                        }
                        RpcActionOutcome::Ok(parsed) => {
                            if !parsed.errors.is_empty() {
                                self.set_status(format!("artifact/list errors: {}", parsed.errors.len()));
                            }
                            self.refresh_artifacts_overlay(thread_id, parsed.artifacts);
                        }
                    }
                }
                PendingAction::ArtifactRead {
                    thread_id,
                    artifact_id,
                    max_bytes,
                    version,
                    approval_id,
                } => {
                    let outcome = match rpc_artifact_read_tui_outcome(
                        app,
                        omne_app_server_protocol::ArtifactReadParams {
                            thread_id,
                            turn_id: None,
                            approval_id: Some(approval_id),
                            artifact_id,
                            version,
                            max_bytes: Some(max_bytes),
                        },
                    )
                    .await
                    {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            if let Some(hint) = artifact_read_error_hint(&err) {
                                self.set_status(hint);
                                return Ok(());
                            }
                            return Err(err);
                        }
                    };
                    match outcome {
                        RpcActionOutcome::NeedsApproval {
                            thread_id,
                            approval_id,
                        } => {
                            self.pending_action = Some(PendingAction::ArtifactRead {
                                thread_id,
                                artifact_id,
                                max_bytes,
                                version,
                                approval_id,
                            });
                            self.set_status("artifact/read still needs approval".to_string());
                            return Ok(());
                        }
                        RpcActionOutcome::Denied { summary } => {
                            self.set_status(format!("artifact/read denied: {summary}"));
                            return Ok(());
                        }
                        RpcActionOutcome::Ok(parsed) => {
                            let text = build_artifact_read_text(&parsed);
                            self.overlays.push(Overlay::Text(TextOverlay {
                                title: format!("Artifact {}", parsed.metadata.artifact_id),
                                text,
                                scroll: 0,
                            }));
                        }
                    }
                }
                PendingAction::ArtifactVersions {
                    thread_id,
                    artifact_id,
                    approval_id,
                } => {
                    let outcome =
                        load_artifact_versions(app, thread_id, artifact_id, Some(approval_id))
                            .await?;
                    match outcome {
                        RpcActionOutcome::NeedsApproval {
                            thread_id,
                            approval_id,
                        } => {
                            self.pending_action = Some(PendingAction::ArtifactVersions {
                                thread_id,
                                artifact_id,
                                approval_id,
                            });
                            self.set_status("artifact/versions still needs approval".to_string());
                            return Ok(());
                        }
                        RpcActionOutcome::Denied { summary } => {
                            self.set_status(format!("artifact/versions denied: {summary}"));
                            return Ok(());
                        }
                        RpcActionOutcome::Ok(parsed) => {
                            self.refresh_artifact_versions_overlay(thread_id, artifact_id, parsed);
                        }
                    }
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
                        sync_versions_for_selected_artifact(view);
                        updated = true;
                    }
                }
            }
            if !updated {
                self.overlays.push(Overlay::Artifacts(ArtifactsOverlay {
                    thread_id,
                    artifacts,
                    selected: 0,
                    versions_for: None,
                    versions: Vec::new(),
                    selected_version: 0,
                    version_cache: HashMap::new(),
                    selected_version_cache: HashMap::new(),
                }));
            }
        }

        fn refresh_artifact_versions_overlay(
            &mut self,
            thread_id: ThreadId,
            artifact_id: ArtifactId,
            parsed: ArtifactVersionsResponse,
        ) {
            for overlay in &mut self.overlays {
                if let Overlay::Artifacts(view) = overlay {
                    if view.thread_id == thread_id {
                        apply_versions_to_artifacts_overlay(view, artifact_id, &parsed);
                    }
                }
            }
        }
    }
