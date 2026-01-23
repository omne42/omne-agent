async fn handle_file_read(server: &Server, params: FileReadParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;
    let (approval_policy, sandbox_policy, sandbox_writable_roots, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.sandbox_policy,
            state.sandbox_writable_roots.clone(),
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_bytes = params.max_bytes.unwrap_or(256 * 1024).min(4 * 1024 * 1024);
    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "path": params.path.clone(),
        "max_bytes": max_bytes,
    });
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/read",
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(result);
    }

    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id: params.turn_id,
                        tool: "file/read".to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: Some(serde_json::json!({
                            "root": file_root.as_str(),
                            "reason": "reference repo is not configured",
                        })),
                    })
                    .await?;
                return Err(err);
            }
        },
    };

    let rel_path = pm_core::modes::relative_path_under_root(&root, Path::new(&params.path));
    if let Ok(rel) = rel_path.as_ref()
        && rel_path_is_secret(rel)
    {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/read".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("refusing to read secrets file (.env)".to_string()),
                result: Some(serde_json::json!({
                    "reason": "secrets file is always denied",
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
        }));
    }
    let catalog = pm_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = match catalog.mode(&mode_name) {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");

            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: params.turn_id,
                    tool: "file/read".to_string(),
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
                        "decision": pm_core::modes::Decision::Deny,
                        "available": available,
                        "load_error": catalog.load_error.clone(),
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": pm_core::modes::Decision::Deny,
                "available": available,
                "load_error": catalog.load_error.clone(),
            }));
        }
    };

    let base_decision = match rel_path.as_ref() {
        Ok(rel) if mode.permissions.edit.is_denied(rel) => pm_core::modes::Decision::Deny,
        Ok(_) => mode.permissions.read,
        Err(_) => pm_core::modes::Decision::Deny,
    };
    let effective_decision = match mode.tool_overrides.get("file/read").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/read".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/read".to_string()),
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
                action: "file/read",
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
                        tool: "file/read".to_string(),
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
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/read".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let outcome: anyhow::Result<(PathBuf, String, bool, usize)> = async {
        let path = match file_root {
            FileRoot::Workspace => {
                resolve_file_for_sandbox(
                    &thread_root,
                    sandbox_policy,
                    &sandbox_writable_roots,
                    Path::new(&params.path),
                    pm_core::PathAccess::Read,
                    false,
                )
                .await?
            }
            FileRoot::Reference => {
                pm_core::resolve_file(&root, Path::new(&params.path), pm_core::PathAccess::Read, false)
                    .await?
            }
        };

        let limit = max_bytes + 1;
        let file = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("open {}", path.display()))?;
        let mut buf = Vec::new();
        file.take(limit).read_to_end(&mut buf).await?;

        let truncated = buf.len() > max_bytes as usize;
        if truncated {
            buf.truncate(max_bytes as usize);
        }
        let bytes = buf.len();
        let text = String::from_utf8(buf).context("file is not valid utf-8")?;
        Ok((path, text, truncated, bytes))
    }
    .await;

    match outcome {
        Ok((path, text, truncated, bytes)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "bytes": bytes,
                        "truncated": truncated,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "resolved_path": path.display().to_string(),
                "root": file_root.as_str(),
                "text": text,
                "truncated": truncated,
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

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".code_pm",
    ".codepm",
    "target",
    "node_modules",
    "example",
];

fn rel_path_is_secret(rel_path: &Path) -> bool {
    rel_path.file_name() == Some(std::ffi::OsStr::new(".env"))
}

fn should_walk_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name();
    if DEFAULT_IGNORED_DIRS
        .iter()
        .any(|dir| name == std::ffi::OsStr::new(*dir))
    {
        return false;
    }

    if (name == std::ffi::OsStr::new("tmp")
        || name == std::ffi::OsStr::new("threads")
        || name == std::ffi::OsStr::new("locks")
        || name == std::ffi::OsStr::new("logs")
        || name == std::ffi::OsStr::new("data")
        || name == std::ffi::OsStr::new("repos")
        || name == std::ffi::OsStr::new("reference"))
        && entry
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|parent| {
                parent == std::ffi::OsStr::new(".codepm_data")
                    || parent == std::ffi::OsStr::new("codepm_data")
            })
    {
        return false;
    }

    true
}

async fn resolve_reference_repo_root(thread_root: &Path) -> anyhow::Result<PathBuf> {
    let rel = Path::new(".codepm_data/reference/repo");
    pm_core::resolve_dir(thread_root, rel)
        .await
        .with_context(|| format!("resolve reference repo root {}", thread_root.join(rel).display()))
}

async fn handle_file_glob(server: &Server, params: FileGlobParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_results = params.max_results.unwrap_or(2000).min(20_000);
    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "pattern": params.pattern.clone(),
        "max_results": max_results,
    });

    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/glob",
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
                    tool: "file/glob".to_string(),
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
    let base_decision = mode.permissions.read;
    let effective_decision = match mode.tool_overrides.get("file/glob").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/glob".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/glob".to_string()),
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
                action: "file/glob",
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
                        tool: "file/glob".to_string(),
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
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/glob".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let pattern = params.pattern.clone();
    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: Some(serde_json::json!({
                            "root": file_root.as_str(),
                            "reason": "reference repo is not configured",
                        })),
                    })
                    .await?;
                return Err(err);
            }
        },
    };
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<(Vec<String>, bool)> {
        let matcher = Glob::new(&pattern)
            .with_context(|| format!("invalid glob pattern: {pattern}"))?
            .compile_matcher();

        let mut paths = Vec::new();
        let mut truncated = false;

        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_walk_entry)
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
            if rel_path_is_secret(rel) {
                continue;
            }
            if matcher.is_match(rel) {
                paths.push(rel.to_string_lossy().to_string());
                if paths.len() >= max_results {
                    truncated = true;
                    break;
                }
            }
        }

        Ok((paths, truncated))
    })
    .await
    .context("join glob task")?;

    match outcome {
        Ok((paths, truncated)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "matches": paths.len(),
                        "truncated": truncated,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "root": file_root.as_str(),
                "paths": paths,
                "truncated": truncated,
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

#[derive(Debug, Serialize)]
struct GrepMatch {
    path: String,
    line_number: u64,
    line: String,
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in line.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

async fn handle_file_grep(server: &Server, params: FileGrepParams) -> anyhow::Result<Value> {
    let (thread_rt, thread_root) = load_thread_root(server, params.thread_id).await?;

    let file_root = params.root.unwrap_or(FileRoot::Workspace);
    let max_matches = params.max_matches.unwrap_or(200).min(2000);
    let max_bytes_per_file = params
        .max_bytes_per_file
        .unwrap_or(1024 * 1024)
        .min(16 * 1024 * 1024);
    let max_files = params.max_files.unwrap_or(20_000).min(200_000);
    let tool_id = pm_protocol::ToolId::new();
    let approval_params = serde_json::json!({
        "root": file_root.as_str(),
        "query": params.query.clone(),
        "is_regex": params.is_regex,
        "include_glob": params.include_glob.clone(),
        "max_matches": max_matches,
        "max_bytes_per_file": max_bytes_per_file,
        "max_files": max_files,
    });

    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };
    if let Some(result) = enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        params.turn_id,
        "file/grep",
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
                    tool: "file/grep".to_string(),
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
    let base_decision = mode.permissions.read;
    let effective_decision = match mode.tool_overrides.get("file/grep").copied() {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };
    if effective_decision == pm_core::modes::Decision::Deny {
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: params.turn_id,
                tool: "file/grep".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: pm_protocol::ToolStatus::Denied,
                error: Some("mode denies file/grep".to_string()),
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
                action: "file/grep",
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
                        tool: "file/grep".to_string(),
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
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(pm_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id: params.turn_id,
            tool: "file/grep".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let pattern = if params.is_regex {
        params.query.clone()
    } else {
        regex::escape(&params.query)
    };
    let re = Regex::new(&pattern).with_context(|| format!("invalid regex: {}", params.query))?;
    let include_matcher = match params.include_glob.as_deref() {
        Some(glob) => Some(
            Glob::new(glob)
                .with_context(|| format!("invalid glob pattern: {glob}"))?
                .compile_matcher(),
        ),
        None => None,
    };

    let root = match file_root {
        FileRoot::Workspace => thread_root.clone(),
        FileRoot::Reference => match resolve_reference_repo_root(&thread_root).await {
            Ok(root) => root,
            Err(err) => {
                thread_rt
                    .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: pm_protocol::ToolStatus::Failed,
                        error: Some(err.to_string()),
                        result: Some(serde_json::json!({
                            "root": file_root.as_str(),
                            "reason": "reference repo is not configured",
                        })),
                    })
                    .await?;
                return Err(err);
            }
        },
    };
    let outcome = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(Vec<GrepMatch>, bool, usize, usize, usize)> {
            let mut matches = Vec::new();
            let mut truncated = false;
            let mut files_scanned = 0usize;
            let mut files_skipped_too_large = 0usize;
            let mut files_skipped_binary = 0usize;

            for entry in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(should_walk_entry)
            {
                let entry = entry?;
                if !entry.file_type().is_file() {
                    continue;
                }
                if files_scanned >= max_files {
                    break;
                }
                let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
                if rel_path_is_secret(rel) {
                    continue;
                }
                if let Some(ref matcher) = include_matcher {
                    if !matcher.is_match(rel) {
                        continue;
                    }
                }

                files_scanned += 1;

                let meta = entry.metadata()?;
                if meta.len() > max_bytes_per_file {
                    files_skipped_too_large += 1;
                    continue;
                }

                let bytes = match std::fs::read(entry.path()) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };
                if bytes.contains(&0) {
                    files_skipped_binary += 1;
                    continue;
                }

                let text = String::from_utf8_lossy(&bytes);
                for (idx, line) in text.lines().enumerate() {
                    if !re.is_match(line) {
                        continue;
                    }

                    matches.push(GrepMatch {
                        path: rel.to_string_lossy().to_string(),
                        line_number: (idx + 1) as u64,
                        line: truncate_line(line, 4000),
                    });
                    if matches.len() >= max_matches {
                        truncated = true;
                        break;
                    }
                }

                if truncated {
                    break;
                }
            }

            Ok((
                matches,
                truncated,
                files_scanned,
                files_skipped_too_large,
                files_skipped_binary,
            ))
        },
    )
    .await
    .context("join grep task")?;

    match outcome {
        Ok((matches, truncated, files_scanned, files_skipped_too_large, files_skipped_binary)) => {
            thread_rt
                .append_event(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "matches": matches.len(),
                        "truncated": truncated,
                        "files_scanned": files_scanned,
                        "files_skipped_too_large": files_skipped_too_large,
                        "files_skipped_binary": files_skipped_binary,
                    })),
                })
                .await?;
            Ok(serde_json::json!({
                "tool_id": tool_id,
                "root": file_root.as_str(),
                "matches": matches,
                "truncated": truncated,
                "files_scanned": files_scanned,
                "files_skipped_too_large": files_skipped_too_large,
                "files_skipped_binary": files_skipped_binary,
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
