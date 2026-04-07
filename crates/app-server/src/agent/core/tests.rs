#[cfg(test)]
mod tool_parallelism_tests {
    use super::*;

    #[test]
    fn parse_bool_value_accepts_common_values() {
        assert_eq!(parse_bool_value("1"), Some(true));
        assert_eq!(parse_bool_value("true"), Some(true));
        assert_eq!(parse_bool_value("YES"), Some(true));
        assert_eq!(parse_bool_value("on"), Some(true));

        assert_eq!(parse_bool_value("0"), Some(false));
        assert_eq!(parse_bool_value("false"), Some(false));
        assert_eq!(parse_bool_value("No"), Some(false));
        assert_eq!(parse_bool_value("off"), Some(false));

        assert_eq!(parse_bool_value("maybe"), None);
        assert_eq!(parse_bool_value(""), None);
    }

    #[test]
    fn usage_total_tokens_prefers_total_tokens() {
        let usage = serde_json::json!({
            "input_tokens": 10u64,
            "output_tokens": 5u64,
            "total_tokens": 20u64,
        });
        assert_eq!(usage_total_tokens(&usage), Some(20));
    }

    #[test]
    fn usage_total_tokens_falls_back_to_input_plus_output() {
        let usage = serde_json::json!({
            "input_tokens": 10u64,
            "output_tokens": 5u64,
        });
        assert_eq!(usage_total_tokens(&usage), Some(15));
    }

    #[test]
    fn token_usage_json_from_ditto_usage_includes_cache_fields() {
        let usage = ditto_core::contracts::Usage {
            input_tokens: Some(100),
            cache_input_tokens: Some(60),
            cache_creation_input_tokens: Some(8),
            output_tokens: Some(20),
            total_tokens: Some(120),
        };
        let json = token_usage_json_from_ditto_usage(&usage).expect("usage json");
        assert_eq!(
            json.get("input_tokens").and_then(serde_json::Value::as_u64),
            Some(100)
        );
        assert_eq!(
            json.get("cache_input_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(60)
        );
        assert_eq!(
            json.get("cache_creation_input_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(8)
        );
        assert_eq!(
            json.get("output_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(20)
        );
        assert_eq!(
            json.get("total_tokens").and_then(serde_json::Value::as_u64),
            Some(120)
        );
    }

    #[test]
    fn token_usage_json_from_ditto_usage_keeps_cache_only_usage() {
        let usage = ditto_core::contracts::Usage {
            input_tokens: None,
            cache_input_tokens: Some(42),
            cache_creation_input_tokens: None,
            output_tokens: None,
            total_tokens: None,
        };
        let json = token_usage_json_from_ditto_usage(&usage).expect("usage json");
        assert_eq!(
            json.get("cache_input_tokens")
                .and_then(serde_json::Value::as_u64),
            Some(42)
        );
    }

    #[test]
    fn tool_is_read_only_is_conservative() {
        assert!(tool_is_read_only("file_read"));
        assert!(tool_is_read_only("file_glob"));
        assert!(tool_is_read_only("file_grep"));
        assert!(tool_is_read_only("process_inspect"));
        assert!(tool_is_read_only("process_tail"));
        assert!(tool_is_read_only("process_follow"));
        assert!(tool_is_read_only("artifact_list"));
        assert!(tool_is_read_only("artifact_read"));
        assert!(tool_is_read_only("thread_state"));
        assert!(tool_is_read_only("thread_usage"));
        assert!(tool_is_read_only("thread_events"));

        assert!(!tool_is_read_only("file_write"));
        assert!(!tool_is_read_only("file_patch"));
        assert!(!tool_is_read_only("file_edit"));
        assert!(!tool_is_read_only("file_delete"));
        assert!(!tool_is_read_only("fs_mkdir"));
        assert!(!tool_is_read_only("process_start"));
        assert!(!tool_is_read_only("process_kill"));
        assert!(!tool_is_read_only("artifact_write"));
        assert!(!tool_is_read_only("artifact_delete"));
        assert!(!tool_is_read_only("thread_hook_run"));
        assert!(!tool_is_read_only("agent_spawn"));
    }

    #[test]
    fn plan_directive_forces_serial_tool_calls() {
        let (parallel, max_parallel) = apply_plan_parallel_tool_call_overrides(true, true, 8);
        assert!(!parallel);
        assert_eq!(max_parallel, 1);
    }

    #[test]
    fn no_plan_directive_keeps_parallel_settings() {
        let (parallel, max_parallel) = apply_plan_parallel_tool_call_overrides(false, true, 8);
        assert!(parallel);
        assert_eq!(max_parallel, 8);
    }

    #[test]
    fn summarize_plan_artifact_uses_first_non_empty_line() {
        let summary = summarize_plan_artifact("\n\n# Plan\n- step 1\n");
        assert_eq!(summary, "# Plan");
    }

    #[test]
    fn summarize_plan_artifact_falls_back_to_plan() {
        let summary = summarize_plan_artifact("   \n\n");
        assert_eq!(summary, "plan");
    }

    #[test]
    fn plan_directive_uses_architect_role_for_routing() {
        assert_eq!(resolve_turn_role_for_routing(true, "coder"), "architect");
    }

    #[test]
    fn non_plan_turn_keeps_thread_mode_for_routing() {
        assert_eq!(resolve_turn_role_for_routing(false, "reviewer"), "reviewer");
    }

    #[test]
    fn parse_role_input_directive_accepts_known_role_prefix() {
        let roles = omne_core::roles::RoleCatalog::builtin();
        let parsed =
            parse_role_input_directive("@{reviewer} focus on correctness", &roles).expect("parsed");
        assert_eq!(parsed.role_name, "reviewer");
        assert_eq!(parsed.content, "focus on correctness");
    }

    #[test]
    fn parse_role_input_directive_rejects_unknown_or_invalid_prefix() {
        let roles = omne_core::roles::RoleCatalog::builtin();
        assert!(parse_role_input_directive("@{missing} hi", &roles).is_none());
        assert!(parse_role_input_directive("@{reviewer}hi", &roles).is_none());
        assert!(parse_role_input_directive("@{reviewer}   ", &roles).is_none());
        assert!(parse_role_input_directive("plain input", &roles).is_none());
    }

    #[test]
    fn role_directive_handling_prioritizes_first_turn_then_compaction() {
        assert_eq!(
            resolve_role_directive_handling(true, true),
            RoleDirectiveHandling::InjectIntoSystem
        );
        assert_eq!(
            resolve_role_directive_handling(false, true),
            RoleDirectiveHandling::AutoCompactThenUser
        );
        assert_eq!(
            resolve_role_directive_handling(false, false),
            RoleDirectiveHandling::UserMessage
        );
    }

    #[test]
    fn replace_latest_user_message_text_updates_last_user_input_text() {
        let mut items = vec![
            serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "old-1" }]
            }),
            serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "ack" }]
            }),
            serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "old-2" }]
            }),
        ];
        assert!(replace_latest_user_message_text(&mut items, "new"));
        assert_eq!(
            items[2]
                .get("content")
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str),
            Some("new")
        );
    }
}

#[cfg(test)]
mod llm_retry_tests {
    use super::*;

    #[test]
    fn parse_csv_list_trims_and_dedupes() {
        assert_eq!(
            parse_csv_list(" openai-a, openai-b,openai-a,, ,openai-c "),
            vec![
                "openai-a".to_string(),
                "openai-b".to_string(),
                "openai-c".to_string()
            ]
        );
    }

    #[test]
    fn build_provider_candidates_keeps_primary_and_uniques() {
        assert_eq!(
            build_provider_candidates(
                "primary",
                vec![
                    "fallback-1".to_string(),
                    "primary".to_string(),
                    "fallback-1".to_string(),
                    "fallback-2".to_string(),
                ]
            ),
            vec![
                "primary".to_string(),
                "fallback-1".to_string(),
                "fallback-2".to_string()
            ]
        );
    }

    #[test]
    fn build_model_candidates_keeps_primary_and_uniques() {
        assert_eq!(
            build_model_candidates(
                "gpt-4.1-mini",
                vec![
                    "gpt-4.1".to_string(),
                    "gpt-4.1-mini".to_string(),
                    "gpt-4.1".to_string(),
                    "gpt-4.1".to_string(),
                ]
            ),
            vec!["gpt-4.1-mini".to_string(), "gpt-4.1".to_string()]
        );
    }

    #[test]
    fn llm_error_classification_is_conservative() {
        use reqwest::StatusCode;

        let rate_limited = LlmAttemptError::Ditto(ditto_core::error::DittoError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "rate limit".to_string(),
        });
        assert!(llm_error_is_retryable(&rate_limited));
        assert!(llm_error_prefers_provider_fallback(&rate_limited));

        let server_error = LlmAttemptError::Ditto(ditto_core::error::DittoError::Api {
            status: StatusCode::BAD_GATEWAY,
            body: "upstream".to_string(),
        });
        assert!(llm_error_is_retryable(&server_error));
        assert!(llm_error_prefers_provider_fallback(&server_error));

        let bad_request = LlmAttemptError::Ditto(ditto_core::error::DittoError::Api {
            status: StatusCode::BAD_REQUEST,
            body: "invalid".to_string(),
        });
        assert!(!llm_error_is_retryable(&bad_request));
        assert!(!llm_error_prefers_provider_fallback(&bad_request));
        assert!(llm_error_prefers_model_fallback(&bad_request));

        let unauthorized = LlmAttemptError::Ditto(ditto_core::error::DittoError::Api {
            status: StatusCode::UNAUTHORIZED,
            body: "auth".to_string(),
        });
        assert!(!llm_error_prefers_model_fallback(&unauthorized));

        let timed_out = LlmAttemptError::TimedOut;
        assert!(llm_error_is_retryable(&timed_out));
        assert!(llm_error_prefers_provider_fallback(&timed_out));
        assert!(!llm_error_prefers_model_fallback(&timed_out));
    }
}

#[cfg(test)]
mod system_prompt_snapshot_tests {
    use super::*;

    #[tokio::test]
    async fn persists_snapshot_once_and_reuses_it() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("AGENTS.md"), "# project prompt A\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let thread_cwd = repo_dir.display().to_string();

        let first =
            resolve_or_persist_thread_system_prompt_snapshot(&thread_rt, Some(&thread_cwd)).await?;
        let second =
            resolve_or_persist_thread_system_prompt_snapshot(&thread_rt, Some(&thread_cwd)).await?;
        assert_eq!(first, second);

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .unwrap_or_default();
        let snapshots = events
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

        assert_eq!(snapshots.len(), 1, "snapshot should only be persisted once");
        assert_eq!(snapshots[0].1, first);
        assert_eq!(snapshots[0].0, system_prompt_sha256(&first));
        Ok(())
    }

    #[tokio::test]
    async fn reuses_persisted_snapshot_after_project_instructions_change() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        let agents_path = repo_dir.join("AGENTS.md");
        tokio::fs::write(&agents_path, "# project prompt A\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let thread_cwd = repo_dir.display().to_string();

        let first =
            resolve_or_persist_thread_system_prompt_snapshot(&thread_rt, Some(&thread_cwd)).await?;
        tokio::fs::write(&agents_path, "# project prompt B\n").await?;

        let reused =
            resolve_or_persist_thread_system_prompt_snapshot(&thread_rt, Some(&thread_cwd)).await?;
        assert_eq!(reused, first);
        Ok(())
    }
}

#[cfg(test)]
mod attachment_upload_tests {
    use super::*;
    use async_trait::async_trait;

    #[derive(Clone)]
    struct DummyLanguageModel;

    #[async_trait]
    impl ditto_core::llm_core::model::LanguageModel for DummyLanguageModel {
        fn provider(&self) -> &str {
            "dummy"
        }

        fn model_id(&self) -> &str {
            "dummy-model"
        }

        async fn generate(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::contracts::GenerateResponse> {
            unimplemented!()
        }

        async fn stream(
            &self,
            _request: ditto_core::contracts::GenerateRequest,
        ) -> ditto_core::error::Result<ditto_core::llm_core::model::StreamResult> {
            unimplemented!()
        }
    }

    #[derive(Clone)]
    struct StubUploader {
        file_id: Option<String>,
    }

    #[async_trait]
    impl FileUploader for StubUploader {
        async fn upload_file(&self, _filename: String, _bytes: Vec<u8>) -> anyhow::Result<String> {
            match self.file_id.as_deref() {
                Some(file_id) => Ok(file_id.to_string()),
                None => Err(anyhow::anyhow!("upload failed")),
            }
        }
    }

    fn dummy_runtime(file_uploader: Option<Arc<dyn FileUploader>>) -> ProviderRuntime {
        ProviderRuntime {
            config: ditto_core::config::ProviderConfig {
                provider: None,
                enabled_capabilities: Vec::new(),
                base_url: Some("http://example.com/v1".to_string()),
                default_model: Some("gpt-4.1".to_string()),
                model_whitelist: Vec::new(),
                http_headers: Default::default(),
                http_query_params: Default::default(),
                auth: None,
                capabilities: Some(ditto_core::config::ProviderCapabilities::openai_responses()),
                upstream_api: None,
                normalize_to: None,
                normalize_endpoint: None,
                openai_compatible: None,
            },
            capabilities: ditto_core::config::ProviderCapabilities::openai_responses(),
            client: Arc::new(DummyLanguageModel),
            openai_responses_client: None,
            file_uploader,
        }
    }

    fn temp_pdf(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, bytes).expect("write temp pdf");
        (dir, path)
    }

    #[tokio::test]
    async fn pdf_upload_min_bytes_zero_never_uploads() {
        let (_dir, path) = temp_pdf(b"hello");
        let runtime = dummy_runtime(Some(Arc::new(StubUploader {
            file_id: Some("file_123".to_string()),
        })));
        let parts = attachments_to_ditto_parts_for_provider(
            ThreadId::new(),
            TurnId::new(),
            "provider",
            &runtime,
            &[ResolvedAttachment::FilePath {
                path: "a.pdf".to_string(),
                resolved: path,
                filename: Some("a.pdf".to_string()),
                media_type: "application/pdf".to_string(),
                size_bytes: 5,
            }],
            0,
        )
        .await
        .expect("build attachment parts");

        assert_eq!(parts.len(), 1);
        let ditto_core::contracts::ContentPart::File { source, .. } = &parts[0] else {
            panic!("expected file part");
        };
        assert!(matches!(source, ditto_core::contracts::FileSource::Base64 { .. }));
    }

    #[tokio::test]
    async fn pdf_upload_uses_file_id_when_above_threshold() {
        let (_dir, path) = temp_pdf(b"hello");
        let runtime = dummy_runtime(Some(Arc::new(StubUploader {
            file_id: Some("file_123".to_string()),
        })));
        let parts = attachments_to_ditto_parts_for_provider(
            ThreadId::new(),
            TurnId::new(),
            "provider",
            &runtime,
            &[ResolvedAttachment::FilePath {
                path: "a.pdf".to_string(),
                resolved: path,
                filename: Some("a.pdf".to_string()),
                media_type: "application/pdf".to_string(),
                size_bytes: 5,
            }],
            1,
        )
        .await
        .expect("build attachment parts");

        assert_eq!(parts.len(), 1);
        let ditto_core::contracts::ContentPart::File { source, .. } = &parts[0] else {
            panic!("expected file part");
        };
        assert!(matches!(
            source,
            ditto_core::contracts::FileSource::FileId { file_id } if file_id == "file_123"
        ));
    }

    #[tokio::test]
    async fn pdf_upload_falls_back_to_base64_on_error() {
        let (_dir, path) = temp_pdf(b"hello");
        let runtime = dummy_runtime(Some(Arc::new(StubUploader { file_id: None })));
        let parts = attachments_to_ditto_parts_for_provider(
            ThreadId::new(),
            TurnId::new(),
            "provider",
            &runtime,
            &[ResolvedAttachment::FilePath {
                path: "a.pdf".to_string(),
                resolved: path,
                filename: Some("a.pdf".to_string()),
                media_type: "application/pdf".to_string(),
                size_bytes: 5,
            }],
            1,
        )
        .await
        .expect("build attachment parts");

        assert_eq!(parts.len(), 1);
        let ditto_core::contracts::ContentPart::File { source, .. } = &parts[0] else {
            panic!("expected file part");
        };
        assert!(matches!(source, ditto_core::contracts::FileSource::Base64 { .. }));
    }
}

#[cfg(test)]
mod loop_detection_tests {
    use super::*;

    #[test]
    fn tool_call_signature_is_stable_for_object_key_order() {
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"b": 2, "a": 1});
        assert_eq!(
            tool_call_signature("file_read", &a),
            tool_call_signature("file_read", &b)
        );
    }

    #[test]
    fn loop_detector_trips_on_consecutive_calls() {
        let mut detector = LoopDetector::new();
        let sig = tool_call_signature("file_read", &serde_json::json!({"path": "a.txt"}));
        assert_eq!(detector.observe(sig), None);
        assert_eq!(detector.observe(sig), None);
        assert_eq!(detector.observe(sig), Some("consecutive"));
    }

    #[test]
    fn loop_detector_trips_on_short_cycle() {
        let mut detector = LoopDetector::new();
        let a = tool_call_signature("file_read", &serde_json::json!({"path": "a.txt"}));
        let b = tool_call_signature("file_read", &serde_json::json!({"path": "b.txt"}));

        assert_eq!(detector.observe(a), None);
        assert_eq!(detector.observe(b), None);
        assert_eq!(detector.observe(a), None);
        assert_eq!(detector.observe(b), Some("cycle"));
    }
}

#[cfg(test)]
mod provider_protocol_tests {
    use super::*;

    fn sample_provider_route_target() -> ProviderRouteTarget {
        ProviderRouteTarget {
            id: "google.providers.yunwu:test".to_string(),
            provider: "google.providers.yunwu".to_string(),
            model: "gemini-3.1-pro-preview".to_string(),
            model_fallbacks: Vec::new(),
            provider_config: ditto_core::config::ProviderConfig {
                provider: None,
                enabled_capabilities: Vec::new(),
                base_url: Some("https://yunwu.ai/v1beta".to_string()),
                default_model: Some("gemini-3.1-pro-preview".to_string()),
                model_whitelist: Vec::new(),
                http_headers: Default::default(),
                http_query_params: Default::default(),
                auth: Some(ditto_core::config::ProviderAuth::HttpHeaderEnv {
                    header: "Authorization".to_string(),
                    keys: vec!["YUNWU_API_KEY".to_string()],
                    prefix: Some("Bearer ".to_string()),
                }),
                capabilities: None,
                upstream_api: Some(ditto_core::config::ProviderApi::GeminiGenerateContent),
                normalize_to: Some(ditto_core::config::ProviderApi::OpenaiChatCompletions),
                normalize_endpoint: Some("/v1/chat/completions".to_string()),
                openai_compatible: None,
            },
        }
    }

    #[test]
    fn gemini_upstream_defaults_to_non_reasoning_capabilities() {
        let config = ditto_core::config::ProviderConfig {
            provider: None,
            enabled_capabilities: Vec::new(),
            base_url: Some("https://yunwu.ai/v1beta".to_string()),
            default_model: Some("gemini-3.1-pro-preview".to_string()),
            model_whitelist: Vec::new(),
            http_headers: Default::default(),
            http_query_params: Default::default(),
            auth: None,
            capabilities: None,
            upstream_api: Some(ditto_core::config::ProviderApi::GeminiGenerateContent),
            normalize_to: Some(ditto_core::config::ProviderApi::OpenaiChatCompletions),
            normalize_endpoint: Some("/v1/chat/completions".to_string()),
            openai_compatible: None,
        };
        let upstream = resolve_provider_upstream_api(&config);
        let caps = resolve_provider_capabilities(&config, upstream);
        assert!(caps.tools);
        assert!(caps.streaming);
        assert!(!caps.reasoning);
    }

    #[tokio::test]
    async fn build_provider_runtime_uses_google_client_for_gemini_upstream_api() {
        let target = sample_provider_route_target();
        let env = ditto_core::config::Env {
            dotenv: std::collections::BTreeMap::from([(
                "YUNWU_API_KEY".to_string(),
                "test-key".to_string(),
            )]),
        };

        let runtime = build_provider_runtime(&target, &env)
            .await
            .expect("build provider runtime");
        assert_eq!(
            runtime.config.upstream_api,
            Some(ditto_core::config::ProviderApi::GeminiGenerateContent)
        );
        assert_eq!(
            runtime.config.normalize_to,
            Some(ditto_core::config::ProviderApi::OpenaiChatCompletions)
        );
        assert!(!runtime.supports_openai_responses_codex_parity());
        assert!(runtime.file_uploader.is_none());
    }

    #[test]
    fn provider_runtime_cache_key_changes_when_dotenv_changes() {
        let target = sample_provider_route_target();
        let env_a = ditto_core::config::Env {
            dotenv: std::collections::BTreeMap::from([(
                "YUNWU_API_KEY".to_string(),
                "key-a".to_string(),
            )]),
        };
        let env_b = ditto_core::config::Env {
            dotenv: std::collections::BTreeMap::from([(
                "YUNWU_API_KEY".to_string(),
                "key-b".to_string(),
            )]),
        };

        assert_ne!(
            provider_runtime_cache_key(&target, &env_a),
            provider_runtime_cache_key(&target, &env_b)
        );
    }

    #[test]
    fn provider_runtime_cache_key_changes_when_auth_contract_changes() {
        let mut target_a = sample_provider_route_target();
        let mut target_b = sample_provider_route_target();
        target_b.provider_config.auth = Some(ditto_core::config::ProviderAuth::HttpHeaderEnv {
            header: "X-API-Key".to_string(),
            keys: vec!["YUNWU_API_KEY".to_string()],
            prefix: None,
        });
        let env = ditto_core::config::Env {
            dotenv: std::collections::BTreeMap::from([(
                "YUNWU_API_KEY".to_string(),
                "shared-key".to_string(),
            )]),
        };

        assert_ne!(
            provider_runtime_cache_key(&target_a, &env),
            provider_runtime_cache_key(&target_b, &env)
        );
        target_a.provider_config.auth = target_b.provider_config.auth.clone();
        assert_eq!(
            provider_runtime_cache_key(&target_a, &env),
            provider_runtime_cache_key(&target_b, &env)
        );
    }

    #[test]
    fn provider_runtime_cache_key_changes_when_http_metadata_changes() {
        let env = ditto_core::config::Env {
            dotenv: std::collections::BTreeMap::from([(
                "YUNWU_API_KEY".to_string(),
                "shared-key".to_string(),
            )]),
        };

        let mut target_a = sample_provider_route_target();
        target_a.provider_config.http_headers =
            std::collections::BTreeMap::from([("x-tenant".to_string(), "alpha".to_string())]);
        target_a.provider_config.http_query_params =
            std::collections::BTreeMap::from([("workspace".to_string(), "main".to_string())]);

        let mut target_b = target_a.clone();
        target_b.provider_config.http_headers =
            std::collections::BTreeMap::from([("x-tenant".to_string(), "beta".to_string())]);

        let mut target_c = target_a.clone();
        target_c.provider_config.http_query_params =
            std::collections::BTreeMap::from([("workspace".to_string(), "review".to_string())]);

        let base_key = provider_runtime_cache_key(&target_a, &env);
        assert_ne!(base_key, provider_runtime_cache_key(&target_b, &env));
        assert_ne!(base_key, provider_runtime_cache_key(&target_c, &env));
    }
}

#[cfg(test)]
mod auto_summary_tests {
    use super::*;

    use omne_protocol::TurnStatus;

    #[test]
    fn should_auto_compact_triggers_at_threshold() {
        assert!(!should_auto_compact(0, None));
        assert!(!should_auto_compact(79, Some(80)));
        assert!(should_auto_compact(80, Some(80)));
        assert!(should_auto_compact(100, Some(80)));
    }

    #[tokio::test]
    async fn build_conversation_prefers_latest_summary_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id1 = TurnId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: turn_id1,
                input: "first".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id1),
                text: "hello".to_string(),
                model: None,
                response_id: None,
                token_usage: None,
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: turn_id1,
                status: TurnStatus::Completed,
                reason: None,
            })
            .await?;

        let _summary = crate::handle_artifact_write(
            &server,
            crate::ArtifactWriteParams {
                thread_id,
                turn_id: Some(turn_id1),
                approval_id: None,
                artifact_id: None,
                artifact_type: "summary".to_string(),
                summary: "summary".to_string(),
                text: "This is the summary.".to_string(),
            },
        )
        .await?;

        let turn_id2 = TurnId::new();
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: turn_id2,
                input: "second".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;

        let items = build_conversation(&server, thread_id).await?;
        assert!(
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("message")
                    && item.get("role").and_then(Value::as_str) == Some("system")
                    && item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| {
                            content.iter().any(|part| {
                                part.get("type").and_then(Value::as_str) == Some("input_text")
                                    && part
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .is_some_and(|text| text.contains("This is the summary."))
                            })
                        })
            }),
            "expected system summary message"
        );
        assert!(
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("message")
                    && item.get("role").and_then(Value::as_str) == Some("user")
                    && item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| {
                            content.iter().any(|part| {
                                part.get("type").and_then(Value::as_str) == Some("input_text")
                                    && part.get("text").and_then(Value::as_str) == Some("second")
                            })
                        })
            }),
            "expected latest turn input"
        );
        assert!(
            !items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("message")
                    && item.get("role").and_then(Value::as_str) == Some("user")
                    && item
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|content| {
                            content.iter().any(|part| {
                                part.get("type").and_then(Value::as_str) == Some("input_text")
                                    && part.get("text").and_then(Value::as_str) == Some("first")
                            })
                        })
            }),
            "expected summary to replace older turn input"
        );

        Ok(())
    }

    #[tokio::test]
    async fn write_plan_artifact_if_needed_emits_plan_ready_marker() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let wrote =
            write_plan_artifact_if_needed(&server, thread_id, turn_id, true, "# Plan\n\n1. step\n")
                .await?;
        assert!(wrote);

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .unwrap_or_default();

        assert!(events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::AttentionMarkerSet {
                    marker,
                    turn_id: event_turn_id,
                    artifact_id: Some(_),
                    artifact_type,
                    ..
                } if *marker == omne_protocol::AttentionMarkerKind::PlanReady
                    && *event_turn_id == Some(turn_id)
                    && artifact_type.as_deref() == Some("plan")
            )
        }));

        Ok(())
    }

    #[tokio::test]
    async fn write_plan_artifact_if_needed_skips_when_disabled_or_empty() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_a = TurnId::new();
        let turn_b = TurnId::new();
        assert!(
            !write_plan_artifact_if_needed(&server, thread_id, turn_a, false, "plan text").await?
        );
        assert!(!write_plan_artifact_if_needed(&server, thread_id, turn_b, true, "  \n").await?);

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .unwrap_or_default();
        assert!(!events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::AttentionMarkerSet {
                    marker: omne_protocol::AttentionMarkerKind::PlanReady,
                    ..
                }
            )
        }));

        Ok(())
    }
}
