#[cfg(test)]
mod thread_manage_worktree_lifecycle_tests {
    use super::*;

    async fn run_git(cwd: &std::path::Path, args: &[&str]) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .await
            .with_context(|| format!("spawn git {} in {}", args.join(" "), cwd.display()))?;
        if output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git {} failed in {} (exit {:?}): stdout={}, stderr={}",
            args.join(" "),
            cwd.display(),
            output.status.code(),
            stdout,
            stderr
        );
    }

    async fn init_repo(repo_dir: &std::path::Path) -> anyhow::Result<()> {
        run_git(repo_dir, &["init"]).await?;
        run_git(repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(repo_dir, &["config", "user.name", "Test User"]).await?;
        tokio::fs::write(repo_dir.join("hello.txt"), "hello\n").await?;
        run_git(repo_dir, &["add", "hello.txt"]).await?;
        run_git(repo_dir, &["commit", "-m", "init"]).await?;
        Ok(())
    }

    async fn create_broken_managed_worktree(
        server: &Server,
        suffix: &str,
    ) -> anyhow::Result<std::path::PathBuf> {
        let worktree_dir = managed_subagent_worktree_root(server)
            .join("broken")
            .join(suffix)
            .join("repo");
        tokio::fs::create_dir_all(&worktree_dir).await?;
        tokio::fs::write(
            worktree_dir.join(".git"),
            "gitdir: /tmp/nonexistent/worktrees/broken\n",
        )
        .await?;
        Ok(worktree_dir)
    }

    #[tokio::test]
    async fn thread_archive_cleans_managed_detached_worktree() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let source_repo = tmp.path().join("source");
        tokio::fs::create_dir_all(&source_repo).await?;
        init_repo(&source_repo).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let worktree_dir = managed_subagent_worktree_root(&server)
            .join("parent-thread")
            .join("archive-case")
            .join("repo");
        if let Some(parent) = worktree_dir.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        omne_git_runtime::create_detached_worktree(
            &source_repo.display().to_string(),
            &worktree_dir.display().to_string(),
            None,
        )
        .await?;

        let handle = server
            .thread_store
            .create_thread(worktree_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        let rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        server.threads.lock().await.insert(thread_id, rt);

        let result = handle_thread_archive(
            &server,
            ThreadArchiveParams {
                thread_id,
                force: false,
                reason: None,
            },
        )
        .await?;
        assert!(result.archived);
        assert!(!worktree_dir.exists());

        let output = tokio::process::Command::new("git")
            .args(["worktree", "list"])
            .current_dir(&source_repo)
            .output()
            .await?;
        assert!(output.status.success());
        let listed = String::from_utf8_lossy(&output.stdout);
        assert!(!listed.contains(worktree_dir.display().to_string().as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn thread_delete_cleans_managed_detached_worktree() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let source_repo = tmp.path().join("source");
        tokio::fs::create_dir_all(&source_repo).await?;
        init_repo(&source_repo).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let worktree_dir = managed_subagent_worktree_root(&server)
            .join("parent-thread")
            .join("delete-case")
            .join("repo");
        if let Some(parent) = worktree_dir.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        omne_git_runtime::create_detached_worktree(
            &source_repo.display().to_string(),
            &worktree_dir.display().to_string(),
            None,
        )
        .await?;

        let handle = server
            .thread_store
            .create_thread(worktree_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_thread_delete(
            &server,
            ThreadDeleteParams {
                thread_id,
                force: false,
            },
        )
        .await?;
        assert!(result.deleted);
        assert!(!worktree_dir.exists());

        let output = tokio::process::Command::new("git")
            .args(["worktree", "list"])
            .current_dir(&source_repo)
            .output()
            .await?;
        assert!(output.status.success());
        let listed = String::from_utf8_lossy(&output.stdout);
        assert!(!listed.contains(worktree_dir.display().to_string().as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn thread_archive_ignores_cleanup_errors_for_managed_broken_worktree()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let broken_worktree = create_broken_managed_worktree(&server, "archive").await?;

        let handle = server
            .thread_store
            .create_thread(broken_worktree.clone())
            .await?;
        let thread_id = handle.thread_id();
        let rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        server.threads.lock().await.insert(thread_id, rt);

        let result = handle_thread_archive(
            &server,
            ThreadArchiveParams {
                thread_id,
                force: false,
                reason: Some("archive broken worktree".to_string()),
            },
        )
        .await?;
        assert!(result.archived);
        assert!(broken_worktree.exists());
        Ok(())
    }

    #[tokio::test]
    async fn thread_delete_ignores_cleanup_errors_for_managed_broken_worktree() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let broken_worktree = create_broken_managed_worktree(&server, "delete").await?;

        let handle = server
            .thread_store
            .create_thread(broken_worktree.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_thread_delete(
            &server,
            ThreadDeleteParams {
                thread_id,
                force: false,
            },
        )
        .await?;
        assert!(result.deleted);
        assert!(broken_worktree.exists());
        Ok(())
    }

    #[tokio::test]
    async fn thread_delete_refuses_active_turn_without_force() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let mut handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        let turn_id = TurnId::new();
        handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "delete me".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        let rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        {
            let mut active = rt.active_turn.lock().await;
            *active = Some(ActiveTurn {
                turn_id,
                cancel: tokio_util::sync::CancellationToken::new(),
                interrupt_reason: None,
            });
        }
        server.threads.lock().await.insert(thread_id, rt);

        let err = handle_thread_delete(
            &server,
            ThreadDeleteParams {
                thread_id,
                force: false,
            },
        )
        .await
        .expect_err("active turn should require force=true");

        assert!(err.to_string().contains("active turn"));
        Ok(())
    }

    #[tokio::test]
    async fn managed_subagent_worktree_path_rejects_noncanonical_escape() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let managed_root = managed_subagent_worktree_root(&server);
        tokio::fs::create_dir_all(managed_root.join("parent")).await?;

        let escape = tmp.path().join("escape").join("repo");
        tokio::fs::create_dir_all(&escape).await?;

        let spoofed = managed_root
            .join("parent")
            .join("..")
            .join("..")
            .join("..")
            .join("escape")
            .join("repo");

        let resolved = managed_subagent_worktree_path(
            &server,
            Some(spoofed.to_string_lossy().as_ref()),
        )
        .await;
        assert!(resolved.is_none(), "noncanonical escape must be rejected");
        Ok(())
    }
}
