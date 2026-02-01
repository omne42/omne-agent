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
    let tool_id = omne_agent_protocol::ToolId::new();
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

    let catalog = omne_agent_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let decision = omne_agent_core::modes::Decision::Deny;

            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "artifact/write".to_string(),
                    params: Some(approval_params.clone()),
                })
                .await?;
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Denied,
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
    if effective_decision == omne_agent_core::modes::Decision::Deny {
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "artifact/write".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_agent_protocol::ToolStatus::Denied,
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

    if effective_decision == omne_agent_core::modes::Decision::Prompt {
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
                    .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "artifact/write".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                    thread_rt
                        .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                            tool_id,
                            status: omne_agent_protocol::ToolStatus::Denied,
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
        .append_event(omne_agent_protocol::ThreadEventKind::ToolStarted {
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
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(response)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_agent_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_agent_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

struct UserArtifactWriteRequest {
    tool_id: omne_agent_protocol::ToolId,
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

    let text = omne_agent_core::redact_text(&text);
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

fn infer_artifact_preview(artifact_type: &str) -> omne_agent_protocol::ArtifactPreview {
    let kind = match artifact_type {
        "diff" => omne_agent_protocol::ArtifactPreviewKind::DiffUnified,
        "patch" => omne_agent_protocol::ArtifactPreviewKind::PatchUnified,
        "html" => omne_agent_protocol::ArtifactPreviewKind::Html,
        "code" => omne_agent_protocol::ArtifactPreviewKind::Code,
        "log" | "log_excerpt" => omne_agent_protocol::ArtifactPreviewKind::Log,
        _ => omne_agent_protocol::ArtifactPreviewKind::Markdown,
    };
    omne_agent_protocol::ArtifactPreview {
        kind,
        language: None,
        title: None,
    }
}

fn artifact_history_max_versions() -> usize {
    std::env::var("OMNE_AGENT_ARTIFACT_HISTORY_MAX_VERSIONS")
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

