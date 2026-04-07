fn app_server_process_env_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::const_new(()))
}

fn set_locked_process_env(key: &str, value: &str) {
    // SAFETY:
    // - Rust 2024 requires `unsafe` for process-environment mutation because the environment is a
    //   process-global table shared with libc and can race with concurrent readers/writers.
    // - Every app-server test that mutates environment variables must hold
    //   `app_server_process_env_lock()` for the entire override lifetime.
    // - That crate-wide mutex is the single audited serialization point for `set_var`/`remove_var`
    //   in this test binary, so this helper is the only place where the unsafe boundary exists.
    // - We intentionally keep this helper tiny because there is no safe in-process alternative for
    //   exercising env-driven behavior through `std::env`.
    unsafe { std::env::set_var(key, value) };
}

fn remove_locked_process_env(key: &str) {
    // SAFETY:
    // - Same boundary and invariants as `set_locked_process_env`: callers must hold the shared
    //   app-server env mutex, and this helper is the single place where test code mutates the
    //   process-global environment.
    // - Rust does not provide a safe API for in-process environment removal on edition 2024.
    unsafe { std::env::remove_var(key) };
}

fn restore_locked_process_env(key: &str, previous: Option<&str>) {
    match previous {
        Some(value) => set_locked_process_env(key, value),
        None => remove_locked_process_env(key),
    }
}

fn build_test_server_shared(omne_root: PathBuf) -> Server {
    build_test_server_shared_with_cwd(omne_root.clone(), omne_root)
}

fn build_test_server_shared_with_cwd(cwd: PathBuf, omne_root: PathBuf) -> Server {
    let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
    Server {
        cwd,
        notify_tx,
        thread_store: ThreadStore::new(PmPaths::new(omne_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        thread_loads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        thread_observation_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
                role: None,
            model: None,
            clear_model: false,
            thinking: None,
            clear_thinking: false,
            show_thinking: None,
            clear_show_thinking: false,
            openai_base_url: None,
            clear_openai_base_url: false,
            allowed_tools: None,
            execpolicy_rules: None,
        clear_execpolicy_rules: false,
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
