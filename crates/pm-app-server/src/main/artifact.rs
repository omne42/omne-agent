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

    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let tool_id = pm_protocol::ToolId::new();
    let bytes_len = params.text.len();
    let artifact_type = params.artifact_type.clone();
    let summary = params.summary.clone();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/write".to_string(),
            params: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
                "artifact_type": artifact_type,
                "summary": summary,
                "bytes": bytes_len,
            })),
        })
        .await?;

    let artifact_id = params.artifact_id.unwrap_or_default();
    let (content_path, metadata_path) = user_artifact_paths(server, params.thread_id, artifact_id);

    let now = OffsetDateTime::now_utc();
    let (created_at, version, created) = match tokio::fs::metadata(&metadata_path).await {
        Ok(_) => {
            let meta = read_artifact_metadata(&metadata_path).await?;
            (meta.created_at, meta.version.saturating_add(1), false)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (now, 1, true),
        Err(err) => return Err(err).with_context(|| format!("stat {}", metadata_path.display())),
    };

    let text = pm_core::redact_text(&params.text);
    let bytes = text.as_bytes().to_vec();
    write_file_atomic(&content_path, &bytes).await?;

    let meta = ArtifactMetadata {
        artifact_id,
        artifact_type: params.artifact_type,
        summary: params.summary,
        created_at,
        updated_at: now,
        version,
        content_path: content_path.display().to_string(),
        size_bytes: bytes.len() as u64,
        provenance: Some(ArtifactProvenance {
            thread_id: params.thread_id,
            turn_id: params.turn_id,
            tool_id: Some(tool_id),
            process_id: None,
        }),
    };

    let meta_bytes = serde_json::to_vec_pretty(&meta).context("serialize artifact metadata")?;
    write_file_atomic(&metadata_path, &meta_bytes).await?;

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
            tool_id,
            status: pm_protocol::ToolStatus::Completed,
            error: None,
            result: Some(serde_json::json!({
                "artifact_id": artifact_id,
                "created": created,
                "content_path": content_path.display().to_string(),
                "metadata_path": metadata_path.display().to_string(),
                "version": version,
                "size_bytes": bytes.len(),
            })),
        })
        .await?;

    Ok(serde_json::json!({
        "tool_id": tool_id,
        "artifact_id": artifact_id,
        "created": created,
        "content_path": content_path.display().to_string(),
        "metadata_path": metadata_path.display().to_string(),
        "metadata": meta,
    }))
}

async fn handle_artifact_list(
    server: &Server,
    params: ArtifactListParams,
) -> anyhow::Result<Value> {
    let dir = user_artifacts_dir_for_thread(server, params.thread_id);
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::json!({
                "artifacts": [],
                "errors": [],
            }));
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };

    let mut artifacts = Vec::<ArtifactMetadata>::new();
    let mut errors = Vec::<Value>::new();

    while let Some(entry) = read_dir.next_entry().await? {
        let ty = entry.file_type().await?;
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

    Ok(serde_json::json!({
        "artifacts": artifacts,
        "errors": errors,
    }))
}

async fn handle_artifact_read(
    server: &Server,
    params: ArtifactReadParams,
) -> anyhow::Result<Value> {
    let max_bytes = params.max_bytes.unwrap_or(256 * 1024).min(4 * 1024 * 1024);
    let (content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let meta = read_artifact_metadata(&metadata_path).await?;

    let bytes = tokio::fs::read(&content_path)
        .await
        .with_context(|| format!("read {}", content_path.display()))?;
    let truncated = bytes.len() > max_bytes as usize;
    let bytes = if truncated {
        bytes[..(max_bytes as usize)].to_vec()
    } else {
        bytes
    };
    let text = String::from_utf8_lossy(&bytes).to_string();
    let text = pm_core::redact_text(&text);

    Ok(serde_json::json!({
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
    let thread_rt = server.get_or_load_thread(params.thread_id).await?;
    let tool_id = pm_protocol::ToolId::new();

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "artifact/delete".to_string(),
            params: Some(serde_json::json!({
                "artifact_id": params.artifact_id,
            })),
        })
        .await?;

    let (content_path, metadata_path) =
        user_artifact_paths(server, params.thread_id, params.artifact_id);

    let mut removed = false;
    for path in [&content_path, &metadata_path] {
        match tokio::fs::remove_file(path).await {
            Ok(()) => removed = true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("remove {}", path.display())),
        }
    }

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

