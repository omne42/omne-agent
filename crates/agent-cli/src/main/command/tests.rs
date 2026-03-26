use super::*;

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn set_locked_process_env(key: &str, value: &str) {
    // SAFETY:
    // - Rust 2024 requires `unsafe` for process-environment mutation because the environment is a
    //   process-global libc data structure.
    // - Every agent-cli test in this binary that mutates env goes through `with_env_vars`, which
    //   holds `env_test_lock()` across the full override lifetime and serializes these mutations.
    // - We need the real process environment here because the code under test reads via `std::env`;
    //   there is no safe in-process substitute for that boundary.
    unsafe { std::env::set_var(key, value) };
}

fn remove_locked_process_env(key: &str) {
    // SAFETY:
    // - Same boundary and invariants as `set_locked_process_env`: callers hold the process-wide
    //   test mutex for this binary, and this helper is the single audited env-removal boundary.
    unsafe { std::env::remove_var(key) };
}

struct EnvVarResetGuard {
    prev: Vec<(String, Option<String>)>,
}

impl EnvVarResetGuard {
    fn new(vars: &[(&str, Option<&str>)]) -> Self {
        let prev = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var(key).ok()))
            .collect();
        for (key, value) in vars {
            match value {
                Some(v) => set_locked_process_env(key, v),
                None => remove_locked_process_env(key),
            }
        }
        Self { prev }
    }
}

impl Drop for EnvVarResetGuard {
    fn drop(&mut self) {
        for (key, value) in self.prev.drain(..) {
            match value {
                Some(v) => set_locked_process_env(&key, &v),
                None => remove_locked_process_env(&key),
            }
        }
    }
}

fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = env_test_lock()
        .lock()
        .expect("env test lock must not be poisoned");
    let _env_guard = EnvVarResetGuard::new(vars);
    f()
}

fn with_env_var<T>(key: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
    with_env_vars(&[(key, value)], f)
}

fn test_scheduling() -> FanOutSchedulingParams {
    FanOutSchedulingParams {
        env_max_concurrent_subagents: 4,
        effective_concurrency_limit: 3,
        priority_aging_rounds: 5,
    }
}

fn artifact_write_params_json(
    params: omne_app_server_protocol::ArtifactWriteParams,
) -> serde_json::Value {
    serde_json::to_value(params).expect("serialize ArtifactWriteParams")
}

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
}

fn thread_start_response_with_auto_hook(
    auto_hook: omne_app_server_protocol::ThreadAutoHookResponse,
) -> omne_app_server_protocol::ThreadStartResponse {
    omne_app_server_protocol::ThreadStartResponse {
        thread_id: ThreadId::new(),
        log_path: "/tmp/.omne_data/threads/thread.log".to_string(),
        last_seq: 1,
        auto_hook,
    }
}

#[path = "tests/validate.rs"]
mod validate;

#[path = "tests/validate_run.rs"]
mod validate_run;

#[path = "tests/workflow_scheduling.rs"]
mod workflow_scheduling;

#[path = "tests/fan_out_completion.rs"]
mod fan_out_completion;

#[path = "tests/fan_out_summary.rs"]
mod fan_out_summary;

#[path = "tests/fan_out_env.rs"]
mod fan_out_env;

#[path = "tests/fan_out_rendering_fan_out.rs"]
mod fan_out_rendering_fan_out;

#[path = "tests/fan_out_rendering_fan_in.rs"]
mod fan_out_rendering_fan_in;

#[path = "tests/fan_out_rendering_reads.rs"]
mod fan_out_rendering_reads;

#[path = "tests/fan_out_approvals.rs"]
mod fan_out_approvals;

#[path = "tests/fan_out_linkage.rs"]
mod fan_out_linkage;

#[path = "tests/fan_out_artifact_params.rs"]
mod fan_out_artifact_params;

#[path = "tests/core.rs"]
mod core;
