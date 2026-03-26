#[cfg(test)]
mod thread_diff_tests {
    use super::*;

    async fn run_git(repo_dir: &Path, args: &[&str]) -> anyhow::Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_dir)
            .output()
            .await
            .with_context(|| format!("spawn git {}", args.join(" ")))?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "git {} failed (exit {:?}): stdout={}, stderr={}",
                args.join(" "),
                output.status.code(),
                stdout,
                stderr
            );
        }
        Ok(())
    }

    async fn init_git_repo(repo_dir: &Path) -> anyhow::Result<()> {
        run_git(repo_dir, &["init"]).await?;
        run_git(repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(repo_dir, &["config", "user.name", "Test User"]).await?;
        Ok(())
    }

    async fn create_initial_hello_commit(repo_dir: &Path) -> anyhow::Result<PathBuf> {
        let file_path = repo_dir.join("hello.txt");
        tokio::fs::write(&file_path, "hello\n").await?;
        run_git(repo_dir, &["add", "hello.txt"]).await?;
        run_git(repo_dir, &["commit", "-m", "init"]).await?;
        Ok(file_path)
    }

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
    async fn thread_diff_writes_diff_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        init_git_repo(&repo_dir).await?;

        let file_path = create_initial_hello_commit(&repo_dir).await?;

        tokio::fs::write(&file_path, "hello\nworld\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let diff = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let diff = match diff {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Ok(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotResponse::Ok, got {other:?}"),
        };
        assert_eq!(diff.thread_id, thread_id);
        assert!(!diff.process_id.to_string().is_empty());
        assert!(diff.exit_code.is_some());

        let artifact_id = diff.artifact.artifact_id;

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

        let meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(meta.artifact_type, "diff");
        assert_eq!(
            meta.preview.as_ref().map(|p| p.kind),
            Some(omne_protocol::ArtifactPreviewKind::DiffUnified)
        );
        assert_eq!(
            meta.preview.as_ref().and_then(|p| p.title.as_deref()),
            Some("git diff --")
        );

        let text = read["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing text"))?;
        assert!(text.contains("diff --git a/hello.txt b/hello.txt"));
        assert!(text.contains("+world"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_writes_patch_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        init_git_repo(&repo_dir).await?;

        let file_path = create_initial_hello_commit(&repo_dir).await?;

        tokio::fs::write(&file_path, "hello\nworld\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let patch = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let patch = match patch {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Ok(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotResponse::Ok, got {other:?}"),
        };
        assert_eq!(patch.thread_id, thread_id);
        assert!(!patch.process_id.to_string().is_empty());
        assert!(patch.exit_code.is_some());

        let artifact_id = patch.artifact.artifact_id;

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

        let meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(meta.artifact_type, "patch");
        assert_eq!(
            meta.preview.as_ref().map(|p| p.kind),
            Some(omne_protocol::ArtifactPreviewKind::PatchUnified)
        );
        assert_eq!(
            meta.preview.as_ref().and_then(|p| p.title.as_deref()),
            Some("git diff --binary --patch")
        );

        let text = read["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing text"))?;
        assert!(text.contains("diff --git a/hello.txt b/hello.txt"));
        assert!(text.contains("+world"));
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        init_git_repo(&repo_dir).await?;

        create_initial_hello_commit(&repo_dir).await?;

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

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("sandbox_policy_denied"));
        let denied = match &result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(detail) => {
                match detail {
                    omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxPolicyDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxNetworkDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(
                        detail,
                    ) => detail.denied,
                }
            }
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(detail) => {
                match detail {
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::Denied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(
                        detail,
                    ) => detail.denied,
                }
            }
        };
        assert!(denied);

        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        init_git_repo(&repo_dir).await?;

        create_initial_hello_commit(&repo_dir).await?;

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

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("sandbox_policy_denied"));
        let denied = match &result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(detail) => {
                match detail {
                    omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxPolicyDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::SandboxNetworkDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(
                        detail,
                    ) => detail.denied,
                }
            }
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(detail) => {
                match detail {
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::Denied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(
                        detail,
                    ) => detail.denied,
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(detail) => {
                        detail.denied
                    }
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(
                        detail,
                    ) => detail.denied,
                }
            }
        };
        assert!(denied);

        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_allowed_tools_denied_returns_typed_detail() -> anyhow::Result<()> {
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
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("allowed_tools_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.tool, "process/start");
                assert_eq!(detail.allowed_tools, vec!["repo/search".to_string()]);
            }
            other => anyhow::bail!("expected process allowed_tools denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_artifact_allowed_tools_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                allowed_tools: Some(Some(vec!["process/start".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("allowed_tools_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.tool, "artifact/write");
                assert_eq!(detail.allowed_tools, vec!["process/start".to_string()]);
            }
            other => anyhow::bail!("expected artifact allowed_tools denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_artifact_allowed_tools_denied_returns_typed_detail()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                allowed_tools: Some(Some(vec!["process/start".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("allowed_tools_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.tool, "artifact/write");
                assert_eq!(detail.allowed_tools, vec!["process/start".to_string()]);
            }
            other => anyhow::bail!("expected artifact allowed_tools denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_artifact_mode_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  artifact-deny:
    description: "deny artifact writes"
    permissions:
      command: { decision: allow }
      artifact: { decision: deny }
"#,
        )
        .await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("artifact-deny".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "artifact-deny");
                assert_eq!(
                    detail.decision,
                    omne_app_server_protocol::ArtifactModeDecision::Deny
                );
                assert_eq!(detail.decision_source, "mode_permission");
                assert!(!detail.tool_override_hit);
            }
            other => anyhow::bail!("expected artifact mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_artifact_mode_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  artifact-deny:
    description: "deny artifact writes"
    permissions:
      command: { decision: allow }
      artifact: { decision: deny }
"#,
        )
        .await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("artifact-deny".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "artifact-deny");
                assert_eq!(
                    detail.decision,
                    omne_app_server_protocol::ArtifactModeDecision::Deny
                );
                assert_eq!(detail.decision_source, "mode_permission");
                assert!(!detail.tool_override_hit);
            }
            other => anyhow::bail!("expected artifact mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_artifact_mode_denied_by_tool_override_returns_typed_detail()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  artifact-override-deny:
    description: "deny artifact/write by tool override"
    permissions:
      command: { decision: allow }
      artifact: { decision: allow }
    tool_overrides:
      - tool: artifact/write
        decision: deny
"#,
        )
        .await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                mode: Some("artifact-override-deny".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "artifact-override-deny");
                assert_eq!(
                    detail.decision,
                    omne_app_server_protocol::ArtifactModeDecision::Deny
                );
                assert_eq!(detail.decision_source, "tool_override");
                assert!(detail.tool_override_hit);
            }
            other => anyhow::bail!("expected artifact mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_artifact_unknown_mode_denied_returns_typed_detail()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  artifact-unknown:
    description: "allow command/artifact initially"
    permissions:
      command: { decision: allow }
      artifact: { decision: allow }
"#,
        )
        .await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                mode: Some("artifact-unknown".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let repo_dir_for_task = repo_dir.clone();
        let mutate_mode_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            write_modes_yaml_shared(
                &repo_dir_for_task,
                r#"
version: 1
modes:
  other-mode:
    description: "placeholder"
    permissions:
      command: { decision: allow }
      artifact: { decision: allow }
"#,
            )
            .await
        });

        let result = handle_thread_git_snapshot(
            &server,
            ThreadGitSnapshotSpec {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
                kind: omne_git_runtime::SnapshotKind::Diff,
                recipe_override: Some(omne_git_runtime::SnapshotRecipe {
                    argv: vec![
                        "sh".to_string(),
                        "-lc".to_string(),
                        "sleep 0.5; printf 'snapshot\\n'".to_string(),
                    ],
                    artifact_type: "diff",
                    summary_clean: "test clean",
                    summary_dirty: "test dirty",
                }),
            },
        )
        .await?;
        mutate_mode_task
            .await
            .context("join mode mutation task")??;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_unknown"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "artifact-unknown");
                assert!(detail.available.contains("other-mode"));
            }
            other => anyhow::bail!("expected artifact unknown mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_artifact_unknown_mode_denied_returns_typed_detail()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  patch-artifact-unknown:
    description: "allow command/artifact initially"
    permissions:
      command: { decision: allow }
      artifact: { decision: allow }
"#,
        )
        .await?;

        init_git_repo(&repo_dir).await?;
        create_initial_hello_commit(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                approval_policy: Some(omne_protocol::ApprovalPolicy::AutoApprove),
                mode: Some("patch-artifact-unknown".to_string()),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let repo_dir_for_task = repo_dir.clone();
        let mutate_mode_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            write_modes_yaml_shared(
                &repo_dir_for_task,
                r#"
version: 1
modes:
  other-mode:
    description: "placeholder"
    permissions:
      command: { decision: allow }
      artifact: { decision: allow }
"#,
            )
            .await
        });

        let result = handle_thread_git_snapshot(
            &server,
            ThreadGitSnapshotSpec {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
                kind: omne_git_runtime::SnapshotKind::Patch,
                recipe_override: Some(omne_git_runtime::SnapshotRecipe {
                    argv: vec![
                        "sh".to_string(),
                        "-lc".to_string(),
                        "sleep 0.5; printf 'snapshot\\n'".to_string(),
                    ],
                    artifact_type: "patch",
                    summary_clean: "test clean",
                    summary_dirty: "test dirty",
                }),
            },
        )
        .await?;
        mutate_mode_task
            .await
            .context("join mode mutation task")??;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_unknown"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "patch-artifact-unknown");
                assert!(detail.available.contains("other-mode"));
            }
            other => anyhow::bail!("expected artifact unknown mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_unknown_mode_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  patch-mode:
    description: "allow git patch"
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
                mode: Some("patch-mode".to_string()),
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

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_unknown"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "patch-mode");
                assert!(detail.available.contains("other-mode"));
            }
            other => anyhow::bail!("expected process unknown_mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_allowed_tools_denied_returns_typed_detail() -> anyhow::Result<()> {
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
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("allowed_tools_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.tool, "process/start");
                assert_eq!(detail.allowed_tools, vec!["repo/search".to_string()]);
            }
            other => anyhow::bail!("expected process allowed_tools denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_unknown_mode_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        write_modes_yaml_shared(
            &repo_dir,
            r#"
version: 1
modes:
  diff-mode:
    description: "allow git diff"
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
                mode: Some("diff-mode".to_string()),
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

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("mode_unknown"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.mode, "diff-mode");
                assert!(detail.available.contains("other-mode"));
            }
            other => anyhow::bail!("expected process unknown_mode denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_execpolicy_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("rules")).await?;
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

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("execpolicy_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail),
            ) => {
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
    async fn thread_diff_execpolicy_load_denied_returns_typed_detail() -> anyhow::Result<()> {
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
                execpolicy_rules: Some(vec!["rules/missing.rules".to_string()]),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("execpolicy_load_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.error, "failed to load thread execpolicy rules");
                assert!(!detail.details.trim().is_empty());
            }
            other => anyhow::bail!("expected process execpolicy load denied detail, got {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_patch_execpolicy_denied_returns_typed_detail() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("rules")).await?;
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

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("execpolicy_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(detail),
            ) => {
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
    async fn thread_patch_execpolicy_load_denied_returns_typed_detail() -> anyhow::Result<()> {
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
                execpolicy_rules: Some(vec!["rules/missing.rules".to_string()]),
                ..thread_configure_defaults(thread_id)
            },
        )
        .await?;

        let result = handle_thread_patch(
            &server,
            ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;
        let result = match result {
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::Denied(response) => response,
            other => anyhow::bail!("expected ThreadGitSnapshotDeniedResponse, got {other:?}"),
        };
        assert!(result.denied);
        assert_eq!(result.thread_id, thread_id);
        assert_eq!(result.error_code.as_deref(), Some("execpolicy_load_denied"));
        match result.detail {
            omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(detail),
            ) => {
                assert!(detail.denied);
                assert_eq!(detail.error, "failed to load thread execpolicy rules");
                assert!(!detail.details.trim().is_empty());
            }
            other => anyhow::bail!("expected process execpolicy load denied detail, got {other:?}"),
        }
        Ok(())
    }
}
