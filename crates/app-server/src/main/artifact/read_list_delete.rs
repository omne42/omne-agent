async fn emit_artifact_tool_denied(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
    turn_id: Option<TurnId>,
    action: &str,
    params: &Value,
    error: String,
    result: Value,
) -> anyhow::Result<()> {
    emit_tool_denied(
        thread_rt,
        tool_id,
        turn_id,
        action,
        Some(params.clone()),
        error,
        result,
    )
    .await
}

struct ArtifactModeApprovalContext<'a> {
    thread_rt: &'a Arc<ThreadRuntime>,
    thread_root: &'a Path,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<omne_protocol::ApprovalId>,
    approval_policy: omne_protocol::ApprovalPolicy,
    mode_name: &'a str,
    action: &'static str,
    tool_id: omne_protocol::ToolId,
    approval_params: &'a Value,
}

async fn enforce_artifact_mode_and_approval(
    server: &Server,
    ctx: ArtifactModeApprovalContext<'_>,
) -> anyhow::Result<Option<Value>> {
    let catalog = omne_core::modes::ModeCatalog::load(ctx.thread_root).await;
    let mode = match catalog.mode(ctx.mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let denied_event_result = serde_json::json!({
                "mode": ctx.mode_name,
                "decision": omne_core::modes::Decision::Deny,
                "available": available.clone(),
                "load_error": catalog.load_error.clone(),
            });
            let denied_response = artifact_unknown_mode_denied_response(
                ctx.tool_id,
                ctx.mode_name.to_string(),
                available,
                catalog.load_error.clone(),
            )?;
            emit_artifact_tool_denied(
                ctx.thread_rt,
                ctx.tool_id,
                ctx.turn_id,
                ctx.action,
                ctx.approval_params,
                "unknown mode".to_string(),
                denied_event_result,
            )
            .await?;
            return Ok(Some(denied_response));
        }
    };

    let mode_decision = resolve_mode_decision_audit(mode, ctx.action, mode.permissions.artifact);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let denied_event_result = serde_json::json!({
            "mode": ctx.mode_name,
            "decision": mode_decision.decision,
            "decision_source": mode_decision.decision_source,
            "tool_override_hit": mode_decision.tool_override_hit,
        });
        let denied_response =
            artifact_mode_denied_response(ctx.tool_id, ctx.mode_name.to_string(), mode_decision)?;
        emit_artifact_tool_denied(
            ctx.thread_rt,
            ctx.tool_id,
            ctx.turn_id,
            ctx.action,
            ctx.approval_params,
            format!("mode denies {}", ctx.action),
            denied_event_result,
        )
        .await?;
        return Ok(Some(denied_response));
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            ctx.thread_rt,
            ctx.thread_id,
            ctx.turn_id,
            ctx.approval_policy,
            ApprovalRequest {
                approval_id: ctx.approval_id,
                action: ctx.action,
                params: ctx.approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let denied_response = artifact_denied_response(ctx.tool_id, Some(remembered))?;
                emit_artifact_tool_denied(
                    ctx.thread_rt,
                    ctx.tool_id,
                    ctx.turn_id,
                    ctx.action,
                    ctx.approval_params,
                    approval_denied_error(remembered).to_string(),
                    serde_json::json!({
                        "approval_policy": ctx.approval_policy,
                    }),
                )
                .await?;
                return Ok(Some(denied_response));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                let result = artifact_needs_approval_response(ctx.thread_id, approval_id)?;
                return Ok(Some(result));
            }
        }
    }

    Ok(None)
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
    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({});
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/list",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return artifact_allowed_tools_denied_response(tool_id, "artifact/list", &allowed_tools);
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
            action: "artifact/list",
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
            tool: "artifact/list".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let dir = user_artifacts_dir_for_thread(server, params.thread_id);

    let outcome: anyhow::Result<(
        Vec<ArtifactMetadata>,
        Vec<omne_app_server_protocol::ArtifactListError>,
    )> = async {
        let mut artifacts = Vec::<ArtifactMetadata>::new();
        let mut errors = Vec::<omne_app_server_protocol::ArtifactListError>::new();

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
                Err(err) => errors.push(omne_app_server_protocol::ArtifactListError {
                    path: path.display().to_string(),
                    error: err.to_string(),
                }),
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
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "artifacts": artifacts.len(),
                        "errors": errors.len(),
                    })),
                })
                .await?;

            let response = omne_app_server_protocol::ArtifactListResponse {
                tool_id,
                artifacts,
                errors,
            };
            serde_json::to_value(response).context("serialize artifact/list response")
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
    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
        "version": params.version,
        "max_bytes": max_bytes,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/read",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return artifact_allowed_tools_denied_response(tool_id, "artifact/read", &allowed_tools);
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
            action: "artifact/read",
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
            tool: "artifact/read".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let (current_content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let meta = match read_artifact_metadata(&metadata_path).await {
        Ok(meta) => meta,
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
    };

    let latest_version = meta.version;
    let selected_version = params.version.unwrap_or(latest_version);
    if selected_version == 0 {
        let err = anyhow::anyhow!("artifact version must be >= 1");
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some(err.to_string()),
                result: None,
            })
            .await?;
        return Err(err);
    }

    let (read_path, historical) = if selected_version == latest_version {
        (current_content_path, false)
    } else if selected_version < latest_version {
        (
            user_artifact_history_path(
                server,
                params.thread_id,
                params.artifact_id,
                selected_version,
            ),
            true,
        )
    } else {
        let err = anyhow::anyhow!(
            "artifact version not found: requested={}, latest={}",
            selected_version,
            latest_version
        );
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some(err.to_string()),
                result: None,
            })
            .await?;
        return Err(err);
    };

    let full_bytes = match tokio::fs::read(&read_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && historical => {
            let err = anyhow::anyhow!(
                "artifact version not retained: requested={}, latest={}",
                selected_version,
                latest_version
            );
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
        Err(err) => {
            let err = anyhow::Error::from(err).context(format!("read {}", read_path.display()));
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
    };
    let selected_size_bytes = full_bytes.len() as u64;
    let redacted_full_text = omne_core::redact_text(&String::from_utf8_lossy(&full_bytes));
    let truncated = full_bytes.len() > max_bytes as usize;
    let bytes = if truncated {
        full_bytes[..(max_bytes as usize)].to_vec()
    } else {
        full_bytes
    };
    let text = if truncated {
        omne_core::redact_text(&String::from_utf8_lossy(&bytes))
    } else {
        redacted_full_text.clone()
    };

    let (mut selected_meta, metadata_source, metadata_fallback_reason) = if historical {
        let history_metadata_path = user_artifact_history_metadata_path(
            server,
            params.thread_id,
            params.artifact_id,
            selected_version,
        );
        match tokio::fs::metadata(&history_metadata_path).await {
            Ok(_) => match read_artifact_metadata(&history_metadata_path).await {
                Ok(history_meta) => (
                    history_meta,
                    omne_protocol::ArtifactReadMetadataSource::HistorySnapshot,
                    None,
                ),
                Err(err) => (
                    meta.clone(),
                    omne_protocol::ArtifactReadMetadataSource::LatestFallback,
                    Some(classify_history_metadata_read_error(&err)),
                ),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (
                meta.clone(),
                omne_protocol::ArtifactReadMetadataSource::LatestFallback,
                Some(omne_protocol::ArtifactReadMetadataFallbackReason::HistoryMetadataMissing),
            ),
            Err(_) => (
                meta.clone(),
                omne_protocol::ArtifactReadMetadataSource::LatestFallback,
                Some(omne_protocol::ArtifactReadMetadataFallbackReason::HistoryMetadataUnreadable),
            ),
        }
    } else {
        (
            meta.clone(),
            omne_protocol::ArtifactReadMetadataSource::Latest,
            None,
        )
    };
    selected_meta.version = selected_version;
    selected_meta.content_path = read_path.display().to_string();
    selected_meta.size_bytes = selected_size_bytes;

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
                "bytes": bytes.len(),
                "truncated": truncated,
                "version": selected_version,
                "latest_version": latest_version,
                "historical": historical,
                "metadata_source": metadata_source,
                "metadata_fallback_reason": metadata_fallback_reason,
            })),
        })
        .await?;

    let prune_report = if selected_meta.artifact_type == "artifact_prune_report" {
        parse_artifact_prune_report_read_payload(&selected_meta.summary, text.as_str())
    } else {
        None
    };
    let fan_in_summary = if selected_meta.artifact_type == "fan_in_summary" {
        parse_fan_in_summary_structured_data(redacted_full_text.as_str())
    } else {
        None
    };
    let fan_out_linkage_issue = if selected_meta.artifact_type == "fan_out_linkage_issue" {
        parse_fan_out_linkage_issue_structured_data(redacted_full_text.as_str())
    } else {
        None
    };
    let fan_out_linkage_issue_clear = if selected_meta.artifact_type == "fan_out_linkage_issue_clear" {
        parse_fan_out_linkage_issue_clear_structured_data(redacted_full_text.as_str())
    } else {
        None
    };
    let fan_out_result = if selected_meta.artifact_type == "fan_out_result" {
        parse_fan_out_result_structured_data(redacted_full_text.as_str())
    } else {
        None
    };
    let response = omne_app_server_protocol::ArtifactReadResponse {
        tool_id,
        metadata: selected_meta,
        text,
        truncated,
        bytes: bytes.len() as u64,
        version: selected_version,
        latest_version,
        historical,
        metadata_source,
        metadata_fallback_reason,
        prune_report,
        fan_in_summary,
        fan_out_linkage_issue,
        fan_out_linkage_issue_clear,
        fan_out_result,
    };
    serde_json::to_value(response).context("serialize artifact/read response")
}

fn artifact_denied_response(
    tool_id: omne_protocol::ToolId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    denied_response_with_remembered(
        tool_id,
        remembered,
        "serialize artifact denied response",
        |tool_id, remembered, error_code| omne_app_server_protocol::ArtifactDeniedResponse {
            tool_id,
            denied: true,
            error_code,
            remembered,
        },
    )
}

fn artifact_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        error_code: Some("allowed_tools_denied".to_string()),
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
    };
    serde_json::to_value(response).context("serialize artifact allowed_tools denied response")
}

fn artifact_needs_approval_response(
    thread_id: omne_protocol::ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Value> {
    needs_approval_response_json(
        approval_id,
        "serialize artifact needs_approval response",
        |approval_id| omne_app_server_protocol::ArtifactNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        },
    )
}

fn artifact_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode: String,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ArtifactModeDeniedResponse {
        tool_id,
        denied: true,
        error_code: Some("mode_denied".to_string()),
        mode,
        decision: map_mode_decision_for_protocol!(
            mode_decision.decision,
            omne_app_server_protocol::ArtifactModeDecision
        ),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
    };
    serde_json::to_value(response).context("serialize artifact mode denied response")
}

fn artifact_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    mode: String,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let response = omne_app_server_protocol::ArtifactUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        error_code: Some("mode_unknown".to_string()),
        mode,
        decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
        available,
        load_error,
    };
    serde_json::to_value(response).context("serialize artifact unknown mode denied response")
}

fn classify_history_metadata_read_error(
    err: &anyhow::Error,
) -> omne_protocol::ArtifactReadMetadataFallbackReason {
    if err.chain().any(|cause| cause.is::<serde_json::Error>()) {
        omne_protocol::ArtifactReadMetadataFallbackReason::HistoryMetadataInvalid
    } else {
        omne_protocol::ArtifactReadMetadataFallbackReason::HistoryMetadataUnreadable
    }
}

fn parse_artifact_prune_report_summary(summary: &str) -> (Option<String>, Option<usize>) {
    let trimmed = summary.trim();
    let prefix = "pruned artifact history: ";
    let Some(without_prefix) = trimmed.strip_prefix(prefix) else {
        return (None, None);
    };
    let Some((artifact_id, rest)) = without_prefix.rsplit_once(" (kept ") else {
        let artifact_id = without_prefix.trim();
        let artifact = (!artifact_id.is_empty()).then(|| artifact_id.to_string());
        return (artifact, None);
    };
    let kept = rest
        .strip_suffix(')')
        .and_then(|value| value.trim().parse::<usize>().ok());
    let artifact_id = artifact_id.trim();
    let artifact = (!artifact_id.is_empty()).then(|| artifact_id.to_string());
    (artifact, kept)
}

fn parse_artifact_prune_report_read_payload(
    summary: &str,
    text: &str,
) -> Option<omne_app_server_protocol::ArtifactPruneReportReadPayload> {
    let (mut source_artifact_id, mut retained_history_versions) =
        parse_artifact_prune_report_summary(summary);
    let mut pruned_count = None;
    let mut pruned_version_details =
        Vec::<omne_app_server_protocol::ArtifactPruneReportVersionDetail>::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some(value) = line.strip_prefix("- artifact_id:") {
            let value = value.trim();
            if !value.is_empty() {
                source_artifact_id = Some(value.to_string());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("- retained_history_versions:") {
            retained_history_versions = value.trim().parse::<usize>().ok();
            continue;
        }
        if let Some(value) = line.strip_prefix("- pruned_count:") {
            pruned_count = value.trim().parse::<usize>().ok();
            continue;
        }

        if !line.starts_with('|') {
            continue;
        }
        let cells = line
            .split('|')
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect::<Vec<_>>();
        if cells.len() != 2 {
            continue;
        }
        if cells[0].eq_ignore_ascii_case("version") || cells[0].starts_with("---") {
            continue;
        }
        let Some(version) = cells[0].parse::<u32>().ok() else {
            continue;
        };
        let size_bytes = if cells[1] == "-" {
            None
        } else {
            cells[1].parse::<u64>().ok()
        };
        pruned_version_details.push(omne_app_server_protocol::ArtifactPruneReportVersionDetail {
            version,
            size_bytes,
        });
    }

    if pruned_count.is_none() && !pruned_version_details.is_empty() {
        pruned_count = Some(pruned_version_details.len());
    }

    if source_artifact_id.is_none()
        && retained_history_versions.is_none()
        && pruned_count.is_none()
        && pruned_version_details.is_empty()
    {
        return None;
    }

    Some(omne_app_server_protocol::ArtifactPruneReportReadPayload {
        source_artifact_id,
        retained_history_versions,
        pruned_count,
        pruned_version_details,
    })
}

fn parse_json_fenced_block(text: &str) -> Option<String> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if !trimmed.starts_with("```json") {
            continue;
        }
        let mut json_lines = Vec::<&str>::new();
        for json_line in lines.by_ref() {
            if json_line.trim() == "```" {
                let payload = json_lines.join("\n");
                if payload.trim().is_empty() {
                    return None;
                }
                return Some(payload);
            }
            json_lines.push(json_line);
        }
        return None;
    }
    None
}

fn parse_fan_in_summary_structured_json_block(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let mut in_section = false;
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if !in_section {
            if trimmed == "## Structured Data" {
                in_section = true;
            }
            continue;
        }

        if trimmed.starts_with("## ") {
            return None;
        }

        if !trimmed.starts_with("```json") {
            continue;
        }

        let mut json_lines = Vec::<&str>::new();
        for json_line in lines.by_ref() {
            let trimmed_json_line = json_line.trim();
            if trimmed_json_line == "```" {
                let payload = json_lines.join("\n");
                if payload.trim().is_empty() {
                    return None;
                }
                return Some(payload);
            }
            json_lines.push(json_line);
        }
        return None;
    }
    None
}

fn parse_fan_out_linkage_issue_structured_data(
    text: &str,
) -> Option<omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData> {
    let json_payload = parse_fan_in_summary_structured_json_block(text)?;
    let payload: omne_workflow_spec::FanOutLinkageIssueStructuredData =
        serde_json::from_str(&json_payload).ok()?;
    if payload.schema_version != omne_workflow_spec::FAN_OUT_LINKAGE_ISSUE_SCHEMA_V1 {
        return None;
    }

    Some(
        omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
            schema_version: payload.schema_version,
            fan_in_summary_artifact_id: payload.fan_in_summary_artifact_id,
            issue: payload.issue,
            issue_truncated: payload.issue_truncated,
        },
    )
}

fn parse_fan_out_linkage_issue_clear_structured_data(
    text: &str,
) -> Option<omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData> {
    let json_payload = parse_fan_in_summary_structured_json_block(text)?;
    let payload: omne_workflow_spec::FanOutLinkageIssueClearStructuredData =
        serde_json::from_str(&json_payload).ok()?;
    if payload.schema_version != omne_workflow_spec::FAN_OUT_LINKAGE_ISSUE_CLEAR_SCHEMA_V1 {
        return None;
    }

    Some(
        omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
            schema_version: payload.schema_version,
            fan_in_summary_artifact_id: payload.fan_in_summary_artifact_id,
        },
    )
}

fn parse_fan_out_result_structured_data(
    text: &str,
) -> Option<omne_app_server_protocol::ArtifactFanOutResultStructuredData> {
    let json_payload = parse_json_fenced_block(text)?;
    let payload: omne_workflow_spec::FanOutResultStructuredData =
        serde_json::from_str(&json_payload).ok()?;
    if payload.schema_version != omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1 {
        return None;
    }
    Some(omne_app_server_protocol::ArtifactFanOutResultStructuredData {
        schema_version: payload.schema_version,
        task_id: payload.task_id,
        thread_id: payload.thread_id,
        turn_id: payload.turn_id,
        workspace_mode: payload.workspace_mode,
        workspace_cwd: payload.workspace_cwd,
        isolated_write_patch: payload.isolated_write_patch.map(|patch| {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWritePatchStructuredData {
                artifact_type: patch.artifact_type,
                artifact_id: patch.artifact_id,
                truncated: patch.truncated,
                read_cmd: patch.read_cmd,
                workspace_cwd: patch.workspace_cwd,
                error: patch.error,
            }
        }),
        isolated_write_handoff: payload.isolated_write_handoff.map(|handoff| {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteHandoffStructuredData {
                workspace_cwd: handoff.workspace_cwd,
                status_argv: handoff.status_argv,
                diff_argv: handoff.diff_argv,
                apply_patch_hint: handoff.apply_patch_hint,
                patch: handoff.patch.map(|patch| {
                    omne_app_server_protocol::ArtifactFanOutResultIsolatedWritePatchStructuredData {
                        artifact_type: patch.artifact_type,
                        artifact_id: patch.artifact_id,
                        truncated: patch.truncated,
                        read_cmd: patch.read_cmd,
                        workspace_cwd: patch.workspace_cwd,
                        error: patch.error,
                    }
                }),
            }
        }),
        isolated_write_auto_apply: payload.isolated_write_auto_apply.map(|auto_apply| {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyStructuredData {
                enabled: auto_apply.enabled,
                attempted: auto_apply.attempted,
                applied: auto_apply.applied,
                workspace_cwd: auto_apply.workspace_cwd,
                target_workspace_cwd: auto_apply.target_workspace_cwd,
                check_argv: auto_apply.check_argv,
                apply_argv: auto_apply.apply_argv,
                patch_artifact_id: auto_apply.patch_artifact_id,
                patch_read_cmd: auto_apply.patch_read_cmd,
                failure_stage: auto_apply
                    .failure_stage
                    .map(map_auto_apply_failure_stage),
                recovery_hint: auto_apply.recovery_hint,
                recovery_commands: auto_apply
                    .recovery_commands
                    .into_iter()
                    .map(|command| {
                        omne_app_server_protocol::ArtifactFanOutResultRecoveryCommandStructuredData {
                            label: command.label,
                            argv: command.argv,
                        }
                    })
                    .collect(),
                error: auto_apply.error,
            }
        }),
        status: payload.status,
        reason: payload.reason,
    })
}

fn map_auto_apply_failure_stage(
    stage: omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage,
) -> omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage {
    match stage {
        omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Precondition => {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::Precondition
        }
        omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CapturePatch => {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CapturePatch
        }
        omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch => {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch
        }
        omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::ApplyPatch => {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::ApplyPatch
        }
        omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Unknown => {
            omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::Unknown
        }
    }
}

fn parse_fan_in_summary_structured_data(
    text: &str,
) -> Option<omne_app_server_protocol::ArtifactFanInSummaryStructuredData> {
    let json_payload = parse_fan_in_summary_structured_json_block(text)?;
    let payload: omne_workflow_spec::FanInSummaryStructuredData =
        serde_json::from_str(&json_payload).ok()?;
    if payload.schema_version != omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1 {
        return None;
    }

    Some(
        omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: payload.schema_version,
            thread_id: payload.thread_id,
            task_count: payload.task_count,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: payload.scheduling.env_max_concurrent_subagents,
                effective_concurrency_limit: payload.scheduling.effective_concurrency_limit,
                priority_aging_rounds: payload.scheduling.priority_aging_rounds,
            },
            tasks: payload
                .tasks
                .into_iter()
                .map(|task| omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: task.task_id,
                    title: task.title,
                    thread_id: task.thread_id,
                    turn_id: task.turn_id,
                    status: task.status,
                    reason: task.reason,
                    dependency_blocked: task.dependency_blocked,
                    dependency_blocker_task_id: task.dependency_blocker_task_id,
                    dependency_blocker_status: task.dependency_blocker_status,
                    result_artifact_id: task.result_artifact_id,
                    result_artifact_error: task.result_artifact_error,
                    result_artifact_error_id: task.result_artifact_error_id,
                    result_artifact_diagnostics: task.result_artifact_diagnostics.map(
                        |diagnostics| {
                            omne_app_server_protocol::ArtifactFanInSummaryResultArtifactDiagnostics {
                                scan_last_seq: diagnostics.scan_last_seq,
                                matched_completion_count: diagnostics.matched_completion_count,
                                pending_matching_tool_ids: diagnostics.pending_matching_tool_ids,
                            }
                        },
                    ),
                    pending_approval: task.pending_approval.map(|pending| {
                        omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                            approval_id: pending.approval_id,
                            action: pending.action,
                            summary: pending.summary,
                            approve_cmd: pending.approve_cmd,
                            deny_cmd: pending.deny_cmd,
                        }
                    }),
                })
                .collect::<Vec<_>>(),
        },
    )
}

fn parse_history_version_filename(name: &str) -> Option<u32> {
    let stem = name.strip_prefix('v')?.strip_suffix(".md")?;
    stem.parse::<u32>().ok()
}

async fn list_artifact_history_versions(history_dir: &std::path::Path) -> anyhow::Result<Vec<u32>> {
    let mut versions = Vec::<u32>::new();
    let mut read_dir = match tokio::fs::read_dir(history_dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(versions),
        Err(err) => return Err(err).with_context(|| format!("read {}", history_dir.display())),
    };

    loop {
        let Some(entry) = read_dir
            .next_entry()
            .await
            .with_context(|| format!("read {}", history_dir.display()))?
        else {
            break;
        };
        let file_type = entry
            .file_type()
            .await
            .with_context(|| format!("stat {}", entry.path().display()))?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(version) = parse_history_version_filename(name) {
            versions.push(version);
        }
    }

    versions.sort_unstable();
    versions.dedup();
    Ok(versions)
}

async fn handle_artifact_versions(
    server: &Server,
    params: ArtifactVersionsParams,
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
    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/versions",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return artifact_allowed_tools_denied_response(tool_id, "artifact/versions", &allowed_tools);
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
            action: "artifact/versions",
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
            tool: "artifact/versions".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let (_, metadata_path) = user_artifact_paths(server, params.thread_id, params.artifact_id);
    let meta = match read_artifact_metadata(&metadata_path).await {
        Ok(meta) => meta,
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
    };

    let history_dir =
        user_artifact_history_dir_for_thread(server, params.thread_id, params.artifact_id);
    let mut history_versions = match list_artifact_history_versions(&history_dir).await {
        Ok(versions) => versions,
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            return Err(err);
        }
    };

    let latest_version = meta.version;
    let mut versions = history_versions.clone();
    versions.push(latest_version);
    versions.sort_unstable();
    versions.dedup();
    versions.reverse();

    history_versions.sort_unstable();
    history_versions.reverse();

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: omne_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
                "latest_version": latest_version,
                "versions": versions.len(),
            })),
        })
        .await?;

    let response = omne_app_server_protocol::ArtifactVersionsResponse {
        tool_id,
        artifact_id: params.artifact_id,
        latest_version,
        versions,
        history_versions,
    };
    serde_json::to_value(response).context("serialize artifact/versions response")
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
    let tool_id = omne_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "artifact_id": params.artifact_id,
    });
    if let Some(_result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "artifact/delete",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return artifact_allowed_tools_denied_response(tool_id, "artifact/delete", &allowed_tools);
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
            action: "artifact/delete",
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
            Err(err) => {
                return Err(err).with_context(|| format!("remove {}", history_dir.display()));
            }
        }
        Ok(removed)
    }
    .await;

    match outcome {
        Ok(removed) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "artifact_id": params.artifact_id,
                        "removed": removed,
                    })),
                })
                .await?;

            let response = omne_app_server_protocol::ArtifactDeleteResponse { tool_id, removed };
            serde_json::to_value(response).context("serialize artifact/delete response")
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
