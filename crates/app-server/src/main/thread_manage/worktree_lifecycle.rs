fn managed_subagent_worktree_root(server: &Server) -> std::path::PathBuf {
    server
        .cwd
        .join(".omne_data")
        .join("tmp")
        .join("subagents")
}

fn managed_subagent_worktree_path(server: &Server, cwd: Option<&str>) -> Option<std::path::PathBuf> {
    let cwd = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    let path = std::path::PathBuf::from(cwd);
    if !path.starts_with(managed_subagent_worktree_root(server)) {
        return None;
    }
    Some(path)
}

async fn cleanup_managed_subagent_worktree(
    server: &Server,
    thread_id: ThreadId,
    cwd: Option<&str>,
    lifecycle: &'static str,
) {
    let Some(path) = managed_subagent_worktree_path(server, cwd) else {
        return;
    };
    let worktree = path.display().to_string();
    match omne_thread_git_snapshot_runtime::remove_detached_worktree_and_prune(&worktree).await {
        Ok(true) => {
            tracing::debug!(thread_id = %thread_id, lifecycle, worktree = %worktree, "cleaned managed worktree");
        }
        Ok(false) => {}
        Err(err) => {
            tracing::warn!(
                thread_id = %thread_id,
                lifecycle,
                worktree = %worktree,
                error = %err,
                "failed to cleanup managed worktree"
            );
        }
    }
}
