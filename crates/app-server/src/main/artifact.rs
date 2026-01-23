async fn handle_artifact_write(
    server: &Server,
    params: ArtifactWriteParams,
) -> anyhow::Result<Value> {
    if params.artifact_type.trim().is_empty() {
        anyhow::bail!("artifact_type must not be empty");
    }
    if params.summary.trim().is_empty() {
        anyhow::bail!("summary must not be empty");
    }

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();
    let bytes_len = params.text.len();
    let artifact_type = params.artifact_type.clone();
    let summary = params.summary.clone();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
        "artifact_type": artifact_type,
        "summary": summary,
        "bytes": bytes_len,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/write",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "artifact/write".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.artifact;
    let effective_decision = match mode.tool_overrides.get("artifact/write").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "artifact/write".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies artifact/write".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "artifact/write",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "artifact/write".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some(approval_denied_error(remembered).to_string()),
                            result: Some(serde_json::json!({
                                "approval_policy": approval_policy,
                            })),
                        })
                        .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/write".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let outcome = write_user_artifact(
        server,
        UserArtifactWriteRequest {
            tool_id,
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            artifact_id: params.artifact_id,
            artifact_type: params.artifact_type,
            summary: params.summary,
            text: params.text,
        },
    )
    .await;

    match outcome {
        Ok((response, completed)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(response)
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

struct UserArtifactWriteRequest {
    tool_id: pm_protocol::ToolId,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    artifact_id: Option<ArtifactId>,
    artifact_type: String,
    summary: String,
    text: String,
}

async fn write_user_artifact(
    server: &Server,
    req: UserArtifactWriteRequest,
) -> anyhow::Result<(Value, Value)> {
    let UserArtifactWriteRequest {
        tool_id,
        thread_id,
        turn_id,
        artifact_id,
        artifact_type,
        summary,
        text,
    } = req;

    let artifact_id = artifact_id.unwrap_or_default();
    let (content_path, metadata_path) = user_artifact_paths(server, thread_id, artifact_id);

    let now = OffsetDateTime::now_utc();
    let (created_at, version, created, previous_version) =
        match tokio::fs::metadata(&metadata_path).await {
            Ok(_) => {
                let meta = read_artifact_metadata(&metadata_path).await?;
                (
                    meta.created_at,
                    meta.version.saturating_add(1),
                    false,
                    Some(meta.version),
                )
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (now, 1, true, None),
            Err(err) => return Err(err).with_context(|| format!("stat {}", metadata_path.display())),
        };

    let history_max_versions = artifact_history_max_versions();
    let mut history_snapshot_version = None;
    let mut history_pruned_versions = Vec::new();
    let mut history_prune_error = None;
    let updating_existing = previous_version.is_some();

    if history_max_versions > 0 {
        if let Some(prev_version) = previous_version {
            if snapshot_user_artifact_version(
                server,
                thread_id,
                artifact_id,
                &content_path,
                prev_version,
            )
            .await?
            {
                history_snapshot_version = Some(prev_version);
            }
        }
    }

    let text = pm_core::redact_text(&text);
    let bytes = text.as_bytes().to_vec();
    write_file_atomic(&content_path, &bytes).await?;

    let preview = Some(infer_artifact_preview(artifact_type.as_str()));
    let meta = ArtifactMetadata {
        artifact_id,
        artifact_type,
        summary,
        preview,
        created_at,
        updated_at: now,
        version,
        content_path: content_path.display().to_string(),
        size_bytes: bytes.len() as u64,
        provenance: Some(ArtifactProvenance {
            thread_id,
            turn_id,
            tool_id: Some(tool_id),
            process_id: None,
        }),
    };

    let meta_bytes = serde_json::to_vec_pretty(&meta).context("serialize artifact metadata")?;
    write_file_atomic(&metadata_path, &meta_bytes).await?;

    if history_max_versions > 0 && updating_existing {
        match prune_user_artifact_history(server, thread_id, artifact_id, history_max_versions).await
        {
            Ok(versions) => history_pruned_versions = versions,
            Err(err) => history_prune_error = Some(err.to_string()),
        }
    }

    let mut completed = serde_json::json!({
        "artifact_id": artifact_id,
        "created": created,
        "content_path": content_path.display().to_string(),
        "metadata_path": metadata_path.display().to_string(),
        "version": version,
        "size_bytes": bytes.len(),
    });

    let mut response = serde_json::json!({
        "tool_id": tool_id,
        "artifact_id": artifact_id,
        "created": created,
        "content_path": content_path.display().to_string(),
        "metadata_path": metadata_path.display().to_string(),
        "metadata": meta,
    });

    if history_max_versions > 0 && updating_existing {
        let history = serde_json::json!({
            "max_versions": history_max_versions,
            "snapshotted_version": history_snapshot_version,
            "pruned_versions": history_pruned_versions,
            "prune_error": history_prune_error,
        });
        if let Some(obj) = completed.as_object_mut() {
            obj.insert("history".to_string(), history.clone());
        }
        if let Some(obj) = response.as_object_mut() {
            obj.insert("history".to_string(), history);
        }
    }

    Ok((response, completed))
}

fn infer_artifact_preview(artifact_type: &str) -> pm_protocol::ArtifactPreview {
    let kind = match artifact_type {
        "diff" => pm_protocol::ArtifactPreviewKind::DiffUnified,
        "patch" => pm_protocol::ArtifactPreviewKind::PatchUnified,
        "html" => pm_protocol::ArtifactPreviewKind::Html,
        "code" => pm_protocol::ArtifactPreviewKind::Code,
        "log" | "log_excerpt" => pm_protocol::ArtifactPreviewKind::Log,
        _ => pm_protocol::ArtifactPreviewKind::Markdown,
    };
    pm_protocol::ArtifactPreview {
        kind,
        language: None,
        title: None,
    }
}

fn artifact_history_max_versions() -> usize {
    std::env::var("CODE_PM_ARTIFACT_HISTORY_MAX_VERSIONS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

async fn snapshot_user_artifact_version(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
    content_path: &Path,
    version: u32,
) -> anyhow::Result<bool> {
    let bytes = match tokio::fs::read(content_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("read {}", content_path.display())),
    };

    let history_path = user_artifact_history_path(server, thread_id, artifact_id, version);
    write_file_atomic(&history_path, &bytes).await?;
    Ok(true)
}

async fn prune_user_artifact_history(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
    max_versions: usize,
) -> anyhow::Result<Vec<u32>> {
    if max_versions == 0 {
        return Ok(Vec::new());
    }

    let history_dir = user_artifact_history_dir_for_thread(server, thread_id, artifact_id);
    let mut dir = match tokio::fs::read_dir(&history_dir).await {
        Ok(dir) => dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read dir {}", history_dir.display())),
    };

    let mut candidates = Vec::new();
    while let Some(entry) = dir
        .next_entry()
        .await
        .with_context(|| format!("read dir {}", history_dir.display()))?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name.strip_prefix('v').and_then(|name| name.strip_suffix(".md")) else {
            continue;
        };
        let Ok(version) = stem.parse::<u32>() else {
            continue;
        };
        candidates.push((version, entry.path()));
    }

    candidates.sort_by_key(|(version, _)| *version);
    if candidates.len() <= max_versions {
        return Ok(Vec::new());
    }

    let mut removed = Vec::new();
    let remove_count = candidates.len() - max_versions;
    for (version, path) in candidates.into_iter().take(remove_count) {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => removed.push(version),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }

    Ok(removed)
}

async fn handle_artifact_list(
    server: &Server,
    params: ArtifactListParams,
) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({});
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/list",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "artifact/list".to_string(),
                    params: Some(approval_params),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.artifact;
    let effective_decision = match mode.tool_overrides.get("artifact/list").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "artifact/list".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies artifact/list".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "artifact/list",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "artifact/list".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some(approval_denied_error(remembered).to_string()),
                            result: Some(serde_json::json!({
                                "approval_policy": approval_policy,
                            })),
                        })
                        .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/list".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let dir = user_artifacts_dir_for_thread(server, params.thread_id);

    let outcome: anyhow::Result<(Vec<ArtifactMetadata>, Vec<Value>)> = async {
        let mut artifacts = Vec::<ArtifactMetadata>::new();
        let mut errors = Vec::<Value>::new();

        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(read_dir) => read_dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok((artifacts, errors));
            }
            Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
        };

        loop {
            let Some(entry) = read_dir
                .next_entry()
                .await
                .with_context(|| format!("read {}", dir.display()))?
            else {
                break;
            };
            let ty = entry
                .file_type()
                .await
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if !ty.is_file() {
                continue;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.ends_with(".metadata.json") {
                continue;
            }
            match read_artifact_metadata(&path).await {
                Ok(meta) => artifacts.push(meta),
                Err(err) => errors.push(serde_json::json!({
                    "path": path.display().to_string(),
                    "error": err.to_string(),
                })),
            }
        }

        artifacts.sort_by(|a, b| {
            b.updated_at
                .unix_timestamp_nanos()
                .cmp(&a.updated_at.unix_timestamp_nanos())
                .then_with(|| b.artifact_id.cmp(&a.artifact_id))
        });

        Ok((artifacts, errors))
    }
    .await;

    match outcome {
        Ok((artifacts, errors)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "artifacts": artifacts.len(),
                        "errors": errors.len(),
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "artifacts": artifacts,
                "errors": errors,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_artifact_read(
    server: &Server,
    params: ArtifactReadParams,
) -> anyhow::Result<Value> {
    let max_bytes = params.max_bytes.unwrap_or(256 * 1024).min(4 * 1024 * 1024);

    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
        "max_bytes": max_bytes,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/read",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "artifact/read".to_string(),
                    params: Some(approval_params),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.artifact;
    let effective_decision = match mode.tool_overrides.get("artifact/read").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "artifact/read".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies artifact/read".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "artifact/read",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "artifact/read".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some(approval_denied_error(remembered).to_string()),
                            result: Some(serde_json::json!({
                                "approval_policy": approval_policy,
                            })),
                        })
                        .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/read".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let (content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let meta = match read_artifact_metadata(&metadata_path).await {
        Ok(meta) => meta,
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
    };

    let bytes = match tokio::fs::read(&content_path)
        .await
        .with_context(|| format!("read {}", content_path.display()))
    {
        Ok(bytes) => bytes,
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
    };
    let truncated = bytes.len() > max_bytes as usize;
    let bytes = if truncated {
        bytes[..(max_bytes as usize)].to_vec()
    } else {
        bytes
    };
    let text = String::from_utf8_lossy(&bytes).to_string();
    let text = pm_core::redact_text(&text);

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
                "bytes": bytes.len(),
                "truncated": truncated,
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "metadata": meta,
        "text": text,
        "truncated": truncated,
        "bytes": bytes.len(),
    }))
}

async fn handle_artifact_delete(
    server: &Server,
    params: ArtifactDeleteParams,
) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/delete",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = pm_core::modes::Decision::Deny;

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "artifact/delete".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Denied,
                    error: Some("unknown mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision": decision,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = mode.permissions.artifact;
    let effective_decision = match mode.tool_overrides.get("artifact/delete").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "artifact/delete".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies artifact/delete".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": effective_decision,
        }));
    }

    if effective_decision == pm_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            &thread_rt,
            params.thread_id,
            params.turn_id,
            approval_policy,
            ApprovalRequest {
                approval_id: params.approval_id,
                action: "artifact/delete",
                params: &approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "artifact/delete".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                    thread_rt
                        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: pm_protocol::ToolStatus::Denied,
                            error: Some(approval_denied_error(remembered).to_string()),
                            result: Some(serde_json::json!({
                                "approval_policy": approval_policy,
                            })),
                        })
                        .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "thread_id": params.thread_id,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/delete".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let (content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let outcome: anyhow::Result<bool> = async {
        let mut removed = false;
        for path in [&content_path, &metadata_path] {
            match tokio::fs::remove_file(path).await {
                Ok(()) => removed = true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
            }
        }
        let history_dir =
            user_artifact_history_dir_for_thread(server, params.thread_id, params.artifact_id);
        match tokio::fs::remove_dir_all(&history_dir).await {
            Ok(()) => removed = true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", history_dir.display())),
        }
        Ok(removed)
    }
    .await;

    match outcome {
        Ok(removed) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "artifact_id": params.artifact_id,
                        "removed": removed,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "removed": removed,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}
