#[cfg(test)]
mod artifact_history_tests {
    use super::*;

    fn build_test_server(agent_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: agent_root.clone(),
            notify_tx,
            notify_hub: default_notify_hub(),
            thread_store: ThreadStore::new(AgentPaths::new(agent_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_agent_execpolicy::Policy::empty(),
            db_vfs: None,
        }
    }

    #[tokio::test]
    async fn snapshot_user_artifact_version_writes_history_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_agent_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, _) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"hello").await?;

        let snapshotted =
            snapshot_user_artifact_version(&server, thread_id, artifact_id, &content_path, 7).await?;
        assert!(snapshotted);

        let history_path = user_artifact_history_path(&server, thread_id, artifact_id, 7);
        let bytes = tokio::fs::read(&history_path)
            .await
            .with_context(|| format!("read {}", history_path.display()))?;
        assert_eq!(String::from_utf8_lossy(&bytes), "hello");
        Ok(())
    }

    #[tokio::test]
    async fn prune_user_artifact_history_keeps_latest_versions() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_agent_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        for version in [1_u32, 2, 3] {
            let path = user_artifact_history_path(&server, thread_id, artifact_id, version);
            write_file_atomic(&path, format!("v{version}").as_bytes()).await?;
        }

        let removed = prune_user_artifact_history(&server, thread_id, artifact_id, 1).await?;
        assert_eq!(removed, vec![1, 2]);

        for version in [1_u32, 2] {
            let path = user_artifact_history_path(&server, thread_id, artifact_id, version);
            match tokio::fs::metadata(&path).await {
                Ok(_) => anyhow::bail!("expected {} to be pruned", path.display()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
            }
        }

        let path = user_artifact_history_path(&server, thread_id, artifact_id, 3);
        tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("stat {}", path.display()))?;

        Ok(())
    }

    #[tokio::test]
    async fn artifact_delete_removes_history_dir() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_agent_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let history_path = user_artifact_history_path(&server, thread_id, artifact_id, 1);
        write_file_atomic(&history_path, b"hello").await?;

        handle_artifact_delete(
            &server,
            ArtifactDeleteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
            },
        )
        .await?;

        let history_dir = user_artifact_history_dir_for_thread(&server, thread_id, artifact_id);
        match tokio::fs::metadata(&history_dir).await {
            Ok(_) => anyhow::bail!("expected {} to be removed", history_dir.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).with_context(|| format!("stat {}", history_dir.display())),
        }
    }
}
