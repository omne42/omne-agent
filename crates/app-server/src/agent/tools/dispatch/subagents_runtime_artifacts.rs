fn spawn_mode_label(mode: AgentSpawnMode) -> &'static str {
    match mode {
        AgentSpawnMode::Fork => "fork",
        AgentSpawnMode::New => "new",
    }
}

fn workspace_mode_label(mode: AgentSpawnWorkspaceMode) -> &'static str {
    match mode {
        AgentSpawnWorkspaceMode::ReadOnly => "read_only",
        AgentSpawnWorkspaceMode::IsolatedWrite => "isolated_write",
    }
}

fn priority_label(priority: AgentSpawnTaskPriority) -> &'static str {
    match priority {
        AgentSpawnTaskPriority::High => "high",
        AgentSpawnTaskPriority::Normal => "normal",
        AgentSpawnTaskPriority::Low => "low",
    }
}

fn task_status_label(status: SubagentTaskStatus) -> &'static str {
    match status {
        SubagentTaskStatus::Pending => "pending",
        SubagentTaskStatus::Running => "running",
        SubagentTaskStatus::Completed => "completed",
        SubagentTaskStatus::Failed => "failed",
    }
}

fn render_fan_in_summary_markdown(
    payload: &omne_workflow_spec::FanInSummaryStructuredData,
) -> String {
    let structured_json = serde_json::to_string_pretty(payload)
        .or_else(|_| serde_json::to_string(payload))
        .unwrap_or_else(|_| "{}".to_string());
    format!("# Fan-in Summary\n\n## Structured Data\n\n```json\n{structured_json}\n```\n")
}

fn render_fan_out_result_error_markdown(
    task_id: &str,
    title: &str,
    child_thread_id: ThreadId,
    turn_id: Option<TurnId>,
    status: omne_protocol::TurnStatus,
    reason: Option<&str>,
    write_error: &str,
) -> String {
    let mut text = String::new();
    text.push_str("# Fan-out Result Artifact Error\n\n");
    text.push_str(&format!("- task_id: `{task_id}`\n"));
    if !title.trim().is_empty() {
        text.push_str(&format!("- title: {title}\n"));
    }
    text.push_str(&format!("- child_thread_id: `{child_thread_id}`\n"));
    text.push_str(&format!(
        "- turn_id: `{}`\n",
        turn_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<none>".to_string())
    ));
    text.push_str(&format!("- status: `{:?}`\n", status));
    if let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) {
        text.push_str(&format!("- reason: {}\n", reason));
    }
    text.push_str(&format!("- error: {}\n", write_error));
    text
}

fn dependency_blocker_details_from_error(error: Option<&str>) -> Option<(String, String)> {
    let error = error?.trim();
    let rest = error.strip_prefix("blocked by dependency: ")?;
    let (dependency_task_id, dependency_status) = rest.split_once(" status=")?;
    let dependency_task_id = dependency_task_id.trim();
    let dependency_status = dependency_status.trim();
    if dependency_task_id.is_empty() || dependency_status.is_empty() {
        return None;
    }
    Some((
        dependency_task_id.to_string(),
        dependency_status.to_string(),
    ))
}

fn parse_subagent_proxy_child_key(params: &Value) -> Option<SubagentApprovalKey> {
    let proxy = params.get("subagent_proxy")?.as_object()?;
    if proxy.get("kind").and_then(Value::as_str) != Some("approval") {
        return None;
    }
    let thread_id = proxy
        .get("child_thread_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    let approval_id = proxy
        .get("child_approval_id")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    Some(SubagentApprovalKey {
        thread_id,
        approval_id,
    })
}

fn decorate_subagent_proxy_forwarded_reason(reason: Option<&str>) -> String {
    let suffix = reason.unwrap_or_default().trim();
    if suffix.is_empty() {
        crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX.to_string()
    } else {
        format!("{} {suffix}", crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX)
    }
}

fn summarize_subagent_pending_approval(
    child_key: SubagentApprovalKey,
    child_turn_id: TurnId,
    child_action: &str,
) -> String {
    format!(
        "child_thread_id={} child_turn_id={} child_approval_id={} child_action={}",
        child_key.thread_id, child_turn_id, child_key.approval_id, child_action
    )
}

async fn start_subagent_turn(
    server: &super::Server,
    parent_thread_id: ThreadId,
    task: &SubagentSpawnTask,
) -> anyhow::Result<TurnId> {
    let rt = server.get_or_load_thread(task.thread_id).await?;
    let server_arc = Arc::new(server.clone());
    let turn_id = rt
        .start_turn(
            server_arc,
            task.input.clone(),
            None,
            None,
            None,
            omne_protocol::TurnPriority::Background,
        )
        .await?;

    let parent_workspace_cwd =
        if matches!(task.workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite) {
            match server.get_or_load_thread(parent_thread_id).await {
                Ok(parent_rt) => {
                    let handle = parent_rt.handle.lock().await;
                    handle.state().cwd.clone()
                }
                Err(_) => None,
            }
        } else {
            None
        };

    let notify_rx = server.notify_tx.subscribe();
    spawn_fan_out_result_writer_with_target_workspace(
        server.clone(),
        notify_rx,
        task.thread_id,
        turn_id,
        task.id.clone(),
        task.expected_artifact_type.clone(),
        task.workspace_mode,
        task.workspace_cwd.clone(),
        parent_workspace_cwd,
        isolated_auto_apply_patch_enabled_from_env(),
    );

    Ok(turn_id)
}

async fn create_new_thread(server: &super::Server, cwd: &str) -> anyhow::Result<SpawnedThread> {
    let handle = server
        .thread_store
        .create_thread(PathBuf::from(cwd))
        .await?;
    let thread_id = handle.thread_id();
    let log_path = handle.log_path().display().to_string();
    let last_seq = handle.last_seq().0;

    let rt = Arc::new(crate::ThreadRuntime::new(
        handle,
        server.notify_tx.clone(),
        server.thread_store.clone(),
    ));
    server.threads.lock().await.insert(thread_id, rt);

    Ok(SpawnedThread {
        thread_id,
        log_path,
        last_seq,
    })
}

const DEFAULT_ISOLATED_MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_ISOLATED_MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const ISOLATED_BACKEND_ENV: &str = "OMNE_SUBAGENT_ISOLATED_BACKEND";
const ISOLATED_WORKTREE_FIRST_ENV: &str = "OMNE_SUBAGENT_ISOLATED_WORKTREE_FIRST";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsolatedWorkspaceRequestedBackend {
    Auto,
    Worktree,
    Copy,
}

impl IsolatedWorkspaceRequestedBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Worktree => "worktree",
            Self::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IsolatedWorkspaceBackend {
    Worktree,
    Copy,
}

impl IsolatedWorkspaceBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Copy => "copy",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct IsolatedWorkspacePreparation {
    cwd: std::path::PathBuf,
    requested_backend: IsolatedWorkspaceRequestedBackend,
    backend: IsolatedWorkspaceBackend,
    fallback_reason: Option<String>,
}

fn parse_isolated_workspace_requested_backend(
    raw_backend: Option<&str>,
    raw_legacy_worktree_first: Option<&str>,
) -> (IsolatedWorkspaceRequestedBackend, Option<String>) {
    if let Some(raw_backend) = raw_backend {
        let normalized = raw_backend.trim().to_ascii_lowercase();
        return match normalized.as_str() {
            "auto" => (IsolatedWorkspaceRequestedBackend::Auto, None),
            "worktree" => (IsolatedWorkspaceRequestedBackend::Worktree, None),
            "copy" => (IsolatedWorkspaceRequestedBackend::Copy, None),
            _ => (
                IsolatedWorkspaceRequestedBackend::Auto,
                Some(format!(
                    "invalid {} value {:?}; fallback to auto",
                    ISOLATED_BACKEND_ENV, raw_backend
                )),
            ),
        };
    }

    let worktree_first = parse_subagent_env_bool(raw_legacy_worktree_first, true);
    if worktree_first {
        (IsolatedWorkspaceRequestedBackend::Auto, None)
    } else {
        (IsolatedWorkspaceRequestedBackend::Copy, None)
    }
}

fn sanitize_isolated_workspace_component(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    let out = if out.is_empty() { "task" } else { out };
    out.chars().take(80).collect::<String>()
}

fn is_isolated_runtime_rel_path(rel: &std::path::Path) -> bool {
    let mut components = rel.components();
    let Some(first) = components.next() else {
        return false;
    };
    let std::path::Component::Normal(first) = first else {
        return false;
    };
    if first != std::ffi::OsStr::new(".omne_data") && first != std::ffi::OsStr::new("omne_data") {
        return false;
    }
    let Some(std::path::Component::Normal(second)) = components.next() else {
        return false;
    };
    matches!(
        second.to_str().unwrap_or_default(),
        "tmp" | "threads" | "locks" | "logs" | "data" | "repos" | "reference"
    )
}

fn should_walk_isolated_workspace_entry(
    source_root: &std::path::Path,
    entry: &walkdir::DirEntry,
) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }
    let rel = entry
        .path()
        .strip_prefix(source_root)
        .unwrap_or(entry.path());
    !is_isolated_runtime_rel_path(rel)
}

#[cfg(unix)]
fn create_isolated_symlink(
    target: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
fn create_isolated_symlink(
    target: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    let metadata = std::fs::metadata(target);
    if metadata.as_ref().is_ok_and(|meta| meta.is_dir()) {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}

async fn prepare_isolated_workspace(
    server: &super::Server,
    parent_thread_id: ThreadId,
    task_id: &str,
    source_root: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    Ok(
        prepare_isolated_workspace_with_details(server, parent_thread_id, task_id, source_root)
            .await?
            .cwd,
    )
}

async fn prepare_isolated_workspace_with_details(
    server: &super::Server,
    parent_thread_id: ThreadId,
    task_id: &str,
    source_root: &std::path::Path,
) -> anyhow::Result<IsolatedWorkspacePreparation> {
    let max_file_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_MAX_FILE_BYTES",
        DEFAULT_ISOLATED_MAX_FILE_BYTES,
        1,
        8 * 1024 * 1024 * 1024,
    );
    let max_total_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_MAX_TOTAL_BYTES",
        DEFAULT_ISOLATED_MAX_TOTAL_BYTES,
        max_file_bytes,
        64 * 1024 * 1024 * 1024,
    );

    let source_root = source_root.to_path_buf();
    let label = sanitize_isolated_workspace_component(task_id);
    let nonce = omne_protocol::ToolId::new().to_string();
    let isolated_root = server
        .cwd
        .join(".omne_data")
        .join("tmp")
        .join("subagents")
        .join(parent_thread_id.to_string())
        .join(format!("{label}-{nonce}"))
        .join("repo");
    let (requested_backend, policy_warning) = parse_isolated_workspace_requested_backend(
        std::env::var(ISOLATED_BACKEND_ENV).ok().as_deref(),
        std::env::var(ISOLATED_WORKTREE_FIRST_ENV).ok().as_deref(),
    );
    if let Some(policy_warning) = policy_warning.as_deref() {
        tracing::warn!(
            task_id = %task_id,
            parent_thread_id = %parent_thread_id,
            warning = %policy_warning,
            "invalid isolated workspace backend policy"
        );
    }
    if matches!(
        requested_backend,
        IsolatedWorkspaceRequestedBackend::Auto | IsolatedWorkspaceRequestedBackend::Worktree
    ) {
        let source_root_text = source_root.display().to_string();
        let isolated_root_text = isolated_root.display().to_string();
        if let Some(parent) = isolated_root.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let worktree_result = omne_git_runtime::create_detached_worktree(
            &source_root_text,
            &isolated_root_text,
            Some("HEAD"),
        )
        .await;
        if worktree_result.is_ok() {
            return Ok(IsolatedWorkspacePreparation {
                cwd: isolated_root,
                requested_backend,
                backend: IsolatedWorkspaceBackend::Worktree,
                fallback_reason: None,
            });
        }
        let err = match worktree_result {
            Ok(()) => unreachable!(),
            Err(err) => err,
        };
        if matches!(
            requested_backend,
            IsolatedWorkspaceRequestedBackend::Worktree
        ) {
            return Err(err).context("isolated worktree backend is required");
        }
        let _ = tokio::fs::remove_dir_all(&isolated_root).await;
        let fallback_reason = format!("worktree preparation failed: {err}");
        copy_workspace_into_isolated_root(
            &source_root,
            &isolated_root,
            max_file_bytes,
            max_total_bytes,
        )
        .await?;
        return Ok(IsolatedWorkspacePreparation {
            cwd: isolated_root,
            requested_backend,
            backend: IsolatedWorkspaceBackend::Copy,
            fallback_reason: Some(fallback_reason),
        });
    }

    copy_workspace_into_isolated_root(
        &source_root,
        &isolated_root,
        max_file_bytes,
        max_total_bytes,
    )
    .await?;
    Ok(IsolatedWorkspacePreparation {
        cwd: isolated_root,
        requested_backend,
        backend: IsolatedWorkspaceBackend::Copy,
        fallback_reason: None,
    })
}

async fn copy_workspace_into_isolated_root(
    source_root: &std::path::Path,
    isolated_root: &std::path::Path,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> anyhow::Result<()> {
    let source_root = source_root.to_path_buf();
    let isolated_root_for_task = isolated_root.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        std::fs::create_dir_all(&isolated_root_for_task).with_context(|| {
            format!(
                "create isolated workspace {}",
                isolated_root_for_task.display()
            )
        })?;

        let mut total_bytes = 0u64;
        for entry in walkdir::WalkDir::new(&source_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| should_walk_isolated_workspace_entry(&source_root, entry))
        {
            let entry = entry?;
            let rel = entry
                .path()
                .strip_prefix(&source_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() || is_isolated_runtime_rel_path(rel) {
                continue;
            }
            let destination = isolated_root_for_task.join(rel);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&destination)
                    .with_context(|| format!("create {}", destination.display()))?;
                continue;
            }
            if entry.file_type().is_symlink() {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                let target = std::fs::read_link(entry.path())
                    .with_context(|| format!("read symlink {}", entry.path().display()))?;
                create_isolated_symlink(&target, &destination).with_context(|| {
                    format!("symlink {} -> {}", destination.display(), target.display())
                })?;
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > max_file_bytes {
                anyhow::bail!(
                    "isolated workspace copy skipped oversized file: {} ({} bytes > {} bytes)",
                    rel.display(),
                    meta.len(),
                    max_file_bytes
                );
            }
            total_bytes = total_bytes.saturating_add(meta.len());
            if total_bytes > max_total_bytes {
                anyhow::bail!(
                    "isolated workspace copy exceeds max_total_bytes={} (current={})",
                    max_total_bytes,
                    total_bytes
                );
            }
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(entry.path(), &destination).with_context(|| {
                format!(
                    "copy {} -> {}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        }

        Ok(())
    })
    .await
    .context("join isolated workspace copy task")??;
    Ok(())
}

#[allow(dead_code)]
fn spawn_fan_out_result_writer(
    server: super::Server,
    notify_rx: tokio::sync::broadcast::Receiver<String>,
    thread_id: omne_protocol::ThreadId,
    turn_id: TurnId,
    task_id: String,
    expected_artifact_type: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    workspace_cwd: Option<String>,
) {
    spawn_fan_out_result_writer_with_target_workspace(
        server,
        notify_rx,
        thread_id,
        turn_id,
        task_id,
        expected_artifact_type,
        workspace_mode,
        workspace_cwd,
        None,
        false,
    );
}

fn spawn_fan_out_result_writer_with_target_workspace(
    server: super::Server,
    mut notify_rx: tokio::sync::broadcast::Receiver<String>,
    thread_id: omne_protocol::ThreadId,
    turn_id: TurnId,
    task_id: String,
    expected_artifact_type: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    workspace_cwd: Option<String>,
    target_workspace_cwd: Option<String>,
    isolated_write_auto_apply_enabled: bool,
) {
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(line) => {
                    let Ok(val) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if val.get("method").and_then(Value::as_str) != Some("turn/completed") {
                        continue;
                    }
                    let Some(params) = val.get("params") else {
                        continue;
                    };
                    let Ok(event) =
                        serde_json::from_value::<omne_protocol::ThreadEvent>(params.clone())
                    else {
                        continue;
                    };
                    if event.thread_id != thread_id {
                        continue;
                    }
                    let omne_protocol::ThreadEventKind::TurnCompleted {
                        turn_id: completed_turn_id,
                        status,
                        reason,
                    } = event.kind
                    else {
                        continue;
                    };
                    if completed_turn_id != turn_id {
                        continue;
                    }

                    let isolated_write_patch = if expected_artifact_type == "fan_out_result"
                        && matches!(workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite)
                    {
                        if let Some(cwd) = workspace_cwd.as_deref() {
                            try_write_isolated_workspace_patch_artifact(
                                &server,
                                thread_id,
                                turn_id,
                                task_id.as_str(),
                                cwd,
                            )
                            .await
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let isolated_write_handoff = if matches!(
                        workspace_mode,
                        AgentSpawnWorkspaceMode::IsolatedWrite
                    ) {
                        let (requested_backend, policy_warning) =
                            parse_isolated_workspace_requested_backend(
                                std::env::var(ISOLATED_BACKEND_ENV).ok().as_deref(),
                                std::env::var(ISOLATED_WORKTREE_FIRST_ENV).ok().as_deref(),
                            );
                        let requested_backend = requested_backend.as_str().to_string();
                        workspace_cwd.as_ref().map(|cwd| {
                                if let Some(policy_warning) = policy_warning.as_deref() {
                                    tracing::warn!(
                                        thread_id = %thread_id,
                                        turn_id = %turn_id,
                                        task_id = %task_id,
                                        warning = %policy_warning,
                                        "invalid isolated workspace backend policy for fan-out handoff"
                                    );
                                }
                                let backend = detect_isolated_workspace_backend(cwd)
                                    .map(|value| value.as_str().to_string());
                                let fallback_reason = if matches!(
                                    requested_backend.as_str(),
                                    "auto" | "worktree"
                                ) && backend.as_deref() == Some("copy")
                                {
                                    Some("worktree backend unavailable; copy backend used".to_string())
                                } else {
                                    None
                                };
                                let mut handoff = serde_json::json!({
                                    "workspace_cwd": cwd,
                                    "status_argv": ["git", "-C", cwd, "status", "--short", "--"],
                                    "diff_argv": ["git", "-C", cwd, "diff", "--binary", "--"],
                                    "apply_patch_hint": "capture diff output and apply in target workspace with git apply",
                                    "requested_backend": requested_backend,
                                    "backend": backend,
                                    "fallback_reason": fallback_reason,
                                });
                                if let Some(patch) = isolated_write_patch.as_ref() {
                                    handoff["patch"] = patch.clone();
                                }
                                handoff
                            })
                    } else {
                        None
                    };
                    let mut isolated_write_auto_apply = if expected_artifact_type
                        == "fan_out_result"
                        && matches!(workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite)
                    {
                        if let Some(cwd) = workspace_cwd.as_deref() {
                            try_auto_apply_isolated_workspace_patch(
                                cwd,
                                target_workspace_cwd.as_deref(),
                                &status,
                                isolated_write_auto_apply_enabled,
                            )
                            .await
                        } else if isolated_write_auto_apply_enabled {
                            let failure_stage =
                                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Precondition;
                            Some(serde_json::json!({
                                "enabled": true,
                                "attempted": false,
                                "applied": false,
                                "failure_stage": failure_stage.as_str(),
                                "recovery_hint": "ensure isolated workspace cwd is available before enabling auto-apply",
                                "error": "isolated workspace cwd is missing",
                            }))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let (Some(auto_apply), Some(patch)) = (
                        isolated_write_auto_apply.as_mut(),
                        isolated_write_patch.as_ref(),
                    ) {
                        if let Some(auto_apply_obj) = auto_apply.as_object_mut() {
                            if let Some(patch_artifact_id) = patch
                                .get("artifact_id")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                auto_apply_obj.insert(
                                    "patch_artifact_id".to_string(),
                                    serde_json::json!(patch_artifact_id),
                                );
                            }
                            if let Some(patch_read_cmd) = patch
                                .get("read_cmd")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                            {
                                auto_apply_obj.insert(
                                    "patch_read_cmd".to_string(),
                                    serde_json::json!(patch_read_cmd),
                                );
                            }
                        }
                    }
                    if let Some(auto_apply) = isolated_write_auto_apply.as_mut() {
                        if let Some(auto_apply_obj) = auto_apply.as_object_mut() {
                            let has_error = auto_apply_obj
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .is_some_and(|value| !value.is_empty());
                            if has_error {
                                let mut recovery_commands = Vec::<Value>::new();

                                if let Some(patch_artifact_id) = auto_apply_obj
                                    .get("patch_artifact_id")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "read_patch_artifact",
                                        "argv": ["omne", "artifact", "read", thread_id.to_string(), patch_artifact_id],
                                    }));
                                } else if let Some(patch_read_cmd) = auto_apply_obj
                                    .get("patch_read_cmd")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "read_patch_artifact",
                                        "argv": patch_read_cmd.split_whitespace().collect::<Vec<_>>(),
                                    }));
                                }

                                if let Some(target_workspace_cwd) = auto_apply_obj
                                    .get("target_workspace_cwd")
                                    .and_then(Value::as_str)
                                    .map(str::trim)
                                    .filter(|value| !value.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "show_target_status",
                                        "argv": ["git", "-C", target_workspace_cwd, "status", "--short", "--"],
                                    }));
                                }

                                if let Some(check_argv) = auto_apply_obj
                                    .get("check_argv")
                                    .and_then(json_value_string_array)
                                    .filter(|argv| !argv.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "check_apply_with_patch_stdin",
                                        "argv": check_argv,
                                    }));
                                }

                                if let Some(apply_argv) = auto_apply_obj
                                    .get("apply_argv")
                                    .and_then(json_value_string_array)
                                    .filter(|argv| !argv.is_empty())
                                {
                                    recovery_commands.push(serde_json::json!({
                                        "label": "apply_with_patch_stdin",
                                        "argv": apply_argv,
                                    }));
                                }

                                if !recovery_commands.is_empty() {
                                    auto_apply_obj.insert(
                                        "recovery_commands".to_string(),
                                        Value::Array(recovery_commands),
                                    );
                                }
                            }
                        }
                    }
                    let mut payload = serde_json::json!({
                        "task_id": task_id,
                        "thread_id": thread_id,
                        "turn_id": turn_id,
                        "workspace_mode": workspace_mode_label(workspace_mode),
                        "workspace_cwd": workspace_cwd,
                        "isolated_write_patch": isolated_write_patch,
                        "isolated_write_handoff": isolated_write_handoff,
                        "isolated_write_auto_apply": isolated_write_auto_apply,
                        "status": status,
                        "reason": reason,
                    });
                    let is_fan_out_result = expected_artifact_type == "fan_out_result";
                    if is_fan_out_result {
                        payload["schema_version"] = serde_json::Value::String(
                            omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1.to_string(),
                        );
                    }
                    let text = match serde_json::to_string_pretty(&payload) {
                        Ok(json) => format!("```json\n{json}\n```\n"),
                        Err(_) => payload.to_string(),
                    };

                    let write_result = super::handle_artifact_write(
                        &server,
                        super::ArtifactWriteParams {
                            thread_id,
                            turn_id: Some(turn_id),
                            approval_id: None,
                            artifact_id: None,
                            artifact_type: expected_artifact_type.clone(),
                            summary: "fan-out result".to_string(),
                            text,
                        },
                    )
                    .await;
                    if is_fan_out_result
                        && let Ok(write) = &write_result
                        && let Some(raw_id) = write.get("artifact_id")
                        && let Ok(artifact_id) =
                            serde_json::from_value::<omne_protocol::ArtifactId>(raw_id.clone())
                    {
                        let auto_apply_error_present = payload
                            .get("isolated_write_auto_apply")
                            .and_then(serde_json::Value::as_object)
                            .and_then(|value| value.get("error"))
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|value| !value.trim().is_empty());

                        let marker_event = if auto_apply_error_present {
                            omne_protocol::ThreadEventKind::AttentionMarkerSet {
                                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                                turn_id: Some(turn_id),
                                artifact_id: Some(artifact_id),
                                artifact_type: Some("fan_out_result".to_string()),
                                process_id: None,
                                exit_code: None,
                                command: None,
                            }
                        } else {
                            omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                                marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                                turn_id: Some(turn_id),
                                reason: Some(
                                    "fan-out auto-apply completed without error".to_string(),
                                ),
                            }
                        };
                        if let Ok(thread_rt) = server.get_or_load_thread(thread_id).await {
                            let _ = thread_rt.append_event(marker_event).await;
                        }
                    }
                    return;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

const DEFAULT_ISOLATED_PATCH_MAX_BYTES: u64 =
    omne_git_runtime::DEFAULT_ISOLATED_PATCH_MAX_BYTES;
const DEFAULT_ISOLATED_PATCH_TIMEOUT_MS: u64 =
    omne_git_runtime::DEFAULT_ISOLATED_PATCH_TIMEOUT_MS;
const ISOLATED_AUTO_APPLY_PATCH_ENV: &str = "OMNE_SUBAGENT_ISOLATED_AUTO_APPLY_PATCH";

fn parse_subagent_env_bool(raw: Option<&str>, default: bool) -> bool {
    let Some(raw) = raw else {
        return default;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn detect_isolated_workspace_backend(workspace_cwd: &str) -> Option<IsolatedWorkspaceBackend> {
    let workspace_cwd = workspace_cwd.trim();
    if workspace_cwd.is_empty() {
        return None;
    }
    let marker = std::path::Path::new(workspace_cwd).join(".git");
    let metadata = std::fs::metadata(&marker).ok()?;
    if metadata.is_file() {
        return Some(IsolatedWorkspaceBackend::Worktree);
    }
    if metadata.is_dir() {
        return Some(IsolatedWorkspaceBackend::Copy);
    }
    None
}

fn json_value_string_array(value: &Value) -> Option<Vec<String>> {
    let values = value.as_array()?;
    let mut out = Vec::with_capacity(values.len());
    for item in values {
        let text = item.as_str()?.trim();
        if text.is_empty() {
            continue;
        }
        out.push(text.to_string());
    }
    Some(out)
}

fn isolated_auto_apply_patch_enabled_from_env() -> bool {
    parse_subagent_env_bool(
        std::env::var(ISOLATED_AUTO_APPLY_PATCH_ENV).ok().as_deref(),
        false,
    )
}

async fn capture_isolated_workspace_patch(cwd: &str) -> anyhow::Result<Option<(String, bool)>> {
    let max_patch_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_PATCH_MAX_BYTES",
        DEFAULT_ISOLATED_PATCH_MAX_BYTES,
        1_024,
        64 * 1024 * 1024,
    ) as usize;
    let timeout_ms = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_PATCH_TIMEOUT_MS",
        DEFAULT_ISOLATED_PATCH_TIMEOUT_MS,
        100,
        120_000,
    );

    let patch = omne_git_runtime::capture_workspace_patch(
        cwd,
        omne_git_runtime::PatchCaptureConfig::new(
            max_patch_bytes,
            std::time::Duration::from_millis(timeout_ms),
        ),
    )
    .await?;

    Ok(patch.map(|value| (value.text, value.truncated)))
}

async fn try_auto_apply_isolated_workspace_patch(
    workspace_cwd: &str,
    target_workspace_cwd: Option<&str>,
    status: &omne_protocol::TurnStatus,
    enabled: bool,
) -> Option<Value> {
    if !enabled {
        return None;
    }

    let mut payload = serde_json::json!({
        "enabled": true,
        "attempted": false,
        "applied": false,
        "workspace_cwd": workspace_cwd,
        "target_workspace_cwd": target_workspace_cwd,
    });

    let set_failure =
        |payload: &mut Value,
         stage: omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage,
         hint: &str,
         error: String| {
            payload["failure_stage"] = serde_json::json!(stage.as_str());
            payload["recovery_hint"] = serde_json::json!(hint);
            payload["error"] = serde_json::json!(error);
        };

    let max_patch_bytes = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_PATCH_MAX_BYTES",
        DEFAULT_ISOLATED_PATCH_MAX_BYTES,
        1_024,
        64 * 1024 * 1024,
    ) as usize;
    let timeout_ms = parse_env_u64(
        "OMNE_SUBAGENT_ISOLATED_PATCH_TIMEOUT_MS",
        DEFAULT_ISOLATED_PATCH_TIMEOUT_MS,
        100,
        120_000,
    );
    let outcome = omne_git_runtime::auto_apply_workspace_patch(
        workspace_cwd,
        target_workspace_cwd,
        matches!(status, omne_protocol::TurnStatus::Completed),
        omne_git_runtime::PatchCaptureConfig::new(
            max_patch_bytes,
            std::time::Duration::from_millis(timeout_ms),
        ),
    )
    .await;

    payload["attempted"] = serde_json::json!(outcome.attempted);
    payload["applied"] = serde_json::json!(outcome.applied);
    if let Some(check_argv) = outcome.check_argv {
        payload["check_argv"] = serde_json::json!(check_argv);
    }
    if let Some(apply_argv) = outcome.apply_argv {
        payload["apply_argv"] = serde_json::json!(apply_argv);
    }
    if let Some(failure) = outcome.failure {
        let stage = match failure.stage {
            omne_git_runtime::AutoApplyFailureStage::Precondition => {
                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::Precondition
            }
            omne_git_runtime::AutoApplyFailureStage::CapturePatch => {
                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CapturePatch
            }
            omne_git_runtime::AutoApplyFailureStage::CheckPatch => {
                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch
            }
            omne_git_runtime::AutoApplyFailureStage::ApplyPatch => {
                omne_workflow_spec::FanOutResultIsolatedWriteAutoApplyFailureStage::ApplyPatch
            }
        };
        set_failure(&mut payload, stage, failure.hint, failure.error);
    }

    Some(payload)
}

async fn try_write_isolated_workspace_patch_artifact(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: TurnId,
    task_id: &str,
    workspace_cwd: &str,
) -> Option<Value> {
    let patch = match capture_isolated_workspace_patch(workspace_cwd).await {
        Ok(Some(patch)) => patch,
        Ok(None) => return None,
        Err(err) => {
            return Some(serde_json::json!({
                "workspace_cwd": workspace_cwd,
                "error": err.to_string(),
            }));
        }
    };

    let summary = format!("fan-out isolated patch ({task_id})");
    let write = match super::handle_artifact_write(
        server,
        super::ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "patch".to_string(),
            summary,
            text: patch.0,
        },
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            return Some(serde_json::json!({
                "workspace_cwd": workspace_cwd,
                "error": format!("patch artifact write failed: {err}"),
            }));
        }
    };

    let Some(artifact_id) = write.get("artifact_id").and_then(Value::as_str) else {
        return Some(serde_json::json!({
            "workspace_cwd": workspace_cwd,
            "error": "patch artifact write response missing artifact_id",
        }));
    };

    Some(serde_json::json!({
        "artifact_type": "patch",
        "artifact_id": artifact_id,
        "truncated": patch.1,
        "read_cmd": format!("omne artifact read {} {}", thread_id, artifact_id),
    }))
}
