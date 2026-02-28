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
    let tool_id = omne_protocol::ToolId::new();
    let bytes_len = params.text.len();
    let artifact_type = params.artifact_type.clone();
    let summary = params.summary.clone();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
        "artifact_type": artifact_type,
        "summary": summary,
        "bytes": bytes_len,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/write",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return artifact_allowed_tools_denied_response(tool_id, "artifact/write", &allowed_tools);
    }

    if let Some(result) = enforce_artifact_mode_and_approval(
        server,
        ArtifactModeApprovalContext {
            thread_rt: &thread_rt,
            thread_root: &thread_root,
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            approval_id: params.approval_id,
            approval_policy,
            mode_name: &mode_name,
            action: "artifact/write",
            tool_id,
            approval_params: &approval_params,
        },
    )
    .await?
    {
        return Ok(result);
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/write".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let attention_artifact_text = params.text.clone();
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
            if let Some(marker_event) = attention_marker_event_for_artifact(
                &response,
                params.turn_id,
                &artifact_type,
                &attention_artifact_text,
            ) {
                thread_rt.append_event(marker_event).await?;
            }
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(completed),
                })
                .await?;
            Ok(response)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

struct UserArtifactWriteRequest {
    tool_id: omne_protocol::ToolId,
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
    let history_max_versions = artifact_history_max_versions();
    write_user_artifact_with_history_limit(server, req, history_max_versions).await
}

async fn write_user_artifact_with_history_limit(
    server: &Server,
    req: UserArtifactWriteRequest,
    history_max_versions: usize,
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
    let (created_at, version, created, previous_version) = match tokio::fs::metadata(&metadata_path)
        .await
    {
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

    let mut history_snapshot_version = None;
    let mut history_pruned_versions = Vec::new();
    let mut history_pruned_version_details = Vec::<PrunedArtifactHistoryVersion>::new();
    let mut history_prune_error = None;
    let mut history_prune_report_artifact_id = None;
    let mut history_prune_report_error = None;
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

    let is_prune_report_artifact = artifact_type == "artifact_prune_report";

    let text = omne_core::redact_text(&text);
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
        match prune_user_artifact_history(server, thread_id, artifact_id, history_max_versions)
            .await
        {
            Ok(version_details) => {
                history_pruned_versions = version_details.iter().map(|v| v.version).collect();
                history_pruned_version_details = version_details;
            }
            Err(err) => history_prune_error = Some(err.to_string()),
        }
    }

    if history_max_versions > 0
        && updating_existing
        && !is_prune_report_artifact
        && !history_pruned_version_details.is_empty()
    {
        match write_artifact_prune_report(
            server,
            thread_id,
            turn_id,
            tool_id,
            artifact_id,
            history_max_versions,
            &history_pruned_version_details,
        )
        .await
        {
            Ok(report_artifact_id) => history_prune_report_artifact_id = Some(report_artifact_id),
            Err(err) => history_prune_report_error = Some(err.to_string()),
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
            "pruned_version_details": history_pruned_version_details,
            "prune_error": history_prune_error,
            "prune_report_artifact_id": history_prune_report_artifact_id,
            "prune_report_error": history_prune_report_error,
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

fn infer_artifact_preview(artifact_type: &str) -> omne_protocol::ArtifactPreview {
    let (kind, title) = match artifact_type {
        "diff" => (
            omne_protocol::ArtifactPreviewKind::DiffUnified,
            Some("git diff --".to_string()),
        ),
        "patch" => (
            omne_protocol::ArtifactPreviewKind::PatchUnified,
            Some("git diff --binary --patch".to_string()),
        ),
        "html" => (omne_protocol::ArtifactPreviewKind::Html, None),
        "code" => (omne_protocol::ArtifactPreviewKind::Code, None),
        "log" | "log_excerpt" => (omne_protocol::ArtifactPreviewKind::Log, None),
        _ => (omne_protocol::ArtifactPreviewKind::Markdown, None),
    };
    omne_protocol::ArtifactPreview {
        kind,
        language: None,
        title,
    }
}

fn attention_marker_event_for_artifact(
    response: &Value,
    turn_id: Option<TurnId>,
    artifact_type: &str,
    artifact_text: &str,
) -> Option<omne_protocol::ThreadEventKind> {
    match artifact_type {
        "plan" | "diff" | "patch" | "fan_out_linkage_issue" => {
            let marker = match artifact_type {
                "plan" => omne_protocol::AttentionMarkerKind::PlanReady,
                "diff" | "patch" => omne_protocol::AttentionMarkerKind::DiffReady,
                "fan_out_linkage_issue" => omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                _ => return None,
            };
            let artifact_id: ArtifactId =
                serde_json::from_value(response.get("artifact_id")?.clone()).ok()?;
            Some(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                marker,
                turn_id,
                artifact_id: Some(artifact_id),
                artifact_type: Some(artifact_type.to_string()),
                process_id: None,
                exit_code: None,
                command: None,
            })
        }
        "fan_out_linkage_issue_clear" => {
            Some(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                turn_id,
                reason: Some("fan-out linkage issue cleared".to_string()),
            })
        }
        "fan_out_result" => {
            let artifact_id: ArtifactId =
                serde_json::from_value(response.get("artifact_id")?.clone()).ok()?;
            let parsed = parse_fan_out_result_structured_data(artifact_text)?;
            let has_auto_apply_error = parsed
                .isolated_write_auto_apply
                .as_ref()
                .and_then(|auto_apply| auto_apply.error.as_ref())
                .is_some_and(|error| !error.trim().is_empty());
            if has_auto_apply_error {
                Some(omne_protocol::ThreadEventKind::AttentionMarkerSet {
                    marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                    turn_id,
                    artifact_id: Some(artifact_id),
                    artifact_type: Some("fan_out_result".to_string()),
                    process_id: None,
                    exit_code: None,
                    command: None,
                })
            } else {
                Some(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                    marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                    turn_id,
                    reason: Some("fan-out auto-apply error cleared".to_string()),
                })
            }
        }
        _ => None,
    }
}

fn artifact_history_max_versions() -> usize {
    std::env::var("OMNE_ARTIFACT_HISTORY_MAX_VERSIONS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
struct PrunedArtifactHistoryVersion {
    version: u32,
    size_bytes: Option<u64>,
}

async fn write_artifact_prune_report(
    server: &Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    tool_id: omne_protocol::ToolId,
    source_artifact_id: ArtifactId,
    history_max_versions: usize,
    pruned: &[PrunedArtifactHistoryVersion],
) -> anyhow::Result<ArtifactId> {
    let report_artifact_id = ArtifactId::new();
    let (content_path, metadata_path) = user_artifact_paths(server, thread_id, report_artifact_id);
    let now = OffsetDateTime::now_utc();

    let summary = format!("pruned artifact history: {source_artifact_id} (kept {history_max_versions})");
    let text = omne_core::redact_text(&render_artifact_prune_report_text(
        source_artifact_id,
        history_max_versions,
        pruned,
    ));
    let bytes = text.as_bytes().to_vec();
    write_file_atomic(&content_path, &bytes).await?;

    let meta = ArtifactMetadata {
        artifact_id: report_artifact_id,
        artifact_type: "artifact_prune_report".to_string(),
        summary,
        preview: Some(infer_artifact_preview("artifact_prune_report")),
        created_at: now,
        updated_at: now,
        version: 1,
        content_path: content_path.display().to_string(),
        size_bytes: bytes.len() as u64,
        provenance: Some(ArtifactProvenance {
            thread_id,
            turn_id,
            tool_id: Some(tool_id),
            process_id: None,
        }),
    };
    let meta_bytes = serde_json::to_vec_pretty(&meta).context("serialize prune report metadata")?;
    write_file_atomic(&metadata_path, &meta_bytes).await?;
    Ok(report_artifact_id)
}

fn render_artifact_prune_report_text(
    source_artifact_id: ArtifactId,
    history_max_versions: usize,
    pruned: &[PrunedArtifactHistoryVersion],
) -> String {
    let mut text = String::new();
    text.push_str("# Artifact History Prune Report\n\n");
    text.push_str(&format!("- artifact_id: {source_artifact_id}\n"));
    text.push_str(&format!("- retained_history_versions: {history_max_versions}\n"));
    text.push_str(&format!("- pruned_count: {}\n\n", pruned.len()));
    text.push_str("| version | size_bytes |\n");
    text.push_str("| --- | --- |\n");
    for item in pruned {
        let size = item
            .size_bytes
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        text.push_str(&format!("| {} | {} |\n", item.version, size));
    }
    text
}

#[cfg(test)]
mod attention_marker_event_tests {
    use super::*;

    #[test]
    fn fan_out_linkage_issue_artifact_maps_to_attention_marker_set() {
        let artifact_id = ArtifactId::new();
        let turn_id = TurnId::new();
        let response = serde_json::json!({
            "artifact_id": artifact_id
        });

        let event = attention_marker_event_for_artifact(
            &response,
            Some(turn_id),
            "fan_out_linkage_issue",
            "",
        );
        let Some(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker,
            turn_id: event_turn_id,
            artifact_id: event_artifact_id,
            artifact_type,
            process_id,
            exit_code,
            command,
        }) = event
        else {
            panic!("expected AttentionMarkerSet for fan_out_linkage_issue");
        };
        assert_eq!(
            marker,
            omne_protocol::AttentionMarkerKind::FanOutLinkageIssue
        );
        assert_eq!(event_turn_id, Some(turn_id));
        assert_eq!(event_artifact_id, Some(artifact_id));
        assert_eq!(artifact_type.as_deref(), Some("fan_out_linkage_issue"));
        assert!(process_id.is_none());
        assert!(exit_code.is_none());
        assert!(command.is_none());
    }

    #[test]
    fn fan_out_linkage_issue_clear_artifact_maps_to_attention_marker_cleared() {
        let turn_id = TurnId::new();
        let response = serde_json::json!({});

        let event = attention_marker_event_for_artifact(
            &response,
            Some(turn_id),
            "fan_out_linkage_issue_clear",
            "",
        );
        let Some(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker,
            turn_id: event_turn_id,
            reason,
        }) = event
        else {
            panic!("expected AttentionMarkerCleared for fan_out_linkage_issue_clear");
        };
        assert_eq!(
            marker,
            omne_protocol::AttentionMarkerKind::FanOutLinkageIssue
        );
        assert_eq!(event_turn_id, Some(turn_id));
        assert_eq!(reason.as_deref(), Some("fan-out linkage issue cleared"));
    }

    #[test]
    fn fan_out_result_with_auto_apply_error_maps_to_attention_marker_set() {
        let artifact_id = ArtifactId::new();
        let turn_id = TurnId::new();
        let response = serde_json::json!({
            "artifact_id": artifact_id
        });
        let text = format!(
            "## fan-out result\n\n```json\n{}\n```",
            serde_json::json!({
                "schema_version": omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1,
                "task_id": "t-1",
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "workspace_mode": "isolated_write",
                "status": "completed",
                "isolated_write_auto_apply": {
                    "enabled": true,
                    "attempted": true,
                    "applied": false,
                    "error": "git apply --check failed"
                }
            })
        );

        let event = attention_marker_event_for_artifact(
            &response,
            Some(turn_id),
            "fan_out_result",
            &text,
        );
        let Some(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker,
            turn_id: event_turn_id,
            artifact_id: event_artifact_id,
            artifact_type,
            process_id,
            exit_code,
            command,
        }) = event
        else {
            panic!("expected AttentionMarkerSet for fan_out_result auto-apply error");
        };
        assert_eq!(
            marker,
            omne_protocol::AttentionMarkerKind::FanOutAutoApplyError
        );
        assert_eq!(event_turn_id, Some(turn_id));
        assert_eq!(event_artifact_id, Some(artifact_id));
        assert_eq!(artifact_type.as_deref(), Some("fan_out_result"));
        assert!(process_id.is_none());
        assert!(exit_code.is_none());
        assert!(command.is_none());
    }

    #[test]
    fn fan_out_result_without_auto_apply_error_maps_to_attention_marker_cleared() {
        let artifact_id = ArtifactId::new();
        let turn_id = TurnId::new();
        let response = serde_json::json!({
            "artifact_id": artifact_id
        });
        let text = format!(
            "## fan-out result\n\n```json\n{}\n```",
            serde_json::json!({
                "schema_version": omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1,
                "task_id": "t-2",
                "thread_id": "thread-2",
                "turn_id": "turn-2",
                "workspace_mode": "isolated_write",
                "status": "completed",
                "isolated_write_auto_apply": {
                    "enabled": true,
                    "attempted": true,
                    "applied": true,
                    "error": null
                }
            })
        );

        let event = attention_marker_event_for_artifact(
            &response,
            Some(turn_id),
            "fan_out_result",
            &text,
        );
        let Some(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker,
            turn_id: event_turn_id,
            reason,
        }) = event
        else {
            panic!("expected AttentionMarkerCleared for fan_out_result without auto-apply error");
        };
        assert_eq!(
            marker,
            omne_protocol::AttentionMarkerKind::FanOutAutoApplyError
        );
        assert_eq!(event_turn_id, Some(turn_id));
        assert_eq!(
            reason.as_deref(),
            Some("fan-out auto-apply error cleared")
        );
    }
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
    // Keep per-version provenance by snapshotting metadata sidecar when available.
    let (_, current_metadata_path) = user_artifact_paths(server, thread_id, artifact_id);
    let history_metadata_path =
        user_artifact_history_metadata_path(server, thread_id, artifact_id, version);
    match tokio::fs::read(&current_metadata_path).await {
        Ok(meta_bytes) => write_file_atomic(&history_metadata_path, &meta_bytes).await?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| format!("read {}", current_metadata_path.display()));
        }
    }
    Ok(true)
}

async fn prune_user_artifact_history(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
    max_versions: usize,
) -> anyhow::Result<Vec<PrunedArtifactHistoryVersion>> {
    if max_versions == 0 {
        return Ok(Vec::new());
    }

    let history_dir = user_artifact_history_dir_for_thread(server, thread_id, artifact_id);
    let mut dir = match tokio::fs::read_dir(&history_dir).await {
        Ok(dir) => dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read dir {}", history_dir.display())),
    };

    let mut content_candidates = Vec::<(u32, PathBuf)>::new();
    let mut metadata_candidates = Vec::<(u32, PathBuf)>::new();
    while let Some(entry) = dir
        .next_entry()
        .await
        .with_context(|| format!("read dir {}", history_dir.display()))?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(stem) = name
            .strip_prefix('v')
            .and_then(|name| name.strip_suffix(".md"))
        {
            let Ok(version) = stem.parse::<u32>() else {
                continue;
            };
            content_candidates.push((version, entry.path()));
            continue;
        }
        if let Some(stem) = name
            .strip_prefix('v')
            .and_then(|name| name.strip_suffix(".metadata.json"))
        {
            let Ok(version) = stem.parse::<u32>() else {
                continue;
            };
            metadata_candidates.push((version, entry.path()));
        }
    }

    content_candidates.sort_by_key(|(version, _)| *version);

    let mut kept_content_versions = std::collections::BTreeSet::<u32>::new();
    let keep_from = content_candidates.len().saturating_sub(max_versions);
    for (version, _) in content_candidates.iter().skip(keep_from) {
        kept_content_versions.insert(*version);
    }

    let mut removed = Vec::new();
    for (version, path) in content_candidates {
        if kept_content_versions.contains(&version) {
            continue;
        }
        let size_bytes = match tokio::fs::metadata(&path).await {
            Ok(meta) => Some(meta.len()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
        };
        match tokio::fs::remove_file(&path).await {
            Ok(()) => removed.push(PrunedArtifactHistoryVersion {
                version,
                size_bytes,
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }

    // Also clean metadata sidecars for removed versions and orphaned entries with no content file.
    for (version, path) in metadata_candidates {
        if kept_content_versions.contains(&version) {
            continue;
        }
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }

    Ok(removed)
}
