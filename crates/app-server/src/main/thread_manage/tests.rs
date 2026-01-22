#[cfg(test)]
mod thread_manage_tests {
    use super::*;

    fn build_test_server(pm_root: PathBuf) -> Server {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        Server {
            cwd: pm_root.clone(),
            out_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
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
}
