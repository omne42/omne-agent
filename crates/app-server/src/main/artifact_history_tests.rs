#[cfg(test)]
mod artifact_history_tests {
    use super::*;

    fn build_test_server(omne_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: omne_root.clone(),
            notify_tx,
            thread_store: ThreadStore::new(PmPaths::new(omne_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: omne_execpolicy::Policy::empty(),
        }
    }

    #[tokio::test]
    async fn snapshot_user_artifact_version_writes_history_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, _) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"hello").await?;

        let snapshotted =
            snapshot_user_artifact_version(&server, thread_id, artifact_id, &content_path, 7).await?;
        assert!(snapshotted);

        let history_path = user_artifact_history_path(&server, thread_id, artifact_id, 7);
        let bytes = tokio::fs::read(&history_path)
            .await
            .with_context(|| format!("read {}", history_path.display()))?;
        assert_eq!(String::from_utf8_lossy(&bytes), "hello");
        Ok(())
    }

    #[tokio::test]
    async fn snapshot_user_artifact_version_writes_history_metadata_when_present()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, metadata_path) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"hello").await?;

        let turn_id = TurnId::new();
        let tool_id = omne_protocol::ToolId::new();
        let now = OffsetDateTime::now_utc();
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "markdown".to_string(),
            summary: "v1".to_string(),
            preview: None,
            created_at: now,
            updated_at: now,
            version: 1,
            content_path: content_path.display().to_string(),
            size_bytes: 5,
            provenance: Some(ArtifactProvenance {
                thread_id,
                turn_id: Some(turn_id),
                tool_id: Some(tool_id),
                process_id: None,
            }),
        };
        write_file_atomic(&metadata_path, &serde_json::to_vec_pretty(&metadata)?).await?;

        let snapshotted =
            snapshot_user_artifact_version(&server, thread_id, artifact_id, &content_path, 1).await?;
        assert!(snapshotted);

        let history_metadata_path =
            user_artifact_history_metadata_path(&server, thread_id, artifact_id, 1);
        let history_meta = read_artifact_metadata(&history_metadata_path).await?;
        assert_eq!(history_meta.version, 1);
        assert_eq!(
            history_meta
                .provenance
                .as_ref()
                .and_then(|p| p.turn_id),
            Some(turn_id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn prune_user_artifact_history_keeps_latest_versions() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        for version in [1_u32, 2, 3] {
            let path = user_artifact_history_path(&server, thread_id, artifact_id, version);
            write_file_atomic(&path, format!("v{version}").as_bytes()).await?;
        }

        let removed = prune_user_artifact_history(&server, thread_id, artifact_id, 1).await?;
        assert_eq!(
            removed.iter().map(|item| item.version).collect::<Vec<_>>(),
            vec![1, 2]
        );

        for version in [1_u32, 2] {
            let path = user_artifact_history_path(&server, thread_id, artifact_id, version);
            match tokio::fs::metadata(&path).await {
                Ok(_) => anyhow::bail!("expected {} to be pruned", path.display()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
            }
        }

        let path = user_artifact_history_path(&server, thread_id, artifact_id, 3);
        tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("stat {}", path.display()))?;

        Ok(())
    }

    #[tokio::test]
    async fn prune_user_artifact_history_removes_metadata_sidecars() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        for version in [1_u32, 2, 3] {
            write_file_atomic(
                &user_artifact_history_path(&server, thread_id, artifact_id, version),
                format!("v{version}").as_bytes(),
            )
            .await?;
            write_file_atomic(
                &user_artifact_history_metadata_path(&server, thread_id, artifact_id, version),
                br#"{"artifact_id":"00000000-0000-0000-0000-000000000000"}"#,
            )
            .await?;
        }

        let removed = prune_user_artifact_history(&server, thread_id, artifact_id, 1).await?;
        assert_eq!(
            removed.iter().map(|item| item.version).collect::<Vec<_>>(),
            vec![1, 2]
        );

        for version in [1_u32, 2] {
            let metadata_path =
                user_artifact_history_metadata_path(&server, thread_id, artifact_id, version);
            match tokio::fs::metadata(&metadata_path).await {
                Ok(_) => anyhow::bail!("expected {} to be pruned", metadata_path.display()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err).with_context(|| format!("stat {}", metadata_path.display())),
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn prune_user_artifact_history_removes_orphan_metadata_sidecars()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        for version in [1_u32, 2] {
            write_file_atomic(
                &user_artifact_history_path(&server, thread_id, artifact_id, version),
                format!("v{version}").as_bytes(),
            )
            .await?;
            write_file_atomic(
                &user_artifact_history_metadata_path(&server, thread_id, artifact_id, version),
                br#"{"artifact_id":"00000000-0000-0000-0000-000000000000"}"#,
            )
            .await?;
        }
        // version=9 metadata exists without corresponding content snapshot.
        let orphan_metadata_path =
            user_artifact_history_metadata_path(&server, thread_id, artifact_id, 9);
        write_file_atomic(
            &orphan_metadata_path,
            br#"{"artifact_id":"00000000-0000-0000-0000-000000000000"}"#,
        )
        .await?;

        let removed = prune_user_artifact_history(&server, thread_id, artifact_id, 2).await?;
        assert!(removed.is_empty());
        match tokio::fs::metadata(&orphan_metadata_path).await {
            Ok(_) => anyhow::bail!("expected {} to be pruned", orphan_metadata_path.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("stat {}", orphan_metadata_path.display())),
        }
        Ok(())
    }

    #[tokio::test]
    async fn write_user_artifact_with_history_limit_writes_prune_report() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let tool_id = omne_protocol::ToolId::new();
        let turn_id = Some(TurnId::new());

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v1".to_string(),
                text: "one".to_string(),
            },
            1,
        )
        .await?;

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v2".to_string(),
                text: "two".to_string(),
            },
            1,
        )
        .await?;

        let (third_response, _) = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v3".to_string(),
                text: "three".to_string(),
            },
            1,
        )
        .await?;

        let history = third_response["history"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("missing history payload"))?;
        assert_eq!(
            history
                .get("pruned_versions")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_u64).collect::<Vec<_>>()),
            Some(vec![1])
        );

        let prune_report_artifact_id: ArtifactId = serde_json::from_value(
            history
                .get("prune_report_artifact_id")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing prune_report_artifact_id"))?,
        )?;
        assert_ne!(prune_report_artifact_id, artifact_id);

        let prune_report = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: prune_report_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        assert_eq!(
            prune_report["metadata"]["artifact_type"].as_str(),
            Some("artifact_prune_report")
        );
        assert!(
            prune_report["metadata"]["summary"]
                .as_str()
                .unwrap_or("")
                .contains("pruned artifact history")
        );
        let text = prune_report["text"].as_str().unwrap_or("");
        assert!(text.contains("# Artifact History Prune Report"));
        assert!(text.contains("| 1 |"));
        let prune_report_payload = prune_report
            .get("prune_report")
            .ok_or_else(|| anyhow::anyhow!("missing prune_report payload"))?;
        let source_artifact_id = artifact_id.to_string();
        assert_eq!(
            prune_report_payload["source_artifact_id"].as_str(),
            Some(source_artifact_id.as_str())
        );
        assert_eq!(
            prune_report_payload["retained_history_versions"].as_u64(),
            Some(1)
        );
        assert_eq!(prune_report_payload["pruned_count"].as_u64(), Some(1));
        assert_eq!(
            prune_report_payload["pruned_version_details"]
                .as_array()
                .map(|arr| arr.len()),
            Some(1)
        );
        assert_eq!(
            prune_report_payload["pruned_version_details"][0]["version"].as_u64(),
            Some(1)
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_malformed_prune_report_text_is_tolerated() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, metadata_path) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"not a prune report").await?;

        let now = OffsetDateTime::now_utc();
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "artifact_prune_report".to_string(),
            summary: "broken summary".to_string(),
            preview: None,
            created_at: now,
            updated_at: now,
            version: 1,
            content_path: content_path.display().to_string(),
            size_bytes: 18,
            provenance: None,
        };
        write_file_atomic(&metadata_path, &serde_json::to_vec_pretty(&metadata)?).await?;

        let read = handle_artifact_read(
            &server,
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

        assert_eq!(read["metadata"]["artifact_type"].as_str(), Some("artifact_prune_report"));
        assert_eq!(read["text"].as_str(), Some("not a prune report"));
        assert!(read.get("prune_report").map(Value::is_null).unwrap_or(true));
        Ok(())
    }

    #[tokio::test]
    async fn prune_user_artifact_history_returns_size_bytes_details() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 1),
            b"one",
        )
        .await?;
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 2),
            b"twotwo",
        )
        .await?;
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 3),
            b"three",
        )
        .await?;

        let removed = prune_user_artifact_history(&server, thread_id, artifact_id, 1).await?;
        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].version, 1);
        assert_eq!(removed[1].version, 2);
        assert_eq!(removed[0].size_bytes, Some(3));
        assert_eq!(removed[1].size_bytes, Some(6));
        Ok(())
    }

    #[tokio::test]
    async fn write_user_artifact_with_history_limit_does_not_recurse_prune_report()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let tool_id = omne_protocol::ToolId::new();
        let turn_id = Some(TurnId::new());

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id,
                artifact_id: Some(artifact_id),
                artifact_type: "artifact_prune_report".to_string(),
                summary: "p1".to_string(),
                text: "one".to_string(),
            },
            1,
        )
        .await?;

        let (second_response, _) = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id,
                artifact_id: Some(artifact_id),
                artifact_type: "artifact_prune_report".to_string(),
                summary: "p2".to_string(),
                text: "two".to_string(),
            },
            1,
        )
        .await?;

        let history = second_response["history"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("missing history payload"))?;
        assert!(history
            .get("prune_report_artifact_id")
            .map(Value::is_null)
            .unwrap_or(true));
        assert!(history
            .get("prune_report_error")
            .map(Value::is_null)
            .unwrap_or(true));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_delete_removes_history_dir() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let history_path = user_artifact_history_path(&server, thread_id, artifact_id, 1);
        write_file_atomic(&history_path, b"hello").await?;

        handle_artifact_delete(
            &server,
            ArtifactDeleteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
            },
        )
        .await?;

        let history_dir = user_artifact_history_dir_for_thread(&server, thread_id, artifact_id);
        match tokio::fs::metadata(&history_dir).await {
            Ok(_) => anyhow::bail!("expected {} to be removed", history_dir.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).with_context(|| format!("stat {}", history_dir.display())),
        }
    }

    #[tokio::test]
    async fn artifact_list_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

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
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_artifact_list(
            &server,
            ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse>(
            result,
        )?;
        assert!(parsed.denied);
        assert_eq!(parsed.tool, "artifact/list");
        assert_eq!(parsed.error_code.as_deref(), Some("allowed_tools_denied"));
        assert_eq!(parsed.allowed_tools, vec!["repo/search".to_string()]);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_write_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

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
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "markdown".to_string(),
                summary: "test".to_string(),
                text: "hello".to_string(),
            },
        )
        .await?;
        let parsed = serde_json::from_value::<omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse>(
            result,
        )?;
        assert!(parsed.denied);
        assert_eq!(parsed.tool, "artifact/write");
        assert_eq!(parsed.error_code.as_deref(), Some("allowed_tools_denied"));
        assert_eq!(parsed.allowed_tools, vec!["repo/search".to_string()]);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_write_denied_by_mode_uses_typed_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  artifact-deny:
    description: "deny artifact writes"
    permissions:
      artifact: { decision: deny }
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("artifact-deny".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        let result = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "markdown".to_string(),
                summary: "test".to_string(),
                text: "hello".to_string(),
            },
        )
        .await?;
        let parsed =
            serde_json::from_value::<omne_app_server_protocol::ArtifactModeDeniedResponse>(result)?;
        assert!(parsed.denied);
        assert_eq!(parsed.error_code.as_deref(), Some("mode_denied"));
        assert_eq!(parsed.mode, "artifact-deny");
        assert_eq!(
            parsed.decision,
            omne_app_server_protocol::ArtifactModeDecision::Deny
        );
        assert_eq!(parsed.decision_source, "mode_permission".to_string());
        assert!(!parsed.tool_override_hit);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_can_load_historical_version() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, metadata_path) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"v3").await?;
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 2),
            b"v2",
        )
        .await?;

        let now = OffsetDateTime::now_utc();
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "markdown".to_string(),
            summary: "test".to_string(),
            preview: None,
            created_at: now,
            updated_at: now,
            version: 3,
            content_path: content_path.display().to_string(),
            size_bytes: 2,
            provenance: None,
        };
        write_file_atomic(&metadata_path, &serde_json::to_vec_pretty(&metadata)?).await?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: Some(2),
                max_bytes: None,
            },
        )
        .await?;

        assert_eq!(read["text"].as_str(), Some("v2"));
        assert_eq!(read["historical"].as_bool(), Some(true));
        assert_eq!(read["version"].as_u64(), Some(2));
        assert_eq!(read["latest_version"].as_u64(), Some(3));
        assert_eq!(read["metadata_source"].as_str(), Some("latest_fallback"));
        assert_eq!(
            read["metadata_fallback_reason"].as_str(),
            Some("history_metadata_missing")
        );

        let returned_meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(returned_meta.version, 2);
        assert_eq!(
            returned_meta.content_path,
            user_artifact_history_path(&server, thread_id, artifact_id, 2)
                .display()
                .to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_in_summary_includes_structured_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = omne_workflow_spec::FanInSummaryStructuredData::new(
            thread_id.to_string(),
            omne_workflow_spec::FanInSchedulingStructuredData {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 1,
            },
            vec![omne_workflow_spec::FanInTaskStructuredData {
                task_id: "task_1".to_string(),
                title: "example".to_string(),
                thread_id: Some(ThreadId::new().to_string()),
                turn_id: Some(TurnId::new().to_string()),
                status: "NeedUserInput".to_string(),
                reason: Some("waiting for approval".to_string()),
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: Some("approval required".to_string()),
                result_artifact_error_id: Some(ArtifactId::new().to_string()),
                result_artifact_diagnostics: Some(
                    omne_workflow_spec::FanInResultArtifactDiagnosticsStructuredData {
                        scan_last_seq: 42,
                        matched_completion_count: 3,
                        pending_matching_tool_ids: 1,
                    },
                ),
                pending_approval: Some(omne_workflow_spec::FanInPendingApprovalStructuredData {
                    approval_id: omne_protocol::ApprovalId::new().to_string(),
                    action: "artifact/read".to_string(),
                    summary: Some("read child artifact".to_string()),
                    approve_cmd: Some("omne approval decide t a --approve".to_string()),
                    deny_cmd: Some("omne approval decide t a --deny".to_string()),
                }),
            }],
        );
        let structured_json = serde_json::to_string_pretty(&payload)?;
        let text = format!(
            "# Fan-in Summary\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
        );

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_in_summary".to_string(),
                summary: "fan-in".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert_eq!(
            read["fan_in_summary"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1)
        );
        let thread_id_text = thread_id.to_string();
        assert_eq!(
            read["fan_in_summary"]["thread_id"].as_str(),
            Some(thread_id_text.as_str())
        );
        assert_eq!(read["fan_in_summary"]["task_count"].as_u64(), Some(1));
        assert_eq!(
            read["fan_in_summary"]["tasks"][0]["pending_approval"]["approve_cmd"].as_str(),
            Some("omne approval decide t a --approve")
        );
        assert_eq!(
            read["fan_in_summary"]["tasks"][0]["pending_approval"]["deny_cmd"].as_str(),
            Some("omne approval decide t a --deny")
        );
        assert_eq!(
            read["fan_in_summary"]["tasks"][0]["result_artifact_diagnostics"]["scan_last_seq"]
                .as_u64(),
            Some(42)
        );
        assert_eq!(
            read["fan_in_summary"]["tasks"][0]["result_artifact_diagnostics"]
                ["matched_completion_count"]
                .as_u64(),
            Some(3)
        );
        assert_eq!(
            read["fan_in_summary"]["tasks"][0]["result_artifact_diagnostics"]
                ["pending_matching_tool_ids"]
                .as_u64(),
            Some(1)
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_in_summary_invalid_structured_data_is_ignored() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let text = "# Fan-in Summary\n\n## Structured Data\n\n```json\n{invalid json}\n```\n";
        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_in_summary".to_string(),
                summary: "fan-in".to_string(),
                text: text.to_string(),
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert!(read["fan_in_summary"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_in_summary_parses_structured_data_when_text_is_truncated()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = omne_workflow_spec::FanInSummaryStructuredData::new(
            thread_id.to_string(),
            omne_workflow_spec::FanInSchedulingStructuredData {
                env_max_concurrent_subagents: 8,
                effective_concurrency_limit: 4,
                priority_aging_rounds: 2,
            },
            vec![omne_workflow_spec::FanInTaskStructuredData {
                task_id: "task_truncated".to_string(),
                title: "still parse".to_string(),
                thread_id: None,
                turn_id: None,
                status: "NeedUserInput".to_string(),
                reason: None,
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: Some(omne_workflow_spec::FanInPendingApprovalStructuredData {
                    approval_id: omne_protocol::ApprovalId::new().to_string(),
                    action: "artifact/read".to_string(),
                    summary: None,
                    approve_cmd: Some("omne approval decide t a --approve".to_string()),
                    deny_cmd: Some("omne approval decide t a --deny".to_string()),
                }),
            }],
        );
        let structured_json = serde_json::to_string_pretty(&payload)?;
        let prefix = "x".repeat(512);
        let text = format!(
            "{prefix}\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
        );

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_in_summary".to_string(),
                summary: "fan-in".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: Some(64),
            },
        )
        .await?;

        assert_eq!(read["truncated"].as_bool(), Some(true));
        assert_eq!(
            read["fan_in_summary"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1)
        );
        assert_eq!(
            read["fan_in_summary"]["tasks"][0]["task_id"].as_str(),
            Some("task_truncated")
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_linkage_issue_includes_structured_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = omne_workflow_spec::FanOutLinkageIssueStructuredData::new(
            ArtifactId::new().to_string(),
            "fan-out linkage issue: blocked by pending approval".to_string(),
            false,
        );
        let structured_json = serde_json::to_string_pretty(&payload)?;
        let text = format!(
            "# Fan-out Linkage Issue\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
        );

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue".to_string(),
                summary: "fan-out linkage issue".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert_eq!(
            read["fan_out_linkage_issue"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_OUT_LINKAGE_ISSUE_SCHEMA_V1)
        );
        assert_eq!(
            read["fan_out_linkage_issue"]["issue"].as_str(),
            Some("fan-out linkage issue: blocked by pending approval")
        );
        assert_eq!(
            read["fan_out_linkage_issue"]["issue_truncated"].as_bool(),
            Some(false)
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_linkage_issue_parses_structured_data_when_text_is_truncated()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = omne_workflow_spec::FanOutLinkageIssueStructuredData::new(
            ArtifactId::new().to_string(),
            "fan-out linkage issue: truncated parse still works".to_string(),
            true,
        );
        let structured_json = serde_json::to_string_pretty(&payload)?;
        let prefix = "x".repeat(512);
        let text = format!(
            "{prefix}\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
        );

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue".to_string(),
                summary: "fan-out linkage issue".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: Some(64),
            },
        )
        .await?;

        assert_eq!(read["truncated"].as_bool(), Some(true));
        assert_eq!(
            read["fan_out_linkage_issue"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_OUT_LINKAGE_ISSUE_SCHEMA_V1)
        );
        assert_eq!(
            read["fan_out_linkage_issue"]["issue_truncated"].as_bool(),
            Some(true)
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_linkage_issue_clear_includes_structured_data()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = omne_workflow_spec::FanOutLinkageIssueClearStructuredData::new(
            ArtifactId::new().to_string(),
        );
        let structured_json = serde_json::to_string_pretty(&payload)?;
        let text = format!(
            "# Fan-out Linkage Issue Cleared\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
        );

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue_clear".to_string(),
                summary: "fan-out linkage issue cleared".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert_eq!(
            read["fan_out_linkage_issue_clear"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_OUT_LINKAGE_ISSUE_CLEAR_SCHEMA_V1)
        );
        assert_eq!(
            read["fan_out_linkage_issue_clear"]["fan_in_summary_artifact_id"].as_str(),
            Some(payload.fan_in_summary_artifact_id.as_str())
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_linkage_issue_clear_parses_structured_data_when_text_is_truncated(
    ) -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = omne_workflow_spec::FanOutLinkageIssueClearStructuredData::new(
            ArtifactId::new().to_string(),
        );
        let structured_json = serde_json::to_string_pretty(&payload)?;
        let prefix = "x".repeat(512);
        let text = format!(
            "{prefix}\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
        );

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue_clear".to_string(),
                summary: "fan-out linkage issue cleared".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: Some(64),
            },
        )
        .await?;

        assert_eq!(read["truncated"].as_bool(), Some(true));
        assert_eq!(
            read["fan_out_linkage_issue_clear"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_OUT_LINKAGE_ISSUE_CLEAR_SCHEMA_V1)
        );
        assert_eq!(
            read["fan_out_linkage_issue_clear"]["fan_in_summary_artifact_id"].as_str(),
            Some(payload.fan_in_summary_artifact_id.as_str())
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_linkage_issue_clear_invalid_structured_data_is_ignored()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let text = "# Fan-out Linkage Issue Cleared\n\n## Structured Data\n\n```json\n{invalid json}\n```\n";
        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_linkage_issue_clear".to_string(),
                summary: "fan-out linkage issue cleared".to_string(),
                text: text.to_string(),
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert!(read["fan_out_linkage_issue_clear"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_result_includes_structured_data() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let patch_artifact_id = ArtifactId::new();
        let payload = serde_json::json!({
            "schema_version": omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1,
            "task_id": "t-isolated",
            "thread_id": thread_id.to_string(),
            "turn_id": omne_protocol::TurnId::new().to_string(),
            "workspace_mode": "isolated_write",
            "workspace_cwd": "/tmp/subagent/repo",
            "isolated_write_patch": {
                "artifact_type": "patch",
                "artifact_id": patch_artifact_id.to_string(),
                "truncated": false,
                "read_cmd": format!("omne artifact read {} {}", thread_id, patch_artifact_id),
            },
            "isolated_write_handoff": {
                "workspace_cwd": "/tmp/subagent/repo",
                "status_argv": ["git", "-C", "/tmp/subagent/repo", "status", "--short", "--"],
                "diff_argv": ["git", "-C", "/tmp/subagent/repo", "diff", "--binary", "--"],
                "apply_patch_hint": "capture diff output and apply in target workspace with git apply"
            },
            "isolated_write_auto_apply": {
                "enabled": true,
                "attempted": true,
                "applied": true,
                "workspace_cwd": "/tmp/subagent/repo",
                "target_workspace_cwd": "/tmp/parent/repo",
                "check_argv": ["git", "-C", "/tmp/parent/repo", "apply", "--check", "--whitespace=nowarn", "-"],
                "apply_argv": ["git", "-C", "/tmp/parent/repo", "apply", "--whitespace=nowarn", "-"],
                "patch_artifact_id": patch_artifact_id.to_string(),
                "patch_read_cmd": format!("omne artifact read {} {}", thread_id, patch_artifact_id),
                "failure_stage": "check_patch",
                "recovery_hint": "resolve apply-check conflicts in parent workspace, then apply patch manually",
                "recovery_commands": [
                    {
                        "label": "read_patch_artifact",
                        "argv": ["omne", "artifact", "read", thread_id.to_string(), patch_artifact_id.to_string()]
                    },
                    {
                        "label": "check_apply_with_patch_stdin",
                        "argv": ["git", "-C", "/tmp/parent/repo", "apply", "--check", "--whitespace=nowarn", "-"]
                    }
                ]
            },
            "status": "completed",
            "reason": null
        });
        let text = format!("```json\n{}\n```\n", serde_json::to_string_pretty(&payload)?);

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_result".to_string(),
                summary: "fan-out result".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        let patch_artifact_id_text = patch_artifact_id.to_string();
        assert_eq!(
            read["fan_out_result"]["schema_version"].as_str(),
            Some(omne_workflow_spec::FAN_OUT_RESULT_SCHEMA_V1)
        );
        assert_eq!(read["fan_out_result"]["task_id"].as_str(), Some("t-isolated"));
        assert_eq!(
            read["fan_out_result"]["workspace_mode"].as_str(),
            Some("isolated_write")
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_patch"]["artifact_type"].as_str(),
            Some("patch")
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_patch"]["artifact_id"].as_str(),
            Some(patch_artifact_id_text.as_str())
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_handoff"]["status_argv"][0].as_str(),
            Some("git")
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["applied"].as_bool(),
            Some(true)
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["failure_stage"].as_str(),
            Some("check_patch")
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["patch_artifact_id"].as_str(),
            Some(patch_artifact_id_text.as_str())
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["recovery_hint"].as_str(),
            Some("resolve apply-check conflicts in parent workspace, then apply patch manually")
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["recovery_commands"][0]["label"]
                .as_str(),
            Some("read_patch_artifact")
        );
        assert_eq!(
            read["fan_out_result"]["isolated_write_auto_apply"]["recovery_commands"][0]["argv"][0]
                .as_str(),
            Some("omne")
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_result_invalid_structured_data_is_ignored()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let text = "```json\n{invalid json}\n```\n";
        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_result".to_string(),
                summary: "fan-out result".to_string(),
                text: text.to_string(),
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert!(read["fan_out_result"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_fan_out_result_wrong_schema_is_ignored() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let payload = serde_json::json!({
            "schema_version": "fan_out_result.v0",
            "task_id": "t-invalid-schema",
            "thread_id": thread_id.to_string(),
            "turn_id": omne_protocol::TurnId::new().to_string(),
            "workspace_mode": "isolated_write",
            "status": "completed"
        });
        let text = format!("```json\n{}\n```\n", serde_json::to_string_pretty(&payload)?);
        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "fan_out_result".to_string(),
                summary: "fan-out result".to_string(),
                text,
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let read = handle_artifact_read(
            &server,
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

        assert!(read["fan_out_result"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_historical_version_uses_snapshotted_metadata_provenance()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let tool_id = omne_protocol::ToolId::new();
        let turn_id_v1 = Some(TurnId::new());
        let turn_id_v2 = Some(TurnId::new());

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id: turn_id_v1,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v1".to_string(),
                text: "one".to_string(),
            },
            8,
        )
        .await?;

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id: turn_id_v2,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v2".to_string(),
                text: "two".to_string(),
            },
            8,
        )
        .await?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: Some(1),
                max_bytes: None,
            },
        )
        .await?;
        assert_eq!(read["historical"].as_bool(), Some(true));
        assert_eq!(read["metadata_source"].as_str(), Some("history_snapshot"));
        assert!(read["metadata_fallback_reason"].is_null());
        let historical_meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(
            historical_meta
                .provenance
                .as_ref()
                .and_then(|p| p.turn_id),
            turn_id_v1
        );

        let latest = handle_artifact_read(
            &server,
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
        assert_eq!(latest["metadata_source"].as_str(), Some("latest"));
        assert!(latest["metadata_fallback_reason"].is_null());
        let latest_meta: ArtifactMetadata = serde_json::from_value(latest["metadata"].clone())?;
        assert_eq!(latest_meta.provenance.as_ref().and_then(|p| p.turn_id), turn_id_v2);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_historical_version_reports_invalid_metadata_fallback()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let tool_id = omne_protocol::ToolId::new();
        let turn_id_v1 = Some(TurnId::new());
        let turn_id_v2 = Some(TurnId::new());

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id: turn_id_v1,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v1".to_string(),
                text: "one".to_string(),
            },
            8,
        )
        .await?;
        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id: turn_id_v2,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v2".to_string(),
                text: "two".to_string(),
            },
            8,
        )
        .await?;

        let broken_history_metadata =
            user_artifact_history_metadata_path(&server, thread_id, artifact_id, 1);
        write_file_atomic(&broken_history_metadata, b"{not-json").await?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: Some(1),
                max_bytes: None,
            },
        )
        .await?;

        assert_eq!(read["historical"].as_bool(), Some(true));
        assert_eq!(read["metadata_source"].as_str(), Some("latest_fallback"));
        assert_eq!(
            read["metadata_fallback_reason"].as_str(),
            Some("history_metadata_invalid")
        );
        let fallback_meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(fallback_meta.provenance.as_ref().and_then(|p| p.turn_id), turn_id_v2);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_historical_version_reports_unreadable_metadata_fallback()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let tool_id = omne_protocol::ToolId::new();
        let turn_id_v1 = Some(TurnId::new());
        let turn_id_v2 = Some(TurnId::new());

        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id: turn_id_v1,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v1".to_string(),
                text: "one".to_string(),
            },
            8,
        )
        .await?;
        let _ = write_user_artifact_with_history_limit(
            &server,
            UserArtifactWriteRequest {
                tool_id,
                thread_id,
                turn_id: turn_id_v2,
                artifact_id: Some(artifact_id),
                artifact_type: "markdown".to_string(),
                summary: "v2".to_string(),
                text: "two".to_string(),
            },
            8,
        )
        .await?;

        let history_metadata_path =
            user_artifact_history_metadata_path(&server, thread_id, artifact_id, 1);
        tokio::fs::remove_file(&history_metadata_path)
            .await
            .with_context(|| format!("remove {}", history_metadata_path.display()))?;
        tokio::fs::create_dir_all(&history_metadata_path)
            .await
            .with_context(|| format!("mkdir {}", history_metadata_path.display()))?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: Some(1),
                max_bytes: None,
            },
        )
        .await?;

        assert_eq!(read["historical"].as_bool(), Some(true));
        assert_eq!(read["metadata_source"].as_str(), Some("latest_fallback"));
        assert_eq!(
            read["metadata_fallback_reason"].as_str(),
            Some("history_metadata_unreadable")
        );
        let fallback_meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(fallback_meta.provenance.as_ref().and_then(|p| p.turn_id), turn_id_v2);
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_rejects_version_newer_than_latest() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let write = handle_artifact_write(
            &server,
            ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "markdown".to_string(),
                summary: "test".to_string(),
                text: "hello".to_string(),
            },
        )
        .await?;
        let artifact_id: ArtifactId = serde_json::from_value(write["artifact_id"].clone())?;

        let err = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: Some(2),
                max_bytes: None,
            },
        )
        .await
        .expect_err("expected version read to fail");
        assert!(err.to_string().contains("artifact version not found"));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_read_reports_not_retained_for_missing_history_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, metadata_path) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"v3").await?;

        let now = OffsetDateTime::now_utc();
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "markdown".to_string(),
            summary: "test".to_string(),
            preview: None,
            created_at: now,
            updated_at: now,
            version: 3,
            content_path: content_path.display().to_string(),
            size_bytes: 2,
            provenance: None,
        };
        write_file_atomic(&metadata_path, &serde_json::to_vec_pretty(&metadata)?).await?;

        let err = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: Some(2),
                max_bytes: None,
            },
        )
        .await
        .expect_err("expected missing historical version to fail");
        assert!(err.to_string().contains("artifact version not retained"));
        Ok(())
    }

    #[tokio::test]
    async fn artifact_versions_lists_latest_and_retained_history() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, metadata_path) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"v4").await?;
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 1),
            b"v1",
        )
        .await?;
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 3),
            b"v3",
        )
        .await?;

        let now = OffsetDateTime::now_utc();
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "markdown".to_string(),
            summary: "test".to_string(),
            preview: None,
            created_at: now,
            updated_at: now,
            version: 4,
            content_path: content_path.display().to_string(),
            size_bytes: 2,
            provenance: None,
        };
        write_file_atomic(&metadata_path, &serde_json::to_vec_pretty(&metadata)?).await?;

        let versions = handle_artifact_versions(
            &server,
            ArtifactVersionsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
            },
        )
        .await?;

        assert_eq!(versions["latest_version"].as_u64(), Some(4));
        assert_eq!(
            versions["versions"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_u64()).collect::<Vec<_>>()),
            Some(vec![4, 3, 1])
        );
        assert_eq!(
            versions["history_versions"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_u64()).collect::<Vec<_>>()),
            Some(vec![3, 1])
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifact_versions_ignores_history_metadata_sidecars() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let artifact_id = ArtifactId::new();
        let (content_path, metadata_path) = user_artifact_paths(&server, thread_id, artifact_id);
        write_file_atomic(&content_path, b"v4").await?;
        write_file_atomic(
            &user_artifact_history_path(&server, thread_id, artifact_id, 1),
            b"v1",
        )
        .await?;
        write_file_atomic(
            &user_artifact_history_metadata_path(&server, thread_id, artifact_id, 1),
            br#"{"artifact_id":"00000000-0000-0000-0000-000000000000"}"#,
        )
        .await?;
        write_file_atomic(
            &user_artifact_history_metadata_path(&server, thread_id, artifact_id, 9),
            br#"{"artifact_id":"00000000-0000-0000-0000-000000000000"}"#,
        )
        .await?;

        let now = OffsetDateTime::now_utc();
        let metadata = ArtifactMetadata {
            artifact_id,
            artifact_type: "markdown".to_string(),
            summary: "test".to_string(),
            preview: None,
            created_at: now,
            updated_at: now,
            version: 4,
            content_path: content_path.display().to_string(),
            size_bytes: 2,
            provenance: None,
        };
        write_file_atomic(&metadata_path, &serde_json::to_vec_pretty(&metadata)?).await?;

        let versions = handle_artifact_versions(
            &server,
            ArtifactVersionsParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
            },
        )
        .await?;

        assert_eq!(versions["latest_version"].as_u64(), Some(4));
        assert_eq!(
            versions["versions"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_u64()).collect::<Vec<_>>()),
            Some(vec![4, 1])
        );
        assert_eq!(
            versions["history_versions"]
                .as_array()
                .map(|values| values.iter().filter_map(|v| v.as_u64()).collect::<Vec<_>>()),
            Some(vec![1])
        );
        Ok(())
    }
}
