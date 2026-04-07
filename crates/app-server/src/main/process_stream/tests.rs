#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn process_logs_rotate_and_follow_reads_across_segments() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let base_path = dir.path().join("stdout.log");

        let payload = "0123456789abcdefghijXXXXX".to_string();
        let payload_bytes = payload.clone().into_bytes();
        let payload_bytes_for_task = payload_bytes.clone();

        let (mut writer, reader) = tokio::io::duplex(64);
        let write_task = tokio::spawn(async move {
            writer.write_all(&payload_bytes_for_task).await?;
            writer.shutdown().await?;
            anyhow::Ok(())
        });

        capture_rotating_log(reader, base_path.clone(), 10).await?;
        write_task.await??;

        let part1 = dir.path().join("stdout.segment-0001.log");
        let part2 = dir.path().join("stdout.segment-0002.log");

        assert_eq!(tokio::fs::metadata(&part1).await?.len(), 10);
        assert_eq!(tokio::fs::metadata(&part2).await?.len(), 10);
        assert_eq!(tokio::fs::metadata(&base_path).await?.len(), 5);

        let (text, next_offset, eof) = read_file_chunk(base_path.clone(), 0, 1024).await?;
        assert_eq!(text, payload);
        assert_eq!(next_offset, payload_bytes.len() as u64);
        assert!(eof);

        Ok(())
    }

    #[tokio::test]
    async fn process_tail_success_returns_typed_response() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        let started = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec![
                    "sh".to_string(),
                    "-lc".to_string(),
                    "printf hello".to_string(),
                ],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;
        let process_id: ProcessId = serde_json::from_value(started["process_id"].clone())
            .map_err(|err| anyhow::anyhow!("missing process_id in process/start response: {err}"))?;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = handle_process_tail(
            &server,
            ProcessTailParams {
                process_id,
                turn_id: None,
                approval_id: None,
                stream: ProcessStream::Stdout,
                max_lines: Some(20),
            },
        )
        .await?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessTailResponse>(result)?;
        assert!(!parsed.tool_id.to_string().is_empty());
        assert!(parsed.text.contains("hello"));

        Ok(())
    }

    #[tokio::test]
    async fn process_tail_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
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
                allowed_tools: None,
                execpolicy_rules: None,
            clear_execpolicy_rules: false,
            },
        )
        .await?;

        let started = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 0.1".to_string()],
                cwd: None,
                timeout_ms: None,
            },
        )
        .await?;
        let process_id: ProcessId = serde_json::from_value(started["process_id"].clone())
            .map_err(|err| anyhow::anyhow!("missing process_id in process/start response: {err}"))?;

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

        let result = handle_process_tail(
            &server,
            ProcessTailParams {
                process_id,
                turn_id: None,
                approval_id: None,
                stream: ProcessStream::Stdout,
                max_lines: Some(20),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("process/tail"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn process_list_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(build_test_server_shared(repo_dir.join(".omne_data")));
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

        let response = handle_process_request(
            &server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({
                "thread_id": thread_id,
            }),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessAllowedToolsDeniedResponse>(result)?;
        assert!(parsed.denied);
        assert_eq!(parsed.tool, "process/list");
        assert_eq!(parsed.error_code.as_deref(), Some("allowed_tools_denied"));
        assert_eq!(parsed.allowed_tools, vec!["repo/search".to_string()]);
        Ok(())
    }

    #[tokio::test]
    async fn process_list_without_thread_id_hides_restricted_threads() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_a = tmp.path().join("repo-a");
        let repo_b = tmp.path().join("repo-b");
        tokio::fs::create_dir_all(&repo_a).await?;
        tokio::fs::create_dir_all(&repo_b).await?;

        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let visible_thread = create_test_thread_shared(&server, repo_a).await?;
        let hidden_thread = create_test_thread_shared(&server, repo_b).await?;

        let visible_process = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id: visible_thread,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 1".to_string()],
                cwd: None,
                timeout_ms: Some(500),
            },
        )
        .await?;
        let visible_process_id: ProcessId =
            serde_json::from_value(visible_process["process_id"].clone())
                .map_err(|err| anyhow::anyhow!("missing visible process_id: {err}"))?;

        let hidden_process = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id: hidden_thread,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 1".to_string()],
                cwd: None,
                timeout_ms: Some(500),
            },
        )
        .await?;
        let hidden_process_id: ProcessId =
            serde_json::from_value(hidden_process["process_id"].clone())
                .map_err(|err| anyhow::anyhow!("missing hidden process_id: {err}"))?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id: hidden_thread,
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

        let response = handle_process_request(
            &server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({}),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessListResponse>(result)?;
        let process_ids = parsed
            .processes
            .into_iter()
            .map(|process| process.process_id)
            .collect::<Vec<_>>();

        assert!(process_ids.contains(&visible_process_id));
        assert!(!process_ids.contains(&hidden_process_id));
        Ok(())
    }

    #[tokio::test]
    async fn process_list_with_thread_id_requires_mode_approval() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  process-list-prompt:
    description: "prompt before inspecting processes"
    permissions:
      read: { decision: allow }
      edit: { decision: deny }
      command: { decision: allow }
      process:
        inspect: { decision: prompt }
        kill: { decision: deny }
        interact: { decision: deny }
      artifact: { decision: allow }
      browser: { decision: deny }
"#,
        )
        .await?;

        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("process-list-prompt".to_string()),
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

        let response = handle_process_request(
            &server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({
                "thread_id": thread_id,
            }),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed =
            serde_json::from_value::<omne_app_server_protocol::ProcessNeedsApprovalResponse>(
                result,
            )?;
        assert!(parsed.needs_approval);
        assert_eq!(parsed.thread_id, thread_id);
        Ok(())
    }

    #[tokio::test]
    async fn process_list_without_thread_id_hides_mode_denied_threads() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_a = tmp.path().join("repo-a");
        let repo_b = tmp.path().join("repo-b");
        tokio::fs::create_dir_all(&repo_a).await?;
        tokio::fs::create_dir_all(&repo_b).await?;

        write_modes_yaml_shared(
            &repo_b,
            r#"
version: 1
modes:
  no-process-inspect:
    description: "deny process inspection"
    permissions:
      read: { decision: allow }
      edit: { decision: deny }
      command: { decision: allow }
      process:
        inspect: { decision: deny }
        kill: { decision: deny }
        interact: { decision: deny }
      artifact: { decision: allow }
      browser: { decision: deny }
"#,
        )
        .await?;

        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let visible_thread = create_test_thread_shared(&server, repo_a).await?;
        let hidden_thread = create_test_thread_shared(&server, repo_b.clone()).await?;

        let visible_process = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id: visible_thread,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 1".to_string()],
                cwd: None,
                timeout_ms: Some(500),
            },
        )
        .await?;
        let visible_process_id: ProcessId =
            serde_json::from_value(visible_process["process_id"].clone())
                .map_err(|err| anyhow::anyhow!("missing visible process_id: {err}"))?;

        let hidden_process = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id: hidden_thread,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 1".to_string()],
                cwd: None,
                timeout_ms: Some(500),
            },
        )
        .await?;
        let hidden_process_id: ProcessId =
            serde_json::from_value(hidden_process["process_id"].clone())
                .map_err(|err| anyhow::anyhow!("missing hidden process_id: {err}"))?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id: hidden_thread,
                approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("no-process-inspect".to_string()),
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

        let response = handle_process_request(
            &server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({}),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessListResponse>(result)?;
        let process_ids = parsed
            .processes
            .into_iter()
            .map(|process| process.process_id)
            .collect::<Vec<_>>();

        assert!(process_ids.contains(&visible_process_id));
        assert!(!process_ids.contains(&hidden_process_id));
        Ok(())
    }

    #[tokio::test]
    async fn process_list_with_thread_id_respects_role_permission_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;
        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                role: Some("chat".to_string()),
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

        let response = handle_process_request(
            &server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({
                "thread_id": thread_id,
            }),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed =
            serde_json::from_value::<omne_app_server_protocol::ProcessModeDeniedResponse>(result)?;
        assert!(parsed.denied);
        assert_eq!(parsed.thread_id, thread_id);
        assert_eq!(parsed.decision, omne_app_server_protocol::ProcessModeDecision::Deny);
        assert_eq!(parsed.decision_source, "role_permission_mode");
        Ok(())
    }

    #[tokio::test]
    async fn process_list_without_thread_id_hides_role_denied_threads() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_a = tmp.path().join("repo-a");
        let repo_b = tmp.path().join("repo-b");
        tokio::fs::create_dir_all(&repo_a).await?;
        tokio::fs::create_dir_all(&repo_b).await?;

        let server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let visible_thread = create_test_thread_shared(&server, repo_a).await?;
        let hidden_thread = create_test_thread_shared(&server, repo_b).await?;

        let visible_process = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id: visible_thread,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 1".to_string()],
                cwd: None,
                timeout_ms: Some(500),
            },
        )
        .await?;
        let visible_process_id: ProcessId =
            serde_json::from_value(visible_process["process_id"].clone())
                .map_err(|err| anyhow::anyhow!("missing visible process_id: {err}"))?;

        let hidden_process = handle_process_start(
            &server,
            ProcessStartParams {
                thread_id: hidden_thread,
                turn_id: None,
                approval_id: None,
                argv: vec!["sh".to_string(), "-lc".to_string(), "sleep 1".to_string()],
                cwd: None,
                timeout_ms: Some(500),
            },
        )
        .await?;
        let hidden_process_id: ProcessId =
            serde_json::from_value(hidden_process["process_id"].clone())
                .map_err(|err| anyhow::anyhow!("missing hidden process_id: {err}"))?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id: hidden_thread,
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                role: Some("chat".to_string()),
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

        let response = handle_process_request(
            &server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({}),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessListResponse>(result)?;
        let process_ids = parsed
            .processes
            .into_iter()
            .map(|process| process.process_id)
            .collect::<Vec<_>>();

        assert!(process_ids.contains(&visible_process_id));
        assert!(!process_ids.contains(&hidden_process_id));
        Ok(())
    }

    #[tokio::test]
    async fn process_list_with_thread_id_does_not_resume_cold_thread() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let seed_server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let mut handle = seed_server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        let turn_id = TurnId::new();
        handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "still running".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(handle);

        let before_events = seed_server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread should exist"))?;
        assert_eq!(before_events.len(), 2);

        let cold_server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let response = handle_process_request(
            &cold_server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({
                "thread_id": thread_id,
            }),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);

        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessListResponse>(result)?;
        assert!(parsed.processes.is_empty());

        let after_events = cold_server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread should exist"))?;
        assert_eq!(after_events.len(), before_events.len());
        assert!(matches!(
            after_events.last().map(|event| &event.kind),
            Some(omne_protocol::ThreadEventKind::TurnStarted { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn process_list_without_thread_id_does_not_resume_cold_thread() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let seed_server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let mut handle = seed_server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        let turn_id = TurnId::new();
        handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: "still running".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(handle);

        let before_events = seed_server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread should exist"))?;
        assert_eq!(before_events.len(), 2);

        let cold_server = Arc::new(build_test_server_shared(tmp.path().join(".omne_data")));
        let response = handle_process_request(
            &cold_server,
            serde_json::json!(1),
            "process/list",
            serde_json::json!({}),
        )
        .await;
        assert!(response.error.is_none(), "unexpected error: {:?}", response.error);

        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing process/list result"))?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessListResponse>(result)?;
        assert!(parsed.processes.is_empty());

        let after_events = cold_server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread should exist"))?;
        assert_eq!(after_events.len(), before_events.len());
        assert!(matches!(
            after_events.last().map(|event| &event.kind),
            Some(omne_protocol::ThreadEventKind::TurnStarted { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn process_inspect_redacts_sensitive_argv_in_response() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir).await?;
        let process_id = ProcessId::new();
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(1);
        let now = "2026-01-01T00:00:00Z".to_string();
        let stdout_path = tmp.path().join("stdout.log");
        let stderr_path = tmp.path().join("stderr.log");

        server.processes.lock().await.insert(
            process_id,
            ProcessEntry {
                thread_id,
                info: Arc::new(tokio::sync::Mutex::new(ProcessInfo {
                    process_id,
                    thread_id,
                    turn_id: None,
                    os_pid: Some(7),
                    argv: vec![
                        "tool".to_string(),
                        "--api-key".to_string(),
                        "super-secret".to_string(),
                    ],
                    cwd: "/tmp/repo".to_string(),
                    started_at: now.clone(),
                    status: ProcessStatus::Running,
                    exit_code: None,
                    stdout_path: stdout_path.display().to_string(),
                    stderr_path: stderr_path.display().to_string(),
                    last_update_at: now,
                })),
                cmd_tx,
                completion: ProcessCompletion::new(),
            },
        );

        tokio::fs::write(&stdout_path, b"stdout").await?;
        tokio::fs::write(&stderr_path, b"stderr").await?;

        let result = handle_process_inspect(
            &server,
            ProcessInspectParams {
                process_id,
                turn_id: None,
                approval_id: None,
                max_lines: Some(20),
            },
        )
        .await?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ProcessInspectResponse>(result)?;

        assert_eq!(
            parsed.process.argv,
            vec![
                "tool".to_string(),
                "--api-key".to_string(),
                "<REDACTED>".to_string(),
            ]
        );

        Ok(())
    }
}
