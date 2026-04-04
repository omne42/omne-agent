fn managed_subagent_worktree_root(server: &Server) -> std::path::PathBuf {
    server.thread_store.root().join("tmp").join("subagents")
}

async fn managed_subagent_worktree_path(
    server: &Server,
    cwd: Option<&str>,
) -> Option<std::path::PathBuf> {
    let cwd = cwd.map(str::trim).filter(|value| !value.is_empty())?;
    let path = tokio::fs::canonicalize(cwd).await.ok()?;
    let root = tokio::fs::canonicalize(managed_subagent_worktree_root(server))
        .await
        .ok()?;
    if !path.starts_with(root) {
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
    let Some(path) = managed_subagent_worktree_path(server, cwd).await else {
        return;
    };
    let worktree = path.display().to_string();
    match omne_git_runtime::remove_detached_worktree_and_prune(&worktree).await {
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
