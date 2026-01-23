#[cfg(test)]
mod repo_index_search_tests {
    use super::*;

    fn build_test_server(pm_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: pm_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    async fn read_user_artifact_text(
        server: &Server,
        thread_id: ThreadId,
        artifact_id: ArtifactId,
    ) -> anyhow::Result<String> {
        let read = handle_artifact_read(
            server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                max_bytes: None,
            },
        )
        .await?;
        Ok(read["text"].as_str().unwrap_or("").to_string())
    }

    #[tokio::test]
    async fn repo_search_writes_artifact_and_skips_secrets() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(
            repo_dir.join("src/lib.rs"),
            "fn main() { println!(\"needle\"); }\n",
        )
        .await?;

        tokio::fs::write(repo_dir.join(".env"), "needle\n").await?;

        tokio::fs::create_dir_all(repo_dir.join(".codepm_data/tmp")).await?;
        tokio::fs::write(repo_dir.join(".codepm_data/tmp/leak.txt"), "needle\n").await?;

        let server = build_test_server(repo_dir.join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_repo_search(
            &server,
            RepoSearchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                query: "needle".to_string(),
                is_regex: false,
                include_glob: None,
                max_matches: Some(20),
                max_bytes_per_file: Some(1024 * 1024),
                max_files: Some(20_000),
            },
        )
        .await?;

        let artifact_id: ArtifactId = serde_json::from_value(result["artifact_id"].clone())
            .context("artifact_id missing")?;
        let text = read_user_artifact_text(&server, thread_id, artifact_id).await?;

        assert!(text.contains("# Repo Search"));
        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains(".env"));
        assert!(!text.contains(".codepm_data/tmp"));

        Ok(())
    }

    #[tokio::test]
    async fn repo_index_writes_artifact_and_ignores_codepm_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(
            repo_dir.join("src/lib.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .await?;

        tokio::fs::write(repo_dir.join(".env"), "should_not_be_indexed\n").await?;
        tokio::fs::create_dir_all(repo_dir.join(".codepm_data/tmp")).await?;
        tokio::fs::write(
            repo_dir.join(".codepm_data/tmp/leak.txt"),
            "should_not_be_indexed\n",
        )
        .await?;

        let server = build_test_server(repo_dir.join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_repo_index(
            &server,
            RepoIndexParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                include_glob: None,
                max_files: Some(20_000),
            },
        )
        .await?;

        let artifact_id: ArtifactId = serde_json::from_value(result["artifact_id"].clone())
            .context("artifact_id missing")?;
        let text = read_user_artifact_text(&server, thread_id, artifact_id).await?;

        assert!(text.contains("# Repo Index"));
        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains(".env"));
        assert!(!text.contains(".codepm_data/tmp"));

        Ok(())
    }

    #[tokio::test]
    async fn repo_search_needs_approval_includes_thread_id() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "needle\n").await?;

        tokio::fs::create_dir_all(repo_dir.join(".codepm_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".codepm_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  prompt-mode:
    description: "prompt mode"
    permissions:
      read:
        decision: allow
      artifact:
        decision: allow
    tool_overrides:
      - tool: repo/search
        decision: prompt
"#,
        )
        .await?;

        let server = build_test_server(repo_dir.join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(pm_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("prompt-mode".to_string()),
                model: None,
                openai_base_url: None,
                allowed_tools: None,
            },
        )
        .await?;

        let result = handle_repo_search(
            &server,
            RepoSearchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                query: "needle".to_string(),
                is_regex: false,
                include_glob: None,
                max_matches: Some(20),
                max_bytes_per_file: Some(1024 * 1024),
                max_files: Some(20_000),
            },
        )
        .await?;

        assert!(result["needs_approval"].as_bool().unwrap_or(false));
        let thread_id_str = thread_id.to_string();
        assert_eq!(result["thread_id"].as_str(), Some(thread_id_str.as_str()));
        assert!(result["approval_id"].is_string());
        Ok(())
    }
}
