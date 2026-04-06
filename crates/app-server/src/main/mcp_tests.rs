#[cfg(test)]
mod mcp_tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::sync::mpsc;
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

    async fn insert_running_mcp_connection(
        server: &Server,
        thread_id: ThreadId,
        server_name: &str,
        config_fingerprint: String,
    ) -> anyhow::Result<(ProcessId, mpsc::Receiver<ProcessCommand>)> {
        let (client_stream, peer_stream) = tokio::io::duplex(1024);
        drop(peer_stream);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let client = omne_jsonrpc::Client::connect_io(client_read, client_write).await?;
        let process_id = ProcessId::new();
        let started_at = time::OffsetDateTime::now_utc().format(&Rfc3339)?;
        let (cmd_tx, cmd_rx) = mpsc::channel(1);

        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id,
                info: Arc::new(tokio::sync::Mutex::new(ProcessInfo {
                    process_id,
                    thread_id,
                    turn_id: None,
                    os_pid: None,
                    argv: vec!["mock-mcp".to_string()],
                    cwd: ".".to_string(),
                    started_at: started_at.clone(),
                    status: ProcessStatus::Running,
                    exit_code: None,
                    stdout_path: String::new(),
                    stderr_path: String::new(),
                    last_update_at: started_at,
                })),
                cmd_tx,
                completion: ProcessCompletion::new(),
            },
        );
        server.mcp.lock().await.connections.insert(
            (thread_id, server_name.to_string()),
            Arc::new(McpConnection {
                process_id,
                config_fingerprint,
                client: tokio::sync::Mutex::new(client),
            }),
        );

        Ok((process_id, cmd_rx))
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
            msg.contains("missing.json")
                && (msg.contains("stat")
                    || msg.contains("read")
                    || msg.contains("config not found")),
            "err={msg}"
        );
    }

    #[tokio::test]
    async fn mcp_list_tools_load_config_failure_still_emits_tool_lifecycle() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("mcp.json"), r#"{ "version": 1, "servers": {}, "extra": 1 }"#).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        let err = handle_mcp_list_tools(
            &server,
            McpListToolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "local".to_string(),
            },
        )
        .await
        .expect_err("invalid config should fail");
        assert!(err.to_string().contains("parse"), "err={err}");

        let events = server
            .thread_store
            .read_events_since(thread_id, omne_protocol::EventSeq::ZERO)
            .await?
            .expect("thread exists");
        let started_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                omne_protocol::ThreadEventKind::ToolStarted { tool_id, tool, .. }
                    if tool == "mcp/list_tools" =>
                {
                    Some(*tool_id)
                }
                _ => None,
            })
            .expect("mcp/list_tools should emit ToolStarted");
        let completed = events
            .iter()
            .find(|event| matches!(
                event.kind,
                omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    ..
                } if tool_id == started_tool_id
            ))
            .expect("mcp/list_tools should emit failed ToolCompleted");
        assert!(matches!(
            completed.kind,
            omne_protocol::ThreadEventKind::ToolCompleted {
                error: Some(_),
                ..
            }
        ));

        Ok(())
    }

    #[tokio::test]
    async fn mcp_denies_generic_launchers_when_network_access_is_denied() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "wrapped": { "transport": "stdio", "argv": ["python", "-m", "http.server"] } } }"#,
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
            },
        )
        .await?;

        let result = handle_mcp_list_tools(
            &server,
            McpListToolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "wrapped".to_string(),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_network_access"].as_str(), Some("deny"));
        Ok(())
    }

    #[tokio::test]
    async fn mcp_denies_path_invocations_when_network_access_is_denied() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "wrapped": { "transport": "stdio", "argv": ["./mock-mcp"] } } }"#,
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Deny),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
            },
        )
        .await?;

        let result = handle_mcp_list_tools(
            &server,
            McpListToolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "wrapped".to_string(),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_network_access"].as_str(), Some("deny"));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mcp_list_tools_denies_non_executable_server_program() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let script_path = repo_dir.join("non-executable-mcp");
        tokio::fs::write(&script_path, "#!/bin/sh\nexit 0\n").await?;
        tokio::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o644)).await?;

        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "local": { "transport": "stdio", "argv": ["./non-executable-mcp"] } } }"#,
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: Some(policy_meta::WriteScope::WorkspaceWrite),
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Allow),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
            },
        )
        .await?;

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
        assert_eq!(
            result["error_code"].as_str(),
            Some("execution_boundary_denied")
        );
        Ok(())
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
        assert_eq!(parsed.servers[0].transport, "stdio");
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_servers_omits_startup_args_and_env_keys() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{
  "version": 1,
  "servers": {
    "local": {
      "transport": "stdio",
      "argv": ["mcp-server", "--api-key", "super-secret", "authorization=Bearer abcdefghijklmnopqrstuvwxyz"]
    }
    }
}"#,
        )
        .await?;

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
            serde_json::from_value(result.clone())?;

        assert_eq!(parsed.servers[0].name, "local");
        assert_eq!(parsed.servers[0].transport, "stdio");
        assert!(result["servers"][0].get("argv").is_none());
        assert!(result["servers"][0].get("env_keys").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_servers_filters_unsupported_transports() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{
  "version": 1,
  "servers": {
    "local": { "transport": "stdio", "argv": ["printf", "ok"] },
    "remote": { "transport": "streamable_http", "url": "https://example.test/mcp" }
  }
}"#,
        )
        .await?;

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
        assert_eq!(parsed.servers[0].transport, "stdio");
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_servers_failure_still_appends_tool_completed() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("mcp.json"), r#"{ "version": 1, "#).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        let err = handle_mcp_list_servers(
            &server,
            McpListServersParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await
        .expect_err("invalid mcp config should fail");
        assert!(err.to_string().contains("parse"), "err={err:#}");

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread events should exist");
        let tool_events = events
            .into_iter()
            .filter_map(|event| match event.kind {
                omne_protocol::ThreadEventKind::ToolStarted { tool, .. } if tool == "mcp/list_servers" => {
                    Some(("started".to_string(), None, None))
                }
                omne_protocol::ThreadEventKind::ToolCompleted {
                    status,
                    error,
                    result,
                    ..
                } => Some(("completed".to_string(), Some(status), Some((error, result)))),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(tool_events.iter().any(|(kind, _, _)| kind == "started"));
        let completed = tool_events
            .iter()
            .find_map(|(kind, status, payload)| {
                (kind == "completed").then_some((*status, payload.clone()))
            })
            .expect("mcp/list_servers should append ToolCompleted on failure");
        assert_eq!(completed.0, Some(omne_protocol::ToolStatus::Failed));
        let (error, result) = completed.1.expect("failure payload");
        assert!(error.is_some());
        assert_eq!(
            result
                .as_ref()
                .and_then(|value| value.get("servers"))
                .and_then(Value::as_u64),
            Some(0)
        );
        Ok(())
    }

    #[tokio::test]
    async fn mcp_list_tools_connection_failure_still_appends_tool_completed() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        let err = handle_mcp_list_tools(
            &server,
            McpListToolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "local".to_string(),
            },
        )
        .await
        .expect_err("printf-backed mcp server should fail initialize");
        assert!(
            err.to_string().contains("mcp initialize failed")
                || err.to_string().contains("mcp request failed"),
            "err={err:#}"
        );

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .expect("thread events should exist");
        let started_tool_id = events
            .iter()
            .find_map(|event| match &event.kind {
                omne_protocol::ThreadEventKind::ToolStarted { tool_id, tool, .. }
                    if tool == "mcp/list_tools" =>
                {
                    Some(*tool_id)
                }
                _ => None,
            })
            .expect("mcp/list_tools should append ToolStarted");
        let completed = events
            .iter()
            .find_map(|event| match &event.kind {
                omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status,
                    error,
                    result,
                    ..
                } if tool_id == &started_tool_id => {
                    Some((*status, error.clone(), result.clone()))
                }
                _ => None,
            })
            .expect("mcp/list_tools should append ToolCompleted on connection failure");

        assert_eq!(completed.0, omne_protocol::ToolStatus::Failed);
        assert!(completed.1.is_some());
        let result = completed.2.expect("failure payload");
        assert_eq!(result.get("server").and_then(Value::as_str), Some("local"));
        assert!(result.get("process_id").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn get_or_start_mcp_connection_invalidates_cached_connection_when_config_changes(
    ) -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let initial_cfg = load_mcp_config(&repo_dir).await?;
        let initial_server_cfg = initial_cfg
            .servers()
            .get("local")
            .expect("initial local server");
        let (_process_id, mut cmd_rx) = insert_running_mcp_connection(
            &server,
            thread_id,
            "local",
            mcp_server_config_fingerprint(initial_server_cfg),
        )
        .await?;

        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "local": { "transport": "stdio", "argv": ["printf", "changed"] } } }"#,
        )
        .await?;
        let changed_cfg = load_mcp_config(&repo_dir).await?;
        let changed_server_cfg = changed_cfg
            .servers()
            .get("local")
            .expect("changed local server");

        let err = get_or_start_mcp_connection(
            &server,
            &thread_rt,
            &repo_dir,
            thread_id,
            None,
            policy_meta::WriteScope::WorkspaceWrite,
            "local",
            changed_server_cfg,
        )
        .await
        .expect_err("changed config should invalidate cache before reconnect");
        assert!(
            err.to_string().contains("mcp initialize failed")
                || err.to_string().contains("mcp initialized notification failed")
                || err.to_string().contains("mcp request failed"),
            "err={err:#}"
        );

        let cmd = cmd_rx.recv().await.expect("old connection should be killed");
        match cmd {
            ProcessCommand::Kill { reason } => {
                assert_eq!(reason.as_deref(), Some("mcp config changed"));
            }
            other => panic!("expected Kill command, got {other:?}"),
        }

        let cached = {
            let manager = server.mcp.lock().await;
            manager.connections.get(&(thread_id, "local".to_string())).cloned()
        };
        if let Some(conn) = cached {
            assert_ne!(
                conn.config_fingerprint,
                mcp_server_config_fingerprint(initial_server_cfg)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn get_or_start_mcp_connection_prunes_exited_cached_process_entry() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_test_mcp_config(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        let thread_rt = server.get_or_load_thread(thread_id).await?;

        let cfg = load_mcp_config(&repo_dir).await?;
        let server_cfg = cfg.servers().get("local").expect("local server");
        let (stale_process_id, _cmd_rx) = insert_running_mcp_connection(
            &server,
            thread_id,
            "local",
            mcp_server_config_fingerprint(server_cfg),
        )
        .await?;

        let stale_entry = {
            let entries = server.processes.lock().await;
            entries
                .get(&stale_process_id)
                .cloned()
                .expect("stale process should exist")
        };
        {
            let mut info = stale_entry.info.lock().await;
            info.status = ProcessStatus::Exited;
            info.exit_code = Some(0);
        }

        let err = get_or_start_mcp_connection(
            &server,
            &thread_rt,
            &repo_dir,
            thread_id,
            None,
            policy_meta::WriteScope::WorkspaceWrite,
            "local",
            server_cfg,
        )
        .await
        .expect_err("exited cached process should be discarded before reconnect");
        assert!(
            err.to_string().contains("mcp initialize failed")
                || err.to_string().contains("mcp initialized notification failed")
                || err.to_string().contains("mcp request failed"),
            "unexpected error: {err:#}"
        );

        assert!(
            !server.processes.lock().await.contains_key(&stale_process_id),
            "exited cached process should be removed before reconnect"
        );
        let cached = {
            let manager = server.mcp.lock().await;
            manager.connections.get(&(thread_id, "local".to_string())).cloned()
        };
        assert!(
            cached
                .as_ref()
                .is_none_or(|conn| conn.process_id != stale_process_id),
            "stale cached process should not survive after reconnect attempt"
        );

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
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
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

    #[tokio::test]
    async fn failed_mcp_request_invalidates_cached_connection() -> anyhow::Result<()> {
        let _lock = MCP_TEST_MUTEX.lock().await;
        let _guard = McpEnabledOverrideGuard::new(Some(true));

        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(
            repo_dir.join("mcp.json"),
            r#"{ "version": 1, "servers": { "local": { "transport": "stdio", "argv": ["cat"] } } }"#,
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: Some(omne_protocol::SandboxNetworkAccess::Allow),
                mode: None,
                role: None,
                model: None,
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: None,
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let cfg = load_mcp_config(&repo_dir).await?;
        let server_cfg = cfg
            .servers()
            .get("local")
            .expect("local server should exist");
        let (process_id, mut cmd_rx) = insert_running_mcp_connection(
            &server,
            thread_id,
            "local",
            mcp_server_config_fingerprint(server_cfg),
        )
        .await?;

        let err = handle_mcp_list_tools(
            &server,
            McpListToolsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                server: "local".to_string(),
            },
        )
        .await
        .expect_err("closed cached connection should fail request");
        assert!(
            err.to_string().contains("mcp request failed"),
            "unexpected error: {err:#}"
        );

        assert!(
            !server
                .mcp
                .lock()
                .await
                .connections
                .contains_key(&(thread_id, "local".to_string()))
        );
        match tokio::time::timeout(Duration::from_secs(1), cmd_rx.recv()).await? {
            Some(ProcessCommand::Kill { reason }) => {
                assert_eq!(reason.as_deref(), Some("mcp request failed"));
            }
            Some(ProcessCommand::Interrupt { .. }) => {
                anyhow::bail!("expected kill command after invalidation, got interrupt command")
            }
            None => anyhow::bail!("expected kill command after invalidation, got channel close"),
        }
        assert!(server.processes.lock().await.contains_key(&process_id));

        Ok(())
    }
}
