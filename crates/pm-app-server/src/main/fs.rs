async fn handle_fs_mkdir(server: &Server, params: FsMkdirParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let (approval_policy, sandbox_policy, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.mode.clone(),
        )
    };
    let tool_id = pm_protocol::ToolId::new();

    let approval_params = serde_json::json!({
        "path": params.path.clone(),
        "recursive": params.recursive,
    });
    if sandbox_policy == pm_protocol::SandboxPolicy::ReadOnly {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "fs/mkdir".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("sandbox_policy=read_only forbids fs/mkdir".to_string()),
                result: Some(serde_json::json!({
                    "sandbox_policy": sandbox_policy,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "sandbox_policy": sandbox_policy,
        }));
    }

    let rel_path = pm_core::modes::relative_path_under_root(&thread_root, Path::new(&params.path));
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
                    tool: "fs/mkdir".to_string(),
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

    let base_decision = match rel_path.as_ref() {
        Ok(rel) => mode.permissions.edit.decision_for_path(rel),
        Err(_) => pm_core::modes::Decision::Deny,
    };
    let effective_decision = match mode.tool_overrides.get("fs/mkdir").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "fs/mkdir".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies fs/mkdir".to_string()),
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

    if approval_policy == pm_protocol::ApprovalPolicy::Manual {
        match params.approval_id {
            Some(approval_id) => {
                ensure_approval(
                    server,
                    params.thread_id,
                    approval_id,
                    "fs/mkdir",
                    &approval_params,
                )
                .await?;
            }
            None => {
                let remembered = remembered_approval_decision(
                    server,
                    params.thread_id,
                    "fs/mkdir",
                    &approval_params,
                )
                .await?;
                match remembered {
                    Some(pm_protocol::ApprovalDecision::Approved) => {}
                    Some(pm_protocol::ApprovalDecision::Denied) => {
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                                tool_id,
                                turn_id: params.turn_id,
                                tool: "fs/mkdir".to_string(),
                                params: Some(approval_params),
                            })
                            .await?;
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                                tool_id,
                                status: pm_protocol::ToolStatus::Denied,
                                error: Some("approval denied (remembered)".to_string()),
                                result: Some(serde_json::json!({
                                    "approval_policy": approval_policy,
                                })),
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "tool_id": tool_id,
                            "denied": true,
                            "remembered": true,
                        }));
                    }
                    None => {
                        let approval_id = pm_protocol::ApprovalId::new();
                        thread_rt
                            .append_event(pm_protocol::ThreadEventKind::ApprovalRequested {
                                approval_id,
                                turn_id: params.turn_id,
                                action: "fs/mkdir".to_string(),
                                params: approval_params,
                            })
                            .await?;
                        return Ok(serde_json::json!({
                            "needs_approval": true,
                            "approval_id": approval_id,
                        }));
                    }
                }
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "fs/mkdir".to_string(),
            params: Some(approval_params),
        })
        .await?;

    let thread_root = thread_root.clone();
    let outcome: anyhow::Result<(bool, PathBuf)> = async {
        let path = resolve_file_for_sandbox(
            &thread_root,
            sandbox_policy,
            Path::new(&params.path),
            pm_core::PathAccess::Write,
            params.recursive,
        )
        .await?;

        if path == thread_root {
            anyhow::bail!("refusing to create thread root: {}", path.display());
        }

        match tokio::fs::create_dir(&path).await {
            Ok(()) => Ok((true, path)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let meta = tokio::fs::metadata(&path)
                    .await
                    .with_context(|| format!("stat {}", path.display()))?;
                if meta.is_dir() {
                    Ok((false, path))
                } else {
                    anyhow::bail!("path exists and is not a directory: {}", path.display());
                }
            }
            Err(err) => Err(err).with_context(|| format!("create dir {}", path.display())),
        }
    }
    .await;

    match outcome {
        Ok((created, path)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "created": created,
                        "path": path.display().to_string(),
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "created": created,
                "resolved_path": path.display().to_string(),
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

fn user_artifacts_dir_for_thread(server: &Server, thread_id: ThreadId) -> PathBuf {
    server
        .thread_store
        .thread_dir(thread_id)
        .join("artifacts")
        .join("user")
}

fn user_artifact_paths(
    server: &Server,
    thread_id: ThreadId,
    artifact_id: ArtifactId,
) -> (PathBuf, PathBuf) {
    let dir = user_artifacts_dir_for_thread(server, thread_id);
    (
        dir.join(format!("{artifact_id}.md")),
        dir.join(format!("{artifact_id}.metadata.json")),
    )
}

async fn read_artifact_metadata(path: &Path) -> anyhow::Result<ArtifactMetadata> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let meta = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse artifact metadata {}", path.display()))?;
    Ok(meta)
}

async fn write_file_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create dir {}", parent.display()))?;

    let pid = std::process::id();
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let tmp_path = path.with_extension(format!("tmp.{pid}.{nanos}"));

    tokio::fs::write(&tmp_path, bytes)
        .await
        .with_context(|| format!("write {}", tmp_path.display()))?;

    if let Err(err) = tokio::fs::rename(&tmp_path, path).await {
        if matches!(
            err.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
        ) {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(remove_err) if remove_err.kind() == std::io::ErrorKind::NotFound => {}
                Err(remove_err) => {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                    return Err(remove_err)
                        .with_context(|| format!("remove old {}", path.display()));
                }
            }
            if let Err(rename_err) = tokio::fs::rename(&tmp_path, path).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(rename_err).with_context(|| {
                    format!("rename {} -> {}", tmp_path.display(), path.display())
                });
            }
        } else {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(err)
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
        }
    }

    Ok(())
}

