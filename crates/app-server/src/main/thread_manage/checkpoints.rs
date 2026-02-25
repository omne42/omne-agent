const CHECKPOINT_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const CHECKPOINT_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
}

fn checkpoint_restore_denied_response(
    response: omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse,
) -> anyhow::Result<Value> {
    serde_json::to_value(response).context("serialize checkpoint restore denied response")
}

fn checkpoint_restore_needs_approval_response(
    thread_id: ThreadId,
    checkpoint_id: omne_protocol::CheckpointId,
    approval_id: omne_protocol::ApprovalId,
    plan: &omne_checkpoint_runtime::RestorePlan,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse {
        thread_id,
        checkpoint_id,
        needs_approval: true,
        approval_id,
        plan: checkpoint_plan_from_runtime(plan),
    };
    serde_json::to_value(response).context("serialize checkpoint restore needs_approval response")
}

fn checkpoint_restore_ok_response(
    thread_id: ThreadId,
    checkpoint_id: omne_protocol::CheckpointId,
    plan: &omne_checkpoint_runtime::RestorePlan,
    duration_ms: u64,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ThreadCheckpointRestoreResponse {
        thread_id,
        checkpoint_id,
        restored: true,
        plan: checkpoint_plan_from_runtime(plan),
        duration_ms,
    };
    serde_json::to_value(response).context("serialize checkpoint restore ok response")
}

fn checkpoint_plan_from_runtime(
    plan: &omne_checkpoint_runtime::RestorePlan,
) -> omne_app_server_protocol::ThreadCheckpointPlan {
    omne_app_server_protocol::ThreadCheckpointPlan {
        create: plan.create,
        modify: plan.modify,
        delete: plan.delete,
    }
}

fn checkpoint_summary_from_manifest(
    manifest: &omne_checkpoint_spec::CheckpointManifestV1,
    manifest_path: &Path,
) -> anyhow::Result<omne_app_server_protocol::ThreadCheckpointSummary> {
    Ok(omne_app_server_protocol::ThreadCheckpointSummary {
        checkpoint_id: manifest.checkpoint_id,
        created_at: manifest.created_at.format(&Rfc3339)?,
        label: manifest.label.clone(),
        snapshot_ref: manifest.snapshot_ref.clone(),
        manifest_path: manifest_path.display().to_string(),
        stats: omne_app_server_protocol::ThreadCheckpointStats {
            file_count: manifest.stats.file_count,
            total_bytes: manifest.stats.total_bytes,
        },
        excluded: omne_app_server_protocol::ThreadCheckpointExcluded {
            symlink_count: manifest.excluded.symlink_count,
            oversize_count: manifest.excluded.oversize_count,
            secret_count: manifest.excluded.secret_count,
        },
        size_limits: omne_app_server_protocol::ThreadCheckpointSizeLimits {
            max_file_bytes: manifest.size_limits.max_file_bytes,
            max_total_bytes: manifest.size_limits.max_total_bytes,
        },
    })
}

async fn list_running_processes(server: &Server, thread_id: ThreadId) -> Vec<ProcessId> {
    let entries = {
        let entries = server.processes.lock().await;
        entries
            .iter()
            .map(|(process_id, entry)| (*process_id, entry.clone()))
            .collect::<Vec<_>>()
    };

    let mut running = Vec::new();
    for (process_id, entry) in entries {
        let info = entry.info.lock().await;
        if info.thread_id != thread_id {
            continue;
        }
        if matches!(info.status, ProcessStatus::Running) {
            running.push(process_id);
        }
    }
    running
}

async fn write_checkpoint_restore_report(
    server: &Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    checkpoint_id: omne_protocol::CheckpointId,
    reason: &str,
    plan: Option<&omne_checkpoint_runtime::RestorePlan>,
) -> Option<ArtifactId> {
    let reason = omne_core::redact_text(reason);
    let summary = format!("rollback failed: {reason}");
    let plan_section = match plan {
        Some(plan) => format!(
            "## Restore Plan\n\n- create: {}\n- modify: {}\n- delete: {}\n",
            plan.create, plan.modify, plan.delete
        ),
        None => "## Restore Plan\n\n- unavailable\n".to_string(),
    };
    let text = format!(
        "# Rollback Report\n\n- checkpoint_id: {checkpoint_id}\n- status: failed\n- reason: {reason}\n\n{plan_section}"
    );

    let written = match handle_artifact_write(
        server,
        ArtifactWriteParams {
            thread_id,
            turn_id,
            approval_id: None,
            artifact_id: None,
            artifact_type: "rollback_report".to_string(),
            summary,
            text,
        },
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(
                thread_id = %thread_id,
                checkpoint_id = %checkpoint_id,
                error = %err,
                "failed to write rollback_report artifact"
            );
            return None;
        }
    };

    let artifact_id = written
        .get("artifact_id")
        .cloned()
        .and_then(|value| serde_json::from_value::<ArtifactId>(value).ok());
    if artifact_id.is_none() {
        tracing::warn!(
            thread_id = %thread_id,
            checkpoint_id = %checkpoint_id,
            "rollback_report write response missing artifact_id"
        );
    }
    artifact_id
}

async fn handle_thread_checkpoint_create(
    server: &Server,
    params: ThreadCheckpointCreateParams,
) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let (thread_cwd, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state
                .cwd
                .clone()
                .ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {}", params.thread_id))?,
            state.active_turn_id,
        )
    };

    if let Some(turn_id) = active_turn_id {
        anyhow::bail!("refusing to create checkpoint with active turn: turn_id={turn_id}");
    }

    let running = list_running_processes(server, params.thread_id).await;
    if !running.is_empty() {
        anyhow::bail!(
            "refusing to create checkpoint with running processes: {:?}",
            running
        );
    }

    let checkpoint_id = omne_protocol::CheckpointId::new();
    let checkpoint_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts")
        .join("checkpoints")
        .join(checkpoint_id.to_string());
    let snapshot_root = checkpoint_dir.join("workspace");
    let manifest_path = checkpoint_dir.join("manifest.json");
    let snapshot_ref = format!("artifacts/checkpoints/{checkpoint_id}");

    let label = params.label.map(|label| omne_core::redact_text(&label));
    let created_at = OffsetDateTime::now_utc();

    let outcome = omne_checkpoint_runtime::snapshot_workspace_to_dir(
        &thread_root,
        &snapshot_root,
        CHECKPOINT_MAX_FILE_BYTES,
        CHECKPOINT_MAX_TOTAL_BYTES,
    )
    .await?;

    let manifest = omne_checkpoint_spec::CheckpointManifestV1 {
        version: omne_checkpoint_spec::CHECKPOINT_MANIFEST_VERSION,
        checkpoint_id,
        created_at,
        label: label.clone(),
        source: omne_checkpoint_spec::CheckpointSource {
            thread_id: params.thread_id,
            cwd: thread_cwd,
        },
        snapshot_ref: snapshot_ref.clone(),
        stats: omne_checkpoint_spec::CheckpointStats {
            file_count: outcome.file_count,
            total_bytes: outcome.total_bytes,
        },
        excluded: omne_checkpoint_spec::CheckpointExcluded {
            symlink_count: outcome.symlink_count,
            oversize_count: outcome.oversize_count,
            secret_count: outcome.secret_count,
        },
        size_limits: omne_checkpoint_spec::CheckpointSizeLimits {
            max_file_bytes: CHECKPOINT_MAX_FILE_BYTES,
            max_total_bytes: CHECKPOINT_MAX_TOTAL_BYTES,
        },
        ignored_globs: omne_checkpoint_runtime::checkpoint_ignored_globs(),
    };
    omne_checkpoint_spec::write_manifest(&manifest_path, &manifest).await?;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::CheckpointCreated {
            checkpoint_id,
            turn_id: None,
            label: label.clone(),
            snapshot_ref: snapshot_ref.clone(),
        })
        .await?;

    let response = omne_app_server_protocol::ThreadCheckpointCreateResponse {
        thread_id: params.thread_id,
        checkpoint_id,
        label,
        created_at: created_at.format(&Rfc3339)?,
        checkpoint_dir: checkpoint_dir.display().to_string(),
        snapshot_ref,
        manifest_path: manifest_path.display().to_string(),
        stats: omne_app_server_protocol::ThreadCheckpointStats {
            file_count: outcome.file_count,
            total_bytes: outcome.total_bytes,
        },
        excluded: omne_app_server_protocol::ThreadCheckpointExcluded {
            symlink_count: outcome.symlink_count,
            oversize_count: outcome.oversize_count,
            secret_count: outcome.secret_count,
        },
        size_limits: omne_app_server_protocol::ThreadCheckpointSizeLimits {
            max_file_bytes: CHECKPOINT_MAX_FILE_BYTES,
            max_total_bytes: CHECKPOINT_MAX_TOTAL_BYTES,
        },
    };
    serde_json::to_value(response).context("serialize thread checkpoint create response")
}

async fn handle_thread_checkpoint_list(
    server: &Server,
    params: ThreadCheckpointListParams,
) -> anyhow::Result<Value> {
    let checkpoints_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts")
        .join("checkpoints");

    let mut read_dir = match tokio::fs::read_dir(&checkpoints_dir).await {
        Ok(dir) => dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let response = omne_app_server_protocol::ThreadCheckpointListResponse {
                thread_id: params.thread_id,
                checkpoints_dir: checkpoints_dir.display().to_string(),
                checkpoints: Vec::new(),
            };
            return serde_json::to_value(response)
                .context("serialize empty thread checkpoint list response");
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", checkpoints_dir.display())),
    };

    let mut checkpoints = Vec::<omne_app_server_protocol::ThreadCheckpointSummary>::new();
    while let Some(entry) = read_dir.next_entry().await? {
        let ty = entry.file_type().await?;
        if !ty.is_dir() {
            continue;
        }
        let dir = entry.path();
        let manifest_path = dir.join("manifest.json");
        let manifest = match omne_checkpoint_spec::read_manifest(&manifest_path).await {
            Ok(manifest) => manifest,
            Err(err) if is_not_found(&err) => continue,
            Err(err) => return Err(err),
        };
        if manifest.version != omne_checkpoint_spec::CHECKPOINT_MANIFEST_VERSION {
            continue;
        }
        checkpoints.push(checkpoint_summary_from_manifest(&manifest, &manifest_path)?);
    }

    checkpoints.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let response = omne_app_server_protocol::ThreadCheckpointListResponse {
        thread_id: params.thread_id,
        checkpoints_dir: checkpoints_dir.display().to_string(),
        checkpoints,
    };
    serde_json::to_value(response).context("serialize thread checkpoint list response")
}

async fn handle_thread_checkpoint_restore(
    server: &Server,
    params: ThreadCheckpointRestoreParams,
) -> anyhow::Result<Value> {
    let started_at = tokio::time::Instant::now();
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let (approval_policy, sandbox_policy, sandbox_writable_roots, mode_name, active_turn_id) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.mode.clone(),
            state.active_turn_id,
        )
    };

    if let Some(turn_id) = active_turn_id {
        anyhow::bail!("refusing to restore checkpoint with active turn: turn_id={turn_id}");
    }

    let running = list_running_processes(server, params.thread_id).await;
    if !running.is_empty() {
        anyhow::bail!(
            "refusing to restore checkpoint with running processes: {:?}",
            running
        );
    }

    if sandbox_policy == omne_protocol::SandboxPolicy::ReadOnly {
        let reason = "sandbox_policy=read_only forbids checkpoint restore".to_string();
        let report_artifact_id = write_checkpoint_restore_report(
            server,
            params.thread_id,
            params.turn_id,
            params.checkpoint_id,
            &reason,
            None,
        )
        .await;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                checkpoint_id: params.checkpoint_id,
                turn_id: params.turn_id,
                status: omne_protocol::CheckpointRestoreStatus::Failed,
                reason: Some(reason),
                report_artifact_id,
            })
            .await?;
        return checkpoint_restore_denied_response(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: params.thread_id,
                checkpoint_id: params.checkpoint_id,
                denied: true,
                error_code: Some("sandbox_policy_denied".to_string()),
                sandbox_policy: Some(sandbox_policy),
                mode: None,
                decision: None,
                available: None,
                load_error: None,
                sandbox_writable_roots: None,
            },
        );
    }

    let checkpoint_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts")
        .join("checkpoints")
        .join(params.checkpoint_id.to_string());
    let manifest_path = checkpoint_dir.join("manifest.json");
    let snapshot_root = checkpoint_dir.join("workspace");

    let manifest = omne_checkpoint_spec::read_manifest(&manifest_path).await?;
    if manifest.version != omne_checkpoint_spec::CHECKPOINT_MANIFEST_VERSION {
        anyhow::bail!(
            "unsupported checkpoint manifest version: {} (expected {})",
            manifest.version,
            omne_checkpoint_spec::CHECKPOINT_MANIFEST_VERSION
        );
    }
    if manifest.checkpoint_id != params.checkpoint_id {
        anyhow::bail!(
            "checkpoint_id mismatch: requested {}, manifest has {}",
            params.checkpoint_id,
            manifest.checkpoint_id
        );
    }

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let reason = "unknown mode".to_string();
            let report_artifact_id = write_checkpoint_restore_report(
                server,
                params.thread_id,
                params.turn_id,
                params.checkpoint_id,
                &reason,
                None,
            )
            .await;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some(reason),
                    report_artifact_id,
                })
                .await?;
            return checkpoint_restore_denied_response(
                omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                    thread_id: params.thread_id,
                    checkpoint_id: params.checkpoint_id,
                    denied: true,
                    error_code: Some("mode_unknown".to_string()),
                    sandbox_policy: None,
                    mode: Some(mode_name),
                    decision: Some(omne_app_server_protocol::ThreadCheckpointDecision::Deny),
                    available: Some(available),
                    load_error: catalog.load_error.clone(),
                    sandbox_writable_roots: None,
                },
            );
        }
    };

    let base_decision = mode.permissions.edit.decision_for_path(Path::new("."));
    let effective_decision = match mode
        .tool_overrides
        .get("thread/checkpoint/restore")
        .copied()
    {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };

    if effective_decision == omne_core::modes::Decision::Deny {
        let reason = "mode denies checkpoint restore".to_string();
        let report_artifact_id = write_checkpoint_restore_report(
            server,
            params.thread_id,
            params.turn_id,
            params.checkpoint_id,
            &reason,
            None,
        )
        .await;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                checkpoint_id: params.checkpoint_id,
                turn_id: params.turn_id,
                status: omne_protocol::CheckpointRestoreStatus::Failed,
                reason: Some(reason),
                report_artifact_id,
            })
            .await?;
        return checkpoint_restore_denied_response(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: params.thread_id,
                checkpoint_id: params.checkpoint_id,
                denied: true,
                error_code: Some("mode_denied".to_string()),
                sandbox_policy: None,
                mode: Some(mode_name),
                decision: Some(omne_app_server_protocol::ThreadCheckpointDecision::Deny),
                available: None,
                load_error: None,
                sandbox_writable_roots: None,
            },
        );
    }

    let plan = omne_checkpoint_runtime::compute_restore_plan(
        &thread_root,
        &snapshot_root,
        CHECKPOINT_MAX_FILE_BYTES,
    )
    .await?;
    let approval_params = serde_json::json!({
        "checkpoint_id": params.checkpoint_id,
        "label": manifest.label,
        "snapshot_ref": manifest.snapshot_ref,
        "plan": plan,
        "approval": { "requirement": "prompt_strict" },
    });

    match gate_approval(
        server,
        &thread_rt,
        params.thread_id,
        params.turn_id,
        approval_policy,
        ApprovalRequest {
            approval_id: params.approval_id,
            action: "thread/checkpoint/restore",
            params: &approval_params,
        },
    )
    .await?
    {
        ApprovalGate::Approved => {}
        ApprovalGate::Denied { remembered: _ } => {
            let reason = "approval denied".to_string();
            let report_artifact_id = write_checkpoint_restore_report(
                server,
                params.thread_id,
                params.turn_id,
                params.checkpoint_id,
                &reason,
                Some(&plan),
            )
            .await;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some(reason),
                    report_artifact_id,
                })
                .await?;
            return checkpoint_restore_denied_response(
                omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                    thread_id: params.thread_id,
                    checkpoint_id: params.checkpoint_id,
                    denied: true,
                    error_code: Some("approval_denied".to_string()),
                    sandbox_policy: None,
                    mode: None,
                    decision: None,
                    available: None,
                    load_error: None,
                    sandbox_writable_roots: None,
                },
            );
        }
        ApprovalGate::NeedsApproval { approval_id } => {
            return checkpoint_restore_needs_approval_response(
                params.thread_id,
                params.checkpoint_id,
                approval_id,
                &plan,
            );
        }
    }

    if !sandbox_writable_roots.is_empty() {
        let reason =
            "checkpoint restore is not supported when sandbox_writable_roots is set".to_string();
        let report_artifact_id = write_checkpoint_restore_report(
            server,
            params.thread_id,
            params.turn_id,
            params.checkpoint_id,
            &reason,
            Some(&plan),
        )
        .await;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                checkpoint_id: params.checkpoint_id,
                turn_id: params.turn_id,
                status: omne_protocol::CheckpointRestoreStatus::Failed,
                reason: Some(reason),
                report_artifact_id,
            })
            .await?;
        return checkpoint_restore_denied_response(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: params.thread_id,
                checkpoint_id: params.checkpoint_id,
                denied: true,
                error_code: Some("sandbox_writable_roots_unsupported".to_string()),
                sandbox_policy: None,
                mode: None,
                decision: None,
                available: None,
                load_error: None,
                sandbox_writable_roots: Some(sandbox_writable_roots),
            },
        );
    }

    let result = omne_checkpoint_runtime::restore_workspace_from_snapshot(
        &thread_root,
        &snapshot_root,
        CHECKPOINT_MAX_FILE_BYTES,
    )
    .await;
    match result {
        Ok(()) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Ok,
                    reason: None,
                    report_artifact_id: None,
                })
                .await?;
            let elapsed_ms = started_at.elapsed().as_millis();
            let duration_ms = elapsed_ms.min(u128::from(u64::MAX)) as u64;
            checkpoint_restore_ok_response(
                params.thread_id,
                params.checkpoint_id,
                &plan,
                duration_ms,
            )
        }
        Err(err) => {
            let reason = err.to_string();
            let report_artifact_id = write_checkpoint_restore_report(
                server,
                params.thread_id,
                params.turn_id,
                params.checkpoint_id,
                &reason,
                Some(&plan),
            )
            .await;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some(reason),
                    report_artifact_id,
                })
                .await?;
            Err(err)
        }
    }
}
