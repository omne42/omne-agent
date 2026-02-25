#[cfg(unix)]
#[tokio::test]
async fn canonical_rel_path_for_write_resolves_ancestor_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = tokio::fs::canonicalize(dir.path())
        .await
        .expect("canonicalize root");

    let allowed = root.join("allowed");
    let denied = root.join("denied");
    tokio::fs::create_dir_all(&allowed).await.expect("mkdir allowed");
    tokio::fs::create_dir_all(&denied).await.expect("mkdir denied");

    let link_dir = allowed.join("link");
    symlink(&denied, &link_dir).expect("symlink");

    let denied_file = denied.join("file.txt");
    tokio::fs::write(&denied_file, b"hi").await.expect("write");

    let requested = link_dir.join("file.txt");
    let rel = canonical_rel_path_for_write(&root, &requested)
        .await
        .expect("canonical rel");
    assert_eq!(rel, std::path::PathBuf::from("denied/file.txt"));
}

#[cfg(unix)]
#[tokio::test]
async fn rel_path_is_secret_cannot_be_bypassed_via_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = tokio::fs::canonicalize(dir.path())
        .await
        .expect("canonicalize root");

    let env = root.join(".env");
    tokio::fs::write(&env, b"SECRET=1\n").await.expect("write .env");

    let link = root.join("link");
    symlink(&env, &link).expect("symlink");

    let resolved = omne_core::resolve_file(
        &root,
        std::path::Path::new("link"),
        omne_core::PathAccess::Read,
        false,
    )
        .await
        .expect("resolve");
    let rel = omne_core::modes::relative_path_under_root(&root, &resolved).expect("relative path");
    assert!(rel_path_is_secret(&rel), "expected .env to be treated as secret");
}

#[cfg(test)]
mod fs_file_mode_override_audit_tests {
    use super::*;

    async fn setup_thread_with_allowed_tools_denied(
    ) -> anyhow::Result<(tempfile::TempDir, Server, ThreadId)> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "needle\n").await?;

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
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
                execpolicy_rules: None,
            },
        )
        .await?;

        Ok((tmp, server, thread_id))
    }

    async fn setup_thread_with_read_only_sandbox(
    ) -> anyhow::Result<(tempfile::TempDir, Server, ThreadId)> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "needle\n").await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;

        handle_thread_configure(
            &server,
            ThreadConfigureParams {
                thread_id,
                approval_policy: None,
                sandbox_policy: Some(omne_protocol::SandboxPolicy::ReadOnly),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        Ok((tmp, server, thread_id))
    }

    async fn setup_thread_with_tool_override(
        mode_name: &str,
        tool: &str,
    ) -> anyhow::Result<(tempfile::TempDir, Server, ThreadId)> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        write_modes_yaml_shared(
            &repo_dir,
            format!(
                r#"
version: 1
modes:
  {mode_name}:
    description: "tool override deny"
    permissions:
      read:
        decision: allow
      edit:
        decision: allow
      command:
        decision: allow
      artifact:
        decision: allow
    tool_overrides:
      - tool: {tool}
        decision: deny
"#
            )
            .as_str(),
        )
        .await?;

        let server = build_test_server_shared(repo_dir.join(".omne_data"));
        let thread_id = create_test_thread_shared(&server, repo_dir.clone()).await?;
        configure_test_thread_mode_shared(&server, thread_id, mode_name).await?;

        Ok((tmp, server, thread_id))
    }

    #[tokio::test]
    async fn fs_mkdir_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) =
            setup_thread_with_tool_override("fs-mkdir-override-deny", "fs/mkdir").await?;

        let result = handle_fs_mkdir(
            &server,
            FsMkdirParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "dir/new".to_string(),
                recursive: true,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_write_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) =
            setup_thread_with_tool_override("file-write-override-deny", "file/write").await?;

        let result = handle_file_write(
            &server,
            FileWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                text: "hello".to_string(),
                create_parent_dirs: Some(true),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_patch_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) =
            setup_thread_with_tool_override("file-patch-override-deny", "file/patch").await?;

        let result = handle_file_patch(
            &server,
            FilePatchParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                patch: "not-used".to_string(),
                max_bytes: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_edit_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) =
            setup_thread_with_tool_override("file-edit-override-deny", "file/edit").await?;

        let result = handle_file_edit(
            &server,
            FileEditParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                edits: vec![FileEditOp {
                    old: "a".to_string(),
                    new: "b".to_string(),
                    expected_replacements: Some(1),
                }],
                max_bytes: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_delete_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) =
            setup_thread_with_tool_override("file-delete-override-deny", "file/delete").await?;

        let result = handle_file_delete(
            &server,
            FileDeleteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                recursive: false,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn file_grep_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_allowed_tools_denied().await?;

        let result = handle_file_grep(
            &server,
            FileGrepParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                root: None,
                query: "needle".to_string(),
                is_regex: false,
                include_glob: None,
                max_matches: Some(20),
                max_bytes_per_file: Some(1024 * 1024),
                max_files: Some(20_000),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("file/grep"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn file_write_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_allowed_tools_denied().await?;

        let result = handle_file_write(
            &server,
            FileWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                text: "hello".to_string(),
                create_parent_dirs: Some(true),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("file/write"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn file_edit_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_allowed_tools_denied().await?;

        let result = handle_file_edit(
            &server,
            FileEditParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                edits: vec![FileEditOp {
                    old: "a".to_string(),
                    new: "b".to_string(),
                    expected_replacements: Some(1),
                }],
                max_bytes: None,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("file/edit"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn fs_mkdir_denied_by_allowed_tools_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_allowed_tools_denied().await?;

        let result = handle_fs_mkdir(
            &server,
            FsMkdirParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "new-dir".to_string(),
                recursive: true,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["tool"].as_str(), Some("fs/mkdir"));
        assert_eq!(result["error_code"].as_str(), Some("allowed_tools_denied"));
        let allowed_tools = result["allowed_tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing allowed_tools"))?;
        assert_eq!(allowed_tools.len(), 1);
        assert_eq!(allowed_tools[0].as_str(), Some("repo/search"));
        Ok(())
    }

    #[tokio::test]
    async fn file_write_denied_by_read_only_sandbox_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_read_only_sandbox().await?;

        let result = handle_file_write(
            &server,
            FileWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                text: "hello".to_string(),
                create_parent_dirs: Some(true),
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_policy"].as_str(), Some("read_only"));
        assert_eq!(result["error_code"].as_str(), Some("sandbox_policy_denied"));
        Ok(())
    }

    #[tokio::test]
    async fn file_delete_denied_by_read_only_sandbox_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_read_only_sandbox().await?;

        let result = handle_file_delete(
            &server,
            FileDeleteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "a.txt".to_string(),
                recursive: false,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_policy"].as_str(), Some("read_only"));
        assert_eq!(result["error_code"].as_str(), Some("sandbox_policy_denied"));
        Ok(())
    }

    #[tokio::test]
    async fn fs_mkdir_denied_by_read_only_sandbox_uses_typed_payload() -> anyhow::Result<()> {
        let (_tmp, server, thread_id) = setup_thread_with_read_only_sandbox().await?;

        let result = handle_fs_mkdir(
            &server,
            FsMkdirParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                path: "new-dir".to_string(),
                recursive: true,
            },
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["sandbox_policy"].as_str(), Some("read_only"));
        assert_eq!(result["error_code"].as_str(), Some("sandbox_policy_denied"));
        Ok(())
    }
}
