async fn build_conversation(
    server: &super::Server,
    thread_id: ThreadId,
) -> anyhow::Result<Vec<OpenAiItem>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    let mut input = Vec::new();

    if let Some((meta, summary_text)) = load_latest_summary_artifact(server, thread_id).await? {
        let event_limit = parse_env_usize(
            "OMNE_AGENT_SUMMARY_CONTEXT_EVENT_LIMIT",
            DEFAULT_SUMMARY_CONTEXT_EVENT_LIMIT,
            0,
            MAX_SUMMARY_CONTEXT_EVENT_LIMIT,
        );

        let summary_text = omne_agent_core::redact_text(&summary_text);
        if !summary_text.trim().is_empty() {
            input.push(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": format!(
                        "{}\n\n# Context summary\n\n{}\n\n(summary artifact_id: {})",
                        AUTO_CONTEXT_SUMMARY_DISCLAIMER,
                        summary_text.trim(),
                        meta.artifact_id
                    ),
                }]
            }));
        }

        let mut start_idx = 0usize;
        if let Some(summary_turn_id) = meta.provenance.as_ref().and_then(|p| p.turn_id) {
            if let Some(idx) = events.iter().rposition(|event| {
                matches!(
                    &event.kind,
                    ThreadEventKind::TurnCompleted { turn_id, .. } if *turn_id == summary_turn_id
                )
            }) {
                start_idx = idx + 1;
            }
        }

        let mut slice = &events[start_idx..];
        if event_limit > 0 && slice.len() > event_limit {
            slice = &slice[slice.len().saturating_sub(event_limit)..];
        }

        for event in slice {
            match &event.kind {
                ThreadEventKind::TurnStarted { input: text, .. } => {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": text }]
                    }));
                }
                ThreadEventKind::AssistantMessage { text, .. } => {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }]
                    }));
                }
                other => {
                    if let Some(text) = format_event_for_context(other) {
                        input.push(serde_json::json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }]
                        }));
                    }
                }
            }
        }

        return Ok(input);
    }

    for event in events {
        match event.kind {
            ThreadEventKind::TurnStarted { input: text, .. } => {
                input.push(serde_json::json!({
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": text }]
                }));
            }
            ThreadEventKind::AssistantMessage { text, .. } => {
                input.push(serde_json::json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": text }]
                }));
            }
            other => {
                if let Some(text) = format_event_for_context(&other) {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }]
                    }));
                }
            }
        }
    }
    Ok(input)
}

async fn load_turn_context_refs(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> anyhow::Result<Vec<omne_agent_protocol::ContextRef>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    for event in events.iter().rev() {
        let ThreadEventKind::TurnStarted {
            turn_id: ev_turn_id,
            context_refs,
            ..
        } = &event.kind
        else {
            continue;
        };
        if *ev_turn_id != turn_id {
            continue;
        }
        return Ok(context_refs.clone().unwrap_or_default());
    }

    Ok(Vec::new())
}

async fn load_turn_attachments(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
) -> anyhow::Result<Vec<omne_agent_protocol::TurnAttachment>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    for event in events.iter().rev() {
        let ThreadEventKind::TurnStarted {
            turn_id: ev_turn_id,
            attachments,
            ..
        } = &event.kind
        else {
            continue;
        };
        if *ev_turn_id != turn_id {
            continue;
        }
        return Ok(attachments.clone().unwrap_or_default());
    }

    Ok(Vec::new())
}

fn infer_image_media_type(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn filename_from_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn filename_from_url(url: &str) -> Option<String> {
    let url = url.split('?').next().unwrap_or(url);
    url.rsplit('/')
        .next()
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.to_string())
}

async fn resolve_attachment_path_and_size(
    thread_root: &Path,
    path: &str,
    max_bytes: u64,
) -> anyhow::Result<(PathBuf, u64)> {
    let rel = Path::new(path);
    if rel.file_name() == Some(std::ffi::OsStr::new(".env")) {
        anyhow::bail!("refusing to attach secrets file (.env)");
    }

    let resolved = omne_agent_core::resolve_file(thread_root, rel, omne_agent_core::PathAccess::Read, false)
        .await
        .with_context(|| format!("resolve attachment path: {}", rel.display()))?;

    if resolved.file_name() == Some(std::ffi::OsStr::new(".env")) {
        anyhow::bail!("refusing to attach secrets file (.env)");
    }

    let metadata = tokio::fs::metadata(&resolved)
        .await
        .with_context(|| format!("stat {}", resolved.display()))?;
    let size_bytes = metadata.len();
    if size_bytes > max_bytes {
        anyhow::bail!(
            "attachment too large: path={} bytes={} max_bytes={}",
            rel.display(),
            size_bytes,
            max_bytes
        );
    }

    Ok((resolved, size_bytes))
}

#[derive(Debug, Clone)]
enum ResolvedAttachment {
    ImageUrl { url: String },
    ImageBytes { media_type: String, bytes: Vec<u8> },
    FileUrl {
        url: String,
        filename: Option<String>,
        media_type: String,
    },
    FilePath {
        path: String,
        resolved: PathBuf,
        filename: Option<String>,
        media_type: String,
        size_bytes: u64,
    },
}

async fn resolve_turn_attachments(
    thread_root: Option<&Path>,
    mode_name: &str,
    allowed_tools: Option<&[String]>,
    attachments: &[omne_agent_protocol::TurnAttachment],
    max_bytes: u64,
) -> anyhow::Result<Vec<ResolvedAttachment>> {
    if max_bytes == 0 {
        anyhow::bail!("attachments are disabled (max_bytes=0)");
    }

    let has_local_paths = attachments.iter().any(|attachment| match attachment {
        omne_agent_protocol::TurnAttachment::Image(image) => {
            matches!(image.source, omne_agent_protocol::AttachmentSource::Path { .. })
        }
        omne_agent_protocol::TurnAttachment::File(file) => {
            matches!(file.source, omne_agent_protocol::AttachmentSource::Path { .. })
        }
    });

    if has_local_paths {
        if let Some(allowed_tools) = allowed_tools
            && !allowed_tools.iter().any(|allowed| allowed == "file/read")
        {
            let allowed_json = serde_json::to_string(allowed_tools)
                .unwrap_or_else(|_| format!("{allowed_tools:?}"));
            anyhow::bail!(
                "attachments with local paths require file/read to be allowed (thread allowed_tools={allowed_json})"
            );
        }

        let Some(thread_root) = thread_root else {
            anyhow::bail!("cannot attach local files without thread cwd/root");
        };

        let catalog = omne_agent_core::modes::ModeCatalog::load(thread_root).await;
        let mode = match catalog.mode(mode_name) {
            Some(mode) => mode,
            None => {
                let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
                anyhow::bail!(
                    "unknown mode: {mode_name} (available: {available}; load_error={})",
                    catalog.load_error.as_deref().unwrap_or("")
                );
            }
        };

        for attachment in attachments {
            let path = match attachment {
                omne_agent_protocol::TurnAttachment::Image(image) => match &image.source {
                    omne_agent_protocol::AttachmentSource::Path { path } => Some(path.as_str()),
                    _ => None,
                },
                omne_agent_protocol::TurnAttachment::File(file) => match &file.source {
                    omne_agent_protocol::AttachmentSource::Path { path } => Some(path.as_str()),
                    _ => None,
                },
            };
            let Some(path) = path else {
                continue;
            };

            let rel = omne_agent_core::modes::relative_path_under_root(thread_root, Path::new(path));
            let base_decision = match rel.as_ref() {
                Ok(rel) if mode.permissions.edit.is_denied(rel) => omne_agent_core::modes::Decision::Deny,
                Ok(_) => mode.permissions.read,
                Err(_) => omne_agent_core::modes::Decision::Deny,
            };
            let effective_decision = match mode.tool_overrides.get("file/read").copied() {
                Some(override_decision) => base_decision.combine(override_decision),
                None => base_decision,
            };
            if effective_decision != omne_agent_core::modes::Decision::Allow {
                anyhow::bail!(
                    "mode denies file attachment read: mode={mode_name} decision={effective_decision:?} path={path}"
                );
            }
        }
    }

    let mut out = Vec::new();
    for attachment in attachments {
        match attachment {
            omne_agent_protocol::TurnAttachment::Image(image) => match &image.source {
                omne_agent_protocol::AttachmentSource::Url { url } => {
                    out.push(ResolvedAttachment::ImageUrl { url: url.clone() });
                }
                omne_agent_protocol::AttachmentSource::Path { path } => {
                    let Some(thread_root) = thread_root else {
                        anyhow::bail!("cannot attach local files without thread cwd/root");
                    };
                    let media_type = image
                        .media_type
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| infer_image_media_type(path))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "unsupported image type: path={path} (expected: png/jpg/jpeg/webp/gif)"
                            )
                        })?;
                    let (resolved, _size_bytes) =
                        resolve_attachment_path_and_size(thread_root, path, max_bytes).await?;
                    let bytes = tokio::fs::read(&resolved)
                        .await
                        .with_context(|| format!("read {}", resolved.display()))?;
                    out.push(ResolvedAttachment::ImageBytes {
                        media_type: media_type.to_string(),
                        bytes,
                    });
                }
            },
            omne_agent_protocol::TurnAttachment::File(file) => match &file.source {
                omne_agent_protocol::AttachmentSource::Url { url } => {
                    let filename = file
                        .filename
                        .clone()
                        .or_else(|| filename_from_url(url));
                    out.push(ResolvedAttachment::FileUrl {
                        url: url.clone(),
                        filename,
                        media_type: file.media_type.clone(),
                    });
                }
                omne_agent_protocol::AttachmentSource::Path { path } => {
                    let Some(thread_root) = thread_root else {
                        anyhow::bail!("cannot attach local files without thread cwd/root");
                    };
                    let (resolved, size_bytes) =
                        resolve_attachment_path_and_size(thread_root, path, max_bytes).await?;
                    let filename = file
                        .filename
                        .clone()
                        .or_else(|| filename_from_path(path));
                    out.push(ResolvedAttachment::FilePath {
                        path: path.clone(),
                        resolved,
                        filename,
                        media_type: file.media_type.clone(),
                        size_bytes,
                    });
                }
            },
        }
    }

    Ok(out)
}

async fn attachments_to_ditto_parts_for_provider(
    thread_id: ThreadId,
    turn_id: TurnId,
    provider_name: &str,
    runtime: &ProviderRuntime,
    attachments: &[ResolvedAttachment],
    pdf_file_id_upload_min_bytes: u64,
) -> anyhow::Result<Vec<ditto_llm::ContentPart>> {
    let mut out = Vec::new();

    for attachment in attachments {
        match attachment {
            ResolvedAttachment::ImageUrl { url } => {
                out.push(ditto_llm::ContentPart::Image {
                    source: ditto_llm::ImageSource::Url { url: url.clone() },
                });
            }
            ResolvedAttachment::ImageBytes { media_type, bytes } => {
                let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                out.push(ditto_llm::ContentPart::Image {
                    source: ditto_llm::ImageSource::Base64 {
                        media_type: media_type.clone(),
                        data,
                    },
                });
            }
            ResolvedAttachment::FileUrl {
                url,
                filename,
                media_type,
            } => {
                out.push(ditto_llm::ContentPart::File {
                    filename: filename.clone(),
                    media_type: media_type.clone(),
                    source: ditto_llm::FileSource::Url { url: url.clone() },
                });
            }
            ResolvedAttachment::FilePath {
                path,
                resolved,
                filename,
                media_type,
                size_bytes,
            } => {
                let should_upload_as_file_id = media_type == "application/pdf"
                    && pdf_file_id_upload_min_bytes > 0
                    && *size_bytes >= pdf_file_id_upload_min_bytes;

                if should_upload_as_file_id {
                    if let Some(uploader) = runtime.file_uploader.as_ref() {
                        let filename = filename
                            .clone()
                            .unwrap_or_else(|| "file.pdf".to_string());
                        let bytes = tokio::fs::read(resolved)
                            .await
                            .with_context(|| {
                                format!("read attachment path={path} resolved={}", resolved.display())
                            })?;
                        match uploader
                            .upload_file(filename.clone(), bytes)
                            .await
                        {
                            Ok(file_id) => {
                                out.push(ditto_llm::ContentPart::File {
                                    filename: Some(filename),
                                    media_type: media_type.clone(),
                                    source: ditto_llm::FileSource::FileId { file_id },
                                });
                                continue;
                            }
                            Err(err) => {
                                tracing::warn!(
                                    thread_id = %thread_id,
                                    turn_id = %turn_id,
                                    provider = provider_name,
                                    path = path,
                                    error = %err,
                                    "failed to upload pdf attachment; falling back to base64"
                                );
                            }
                        }
                    }
                }

                let bytes = tokio::fs::read(resolved)
                    .await
                    .with_context(|| {
                        format!("read attachment path={path} resolved={}", resolved.display())
                    })?;
                let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                out.push(ditto_llm::ContentPart::File {
                    filename: filename.clone(),
                    media_type: media_type.clone(),
                    source: ditto_llm::FileSource::Base64 { data },
                });
            }
        }
    }

    Ok(out)
}

async fn context_refs_to_messages(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    refs: &[omne_agent_protocol::ContextRef],
    cancel: CancellationToken,
) -> anyhow::Result<Vec<OpenAiItem>> {
    const DEFAULT_CONTEXT_FILE_MAX_BYTES: u64 = 64 * 1024;
    const MAX_CONTEXT_FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;
    const DEFAULT_CONTEXT_DIFF_MAX_BYTES: u64 = 1024 * 1024;
    const MAX_CONTEXT_DIFF_MAX_BYTES: u64 = 16 * 1024 * 1024;

    let mut out = Vec::new();

    for ctx in refs {
        match ctx {
            omne_agent_protocol::ContextRef::File(file) => {
                let max_bytes = file
                    .max_bytes
                    .unwrap_or(DEFAULT_CONTEXT_FILE_MAX_BYTES)
                    .min(MAX_CONTEXT_FILE_MAX_BYTES);

                let args = serde_json::json!({
                    "path": file.path,
                    "max_bytes": max_bytes,
                });

                let (output, hook_messages) =
                    match run_tool_call(
                        server,
                        thread_id,
                        Some(turn_id),
                        "file_read",
                        args,
                        cancel.clone(),
                        true,
                    )
                    .await
                    {
                        Ok(outcome) => (outcome.output, outcome.hook_messages),
                        Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                    };

                out.extend(hook_messages);

                let denied = output
                    .get("denied")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let resolved_path = output
                    .get("resolved_path")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let truncated = output
                    .get("truncated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut text = output.get("text").and_then(Value::as_str).unwrap_or("").to_string();

                if !denied && (file.start_line.is_some() || file.end_line.is_some()) {
                    let start_line = file.start_line.unwrap_or(1);
                    let end_line = file.end_line;
                    let lines = text.lines().collect::<Vec<_>>();
                    let start_idx = usize::try_from(start_line.saturating_sub(1)).unwrap_or(usize::MAX);
                    let end_idx = end_line
                        .and_then(|v| usize::try_from(v).ok())
                        .unwrap_or(lines.len());
                    if start_idx < lines.len() {
                        let end_idx = end_idx.clamp(start_idx, lines.len());
                        text = lines[start_idx..end_idx].join("\n");
                    } else if !truncated {
                        text.clear();
                    }
                }

                let mut msg = String::new();
                msg.push_str("# Context (@file)\n\n");
                msg.push_str(&format!("path: {}\n", file.path.trim()));
                if let Some(start) = file.start_line {
                    msg.push_str(&format!(
                        "range: L{}{}\n",
                        start,
                        file.end_line.map(|e| format!("-L{}", e)).unwrap_or_default()
                    ));
                }
                if !resolved_path.trim().is_empty() {
                    msg.push_str(&format!("resolved_path: {}\n", resolved_path.trim()));
                }
                if truncated {
                    msg.push_str("truncated: true\n");
                }
                if denied {
                    msg.push_str("\nstatus: denied\n");
                    msg.push_str(&format!("details: {}\n", json_one_line(&output, 2000)));
                    out.push(serde_json::json!({
                        "type": "message",
                        "role": "system",
                        "content": [{ "type": "input_text", "text": msg }],
                    }));
                    continue;
                }

                msg.push_str("\n```text\n");
                msg.push_str(text.trim_end());
                msg.push_str("\n```\n");

                out.push(serde_json::json!({
                    "type": "message",
                    "role": "system",
                    "content": [{ "type": "input_text", "text": msg }],
                }));
            }
            omne_agent_protocol::ContextRef::Diff(diff) => {
                let max_bytes = diff
                    .max_bytes
                    .unwrap_or(DEFAULT_CONTEXT_DIFF_MAX_BYTES)
                    .min(MAX_CONTEXT_DIFF_MAX_BYTES);

                let args = serde_json::json!({
                    "max_bytes": max_bytes,
                });
                let (output, hook_messages) =
                    match run_tool_call(
                        server,
                        thread_id,
                        Some(turn_id),
                        "thread_diff",
                        args,
                        cancel.clone(),
                        true,
                    )
                    .await
                    {
                        Ok(outcome) => (outcome.output, outcome.hook_messages),
                        Err(err) => (serde_json::json!({ "error": err.to_string() }), Vec::new()),
                    };

                out.extend(hook_messages);

                let denied = output
                    .get("denied")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let artifact = output.get("artifact").cloned().unwrap_or(Value::Null);
                let artifact_id = artifact.get("artifact_id").and_then(Value::as_str).unwrap_or("");
                let summary = artifact.get("summary").and_then(Value::as_str).unwrap_or("");

                let mut msg = String::new();
                msg.push_str("# Context (@diff)\n\n");
                if !artifact_id.trim().is_empty() {
                    msg.push_str(&format!("artifact_id: {artifact_id}\n"));
                }
                if !summary.trim().is_empty() {
                    msg.push_str(&format!("summary: {summary}\n"));
                }
                if denied {
                    msg.push_str("\nstatus: denied\n");
                    msg.push_str(&format!("details: {}\n", json_one_line(&output, 2000)));
                } else {
                    msg.push_str("\nNote: diff content is stored as an artifact. Use `artifact_read` if you need the full text.\n");
                }

                out.push(serde_json::json!({
                    "type": "message",
                    "role": "system",
                    "content": [{ "type": "input_text", "text": msg }],
                }));
            }
        }
    }

    Ok(out)
}

fn insert_context_before_last_user_message(
    input_items: &mut Vec<OpenAiItem>,
    ctx_items: Vec<OpenAiItem>,
) {
    if ctx_items.is_empty() {
        return;
    }

    let insert_at = input_items
        .iter()
        .rposition(|item| {
            item.get("type").and_then(Value::as_str) == Some("message")
                && item.get("role").and_then(Value::as_str) == Some("user")
        })
        .unwrap_or(input_items.len());

    input_items.splice(insert_at..insert_at, ctx_items);
}

fn format_event_for_context(kind: &ThreadEventKind) -> Option<String> {
    match kind {
        ThreadEventKind::ThreadArchived { reason } => Some(format!(
            "[thread/archived] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ThreadUnarchived { reason } => Some(format!(
            "[thread/unarchived] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ThreadPaused { reason } => Some(format!(
            "[thread/paused] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ThreadUnpaused { reason } => Some(format!(
            "[thread/unpaused] reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::TurnInterruptRequested { turn_id, reason } => Some(format!(
            "[turn/interrupt_requested] turn_id={turn_id} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } if !matches!(status, omne_agent_protocol::TurnStatus::Completed) || reason.is_some() => {
            Some(format!(
                "[turn/completed] turn_id={turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            ))
        }
        ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            openai_provider,
            model,
            thinking,
            openai_base_url,
            allowed_tools,
        } => Some(format!(
            "[thread/config] approval_policy={approval_policy:?} sandbox_policy={} sandbox_writable_roots={} sandbox_network_access={} mode={} openai_provider={} model={} thinking={} openai_base_url={} allowed_tools={}",
            sandbox_policy
                .as_ref()
                .map(|v| format!("{v:?}"))
                .unwrap_or_else(|| "<unchanged>".to_string()),
            sandbox_writable_roots
                .as_ref()
                .map(|roots| json_one_line(&serde_json::json!(roots), 2000))
                .unwrap_or_else(|| "<unchanged>".to_string()),
            sandbox_network_access
                .as_ref()
                .map(|access| format!("{access:?}"))
                .unwrap_or_else(|| "<unchanged>".to_string()),
            mode.as_deref().unwrap_or("<unchanged>"),
            openai_provider.as_deref().unwrap_or("<unchanged>"),
            model.as_deref().unwrap_or("<unchanged>"),
            thinking.as_deref().unwrap_or("<unchanged>"),
            openai_base_url.as_deref().unwrap_or("<unchanged>"),
            match allowed_tools {
                None => "<unchanged>".to_string(),
                Some(None) => "null".to_string(),
                Some(Some(tools)) => json_one_line(&serde_json::json!(tools), 2000),
            },
        )),
        ThreadEventKind::ApprovalRequested {
            approval_id,
            turn_id,
            action,
            params,
        } => Some(format!(
            "[approval/request] approval_id={approval_id} turn_id={} action={action} params={}",
            turn_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            json_one_line(params, 4000),
        )),
        ThreadEventKind::ApprovalDecided {
            approval_id,
            decision,
            remember,
            reason,
        } => Some(format!(
            "[approval/decide] approval_id={approval_id} decision={decision:?} remember={remember} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool,
            params,
        } => Some(format!(
            "[tool/start] tool_id={tool_id} turn_id={} tool={tool} params={}",
            turn_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            params
                .as_ref()
                .map(|v| json_one_line(v, 4000))
                .unwrap_or_else(|| "{}".to_string()),
        )),
        ThreadEventKind::ToolCompleted {
            tool_id,
            status,
            error,
            result,
        } => Some(format!(
            "[tool/done] tool_id={tool_id} status={status:?} error={} result={}",
            error.as_deref().unwrap_or(""),
            result
                .as_ref()
                .map(|v| json_one_line(v, 4000))
                .unwrap_or_else(|| "{}".to_string()),
        )),
        ThreadEventKind::ProcessStarted {
            process_id,
            turn_id,
            argv,
            cwd,
            stdout_path,
            stderr_path,
        } => Some(format!(
            "[process/start] process_id={process_id} turn_id={} argv={} cwd={cwd} stdout={stdout_path} stderr={stderr_path}",
            turn_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "-".to_string()),
            json_one_line(&serde_json::json!(argv), 2000),
        )),
        ThreadEventKind::ProcessInterruptRequested { process_id, reason } => Some(format!(
            "[process/interrupt_requested] process_id={process_id} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ProcessKillRequested { process_id, reason } => Some(format!(
            "[process/kill_requested] process_id={process_id} reason={}",
            reason.as_deref().unwrap_or("")
        )),
        ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => Some(format!(
            "[process/exited] process_id={process_id} exit_code={} reason={}",
            exit_code
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".to_string()),
            reason.as_deref().unwrap_or("")
        )),
        _ => None,
    }
}

fn json_one_line(value: &Value, max_chars: usize) -> String {
    match serde_json::to_string(value) {
        Ok(s) => truncate_chars(&s, max_chars),
        Err(_) => "<invalid-json>".to_string(),
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }

    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn extract_assistant_text(items: &[OpenAiItem]) -> String {
    let mut out = String::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        if item.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(content) = item.get("content").and_then(Value::as_array) {
            for part in content {
                if part.get("type").and_then(Value::as_str) != Some("output_text") {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
        }
    }
    out
}
