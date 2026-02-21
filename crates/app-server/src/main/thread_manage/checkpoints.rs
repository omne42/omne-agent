const CHECKPOINT_MANIFEST_VERSION: u32 = 1;
const CHECKPOINT_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const CHECKPOINT_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointManifestV1 {
    version: u32,
    checkpoint_id: omne_protocol::CheckpointId,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    source: CheckpointSource,
    snapshot_ref: String,
    stats: CheckpointStats,
    excluded: CheckpointExcluded,
    size_limits: CheckpointSizeLimits,
    ignored_globs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointSource {
    thread_id: ThreadId,
    cwd: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointStats {
    file_count: u64,
    total_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointExcluded {
    symlink_count: u64,
    oversize_count: u64,
    secret_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckpointSizeLimits {
    max_file_bytes: u64,
    max_total_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct SnapshotOutcome {
    file_count: u64,
    total_bytes: u64,
    symlink_count: u64,
    oversize_count: u64,
    secret_count: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct RestorePlan {
    create: u64,
    modify: u64,
    delete: u64,
}

fn checkpoint_ignored_globs() -> Vec<String> {
    vec![
        ".git/**".to_string(),
        ".omne/**".to_string(),
        ".omne/**".to_string(),
        "target/**".to_string(),
        "node_modules/**".to_string(),
        "example/**".to_string(),
        ".omne_data/tmp/**".to_string(),
        ".omne_data/threads/**".to_string(),
        ".omne_data/locks/**".to_string(),
        ".omne_data/logs/**".to_string(),
        ".omne_data/data/**".to_string(),
        ".omne_data/repos/**".to_string(),
        ".omne_data/reference/**".to_string(),
        "**/.env".to_string(),
        "**/.env.*".to_string(),
        "**/.envrc".to_string(),
        "**/*.pem".to_string(),
        "**/*.key".to_string(),
        "**/.ssh/**".to_string(),
        "**/.aws/**".to_string(),
        "**/.kube/**".to_string(),
    ]
}

fn rel_path_is_checkpoint_secret(rel_path: &Path) -> bool {
    if rel_path
        .components()
        .any(|c| matches!(c, std::path::Component::Normal(os) if os == ".ssh" || os == ".aws" || os == ".kube"))
    {
        return true;
    }

    let Some(file_name) = rel_path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };

    if file_name == ".env" || file_name == ".envrc" || file_name.starts_with(".env.") {
        return true;
    }

    file_name.ends_with(".pem") || file_name.ends_with(".key")
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

async fn snapshot_workspace_to_dir(
    thread_root: &Path,
    snapshot_root: &Path,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> anyhow::Result<SnapshotOutcome> {
    let thread_root = thread_root.to_path_buf();
    let snapshot_root = snapshot_root.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<SnapshotOutcome> {
        std::fs::create_dir_all(&snapshot_root)
            .with_context(|| format!("create {}", snapshot_root.display()))?;

        let mut out = SnapshotOutcome::default();

        for entry in WalkDir::new(&thread_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_walk_entry)
        {
            let entry = entry?;

            if entry.file_type().is_symlink() {
                out.symlink_count += 1;
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }

            let rel = entry.path().strip_prefix(&thread_root).unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            if rel_path_is_checkpoint_secret(rel) {
                out.secret_count += 1;
                continue;
            }

            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > max_file_bytes {
                out.oversize_count += 1;
                continue;
            }

            out.file_count += 1;
            out.total_bytes = out
                .total_bytes
                .checked_add(meta.len())
                .ok_or_else(|| anyhow::anyhow!("checkpoint size overflow"))?;
            if out.total_bytes > max_total_bytes {
                anyhow::bail!(
                    "checkpoint exceeds max_total_bytes={} (current={})",
                    max_total_bytes,
                    out.total_bytes
                );
            }

            let dest = snapshot_root.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(entry.path(), &dest).with_context(|| {
                format!("copy {} -> {}", entry.path().display(), dest.display())
            })?;
        }

        Ok(out)
    })
    .await
    .context("join checkpoint snapshot task")?
}

async fn compute_restore_plan(thread_root: &Path, snapshot_root: &Path) -> anyhow::Result<RestorePlan> {
    let thread_root = thread_root.to_path_buf();
    let snapshot_root = snapshot_root.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<RestorePlan> {
        let mut snapshot_sizes = BTreeMap::<String, u64>::new();
        for entry in WalkDir::new(&snapshot_root).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&snapshot_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            snapshot_sizes.insert(rel.to_string_lossy().to_string(), meta.len());
        }

        let mut current_sizes = BTreeMap::<String, u64>::new();
        for entry in WalkDir::new(&thread_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_walk_entry)
        {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&thread_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            if rel_path_is_checkpoint_secret(rel) {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > CHECKPOINT_MAX_FILE_BYTES {
                continue;
            }
            current_sizes.insert(rel.to_string_lossy().to_string(), meta.len());
        }

        let snapshot_paths = snapshot_sizes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let current_paths = current_sizes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        let create = snapshot_paths.difference(&current_paths).count() as u64;
        let delete = current_paths.difference(&snapshot_paths).count() as u64;

        let mut modify = 0u64;
        for path in snapshot_paths.intersection(&current_paths) {
            let Some(snap_len) = snapshot_sizes.get(path) else {
                continue;
            };
            let Some(cur_len) = current_sizes.get(path) else {
                continue;
            };
            if snap_len != cur_len {
                modify += 1;
            }
        }

        Ok(RestorePlan {
            create,
            modify,
            delete,
        })
    })
    .await
    .context("join checkpoint plan task")?
}

async fn restore_workspace_from_snapshot(thread_root: &Path, snapshot_root: &Path) -> anyhow::Result<()> {
    let thread_root = thread_root.to_path_buf();
    let snapshot_root = snapshot_root.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut snapshot_paths = std::collections::BTreeSet::<String>::new();
        for entry in WalkDir::new(&snapshot_root).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&snapshot_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            snapshot_paths.insert(rel.to_string_lossy().to_string());
        }

        let mut current_paths = Vec::<String>::new();
        for entry in WalkDir::new(&thread_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_walk_entry)
        {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&thread_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            if rel_path_is_checkpoint_secret(rel) {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > CHECKPOINT_MAX_FILE_BYTES {
                continue;
            }
            current_paths.push(rel.to_string_lossy().to_string());
        }

        for rel in current_paths {
            if snapshot_paths.contains(&rel) {
                continue;
            }
            let path = thread_root.join(&rel);
            std::fs::remove_file(&path)
                .with_context(|| format!("remove {}", path.display()))?;
        }

        for rel in &snapshot_paths {
            let src = snapshot_root.join(rel);
            let dst = thread_root.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(&src, &dst)
                .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
        }

        Ok(())
    })
    .await
    .context("join checkpoint restore task")?
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
        anyhow::bail!(
            "refusing to create checkpoint with active turn: turn_id={turn_id}"
        );
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

    let outcome = snapshot_workspace_to_dir(
        &thread_root,
        &snapshot_root,
        CHECKPOINT_MAX_FILE_BYTES,
        CHECKPOINT_MAX_TOTAL_BYTES,
    )
    .await?;

    let manifest = CheckpointManifestV1 {
        version: CHECKPOINT_MANIFEST_VERSION,
        checkpoint_id,
        created_at,
        label: label.clone(),
        source: CheckpointSource {
            thread_id: params.thread_id,
            cwd: thread_cwd,
        },
        snapshot_ref: snapshot_ref.clone(),
        stats: CheckpointStats {
            file_count: outcome.file_count,
            total_bytes: outcome.total_bytes,
        },
        excluded: CheckpointExcluded {
            symlink_count: outcome.symlink_count,
            oversize_count: outcome.oversize_count,
            secret_count: outcome.secret_count,
        },
        size_limits: CheckpointSizeLimits {
            max_file_bytes: CHECKPOINT_MAX_FILE_BYTES,
            max_total_bytes: CHECKPOINT_MAX_TOTAL_BYTES,
        },
        ignored_globs: checkpoint_ignored_globs(),
    };

    let bytes = serde_json::to_vec_pretty(&manifest).context("serialize checkpoint manifest")?;
    tokio::fs::create_dir_all(&checkpoint_dir)
        .await
        .with_context(|| format!("create {}", checkpoint_dir.display()))?;
    tokio::fs::write(&manifest_path, bytes)
        .await
        .with_context(|| format!("write {}", manifest_path.display()))?;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::CheckpointCreated {
            checkpoint_id,
            turn_id: None,
            label: label.clone(),
            snapshot_ref: snapshot_ref.clone(),
        })
        .await?;

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "checkpoint_id": checkpoint_id,
        "label": label,
        "created_at": created_at.format(&Rfc3339)?,
        "checkpoint_dir": checkpoint_dir.display().to_string(),
        "snapshot_ref": snapshot_ref,
        "manifest_path": manifest_path.display().to_string(),
        "stats": {
            "file_count": outcome.file_count,
            "total_bytes": outcome.total_bytes,
        },
        "excluded": {
            "symlink_count": outcome.symlink_count,
            "oversize_count": outcome.oversize_count,
            "secret_count": outcome.secret_count,
        },
        "size_limits": {
            "max_file_bytes": CHECKPOINT_MAX_FILE_BYTES,
            "max_total_bytes": CHECKPOINT_MAX_TOTAL_BYTES,
        },
    }))
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
            return Ok(serde_json::json!({
                "thread_id": params.thread_id,
                "checkpoints_dir": checkpoints_dir.display().to_string(),
                "checkpoints": [],
            }));
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", checkpoints_dir.display())),
    };

    let mut checkpoints = Vec::<serde_json::Value>::new();
    while let Some(entry) = read_dir.next_entry().await? {
        let ty = entry.file_type().await?;
        if !ty.is_dir() {
            continue;
        }
        let dir = entry.path();
        let manifest_path = dir.join("manifest.json");
        let raw = match tokio::fs::read_to_string(&manifest_path).await {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("read {}", manifest_path.display())),
        };
        let manifest: CheckpointManifestV1 =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", manifest_path.display()))?;
        if manifest.version != CHECKPOINT_MANIFEST_VERSION {
            continue;
        }
        checkpoints.push(serde_json::json!({
            "checkpoint_id": manifest.checkpoint_id,
            "created_at": manifest.created_at.format(&Rfc3339)?,
            "label": manifest.label,
            "snapshot_ref": manifest.snapshot_ref,
            "manifest_path": manifest_path.display().to_string(),
            "stats": {
                "file_count": manifest.stats.file_count,
                "total_bytes": manifest.stats.total_bytes,
            },
            "excluded": {
                "symlink_count": manifest.excluded.symlink_count,
                "oversize_count": manifest.excluded.oversize_count,
                "secret_count": manifest.excluded.secret_count,
            },
            "size_limits": {
                "max_file_bytes": manifest.size_limits.max_file_bytes,
                "max_total_bytes": manifest.size_limits.max_total_bytes,
            },
        }));
    }

    checkpoints.sort_by(|a, b| {
        b.get("created_at")
            .and_then(|v| v.as_str())
            .cmp(&a.get("created_at").and_then(|v| v.as_str()))
    });

    Ok(serde_json::json!({
        "thread_id": params.thread_id,
        "checkpoints_dir": checkpoints_dir.display().to_string(),
        "checkpoints": checkpoints,
    }))
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
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                checkpoint_id: params.checkpoint_id,
                turn_id: params.turn_id,
                status: omne_protocol::CheckpointRestoreStatus::Failed,
                reason: Some("sandbox_policy=read_only forbids checkpoint restore".to_string()),
                report_artifact_id: None,
            })
            .await?;
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "checkpoint_id": params.checkpoint_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let checkpoint_dir = server
        .thread_store
        .thread_dir(params.thread_id)
        .join("artifacts")
        .join("checkpoints")
        .join(params.checkpoint_id.to_string());
    let manifest_path = checkpoint_dir.join("manifest.json");
    let snapshot_root = checkpoint_dir.join("workspace");

    let raw = tokio::fs::read_to_string(&manifest_path)
        .await
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let manifest: CheckpointManifestV1 =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", manifest_path.display()))?;
    if manifest.version != CHECKPOINT_MANIFEST_VERSION {
        anyhow::bail!(
            "unsupported checkpoint manifest version: {} (expected {})",
            manifest.version,
            CHECKPOINT_MANIFEST_VERSION
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
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some("unknown mode".to_string()),
                    report_artifact_id: None,
                })
                .await?;
            return Ok(serde_json::json!({
                "thread_id": params.thread_id,
                "checkpoint_id": params.checkpoint_id,
                "denied": true,
                "mode": mode_name,
                "decision": omne_core::modes::Decision::Deny,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
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
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                checkpoint_id: params.checkpoint_id,
                turn_id: params.turn_id,
                status: omne_protocol::CheckpointRestoreStatus::Failed,
                reason: Some("mode denies checkpoint restore".to_string()),
                report_artifact_id: None,
            })
            .await?;
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "checkpoint_id": params.checkpoint_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    let plan = compute_restore_plan(&thread_root, &snapshot_root).await?;
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
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some("approval denied".to_string()),
                    report_artifact_id: None,
                })
                .await?;
            return Ok(serde_json::json!({
                "thread_id": params.thread_id,
                "checkpoint_id": params.checkpoint_id,
                "denied": true,
            }));
        }
        ApprovalGate::NeedsApproval { approval_id } => {
            return Ok(serde_json::json!({
                "thread_id": params.thread_id,
                "checkpoint_id": params.checkpoint_id,
                "needs_approval": true,
                "approval_id": approval_id,
                "plan": plan,
            }));
        }
    }

    if !sandbox_writable_roots.is_empty() {
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                checkpoint_id: params.checkpoint_id,
                turn_id: params.turn_id,
                status: omne_protocol::CheckpointRestoreStatus::Failed,
                reason: Some(
                    "checkpoint restore is not supported when sandbox_writable_roots is set"
                        .to_string(),
                ),
                report_artifact_id: None,
            })
            .await?;
        return Ok(serde_json::json!({
            "thread_id": params.thread_id,
            "checkpoint_id": params.checkpoint_id,
            "denied": true,
            "sandbox_writable_roots": sandbox_writable_roots,
        }));
    }

    let result = restore_workspace_from_snapshot(&thread_root, &snapshot_root).await;
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
            Ok(serde_json::json!({
                "thread_id": params.thread_id,
                "checkpoint_id": params.checkpoint_id,
                "restored": true,
                "plan": plan,
                "duration_ms": started_at.elapsed().as_millis(),
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: params.checkpoint_id,
                    turn_id: params.turn_id,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some(err.to_string()),
                    report_artifact_id: None,
                })
                .await?;
            Err(err)
        }
    }
}
