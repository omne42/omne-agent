#[cfg(test)]
mod reference_repo_file_tools_tests {
    use super::*;

    async fn append_plan_turn_started(
        server: &super::super::Server,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "/plan".to_string(),
                context_refs: None,
                attachments: None,
                directives: Some(vec![omne_protocol::TurnDirective::Plan]),
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        Ok(())
    }

    async fn append_thread_approval_policy(
        server: &super::super::Server,
        thread_id: ThreadId,
        approval_policy: omne_protocol::ApprovalPolicy,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;
        Ok(())
    }

    async fn append_thread_mode(
        server: &super::super::Server,
        thread_id: ThreadId,
        mode: &str,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::Manual,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some(mode.to_string()),
                model: None,
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;
        Ok(())
    }

    async fn append_process_started(
        server: &super::super::Server,
        thread_id: ThreadId,
        process_id: omne_protocol::ProcessId,
    ) -> anyhow::Result<()> {
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ProcessStarted {
                process_id,
                turn_id: None,
                argv: vec!["echo".to_string(), "cross-thread".to_string()],
                cwd: "/tmp".to_string(),
                stdout_path: "/tmp/omne-test.stdout.log".to_string(),
                stderr_path: "/tmp/omne-test.stderr.log".to_string(),
            })
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_excludes_omne_reference_dir_for_workspace_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".omne_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".omne_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "pattern": "**/*.txt" }),
            None,
        )
        .await?;

        let paths = result["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("hello.txt")));
        assert!(
            !paths
                .iter()
                .any(|p| p.as_str().unwrap_or("").contains(".omne_data/reference/"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_and_read_can_use_reference_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".omne_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".omne_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let glob = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "root": "reference", "pattern": "**/*.txt" }),
            None,
        )
        .await?;
        let paths = glob["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("ref.txt")));
        assert!(!paths.iter().any(|p| p.as_str() == Some("hello.txt")));

        let read = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await?;
        assert_eq!(read["text"].as_str(), Some("ref\n"));
        assert_eq!(read["root"].as_str(), Some("reference"));
        Ok(())
    }

    #[tokio::test]
    async fn reference_root_fails_closed_when_not_configured() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let err = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await
        .expect_err("expected root=reference to fail when not configured");
        assert!(
            err.to_string().contains("reference repo root")
                || err.to_string().contains(".omne_data/reference/repo")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_blocks_side_effect_tool_calls() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server
            .thread_store
            .create_thread(project_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_write",
            serde_json::json!({ "path": "blocked.txt", "text": "blocked\n" }),
            None,
        )
        .await
        .expect_err("expected file_write to be blocked by /plan directive");
        assert!(err.to_string().contains("tool blocked by /plan directive"));
        assert!(
            !tokio::fs::try_exists(project_dir.join("blocked.txt")).await?,
            "blocked file_write should not create files"
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_allows_read_only_tool_calls() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(project_dir.join("note.txt"), "hello-plan\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "note.txt" }),
            None,
        )
        .await?;

        assert_eq!(result["text"].as_str(), Some("hello-plan\n"));
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_uses_architect_mode_gate_for_read_only_tools() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("note.txt"), "hello-plan\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect override"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/read"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "note.txt" }),
            None,
        )
        .await
        .expect_err("expected file_read to be denied by architect mode under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_prompt_returns_needs_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("note.txt"), "hello-plan\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect prompt override"
    permissions:
      read: { decision: prompt }
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_approval_policy(&server, thread_id, omne_protocol::ApprovalPolicy::Manual)
            .await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "note.txt" }),
            None,
        )
        .await?;

        assert_eq!(result["needs_approval"].as_bool(), Some(true));
        assert!(result["approval_id"].as_str().is_some());
        assert!(result.get("text").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_read_honors_deny_globs() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("blocked.txt"), "blocked\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect read deny globs"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked.txt"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_read",
            serde_json::json!({ "path": "blocked.txt" }),
            None,
        )
        .await
        .expect_err("expected file_read to be denied by architect deny_globs under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_glob_honors_deny_globs_for_explicit_path()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("blocked.txt"), "blocked\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect glob deny globs"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked.txt"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_glob",
            serde_json::json!({ "pattern": "blocked.txt" }),
            None,
        )
        .await
        .expect_err("expected file_glob to be denied for explicit denied path under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_grep_honors_deny_globs_for_explicit_include()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(project_dir.join("blocked.txt"), "blocked content\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect grep deny globs"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked.txt"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_grep",
            serde_json::json!({ "query": "blocked", "include_glob": "blocked.txt" }),
            None,
        )
        .await
        .expect_err("expected file_grep to be denied for explicit denied include_glob under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_glob_honors_deny_globs_for_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "a\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect glob prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_glob",
            serde_json::json!({ "pattern": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected file_glob to be denied for denied glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_file_grep_honors_deny_globs_for_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "blocked text\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect grep prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "file_grep",
            serde_json::json!({ "query": "blocked", "include_glob": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected file_grep to be denied for denied include_glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_repo_search_honors_deny_globs_for_include_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "blocked text\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect repo search prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "repo_search",
            serde_json::json!({ "query": "blocked", "include_glob": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected repo_search to be denied for denied include_glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_repo_index_honors_deny_globs_for_include_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.txt"), "hello\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect repo index prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "repo_index",
            serde_json::json!({ "include_glob": "blocked/**/*.txt" }),
            None,
        )
        .await
        .expect_err("expected repo_index to be denied for denied include_glob prefix under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_repo_symbols_honors_deny_globs_for_include_glob_prefix()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(project_dir.join(".omne_data/spec")).await?;
        tokio::fs::create_dir_all(project_dir.join("blocked/sub")).await?;
        tokio::fs::write(project_dir.join("blocked/sub/a.rs"), "fn blocked() {}\n").await?;
        tokio::fs::write(
            project_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  architect:
    description: "plan architect repo symbols prefix deny"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
        deny_globs: ["blocked/**"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "repo_symbols",
            serde_json::json!({ "include_glob": "blocked/**/*.rs" }),
            None,
        )
        .await
        .expect_err(
            "expected repo_symbols to be denied for denied include_glob prefix under /plan",
        );

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_state_blocks_cross_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "thread_state",
            serde_json::json!({ "thread_id": thread_b.to_string() }),
            None,
        )
        .await
        .expect_err("expected thread_state to be denied for cross-thread target under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_state_allows_same_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "thread_state",
            serde_json::json!({ "thread_id": thread_id.to_string() }),
            None,
        )
        .await?;

        let expected = thread_id.to_string();
        assert_eq!(result["thread_id"].as_str(), Some(expected.as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_usage_blocks_cross_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "thread_usage",
            serde_json::json!({ "thread_id": thread_b.to_string() }),
            None,
        )
        .await
        .expect_err("expected thread_usage to be denied for cross-thread target under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_usage_allows_same_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "thread_usage",
            serde_json::json!({ "thread_id": thread_id.to_string() }),
            None,
        )
        .await?;

        let expected = thread_id.to_string();
        assert_eq!(result["thread_id"].as_str(), Some(expected.as_str()));
        assert!(result["total_tokens_used"].as_u64().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_events_blocks_cross_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "thread_events",
            serde_json::json!({
                "thread_id": thread_b.to_string(),
                "since_seq": 0
            }),
            None,
        )
        .await
        .expect_err("expected thread_events to be denied for cross-thread target under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_thread_events_allows_same_thread_reads() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "thread_events",
            serde_json::json!({
                "thread_id": thread_id.to_string(),
                "since_seq": 0
            }),
            None,
        )
        .await?;

        assert!(result["events"].as_array().is_some());
        assert!(result["last_seq"].as_u64().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_inspect_blocks_cross_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_b, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "process_inspect",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "max_lines": 10
            }),
            None,
        )
        .await
        .expect_err("expected process_inspect to be denied for cross-thread process under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_inspect_allows_same_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "process_inspect",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "max_lines": 10
            }),
            None,
        )
        .await?;

        let expected = process_id.to_string();
        assert_eq!(
            result["process"]["process_id"].as_str(),
            Some(expected.as_str())
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_tail_blocks_cross_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_b, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "process_tail",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "max_lines": 10
            }),
            None,
        )
        .await
        .expect_err("expected process_tail to be denied for cross-thread process under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_tail_allows_same_thread_process() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "process_tail",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "max_lines": 10
            }),
            None,
        )
        .await?;

        assert!(result.get("text").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_follow_blocks_cross_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_a = tmp.path().join("project-a");
        let project_b = tmp.path().join("project-b");
        tokio::fs::create_dir_all(&project_a).await?;
        tokio::fs::create_dir_all(&project_b).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle_a = server.thread_store.create_thread(project_a).await?;
        let thread_a = handle_a.thread_id();
        drop(handle_a);
        let handle_b = server.thread_store.create_thread(project_b).await?;
        let thread_b = handle_b.thread_id();
        drop(handle_b);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_b, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_a, turn_id).await?;

        let err = run_tool_call_once(
            &server,
            thread_a,
            Some(turn_id),
            "process_follow",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "since_offset": 0,
                "max_bytes": 128
            }),
            None,
        )
        .await
        .expect_err("expected process_follow to be denied for cross-thread process under /plan");

        assert!(
            err.to_string()
                .contains("tool blocked by /plan architect mode gate")
        );
        Ok(())
    }

    #[tokio::test]
    async fn plan_directive_architect_process_follow_allows_same_thread_process()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(&project).await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let turn_id = TurnId::new();
        append_plan_turn_started(&server, thread_id, turn_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            Some(turn_id),
            "process_follow",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "since_offset": 0,
                "max_bytes": 128
            }),
            None,
        )
        .await?;

        assert_eq!(result["next_offset"].as_u64(), Some(0));
        assert_eq!(result["eof"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_inspect_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny inspect"
    permissions:
      process:
        inspect: { decision: allow }
    tool_overrides:
      - tool: "process/inspect"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "process_inspect",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "max_lines": 10
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_tail_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny tail"
    permissions:
      process:
        inspect: { decision: allow }
    tool_overrides:
      - tool: "process/tail"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "process_tail",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "max_lines": 10
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn process_follow_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny follow"
    permissions:
      process:
        inspect: { decision: allow }
    tool_overrides:
      - tool: "process/follow"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let process_id = omne_protocol::ProcessId::new();
        append_process_started(&server, thread_id, process_id).await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "process_follow",
            serde_json::json!({
                "process_id": process_id.to_string(),
                "stream": "stdout",
                "since_offset": 0,
                "max_bytes": 128
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_read_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(project.join("note.txt"), "hello\n").await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny file read"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/read"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({
                "path": "note.txt"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(project.join("note.txt"), "hello\n").await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny file glob"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/glob"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({
                "pattern": "*.txt"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_grep_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(project.join("note.txt"), "hello\n").await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny file grep"
    permissions:
      read: { decision: allow }
    tool_overrides:
      - tool: "file/grep"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_grep",
            serde_json::json!({
                "query": "hello",
                "include_glob": "*.txt"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_write_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact write"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/write"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_write",
            serde_json::json!({
                "artifact_type": "test",
                "summary": "s",
                "text": "hello"
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_list_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact list"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/list"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_list",
            serde_json::json!({}),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact read"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/read"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let artifact_id = omne_protocol::ArtifactId::new();
        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_read",
            serde_json::json!({
                "artifact_id": artifact_id.to_string()
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_delete_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let project = tmp.path().join("project");
        tokio::fs::create_dir_all(project.join(".omne_data/spec")).await?;
        tokio::fs::write(
            project.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder override deny artifact delete"
    permissions:
      artifact: { decision: allow }
    tool_overrides:
      - tool: "artifact/delete"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join("omne_root"));
        let handle = server.thread_store.create_thread(project).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_thread_mode(&server, thread_id, "coder").await?;

        let artifact_id = omne_protocol::ArtifactId::new();
        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "artifact_delete",
            serde_json::json!({
                "artifact_id": artifact_id.to_string()
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }
}
