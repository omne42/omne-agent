#[cfg(test)]
mod thread_manage_tests {
    use super::*;

    fn thread_configure_defaults(thread_id: ThreadId) -> ThreadConfigureParams {
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
            allowed_tools: None,
            execpolicy_rules: None,
        }
    }

    #[tokio::test]
    async fn thread_state_includes_cache_token_aggregates() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        rt.append_event(omne_protocol::ThreadEventKind::AgentStep {
            turn_id: omne_protocol::TurnId::new(),
            step: 1,
            model: "gpt-5".to_string(),
            response_id: "resp_1".to_string(),
            text: Some("step".to_string()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            token_usage: Some(serde_json::json!({
                "total_tokens": 100,
                "input_tokens": 70,
                "output_tokens": 30,
                "cache_input_tokens": 55,
                "cache_creation_input_tokens": 9
            })),
            warnings_count: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "final".to_string(),
            model: Some("gpt-5".to_string()),
            response_id: Some("resp_1".to_string()),
            token_usage: Some(serde_json::json!({
                "total_tokens": 100,
                "input_tokens": 70,
                "output_tokens": 30,
                "cache_input_tokens": 55,
                "cache_creation_input_tokens": 9
            })),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "final2".to_string(),
            model: Some("gpt-5".to_string()),
            response_id: Some("resp_2".to_string()),
            token_usage: Some(serde_json::json!({
                "input_tokens": 20,
                "output_tokens": 10,
                "cache_input_tokens": 7
            })),
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/state",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/state result"))?;
        let state: omne_app_server_protocol::ThreadStateResponse =
            serde_json::from_value(result).context("parse thread/state response")?;

        assert_eq!(state.total_tokens_used, 130);
        assert_eq!(state.input_tokens_used, 90);
        assert_eq!(state.output_tokens_used, 40);
        assert_eq!(state.cache_input_tokens_used, 62);
        assert_eq!(state.cache_creation_input_tokens_used, 9);

        let usage_response = handle_thread_request(
            &server,
            serde_json::json!(2),
            "thread/usage",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await;
        assert!(usage_response.error.is_none());
        let usage_value = usage_response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/usage result"))?;
        let usage: omne_app_server_protocol::ThreadUsageResponse =
            serde_json::from_value(usage_value).context("parse thread/usage response")?;
        assert_eq!(usage.thread_id, thread_id);
        assert_eq!(usage.total_tokens_used, 130);
        assert_eq!(usage.input_tokens_used, 90);
        assert_eq!(usage.output_tokens_used, 40);
        assert_eq!(usage.cache_input_tokens_used, 62);
        assert_eq!(usage.cache_creation_input_tokens_used, 9);
        assert_eq!(usage.non_cache_input_tokens_used, 28);
        assert_eq!(
            usage.cache_input_ratio,
            Some(62.0 / 90.0),
            "cache_input_ratio should be cache/input"
        );
        assert_eq!(
            usage.output_ratio,
            Some(40.0 / 130.0),
            "output_ratio should be output/total"
        );
        assert_eq!(
            state.token_budget_limit, usage.token_budget_limit,
            "thread/state and thread/usage should share token_budget_limit"
        );
        assert_eq!(
            state.token_budget_remaining, usage.token_budget_remaining,
            "thread/state and thread/usage should share token_budget_remaining"
        );
        assert_eq!(
            state.token_budget_utilization, usage.token_budget_utilization,
            "thread/state and thread/usage should share token_budget_utilization"
        );
        assert_eq!(
            state.token_budget_exceeded, usage.token_budget_exceeded,
            "thread/state and thread/usage should share token_budget_exceeded"
        );
        assert_eq!(
            state.token_budget_warning_active, usage.token_budget_warning_active,
            "thread/state and thread/usage should share token_budget_warning_active"
        );
        assert_eq!(
            state.current_context_tokens_estimate, usage.current_context_tokens_estimate,
            "thread/state and thread/usage should share current_context_tokens_estimate"
        );
        assert!(
            state.current_context_tokens_estimate.is_some(),
            "thread/state should expose current_context_tokens_estimate"
        );
        if let Some(limit) = usage.token_budget_limit {
            assert_eq!(
                usage.token_budget_remaining,
                Some(limit.saturating_sub(130)),
                "token_budget_remaining should be derived from budget limit"
            );
            assert_eq!(
                usage.token_budget_utilization,
                Some(130.0 / limit as f64),
                "token_budget_utilization should be used/limit"
            );
            assert_eq!(
                usage.token_budget_exceeded,
                Some(130 > limit),
                "token_budget_exceeded should reflect used>limit"
            );
            assert_eq!(
                usage.token_budget_warning_active,
                Some(130 <= limit && (130.0 / limit as f64) >= token_budget_warning_threshold_ratio()),
                "token_budget_warning_active should reflect threshold and exceeded state"
            );
        } else {
            assert_eq!(usage.token_budget_remaining, None);
            assert_eq!(usage.token_budget_utilization, None);
            assert_eq!(usage.token_budget_exceeded, None);
            assert_eq!(usage.token_budget_warning_active, None);
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_usage_attention_and_list_meta_share_token_budget_snapshot_contract()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        rt.append_event(omne_protocol::ThreadEventKind::AgentStep {
            turn_id: omne_protocol::TurnId::new(),
            step: 1,
            model: "gpt-5".to_string(),
            response_id: "resp_1".to_string(),
            text: Some("step".to_string()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
            token_usage: Some(serde_json::json!({
                "total_tokens": 120,
                "input_tokens": 80,
                "output_tokens": 40,
                "cache_input_tokens": 50,
                "cache_creation_input_tokens": 8
            })),
            warnings_count: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: None,
            text: "final".to_string(),
            model: Some("gpt-5".to_string()),
            response_id: Some("resp_1".to_string()),
            token_usage: Some(serde_json::json!({
                "total_tokens": 120,
                "input_tokens": 80,
                "output_tokens": 40,
                "cache_input_tokens": 50,
                "cache_creation_input_tokens": 8
            })),
        })
        .await?;

        let usage_response = handle_thread_request(
            &server,
            serde_json::json!(11),
            "thread/usage",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await;
        assert!(usage_response.error.is_none());
        let usage: omne_app_server_protocol::ThreadUsageResponse = serde_json::from_value(
            usage_response
                .result
                .ok_or_else(|| anyhow::anyhow!("missing thread/usage result"))?,
        )
        .context("parse thread/usage response")?;

        let attention_response = handle_thread_request(
            &server,
            serde_json::json!(12),
            "thread/attention",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await;
        assert!(attention_response.error.is_none());
        let attention: omne_app_server_protocol::ThreadAttentionResponse = serde_json::from_value(
            attention_response
                .result
                .ok_or_else(|| anyhow::anyhow!("missing thread/attention result"))?,
        )
        .context("parse thread/attention response")?;

        let list_meta_response = handle_thread_request(
            &server,
            serde_json::json!(13),
            "thread/list_meta",
            serde_json::json!({
                "include_archived": true,
                "include_attention_markers": false
            }),
        )
        .await;
        assert!(list_meta_response.error.is_none());
        let list_meta: omne_app_server_protocol::ThreadListMetaResponse = serde_json::from_value(
            list_meta_response
                .result
                .ok_or_else(|| anyhow::anyhow!("missing thread/list_meta result"))?,
        )
        .context("parse thread/list_meta response")?;
        let list_meta_item = list_meta
            .threads
            .iter()
            .find(|item| item.thread_id == thread_id)
            .ok_or_else(|| anyhow::anyhow!("thread/list_meta row not found"))?;

        assert_eq!(attention.token_budget_limit, usage.token_budget_limit);
        assert_eq!(attention.token_budget_remaining, usage.token_budget_remaining);
        assert_eq!(attention.token_budget_utilization, usage.token_budget_utilization);
        assert_eq!(attention.token_budget_exceeded, usage.token_budget_exceeded);
        assert_eq!(
            attention.token_budget_warning_active,
            usage.token_budget_warning_active
        );

        assert_eq!(list_meta_item.token_budget_limit, usage.token_budget_limit);
        assert_eq!(
            list_meta_item.token_budget_remaining,
            usage.token_budget_remaining
        );
        assert_eq!(
            list_meta_item.token_budget_utilization,
            usage.token_budget_utilization
        );
        assert_eq!(list_meta_item.token_budget_exceeded, usage.token_budget_exceeded);
        assert_eq!(
            list_meta_item.token_budget_warning_active,
            usage.token_budget_warning_active
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_skips_when_config_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(response) => response,
            other => anyhow::bail!("expected ThreadHookRunResponse::Ok, got {other:?}"),
        };
        assert!(result.skipped);
        assert_eq!(result.hook, "setup");
        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_starts_process_from_config() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;

        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(response) => response,
            other => anyhow::bail!("expected ThreadHookRunResponse::Ok, got {other:?}"),
        };
        assert!(result.ok);
        assert_eq!(result.hook, "setup");
        let process_id = result
            .process_id
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let status = {
                let entry = {
                    let processes = server.processes.lock().await;
                    processes
                        .get(&process_id)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("missing process entry"))?
                };
                let info = entry.info.lock().await;
                info.status.clone()
            };

            if matches!(status, ProcessStatus::Exited) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("process did not exit in time");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                sandbox_policy: Some(policy_meta::WriteScope::ReadOnly),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadHookRunDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.hook, "setup");
        assert_eq!(result.error_code.as_deref(), Some("sandbox_policy_denied"));
        assert!(result.config_path.is_some());
        let denied = match &result.detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(detail) => detail.denied,
            omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail) => {
                detail.denied
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => detail.denied,
            omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail) => {
                detail.denied
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxPolicyDenied(detail) => {
                detail.denied
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxNetworkDenied(detail) => {
                detail.denied
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail) => {
                detail.denied
            }
            omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail) => {
                detail.denied
            }
        };
        assert!(denied);

        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_allowed_tools_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadHookRunDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.hook, "setup");
        assert_eq!(result.error_code.as_deref(), Some("allowed_tools_denied"));
        assert!(result.config_path.is_some());
        match result.detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail) => {
                assert!(detail.denied);
                assert_eq!(detail.tool, "process/start");
                assert_eq!(detail.allowed_tools, vec!["repo/search".to_string()]);
            }
            other => anyhow::bail!("expected process allowed_tools denied detail, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_mode_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  hook-mode-deny:
    description: "deny command permission"
    permissions:
      command:
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("hook-mode-deny".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadHookRunDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.hook, "setup");
        assert_eq!(result.error_code.as_deref(), Some("mode_denied"));
        assert!(result.config_path.is_some());
        match result.detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "hook-mode-deny");
                assert_eq!(
                    detail.decision,
                    omne_app_server_protocol::ProcessModeDecision::Deny
                );
                assert_eq!(detail.decision_source, "mode_permission");
                assert!(!detail.tool_override_hit);
            }
            other => anyhow::bail!("expected process mode denied detail, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_unknown_mode_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  hook-mode:
    description: "allow setup hook"
    permissions:
      read:
        decision: allow
      edit:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("hook-mode".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  other-mode:
    description: "placeholder"
    permissions:
      read:
        decision: allow
      edit:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
"#,
        )
        .await?;

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadHookRunDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.hook, "setup");
        assert_eq!(result.error_code.as_deref(), Some("mode_unknown"));
        assert!(result.config_path.is_some());
        match result.detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "hook-mode");
                assert!(detail.available.contains("other-mode"));
            }
            other => anyhow::bail!("expected process unknown_mode denied detail, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_execpolicy_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::create_dir_all(repo_dir.join("rules")).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["git", "status"]
"#,
        )
        .await?;
        tokio::fs::write(
            repo_dir.join("rules/thread.rules"),
            r#"
prefix_rule(
    pattern = ["git"],
    decision = "forbidden",
)
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                execpolicy_rules: Some(vec!["rules/thread.rules".to_string()]),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadHookRunDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.hook, "setup");
        assert_eq!(result.error_code.as_deref(), Some("execpolicy_denied"));
        assert!(result.config_path.is_some());
        match result.detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail) => {
                assert!(detail.denied);
                assert_eq!(
                    detail.decision,
                    omne_app_server_protocol::ExecPolicyDecision::Forbidden
                );
                assert!(!detail.matched_rules.is_empty());
            }
            other => anyhow::bail!("expected process execpolicy denied detail, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_execpolicy_load_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["git", "status"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                execpolicy_rules: Some(vec!["rules/missing.rules".to_string()]),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_hook_run(
            &server,
            ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                hook: WorkspaceHookName::Setup,
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadHookRunRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadHookRunDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.hook, "setup");
        assert_eq!(result.error_code.as_deref(), Some("execpolicy_load_denied"));
        assert!(result.config_path.is_some());
        match result.detail {
            omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail) => {
                assert!(detail.denied);
                assert_eq!(detail.error, "failed to load thread execpolicy rules");
                assert!(!detail.details.trim().is_empty());
            }
            other => anyhow::bail!("expected process execpolicy load denied detail, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_start_runs_setup_hook_automatically() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/start",
            serde_json::json!({ "cwd": repo_dir.display().to_string() }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/start result"))?;
        assert!(result["thread_id"].is_string());
        assert_eq!(result["auto_hook"]["hook"].as_str().unwrap_or(""), "setup");
        assert!(result["auto_hook"]["ok"].as_bool().unwrap_or(false));
        assert!(result["auto_hook"]["process_id"].is_string());

        Ok(())
    }

    #[tokio::test]
    async fn thread_start_auto_setup_hook_mode_denied_returns_typed_auto_hook()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;
        tokio::fs::write(
            config_dir.join("modes.yaml"),
            r#"
version: 1
modes:
  code:
    description: "deny commands in default mode"
    permissions:
      command:
        decision: deny
"#,
        )
        .await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/start",
            serde_json::json!({ "cwd": repo_dir.display().to_string() }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/start result"))?;
        let result = serde_json::from_value::<omne_app_server_protocol::ThreadStartResponse>(result)?;

        match result.auto_hook {
            omne_app_server_protocol::ThreadAutoHookResponse::Denied(denied) => {
                assert!(denied.denied);
                assert_eq!(denied.hook, "setup");
                assert_eq!(denied.error_code.as_deref(), Some("mode_denied"));
                assert!(denied.config_path.is_some());
                match denied.detail {
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
                        assert!(detail.denied);
                        assert_eq!(detail.mode, "code");
                        assert_eq!(
                            detail.decision,
                            omne_app_server_protocol::ProcessModeDecision::Deny
                        );
                    }
                    other => anyhow::bail!(
                        "expected ThreadProcessDeniedDetail::ModeDenied, got {other:?}"
                    ),
                }
            }
            other => anyhow::bail!("expected ThreadAutoHookResponse::Denied, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_fork_skips_active_turn_approval_request_and_decision() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;

        let completed_turn_id = TurnId::new();
        let completed_approval_id = omne_protocol::ApprovalId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id: completed_turn_id,
            input: "completed turn".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: completed_approval_id,
            turn_id: Some(completed_turn_id),
            action: "process/start".to_string(),
            params: serde_json::json!({ "argv": ["echo", "ok"] }),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
            approval_id: completed_approval_id,
            decision: omne_protocol::ApprovalDecision::Approved,
            remember: false,
            reason: Some("approved".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id: completed_turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let active_turn_id = TurnId::new();
        let active_approval_id = omne_protocol::ApprovalId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id: active_turn_id,
            input: "active turn".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: active_approval_id,
            turn_id: Some(active_turn_id),
            action: "process/start".to_string(),
            params: serde_json::json!({ "argv": ["echo", "active"] }),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
            approval_id: active_approval_id,
            decision: omne_protocol::ApprovalDecision::Approved,
            remember: false,
            reason: Some("approved active".to_string()),
        })
        .await?;

        let forked = handle_thread_fork(
            &server,
            ThreadForkParams {
                thread_id,
                cwd: None,
            },
        )
        .await?;
        let forked_thread_id = forked.thread_id;

        let forked_events = server
            .thread_store
            .read_events_since(forked_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("forked thread not found: {forked_thread_id}"))?;

        let mut requested = std::collections::HashSet::<omne_protocol::ApprovalId>::new();
        let mut decided = std::collections::HashSet::<omne_protocol::ApprovalId>::new();
        for event in forked_events {
            match event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested { approval_id, .. } => {
                    requested.insert(approval_id);
                }
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                    decided.insert(approval_id);
                }
                _ => {}
            }
        }

        assert!(requested.contains(&completed_approval_id));
        assert!(decided.contains(&completed_approval_id));
        assert!(!requested.contains(&active_approval_id));
        assert!(!decided.contains(&active_approval_id));
        Ok(())
    }

    #[tokio::test]
    async fn thread_fork_keeps_turnless_approval_when_active_turn_exists() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let active_turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id: active_turn_id,
            input: "active turn".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;

        let turnless_approval_id = omne_protocol::ApprovalId::new();
        rt.append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id: turnless_approval_id,
            turn_id: None,
            action: "process/start".to_string(),
            params: serde_json::json!({ "argv": ["echo", "turnless"] }),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
            approval_id: turnless_approval_id,
            decision: omne_protocol::ApprovalDecision::Approved,
            remember: false,
            reason: Some("approved turnless".to_string()),
        })
        .await?;

        let forked = handle_thread_fork(
            &server,
            ThreadForkParams {
                thread_id,
                cwd: None,
            },
        )
        .await?;
        let forked_thread_id = forked.thread_id;

        let forked_events = server
            .thread_store
            .read_events_since(forked_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("forked thread not found: {forked_thread_id}"))?;

        let mut requested = std::collections::HashSet::<omne_protocol::ApprovalId>::new();
        let mut decided = std::collections::HashSet::<omne_protocol::ApprovalId>::new();
        for event in forked_events {
            match event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested { approval_id, .. } => {
                    requested.insert(approval_id);
                }
                omne_protocol::ThreadEventKind::ApprovalDecided { approval_id, .. } => {
                    decided.insert(approval_id);
                }
                _ => {}
            }
        }

        assert!(requested.contains(&turnless_approval_id));
        assert!(decided.contains(&turnless_approval_id));
        Ok(())
    }

    #[tokio::test]
    async fn thread_fork_preserves_system_prompt_snapshot() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let expected_hash = "snapshot-hash".to_string();
        let expected_text = "# frozen system prompt".to_string();
        let rt = server.get_or_load_thread(thread_id).await?;
        rt.append_event(omne_protocol::ThreadEventKind::ThreadSystemPromptSnapshot {
            prompt_sha256: expected_hash.clone(),
            prompt_text: expected_text.clone(),
            source: Some("test".to_string()),
        })
        .await?;

        let forked = handle_thread_fork(
            &server,
            ThreadForkParams {
                thread_id,
                cwd: None,
            },
        )
        .await?;
        let forked_thread_id = forked.thread_id;

        let forked_rt = server.get_or_load_thread(forked_thread_id).await?;
        let (fork_hash, fork_text) = {
            let handle = forked_rt.handle.lock().await;
            let state = handle.state();
            (
                state.system_prompt_sha256.clone(),
                state.system_prompt_text.clone(),
            )
        };
        assert_eq!(fork_hash.as_deref(), Some(expected_hash.as_str()));
        assert_eq!(fork_text.as_deref(), Some(expected_text.as_str()));

        let forked_events = server
            .thread_store
            .read_events_since(forked_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("forked thread not found: {forked_thread_id}"))?;
        let snapshots = forked_events
            .iter()
            .filter_map(|event| match &event.kind {
                omne_protocol::ThreadEventKind::ThreadSystemPromptSnapshot {
                    prompt_sha256,
                    prompt_text,
                    ..
                } => Some((prompt_sha256.clone(), prompt_text.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(snapshots, vec![(expected_hash, expected_text)]);

        Ok(())
    }

    #[tokio::test]
    async fn thread_events_supports_kind_filtering() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "events filter".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            artifact_id: Some(ArtifactId::new()),
            artifact_type: Some("plan".to_string()),
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            reason: Some("new turn started".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/events result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing events"))?;
        assert_eq!(events.len(), 2);
        let kinds = events
            .iter()
            .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["attention_marker_set", "attention_marker_cleared"]
        );
        assert_eq!(result["last_seq"].as_u64(), Some(4));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(5));
        Ok(())
    }

    #[tokio::test]
    async fn thread_events_kind_filter_includes_token_budget_markers() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "events token budget markers".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/events result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing events"))?;
        assert_eq!(events.len(), 4);

        let kinds = events
            .iter()
            .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "attention_marker_set",
                "attention_marker_cleared",
                "attention_marker_set",
                "attention_marker_cleared"
            ]
        );

        let markers = events
            .iter()
            .filter_map(|event| event.get("marker").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            markers,
            vec![
                "token_budget_warning",
                "token_budget_warning",
                "token_budget_exceeded",
                "token_budget_exceeded"
            ]
        );
        assert_eq!(
            events[1]["reason"].as_str(),
            Some("token budget warning cleared")
        );
        assert_eq!(
            events[3]["reason"].as_str(),
            Some("token budget exceeded cleared")
        );
        assert_eq!(result["last_seq"].as_u64(), Some(6));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(7));
        Ok(())
    }

    #[tokio::test]
    async fn thread_events_kind_filter_normalizes_case_and_whitespace() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "events normalization".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            artifact_id: Some(ArtifactId::new()),
            artifact_type: Some("plan".to_string()),
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            reason: Some("new turn started".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "kinds": [" ATTENTION_MARKER_SET ", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/events result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing events"))?;
        assert_eq!(events.len(), 2);
        let kinds = events
            .iter()
            .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["attention_marker_set", "attention_marker_cleared"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_events_kind_filter_respects_max_events_and_has_more() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "events filter max".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            artifact_id: Some(ArtifactId::new()),
            artifact_type: Some("plan".to_string()),
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            reason: Some("new turn started".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "max_events": 1,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/events result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing events"))?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"].as_str(), Some("attention_marker_set"));
        assert_eq!(result["has_more"].as_bool(), Some(true));
        assert_eq!(result["last_seq"].as_u64(), Some(3));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(5));
        Ok(())
    }

    #[tokio::test]
    async fn thread_events_token_budget_kind_filter_respects_max_events_and_has_more()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "events token budget filter max".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "max_events": 2,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/events result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing events"))?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"].as_str(), Some("attention_marker_set"));
        assert_eq!(events[0]["marker"].as_str(), Some("token_budget_warning"));
        assert_eq!(events[1]["type"].as_str(), Some("attention_marker_cleared"));
        assert_eq!(events[1]["marker"].as_str(), Some("token_budget_warning"));
        assert_eq!(
            events[1]["reason"].as_str(),
            Some("token budget warning cleared")
        );
        assert_eq!(result["has_more"].as_bool(), Some(true));
        assert_eq!(result["last_seq"].as_u64(), Some(4));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(7));
        Ok(())
    }

    #[tokio::test]
    async fn thread_events_token_budget_kind_filter_since_seq_resume_skips_warning_events()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "events token budget filter since_seq".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let first_response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(first_response.error.is_none());
        let first_result = first_response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing first thread/events result"))?;
        let first_events = first_result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing first events"))?;
        assert_eq!(first_events.len(), 4);

        let warning_clear_seq = first_events
            .iter()
            .find_map(|event| {
                let is_warning_clear = event["type"].as_str() == Some("attention_marker_cleared")
                    && event["marker"].as_str() == Some("token_budget_warning");
                if is_warning_clear {
                    event["seq"].as_u64()
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("missing warning-clear marker seq"))?;

        let resume_response = handle_thread_request(
            &server,
            serde_json::json!(2),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": warning_clear_seq,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(resume_response.error.is_none());
        let resume_result = resume_response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing resume thread/events result"))?;
        let resumed_events = resume_result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing resume events"))?;
        assert_eq!(resumed_events.len(), 2);
        assert_eq!(resumed_events[0]["type"].as_str(), Some("attention_marker_set"));
        assert_eq!(resumed_events[0]["marker"].as_str(), Some("token_budget_exceeded"));
        assert_eq!(
            resumed_events[1]["type"].as_str(),
            Some("attention_marker_cleared")
        );
        assert_eq!(
            resumed_events[1]["marker"].as_str(),
            Some("token_budget_exceeded")
        );
        assert_eq!(
            resumed_events[1]["reason"].as_str(),
            Some("token budget exceeded cleared")
        );
        assert!(
            resumed_events.iter().all(|event| {
                event["seq"]
                    .as_u64()
                    .is_some_and(|seq| seq > warning_clear_seq)
            }),
            "resumed marker events should all be strictly newer than warning-clear seq"
        );
        assert_eq!(resume_result["has_more"].as_bool(), Some(false));

        let resume_last_seq = resume_result["last_seq"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing resume last_seq"))?;
        let resume_thread_last_seq = resume_result["thread_last_seq"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing resume thread_last_seq"))?;
        let first_thread_last_seq = first_result["thread_last_seq"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing first thread_last_seq"))?;
        let resumed_last_event_seq = resumed_events
            .last()
            .and_then(|event| event["seq"].as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing resumed last event seq"))?;
        assert_eq!(resume_last_seq, resumed_last_event_seq);
        assert_eq!(resume_thread_last_seq, first_thread_last_seq);

        Ok(())
    }

    #[tokio::test]
    async fn thread_subscribe_since_seq_resume_skips_warning_marker_events() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "subscribe token budget since_seq".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let first_response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "wait_ms": 0,
            }),
        )
        .await;
        assert!(first_response.error.is_none());
        let first_result = first_response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing first thread/subscribe result"))?;
        let first_events = first_result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing first subscribe events"))?;
        assert!(
            !first_events.is_empty(),
            "first subscribe response should contain events"
        );
        let warning_clear_seq = first_events
            .iter()
            .find_map(|event| {
                let is_warning_clear = event["type"].as_str() == Some("attention_marker_cleared")
                    && event["marker"].as_str() == Some("token_budget_warning");
                if is_warning_clear {
                    event["seq"].as_u64()
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("missing warning-clear marker seq"))?;
        let first_thread_last_seq = first_result["thread_last_seq"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing first thread_last_seq"))?;

        let resume_response = handle_thread_request(
            &server,
            serde_json::json!(2),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": warning_clear_seq,
                "wait_ms": 0,
            }),
        )
        .await;
        assert!(resume_response.error.is_none());
        let resume_result = resume_response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing resume thread/subscribe result"))?;
        let resumed_events = resume_result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing resumed subscribe events"))?;
        assert!(
            resumed_events
                .iter()
                .all(|event| event["seq"].as_u64().is_some_and(|seq| seq > warning_clear_seq)),
            "resumed subscribe events should all be strictly newer than warning-clear seq"
        );
        assert!(
            resumed_events
                .iter()
                .any(|event| event["type"].as_str() == Some("turn_completed")),
            "resume should include non-marker events newer than since_seq"
        );
        assert!(
            resumed_events
                .iter()
                .all(|event| event["marker"].as_str() != Some("token_budget_warning")),
            "warning marker events should not reappear after warning-clear seq"
        );

        let resumed_exceeded_markers = resumed_events
            .iter()
            .filter_map(|event| {
                let marker = event["marker"].as_str()?;
                if marker == "token_budget_exceeded" {
                    event["type"].as_str().map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            resumed_exceeded_markers,
            vec![
                "attention_marker_set".to_string(),
                "attention_marker_cleared".to_string()
            ]
        );

        assert_eq!(resume_result["has_more"].as_bool(), Some(false));
        assert_eq!(resume_result["timed_out"].as_bool(), Some(false));
        let resume_last_seq = resume_result["last_seq"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing resume last_seq"))?;
        let resumed_last_event_seq = resumed_events
            .last()
            .and_then(|event| event["seq"].as_u64())
            .ok_or_else(|| anyhow::anyhow!("missing resumed last event seq"))?;
        assert_eq!(resume_last_seq, resumed_last_event_seq);
        assert_eq!(
            resume_result["thread_last_seq"].as_u64(),
            Some(first_thread_last_seq)
        );

        Ok(())
    }

    #[tokio::test]
    async fn thread_subscribe_kind_filter_includes_token_budget_markers() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "subscribe token budget markers".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "wait_ms": 0,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/subscribe result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing subscribe events"))?;
        assert_eq!(events.len(), 4);
        let kinds = events
            .iter()
            .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "attention_marker_set",
                "attention_marker_cleared",
                "attention_marker_set",
                "attention_marker_cleared"
            ]
        );
        let markers = events
            .iter()
            .filter_map(|event| event.get("marker").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            markers,
            vec![
                "token_budget_warning",
                "token_budget_warning",
                "token_budget_exceeded",
                "token_budget_exceeded"
            ]
        );
        assert_eq!(
            events[1]["reason"].as_str(),
            Some("token budget warning cleared")
        );
        assert_eq!(
            events[3]["reason"].as_str(),
            Some("token budget exceeded cleared")
        );
        assert_eq!(result["last_seq"].as_u64(), Some(6));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(7));
        assert_eq!(result["has_more"].as_bool(), Some(false));
        assert_eq!(result["timed_out"].as_bool(), Some(false));
        Ok(())
    }

    #[tokio::test]
    async fn thread_subscribe_kind_filter_normalizes_case_and_whitespace() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "subscribe normalization".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            artifact_id: Some(ArtifactId::new()),
            artifact_type: Some("plan".to_string()),
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::PlanReady,
            turn_id: Some(turn_id),
            reason: Some("new turn started".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "wait_ms": 0,
                "kinds": [" ATTENTION_MARKER_SET ", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/subscribe result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing subscribe events"))?;
        assert_eq!(events.len(), 2);
        let kinds = events
            .iter()
            .filter_map(|event| event.get("type").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec!["attention_marker_set", "attention_marker_cleared"]
        );
        assert_eq!(result["thread_last_seq"].as_u64(), Some(5));
        Ok(())
    }

    #[tokio::test]
    async fn thread_subscribe_kind_filter_respects_max_events_and_has_more()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "subscribe token budget max events".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "max_events": 2,
                "wait_ms": 0,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/subscribe result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing subscribe events"))?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"].as_str(), Some("attention_marker_set"));
        assert_eq!(events[0]["marker"].as_str(), Some("token_budget_warning"));
        assert_eq!(events[1]["type"].as_str(), Some("attention_marker_cleared"));
        assert_eq!(events[1]["marker"].as_str(), Some("token_budget_warning"));
        assert_eq!(result["has_more"].as_bool(), Some(true));
        assert_eq!(result["last_seq"].as_u64(), Some(4));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(7));
        assert_eq!(result["timed_out"].as_bool(), Some(false));
        Ok(())
    }

    #[tokio::test]
    async fn thread_subscribe_kind_filter_since_seq_respects_max_events() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "subscribe token budget since+max".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetWarning,
            turn_id: Some(turn_id),
            reason: Some("token budget warning cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            artifact_id: None,
            artifact_type: None,
            process_id: None,
            exit_code: None,
            command: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker: omne_protocol::AttentionMarkerKind::TokenBudgetExceeded,
            turn_id: Some(turn_id),
            reason: Some("token budget exceeded cleared".to_string()),
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 4,
                "max_events": 1,
                "wait_ms": 0,
                "kinds": ["attention_marker_set", "attention_marker_cleared"],
            }),
        )
        .await;
        assert!(response.error.is_none());
        let result = response
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/subscribe result"))?;
        let events = result["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing subscribe events"))?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["seq"].as_u64(), Some(5));
        assert_eq!(events[0]["type"].as_str(), Some("attention_marker_set"));
        assert_eq!(events[0]["marker"].as_str(), Some("token_budget_exceeded"));
        assert_eq!(result["has_more"].as_bool(), Some(true));
        assert_eq!(result["last_seq"].as_u64(), Some(5));
        assert_eq!(result["thread_last_seq"].as_u64(), Some(7));
        assert_eq!(result["timed_out"].as_bool(), Some(false));
        Ok(())
    }

    #[tokio::test]
    async fn thread_subscribe_rejects_unknown_kind_filter_values() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/subscribe",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "wait_ms": 0,
                "kinds": ["turn_started", "not_a_real_event_kind"],
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/subscribe to reject unknown kinds"))?;
        assert_eq!(err.code, JSONRPC_INVALID_PARAMS);
        assert_eq!(err.message, "invalid params");
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected error data for invalid kinds"))?;
        let msg = data["error"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("expected error message"))?;
        assert!(msg.contains("not_a_real_event_kind"));
        let supported = data["supported_kinds"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("expected supported_kinds list"))?;
        assert!(!supported.is_empty());
        assert!(
            supported
                .iter()
                .any(|kind| kind.as_str() == Some("attention_marker_set"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_events_rejects_unknown_kind_filter_values() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": 0,
                "kinds": ["turn_started", "not_a_real_event_kind"],
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/events to reject unknown kinds"))?;
        assert_eq!(err.code, JSONRPC_INVALID_PARAMS);
        assert_eq!(err.message, "invalid params");
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected error data for invalid kinds"))?;
        let msg = data["error"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("expected error message"))?;
        assert!(msg.contains("not_a_real_event_kind"));
        let supported = data["supported_kinds"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("expected supported_kinds list"))?;
        assert!(!supported.is_empty());
        assert!(
            supported
                .iter()
                .any(|kind| kind.as_str() == Some("attention_marker_set"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_list_and_loaded_param_contracts() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        let thread_id_string = thread_id.to_string();
        let runtime = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
        server.threads.lock().await.insert(thread_id, runtime);

        let list_null = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/list",
            serde_json::Value::Null,
        )
        .await;
        assert!(list_null.error.is_none(), "thread/list should accept null params");
        let list_null_result = list_null
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/list result for null params"))?;
        let list_null_threads = list_null_result["threads"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing threads array for thread/list null params"))?;
        assert!(list_null_threads
            .iter()
            .any(|id| id.as_str() == Some(thread_id_string.as_str())));

        let list_empty = handle_thread_request(
            &server,
            serde_json::json!(2),
            "thread/list",
            serde_json::json!({}),
        )
        .await;
        assert!(
            list_empty.error.is_none(),
            "thread/list should accept empty object params"
        );

        let loaded_null = handle_thread_request(
            &server,
            serde_json::json!(3),
            "thread/loaded",
            serde_json::Value::Null,
        )
        .await;
        assert!(
            loaded_null.error.is_none(),
            "thread/loaded should accept null params"
        );
        let loaded_null_result = loaded_null
            .result
            .ok_or_else(|| anyhow::anyhow!("missing thread/loaded result for null params"))?;
        let loaded_null_threads = loaded_null_result["threads"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing threads array for thread/loaded null params"))?;
        assert!(loaded_null_threads
            .iter()
            .any(|id| id.as_str() == Some(thread_id_string.as_str())));

        let loaded_empty = handle_thread_request(
            &server,
            serde_json::json!(4),
            "thread/loaded",
            serde_json::json!({}),
        )
        .await;
        assert!(
            loaded_empty.error.is_none(),
            "thread/loaded should accept empty object params"
        );

        for (method, id) in [("thread/list", 5), ("thread/loaded", 6)] {
            let invalid = handle_thread_request(
                &server,
                serde_json::json!(id),
                method,
                serde_json::json!("invalid"),
            )
            .await;
            let err = invalid
                .error
                .ok_or_else(|| anyhow::anyhow!("expected invalid params error for {method}"))?;
            assert_eq!(err.code, JSONRPC_INVALID_PARAMS);
            assert_eq!(err.message, "invalid params");
        }

        Ok(())
    }

    #[tokio::test]
    async fn thread_archive_runs_archive_hook_automatically() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  archive: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
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
        let auto_hook = result
            .auto_hook
            .ok_or_else(|| anyhow::anyhow!("missing auto_hook"))?;
        let auto_hook = match auto_hook {
            omne_app_server_protocol::ThreadAutoHookResponse::Ok(response) => response,
            other => anyhow::bail!("expected ThreadAutoHookResponse::Ok, got {other:?}"),
        };
        assert_eq!(auto_hook.hook, "archive");
        assert!(auto_hook.ok);
        assert!(auto_hook.process_id.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn thread_archive_auto_hook_mode_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  archive: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;
        tokio::fs::write(
            config_dir.join("modes.yaml"),
            r#"
version: 1
modes:
  archive-deny:
    description: "deny command for archive hook"
    permissions:
      command:
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("archive-deny".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

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
        let auto_hook = result
            .auto_hook
            .ok_or_else(|| anyhow::anyhow!("missing auto_hook"))?;
        match auto_hook {
            omne_app_server_protocol::ThreadAutoHookResponse::Denied(denied) => {
                assert!(denied.denied);
                assert_eq!(denied.hook, "archive");
                assert_eq!(denied.error_code.as_deref(), Some("mode_denied"));
                assert!(denied.config_path.is_some());
                match denied.detail {
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
                        assert!(detail.denied);
                        assert_eq!(detail.mode, "archive-deny");
                        assert_eq!(
                            detail.decision,
                            omne_app_server_protocol::ProcessModeDecision::Deny
                        );
                    }
                    other => anyhow::bail!(
                        "expected ThreadProcessDeniedDetail::ModeDenied, got {other:?}"
                    ),
                }
            }
            other => anyhow::bail!("expected ThreadAutoHookResponse::Denied, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_create_list_restore_roundtrip() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;
        tokio::fs::write(
            repo_dir.join(".env"),
            "SECRET=sk-should-not-be-snapshotted\n",
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let created = handle_thread_checkpoint_create(
            &server,
            ThreadCheckpointCreateParams {
                thread_id,
                label: Some("before changes".to_string()),
            },
        )
        .await?;
        let created = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointCreateResponse,
        >(created)?;
        let checkpoint_id = created.checkpoint_id;

        let listed =
            handle_thread_checkpoint_list(&server, ThreadCheckpointListParams { thread_id })
                .await?;
        let listed = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointListResponse,
        >(listed)?;
        assert_eq!(listed.checkpoints.len(), 1);
        assert_eq!(listed.checkpoints[0].checkpoint_id, checkpoint_id);

        tokio::fs::write(repo_dir.join("foo.txt"), "v2\n").await?;
        tokio::fs::write(repo_dir.join("bar.txt"), "new file\n").await?;

        let first_restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let first_restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse,
        >(first_restore)?;
        assert!(first_restore.needs_approval);
        assert_eq!(first_restore.plan.create, 0);
        assert_eq!(first_restore.plan.delete, 1);
        let approval_id = first_restore.approval_id;

        handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id,
                approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: None,
            },
        )
        .await?;

        let second_restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: Some(approval_id),
            },
        )
        .await?;
        let second_restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreResponse,
        >(second_restore)?;
        assert!(second_restore.restored);
        assert_eq!(second_restore.plan.create, 0);
        assert_eq!(second_restore.plan.delete, 1);
        assert!(second_restore.duration_ms <= 60_000);

        assert_eq!(
            tokio::fs::read_to_string(repo_dir.join("foo.txt")).await?,
            "v1\n"
        );
        assert!(!repo_dir.join("bar.txt").exists());
        assert_eq!(
            tokio::fs::read_to_string(repo_dir.join(".env")).await?,
            "SECRET=sk-should-not-be-snapshotted\n"
        );

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        assert!(events.iter().any(|e| matches!(e.kind, omne_protocol::ThreadEventKind::CheckpointCreated { checkpoint_id: got, .. } if got == checkpoint_id)));
        assert!(events.iter().any(|e| matches!(e.kind, omne_protocol::ThreadEventKind::CheckpointRestored { checkpoint_id: got, status: omne_protocol::CheckpointRestoreStatus::Ok, .. } if got == checkpoint_id)));

        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_restore_denied_writes_rollback_report() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let created = handle_thread_checkpoint_create(
            &server,
            ThreadCheckpointCreateParams {
                thread_id,
                label: Some("before failure".to_string()),
            },
        )
        .await?;
        let created = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointCreateResponse,
        >(created)?;
        let checkpoint_id = created.checkpoint_id;

        let first_restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let first_restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse,
        >(first_restore)?;
        assert!(first_restore.needs_approval);
        assert_eq!(first_restore.plan.create, 0);
        assert_eq!(first_restore.plan.modify, 0);
        assert_eq!(first_restore.plan.delete, 0);
        let approval_id = first_restore.approval_id;

        handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id,
                approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: None,
            },
        )
        .await?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                sandbox_policy: Some(policy_meta::WriteScope::ReadOnly),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let restore_result = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: Some(approval_id),
            },
        )
        .await?;
        let restore_result = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse,
        >(restore_result)?;
        assert!(restore_result.denied);
        assert_eq!(
            restore_result.error_code.as_deref(),
            Some("sandbox_policy_denied")
        );
        assert_eq!(
            restore_result.sandbox_policy,
            Some(policy_meta::WriteScope::ReadOnly)
        );

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let report_artifact_id = events
            .iter()
            .rev()
            .find_map(|event| match &event.kind {
                omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: got,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    report_artifact_id: Some(report_artifact_id),
                    ..
                } if *got == checkpoint_id => Some(*report_artifact_id),
                _ => None,
            })
            .ok_or_else(|| {
                anyhow::anyhow!("missing failed CheckpointRestored.report_artifact_id")
            })?;

        let listed = handle_artifact_list(
            &server,
            ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let artifacts: Vec<ArtifactMetadata> = serde_json::from_value(listed["artifacts"].clone())?;
        assert!(artifacts.iter().any(|meta| {
            meta.artifact_id == report_artifact_id && meta.artifact_type == "rollback_report"
        }));
        assert!(artifacts.iter().any(|meta| {
            meta.artifact_id == report_artifact_id
                && meta.summary.contains("sandbox_policy=read_only")
        }));

        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_restore_approval_denied_reports_error_code() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let created = handle_thread_checkpoint_create(
            &server,
            ThreadCheckpointCreateParams {
                thread_id,
                label: Some("before approval denied".to_string()),
            },
        )
        .await?;
        let created = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointCreateResponse,
        >(created)?;
        let checkpoint_id = created.checkpoint_id;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoDeny),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let restore_result = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let restore_result = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse,
        >(restore_result)?;
        assert!(restore_result.denied);
        assert_eq!(restore_result.error_code.as_deref(), Some("approval_denied"));

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
        let report_artifact_id = events
            .iter()
            .rev()
            .find_map(|event| match &event.kind {
                omne_protocol::ThreadEventKind::CheckpointRestored {
                    checkpoint_id: got,
                    status: omne_protocol::CheckpointRestoreStatus::Failed,
                    reason: Some(reason),
                    report_artifact_id: Some(report_artifact_id),
                    ..
                } if *got == checkpoint_id && reason.contains("approval denied") => {
                    Some(*report_artifact_id)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing failed CheckpointRestored with report"))?;

        let listed = handle_artifact_list(
            &server,
            ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let artifacts: Vec<ArtifactMetadata> = serde_json::from_value(listed["artifacts"].clone())?;
        assert!(artifacts.iter().any(|meta| {
            meta.artifact_id == report_artifact_id
                && meta.artifact_type == "rollback_report"
                && meta.summary.contains("approval denied")
        }));

        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_restore_mode_deny_reports_typed_decision() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  checkpoint-deny:
    description: "deny checkpoint restore"
    permissions:
      edit:
        decision: deny
"#,
        )
        .await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let created = handle_thread_checkpoint_create(
            &server,
            ThreadCheckpointCreateParams {
                thread_id,
                label: Some("before deny".to_string()),
            },
        )
        .await?;
        let created = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointCreateResponse,
        >(created)?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("checkpoint-deny".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id: created.checkpoint_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse,
        >(restore)?;
        assert!(restore.denied);
        assert_eq!(restore.error_code.as_deref(), Some("mode_denied"));
        assert_eq!(restore.mode.as_deref(), Some("checkpoint-deny"));
        assert_eq!(
            restore.decision,
            Some(omne_app_server_protocol::ThreadCheckpointDecision::Deny)
        );
        assert!(restore.available.is_none());
        assert!(restore.load_error.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_restore_unknown_mode_reports_typed_decision() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  checkpoint-restore-mode:
    description: "allow checkpoint restore"
    permissions:
      edit:
        decision: allow
"#,
        )
        .await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let created = handle_thread_checkpoint_create(
            &server,
            ThreadCheckpointCreateParams {
                thread_id,
                label: Some("before unknown mode".to_string()),
            },
        )
        .await?;
        let created = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointCreateResponse,
        >(created)?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("checkpoint-restore-mode".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  other-mode:
    description: "placeholder"
    permissions:
      edit:
        decision: allow
"#,
        )
        .await?;

        let restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id: created.checkpoint_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse,
        >(restore)?;
        assert!(restore.denied);
        assert_eq!(restore.error_code.as_deref(), Some("mode_unknown"));
        assert_eq!(restore.mode.as_deref(), Some("checkpoint-restore-mode"));
        assert_eq!(
            restore.decision,
            Some(omne_app_server_protocol::ThreadCheckpointDecision::Deny)
        );
        assert!(restore.available.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_restore_with_writable_roots_reports_typed_denial() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let created = handle_thread_checkpoint_create(
            &server,
            ThreadCheckpointCreateParams {
                thread_id,
                label: Some("before writable roots".to_string()),
            },
        )
        .await?;
        let created = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointCreateResponse,
        >(created)?;
        let checkpoint_id = created.checkpoint_id;

        let first_restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let first_restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse,
        >(first_restore)?;
        assert!(first_restore.needs_approval);
        let approval_id = first_restore.approval_id;

        handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id,
                approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: None,
            },
        )
        .await?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                sandbox_writable_roots: Some(vec![".".to_string()]),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let restore = handle_thread_checkpoint_restore(
            &server,
            ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id: Some(approval_id),
            },
        )
        .await?;
        let restore = serde_json::from_value::<
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse,
        >(restore)?;
        assert!(restore.denied);
        assert_eq!(
            restore.error_code.as_deref(),
            Some("sandbox_writable_roots_unsupported")
        );
        let roots = restore
            .sandbox_writable_roots
            .ok_or_else(|| anyhow::anyhow!("missing sandbox_writable_roots"))?;
        assert_eq!(roots.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn thread_allowed_tools_denies_file_read() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "hello\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_file_read(
            &server,
            FileReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                path: "foo.txt".to_string(),
                max_bytes: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str().unwrap_or(""), "repo/search");
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_unknown_mode_includes_error_code_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "mode": "mode-does-not-exist"
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure unknown mode failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(data["error_code"].as_str(), Some("mode_unknown"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_unknown_role_includes_error_code_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "role": "role-does-not-exist"
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure unknown role failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(data["error_code"].as_str(), Some("role_unknown"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_custom_mode_name_as_role_is_rejected() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  role-custom:
    description: "custom role-like mode"
    permissions:
      read: { decision: allow }
      edit: { decision: allow }
      command: { decision: allow }
      artifact: { decision: allow }
"#,
        )
        .await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "role": "role-custom",
                "allowed_tools": ["process/start"]
            }),
        )
        .await;

        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure unknown role failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(data["error_code"].as_str(), Some("role_unknown"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_mode_update_does_not_implicitly_overwrite_role()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let set_role = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "role": "chatter"
            }),
        )
        .await;
        if let Some(err) = set_role.error {
            anyhow::bail!("set role failed unexpectedly: {err:?}");
        }

        let set_mode = handle_thread_request(
            &server,
            serde_json::json!(2),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "mode": "code"
            }),
        )
        .await;
        if let Some(err) = set_mode.error {
            anyhow::bail!("set mode failed unexpectedly: {err:?}");
        }

        let state_resp = handle_thread_request(
            &server,
            serde_json::json!(3),
            "thread/state",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await;
        if let Some(err) = state_resp.error {
            anyhow::bail!("thread/state failed unexpectedly: {err:?}");
        }
        let state: omne_app_server_protocol::ThreadStateResponse = serde_json::from_value(
            state_resp
                .result
                .ok_or_else(|| anyhow::anyhow!("missing thread/state result"))?,
        )?;

        assert_eq!(state.mode, "code");
        assert_eq!(state.role, "chatter");
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_unknown_allowed_tool_includes_error_code_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "mode": "coder",
                "allowed_tools": ["tool/does_not_exist"]
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure unknown tool failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(
            data["error_code"].as_str(),
            Some("allowed_tools_unknown_tool")
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_mode_denied_tool_includes_error_code_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "mode": "reviewer",
                "role": "reviewer",
                "allowed_tools": ["file/write"]
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure mode denied failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(data["error_code"].as_str(), Some("allowed_tools_mode_denied"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_allowed_tools_validates_mode_and_role_intersection()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "mode": "chat",
                "role": "coder",
                "allowed_tools": ["process/start"]
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure mode-role denied failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(data["error_code"].as_str(), Some("allowed_tools_mode_denied"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_invalid_thinking_includes_error_code_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "thinking": "ultra"
            }),
        )
        .await;
        let err = response
            .error
            .ok_or_else(|| anyhow::anyhow!("expected thread/configure invalid thinking failure"))?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(data["error_code"].as_str(), Some("thinking_invalid"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_configure_invalid_sandbox_root_includes_error_code_data()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = Arc::new(crate::build_test_server_shared(tmp.path().join(".omne_data")));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let response = handle_thread_request(
            &server,
            serde_json::json!(1),
            "thread/configure",
            serde_json::json!({
                "thread_id": thread_id,
                "sandbox_writable_roots": ["../escape"]
            }),
        )
        .await;
        let err = response.error.ok_or_else(|| {
            anyhow::anyhow!("expected thread/configure invalid sandbox root failure")
        })?;
        assert_eq!(err.code, JSONRPC_INTERNAL_ERROR);
        let data = err
            .data
            .ok_or_else(|| anyhow::anyhow!("expected json-rpc error data"))?;
        assert_eq!(
            data["error_code"].as_str(),
            Some("sandbox_writable_root_invalid")
        );
        Ok(())
    }

    #[tokio::test]
    async fn thread_config_explain_includes_execpolicy_rules() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                execpolicy_rules: Some(vec![
                    "rules/thread.rules".to_string(),
                    "rules/thread.rules".to_string(),
                    "  ".to_string(),
                ]),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let explain =
            handle_thread_config_explain(&server, ThreadConfigExplainParams { thread_id }).await?;
        let rules = explain.effective.execpolicy_rules;
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0], "rules/thread.rules");
        Ok(())
    }

    #[tokio::test]
    async fn thread_config_explain_includes_role_catalog_layer_for_builtin_role()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                role: Some("chatter".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let explain =
            handle_thread_config_explain(&server, ThreadConfigExplainParams { thread_id }).await?;
        let role_layer = explain
            .layers
            .iter()
            .find(|layer| layer.get("source").and_then(|v| v.as_str()) == Some("role_catalog"))
            .ok_or_else(|| anyhow::anyhow!("missing role_catalog layer"))?;

        assert_eq!(
            role_layer
                .get("effective_role")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "chatter"
        );
        assert_eq!(
            role_layer
                .get("permission_mode")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "chatter"
        );
        assert_eq!(
            role_layer
                .get("resolution_source")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "role_catalog"
        );
        let available_roles = role_layer
            .get("available_roles")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("missing available_roles"))?;
        assert!(available_roles
            .iter()
            .any(|v| v.as_str() == Some("chatter")));
        Ok(())
    }

    #[tokio::test]
    async fn thread_config_explain_role_unknown_falls_back_to_mode_only() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        rt.append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
            sandbox_policy: None,
            sandbox_writable_roots: None,
            sandbox_network_access: None,
            mode: Some("chat".to_string()),
            role: Some("legacy-role".to_string()),
            model: None,
            thinking: None,
            show_thinking: None,
            openai_base_url: None,
            allowed_tools: None,
            execpolicy_rules: None,
        })
        .await?;

        let explain =
            handle_thread_config_explain(&server, ThreadConfigExplainParams { thread_id }).await?;
        let role_layer = explain
            .layers
            .iter()
            .find(|layer| layer.get("source").and_then(|v| v.as_str()) == Some("role_catalog"))
            .ok_or_else(|| anyhow::anyhow!("missing role_catalog layer"))?;

        assert_eq!(
            role_layer
                .get("effective_role")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "legacy-role"
        );
        assert_eq!(
            role_layer
                .get("permission_mode")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "chat"
        );
        assert_eq!(
            role_layer
                .get("resolution_source")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "none"
        );
        assert_eq!(explain.effective.permission_mode, "chat");
        Ok(())
    }

    #[tokio::test]
    async fn thread_config_explain_includes_effective_permissions_intersection()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("chat".to_string()),
                role: Some("coder".to_string()),
                allowed_tools: Some(Some(vec!["file/read".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let explain =
            handle_thread_config_explain(&server, ThreadConfigExplainParams { thread_id }).await?;
        assert_eq!(explain.effective.permission_mode, "coder");
        let effective_permissions = explain
            .effective
            .effective_permissions
            .ok_or_else(|| anyhow::anyhow!("missing effective_permissions"))?;
        assert!(effective_permissions
            .iter()
            .any(|tool| tool == "file/read"));
        assert!(!effective_permissions
            .iter()
            .any(|tool| tool == "process/start"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_config_explain_includes_preset_layer() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let written = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "preset_applied".to_string(),
                summary: "preset applied: coder-default".to_string(),
                text: "{\"preset_name\":\"coder-default\"}".to_string(),
            },
        )
        .await?;
        let artifact_id: ArtifactId =
            serde_json::from_value(written["artifact_id"].clone()).context("parse artifact_id")?;

        let explain =
            handle_thread_config_explain(&server, ThreadConfigExplainParams { thread_id }).await?;
        let preset_layer = explain
            .layers
            .iter()
            .find(|layer| layer.get("source").and_then(|v| v.as_str()) == Some("preset"))
            .ok_or_else(|| anyhow::anyhow!("missing preset layer"))?;
        assert_eq!(
            preset_layer
                .get("artifact_id")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            artifact_id.to_string()
        );
        assert_eq!(
            preset_layer
                .get("artifact_type")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            "preset_applied"
        );
        assert!(
            preset_layer
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("preset applied")
        );
        Ok(())
    }
}
