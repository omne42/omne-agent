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
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
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
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
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
                thinking: None,
                show_thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
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
}
