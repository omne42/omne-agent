#[cfg(test)]
mod thread_manage_tests {
    use super::*;

    fn build_test_server(pm_root: PathBuf) -> Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        Server {
            cwd: pm_root.clone(),
            notify_tx,
            notify_hub: default_notify_hub(),
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
            db_vfs: None,
        }
    }

    async fn write_project_config(repo_dir: &Path, contents: &str) -> anyhow::Result<()> {
        let config_dir = repo_dir.join(".codepm_data");
        tokio::fs::create_dir_all(&config_dir).await?;
        tokio::fs::write(config_dir.join("config.toml"), contents).await?;
        Ok(())
    }

    async fn spawn_models_endpoint(models: &[&str]) -> anyhow::Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        let body = {
            let items = models
                .iter()
                .map(|id| format!(r#"{{"id":"{}"}}"#, id))
                .collect::<Vec<_>>()
                .join(",");
            format!(r#"{{"data":[{}]}}"#, items)
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );

        tokio::spawn(async move {
            let Ok((mut socket, _peer)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(response.as_bytes()).await;
        });

        Ok(format!("http://{}/v1", addr))
    }

    #[tokio::test]
    async fn thread_models_filters_by_model_whitelist() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let base_url = spawn_models_endpoint(&["gpt-4.1", "gpt-4.1-mini", "gpt-4o-mini"]).await?;
        write_project_config(
            &repo_dir,
            &format!(
                r#"
[project_config]
enabled = true

[openai]
provider = "test"

[openai.providers.test]
base_url = "{base_url}"
model_whitelist = ["gpt-4.1-mini"]
"#
            ),
        )
        .await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = handle_thread_models(&server, ThreadModelsParams { thread_id }).await?;
        let models = result["models"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing models"))?
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(models, vec!["gpt-4.1-mini"]);
        assert!(result["models_error"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn thread_models_falls_back_when_list_models_fails() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        write_project_config(
            &repo_dir,
            r#"
[project_config]
enabled = true

[openai]
provider = "test"
model = "project-model"

[openai.providers.test]
base_url = "http://127.0.0.1:1/v1"
default_model = "default-model"
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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
                openai_provider: None,
                model: Some("thread-model".to_string()),
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
            },
        )
        .await?;

        let result = handle_thread_models(&server, ThreadModelsParams { thread_id }).await?;
        let models = result["models"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing models"))?
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>();
        assert_eq!(models, vec!["thread-model", "project-model", "default-model"]);
        let models_error = result["models_error"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing models_error"))?;
        assert!(!models_error.trim().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_skips_when_config_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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

        assert!(result["skipped"].as_bool().unwrap_or(false));
        Ok(())
    }

    #[tokio::test]
    async fn thread_hook_run_starts_process_from_config() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let config_dir = repo_dir.join(".codepm_data").join("spec");
        tokio::fs::create_dir_all(&config_dir).await?;

        tokio::fs::write(
            config_dir.join("workspace.yaml"),
            r#"
hooks:
  setup: ["sh", "-c", "exit 0"]
"#,
        )
        .await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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

        assert!(result["ok"].as_bool().unwrap_or(false));
        assert_eq!(result["hook"].as_str().unwrap_or(""), "setup");
        let process_id: ProcessId = result["process_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing process_id"))?
            .parse()?;

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
    async fn checkpoint_create_list_restore_roundtrip() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        tokio::fs::write(repo_dir.join("foo.txt"), "v1\n").await?;
        tokio::fs::write(repo_dir.join(".env"), "SECRET=sk-should-not-be-snapshotted\n").await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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

        let checkpoint_id: pm_protocol::CheckpointId = created["checkpoint_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing checkpoint_id"))?
            .parse()?;

        let listed = handle_thread_checkpoint_list(&server, ThreadCheckpointListParams { thread_id }).await?;
        let checkpoints = listed["checkpoints"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing checkpoints"))?;
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(
            checkpoints[0]["checkpoint_id"].as_str().unwrap_or(""),
            checkpoint_id.to_string()
        );

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

        assert!(first_restore["needs_approval"].as_bool().unwrap_or(false));
        let approval_id: pm_protocol::ApprovalId = first_restore["approval_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing approval_id"))?
            .parse()?;

        handle_approval_decide(
            &server,
            ApprovalDecideParams {
                thread_id,
                approval_id,
                decision: pm_protocol::ApprovalDecision::Approved,
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
        assert!(second_restore["restored"].as_bool().unwrap_or(false));

        assert_eq!(tokio::fs::read_to_string(repo_dir.join("foo.txt")).await?, "v1\n");
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
        assert!(events.iter().any(|e| matches!(e.kind, pm_protocol::ThreadEventKind::CheckpointCreated { checkpoint_id: got, .. } if got == checkpoint_id)));
        assert!(events.iter().any(|e| matches!(e.kind, pm_protocol::ThreadEventKind::CheckpointRestored { checkpoint_id: got, status: pm_protocol::CheckpointRestoreStatus::Ok, .. } if got == checkpoint_id)));

        Ok(())
    }

    #[tokio::test]
    async fn thread_allowed_tools_denies_file_read() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        tokio::fs::write(repo_dir.join("foo.txt"), "hello\n").await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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
                openai_provider: None,
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: Some(Some(vec!["repo/search".to_string()])),
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
}
