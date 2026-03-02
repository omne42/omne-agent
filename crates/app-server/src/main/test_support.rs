fn build_test_server_shared(omne_root: PathBuf) -> Server {
    let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
    Server {
        cwd: omne_root.clone(),
        notify_tx,
        thread_store: ThreadStore::new(PmPaths::new(omne_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        mcp: Arc::new(tokio::sync::Mutex::new(McpManager::default())),
        disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        provider_runtimes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        exec_policy: omne_execpolicy::Policy::empty(),
    }
}

async fn create_test_thread_shared(server: &Server, repo_dir: PathBuf) -> anyhow::Result<ThreadId> {
    let handle = server.thread_store.create_thread(repo_dir).await?;
    let thread_id = handle.thread_id();
    drop(handle);
    Ok(thread_id)
}

async fn write_modes_yaml_shared(repo_dir: &Path, mode_yaml: &str) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
    tokio::fs::write(repo_dir.join(".omne_data/spec/modes.yaml"), mode_yaml).await?;
    Ok(())
}

async fn configure_test_thread_mode_shared(
    server: &Server,
    thread_id: ThreadId,
    mode_name: &str,
) -> anyhow::Result<()> {
    handle_thread_configure(
        server,
        ThreadConfigureParams {
            thread_id,
            approval_policy: Some(omne_protocol::ApprovalPolicy::Manual),
            sandbox_policy: None,
            sandbox_writable_roots: None,
            sandbox_network_access: None,
            mode: Some(mode_name.to_string()),
            model: None,
            thinking: None,
            show_thinking: None,
            openai_base_url: None,
            allowed_tools: None,
            execpolicy_rules: None,
        },
    )
    .await?;
    Ok(())
}

async fn setup_test_thread_mode_shared(
    repo_dir: &Path,
    mode_name: &str,
    mode_yaml: &str,
) -> anyhow::Result<(Server, ThreadId)> {
    write_modes_yaml_shared(repo_dir, mode_yaml).await?;
    let server = build_test_server_shared(repo_dir.join(".omne_data"));
    let thread_id = create_test_thread_shared(&server, repo_dir.to_path_buf()).await?;
    configure_test_thread_mode_shared(&server, thread_id, mode_name).await?;
    Ok((server, thread_id))
}
