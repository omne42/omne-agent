use super::*;

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
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
        unsafe {
            for (key, value) in vars {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
        Self { prev }
    }
}

impl Drop for EnvVarResetGuard {
    fn drop(&mut self) {
        unsafe {
            for (key, value) in self.prev.drain(..) {
                match value {
                    Some(v) => std::env::set_var(&key, v),
                    None => std::env::remove_var(&key),
                }
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
