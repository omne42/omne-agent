#[cfg(test)]
mod mcp_tests {
    use super::*;
    use tokio::sync::Mutex;

    static MCP_TEST_MUTEX: Mutex<()> = Mutex::const_new(());

    struct McpEnabledOverrideGuard;

    impl McpEnabledOverrideGuard {
        fn new(value: Option<bool>) -> Self {
            set_mcp_enabled_override_for_tests(value);
            Self
        }
    }

    impl Drop for McpEnabledOverrideGuard {
        fn drop(&mut self) {
            set_mcp_enabled_override_for_tests(None);
        }
    }

    async fn write_test_mcp_config(repo_dir: &Path) -> anyhow::Result<()> {
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "local": { "transport": "stdio", "argv": ["printf", "ok"] } } }"#,
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn load_mcp_config_defaults_to_empty_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_mcp_config(dir.path()).await.unwrap();
        assert!(cfg.path().is_none());
        assert!(cfg.servers().is_empty());
    }

    #[tokio::test]
    async fn load_mcp_config_parses_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("mcp.json"),
            r#"{ "version": 1, "servers": { "rg": { "transport": "stdio", "argv": ["mcp-rg", "--stdio"], "env": { "NO_COLOR": "1" } } } }"#,
        )
        .await
        .unwrap();

        let cfg = load_mcp_config(dir.path()).await.unwrap();
        assert!(cfg.path().is_some());
        assert_eq!(cfg.servers().len(), 1);
        let server = cfg.servers().get("rg").unwrap();
        assert_eq!(
            server.argv(),
            vec!["mcp-rg".to_string(), "--stdio".to_string()].as_slice()
        );
        assert!(server.env().contains_key("NO_COLOR"));
    }

    #[tokio::test]
    async fn load_mcp_config_denies_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("mcp.json"),
            r#"{ "version": 1, "servers": {}, "extra": 123 }"#,
        )
        .await
        .unwrap();

        let err = load_mcp_config(dir.path()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("parse"), "err={msg}");
    }

    #[tokio::test]
    async fn load_mcp_config_denies_invalid_server_names() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("mcp.json"),
            r#"{ "version": 1, "servers": { "bad name": { "transport": "stdio", "argv": ["x"] } } }"#,
        )
        .await
        .unwrap();

        let err = load_mcp_config(dir.path()).await.unwrap_err();
        assert!(err.to_string().contains("invalid mcp server name"));
    }

    #[tokio::test]
    async fn load_mcp_config_env_path_is_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_mcp_config_inner(dir.path(), Some(PathBuf::from("missing.json")))
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing.json") && (msg.contains("stat") || msg.contains("read")),
            "err={msg}"
        );
    }

    #[tokio::test]
    async fn mcp_list_servers_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        let mode_name = "mcp-list-servers-override-deny";
        let mode_yaml = r#"
version: 1
modes:
  mcp-list-servers-override-deny:
    description: "mcp list servers deny override"
    permissions:
      read:
        decision: allow
    tool_overrides:
      - tool: mcp/list_servers
        decision: deny
"#;
        let (server, thread_id) =
            setup_test_thread_mode_shared(&repo_dir, mode_name, mode_yaml).await?;

        let result = handle_mcp_list_servers(
            &server,
            McpListServersParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_servers_success_returns_typed_response() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        let result = handle_mcp_list_servers(
            &server,
            McpListServersParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let parsed: omne_app_server_protocol::McpListServersResponse =
            serde_json::from_value(result)?;

        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "local");
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_servers_denied_by_mode_permission_reports_decision_source() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        let mode_name = "mcp-list-servers-mode-deny";
        let mode_yaml = r#"
version: 1
modes:
  mcp-list-servers-mode-deny:
    description: "mcp list servers mode deny"
    permissions:
      read:
        decision: deny
"#;
        let (server, thread_id) =
            setup_test_thread_mode_shared(&repo_dir, mode_name, mode_yaml).await?;

        let result = handle_mcp_list_servers(
            &server,
            McpListServersParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["error_code"].as_str(), Some("mode_denied"));
        assert_eq!(result["decision_source"].as_str(), Some("mode_permission"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(false));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_servers_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

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
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_mcp_list_servers(
            &server,
            McpListServersParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("mcp/list_servers"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_tools_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;
        let mode_name = "mcp-list-tools-override-deny";
        let mode_yaml = r#"
version: 1
modes:
  mcp-list-tools-override-deny:
    description: "mcp list tools deny override"
    permissions:
      read:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
    tool_overrides:
      - tool: mcp/list_tools
        decision: deny
"#;
        let (server, thread_id) =
            setup_test_thread_mode_shared(&repo_dir, mode_name, mode_yaml).await?;

        let result = handle_mcp_list_tools(
            &server,
            McpListToolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "local".to_string(),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_resources_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;
        let mode_name = "mcp-list-resources-override-deny";
        let mode_yaml = r#"
version: 1
modes:
  mcp-list-resources-override-deny:
    description: "mcp list resources deny override"
    permissions:
      read:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
    tool_overrides:
      - tool: mcp/list_resources
        decision: deny
"#;
        let (server, thread_id) =
            setup_test_thread_mode_shared(&repo_dir, mode_name, mode_yaml).await?;

        let result = handle_mcp_list_resources(
            &server,
            McpListResourcesParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "local".to_string(),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_call_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;
        let mode_name = "mcp-call-override-deny";
        let mode_yaml = r#"
version: 1
modes:
  mcp-call-override-deny:
    description: "mcp call deny override"
    permissions:
      read:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
    tool_overrides:
      - tool: mcp/call
        decision: deny
"#;
        let (server, thread_id) =
            setup_test_thread_mode_shared(&repo_dir, mode_name, mode_yaml).await?;

        let result = handle_mcp_call(
            &server,
            McpCallParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "local".to_string(),
                tool: "noop".to_string(),
                arguments: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }
}
