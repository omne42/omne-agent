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

        let rate_limited = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "rate limit".to_string(),
        });
        assert!(llm_error_is_retryable(&rate_limited));
        assert!(llm_error_prefers_provider_fallback(&rate_limited));

        let server_error = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::BAD_GATEWAY,
            body: "upstream".to_string(),
        });
        assert!(llm_error_is_retryable(&server_error));
        assert!(llm_error_prefers_provider_fallback(&server_error));

        let bad_request = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
            status: StatusCode::BAD_REQUEST,
            body: "invalid".to_string(),
        });
        assert!(!llm_error_is_retryable(&bad_request));
        assert!(!llm_error_prefers_provider_fallback(&bad_request));
        assert!(llm_error_prefers_model_fallback(&bad_request));

        let unauthorized = LlmAttemptError::Ditto(ditto_llm::DittoError::Api {
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
mod attachment_upload_tests {
    use super::*;
    use async_trait::async_trait;

    #[derive(Clone)]
    struct DummyLanguageModel;

    #[async_trait]
    impl ditto_llm::LanguageModel for DummyLanguageModel {
        fn provider(&self) -> &str {
            "dummy"
        }

        fn model_id(&self) -> &str {
            "dummy-model"
        }

        async fn generate(
            &self,
            _request: ditto_llm::GenerateRequest,
        ) -> ditto_llm::Result<ditto_llm::GenerateResponse> {
            unimplemented!()
        }

        async fn stream(
            &self,
            _request: ditto_llm::GenerateRequest,
        ) -> ditto_llm::Result<ditto_llm::StreamResult> {
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
            config: ditto_llm::ProviderConfig {
                base_url: Some("http://example.com/v1".to_string()),
                default_model: Some("gpt-4.1".to_string()),
                model_whitelist: Vec::new(),
                http_headers: Default::default(),
                http_query_params: Default::default(),
                auth: None,
                capabilities: Some(ditto_llm::ProviderCapabilities::openai_responses()),
            },
            capabilities: ditto_llm::ProviderCapabilities::openai_responses(),
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
        let ditto_llm::ContentPart::File { source, .. } = &parts[0] else {
            panic!("expected file part");
        };
        assert!(matches!(source, ditto_llm::FileSource::Base64 { .. }));
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
        let ditto_llm::ContentPart::File { source, .. } = &parts[0] else {
            panic!("expected file part");
        };
        assert!(matches!(
            source,
            ditto_llm::FileSource::FileId { file_id } if file_id == "file_123"
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
        let ditto_llm::ContentPart::File { source, .. } = &parts[0] else {
            panic!("expected file part");
        };
        assert!(matches!(source, ditto_llm::FileSource::Base64 { .. }));
    }
}

#[cfg(test)]
mod loop_detection_tests {
    use super::*;

    #[test]
    fn tool_call_signature_is_stable_for_object_key_order() {
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"b": 2, "a": 1});
        assert_eq!(tool_call_signature("file_read", &a), tool_call_signature("file_read", &b));
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
mod auto_summary_tests {
    use super::*;

    use pm_core::{PmPaths, ThreadStore};
    use pm_protocol::TurnStatus;
    use tokio::sync::broadcast;

    fn build_test_server(pm_root: PathBuf) -> crate::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        crate::Server {
            cwd: pm_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(crate::McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    #[test]
    fn should_auto_compact_triggers_at_threshold() {
        assert!(!should_auto_compact(0, None, 0, 80));
        assert!(!should_auto_compact(79, None, 100, 80));
        assert!(should_auto_compact(80, None, 100, 80));
        assert!(should_auto_compact(100, None, 100, 80));
        assert!(should_auto_compact(90, Some(80), 0, 0));
    }

    #[tokio::test]
    async fn build_conversation_prefers_latest_summary_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        let turn_id1 = TurnId::new();
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id: turn_id1,
                input: "first".to_string(),
                context_refs: None,
                attachments: None,
                priority: pm_protocol::TurnPriority::Foreground,
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::AssistantMessage {
                turn_id: Some(turn_id1),
                text: "hello".to_string(),
                model: None,
                response_id: None,
                token_usage: None,
            })
            .await?;
        thread_rt
            .append_event(pm_protocol::ThreadEventKind::TurnCompleted {
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
            .append_event(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id: turn_id2,
                input: "second".to_string(),
                context_refs: None,
                attachments: None,
                priority: pm_protocol::TurnPriority::Foreground,
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
}
