#[cfg(test)]
mod repo_index_search_tests {
    use super::*;

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
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        Ok(read["text"].as_str().unwrap_or("").to_string())
    }

    async fn configure_mode_with_tool_override(
        repo_dir: &std::path::Path,
        server: &Server,
        thread_id: ThreadId,
        mode_name: &str,
        tool: &str,
    ) -> anyhow::Result<()> {
        write_modes_yaml_shared(
            repo_dir,
            format!(
                r#"
version: 1
modes:
  {mode_name}:
    description: "tool override deny"
    permissions:
      read:
        decision: allow
      edit:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
    tool_overrides:
      - tool: {tool}
        decision: deny
"#
            )
            .as_str(),
        )
        .await?;
        configure_test_thread_mode_shared(server, thread_id, mode_name).await?;
        Ok(())
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

        tokio::fs::create_dir_all(repo_dir.join(".omne_data/tmp")).await?;
        tokio::fs::write(repo_dir.join(".omne_data/tmp/leak.txt"), "needle\n").await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

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

        let response: omne_app_server_protocol::RepoSearchResponse =
            serde_json::from_value(result).context("parse repo/search response")?;
        assert_eq!(response.root, "workspace");
        let artifact_id = response.artifact_id;
        let text = read_user_artifact_text(&server, thread_id, artifact_id).await?;

        assert!(text.contains("# Repo Search"));
        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains(".env"));
        assert!(!text.contains(".omne_data/tmp"));

        Ok(())
    }

    #[tokio::test]
    async fn repo_index_writes_artifact_and_ignores_omne_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(
            repo_dir.join("src/lib.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .await?;

        tokio::fs::write(repo_dir.join(".env"), "should_not_be_indexed\n").await?;
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/tmp")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/tmp/leak.txt"),
            "should_not_be_indexed\n",
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

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

        let response: omne_app_server_protocol::RepoIndexResponse =
            serde_json::from_value(result).context("parse repo/index response")?;
        assert_eq!(response.root, "workspace");
        let artifact_id = response.artifact_id;
        let text = read_user_artifact_text(&server, thread_id, artifact_id).await?;

        assert!(text.contains("# Repo Index"));
        assert!(text.contains("src/lib.rs"));
        assert!(!text.contains(".env"));
        assert!(!text.contains(".omne_data/tmp"));

        Ok(())
    }

    #[tokio::test]
    async fn repo_search_needs_approval_includes_thread_id() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "needle\n").await?;

        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
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

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("prompt-mode".to_string()),
                role: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
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

    #[tokio::test]
    async fn repo_search_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "needle\n").await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                role: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/index".to_string()])),
                execpolicy_rules: None,
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

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("repo/search"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/index"));
        Ok(())
    }

    #[tokio::test]
    async fn repo_symbols_writes_artifact_and_extracts_rust_symbols() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");

        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(
            repo_dir.join("src/lib.rs"),
            r#"
mod foo {
    pub struct Bar;
    fn baz() {}
}

fn top_level() {}
"#,
        )
        .await?;

        tokio::fs::write(repo_dir.join(".env"), "should_not_be_indexed\n").await?;
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/tmp")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/tmp/leak.txt"),
            "should_not_be_indexed\n",
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        let result = handle_repo_symbols(
            &server,
            RepoSymbolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                include_glob: None,
                max_files: Some(20_000),
                max_bytes_per_file: Some(1024 * 1024),
                max_symbols: Some(2000),
            },
        )
        .await?;

        let response: omne_app_server_protocol::RepoSymbolsResponse =
            serde_json::from_value(result).context("parse repo/symbols response")?;
        assert_eq!(response.root, "workspace");
        let artifact_id = response.artifact_id;
        let text = read_user_artifact_text(&server, thread_id, artifact_id).await?;

        assert!(text.contains("# Repo Symbols (Rust)"));
        assert!(text.contains("src/lib.rs"));
        assert!(text.contains("foo::Bar"));
        assert!(text.contains("foo::baz"));
        assert!(text.contains("top_level"));
        assert!(!text.contains(".env"));
        assert!(!text.contains(".omne_data/tmp"));

        Ok(())
    }

    #[tokio::test]
    async fn repo_search_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        configure_mode_with_tool_override(
            &repo_dir,
            &server,
            thread_id,
            "repo-search-override-deny",
            "repo/search",
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

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn repo_index_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        configure_mode_with_tool_override(
            &repo_dir,
            &server,
            thread_id,
            "repo-index-override-deny",
            "repo/index",
        )
        .await?;

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

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn repo_symbols_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        configure_mode_with_tool_override(
            &repo_dir,
            &server,
            thread_id,
            "repo-symbols-override-deny",
            "repo/symbols",
        )
        .await?;

        let result = handle_repo_symbols(
            &server,
            RepoSymbolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                include_glob: None,
                max_files: Some(20_000),
                max_bytes_per_file: Some(1024 * 1024),
                max_symbols: Some(2000),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }
}
