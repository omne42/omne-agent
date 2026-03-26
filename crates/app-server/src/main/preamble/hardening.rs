use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use omne_core::{PmPaths, ThreadStore};
use omne_execpolicy::{Decision as ExecDecision, RuleMatch as ExecRuleMatch};
use omne_protocol::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, EventSeq, ProcessId, ThreadEvent, ThreadId,
    TurnId, TurnStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use walkdir::WalkDir;

const CHILD_PROCESS_ENV_SCRUB_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "OMNE_OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENROUTER_API_KEY",
    "GEMINI_API_KEY",
];

const DEFAULT_PROCESS_LOG_MAX_BYTES_PER_PART: u64 = 8 * 1024 * 1024;
const MAX_PROCESS_LOG_MAX_BYTES_PER_PART: u64 = 512 * 1024 * 1024;

const DEFAULT_PROCESS_IDLE_WINDOW_SECONDS: u64 = 300;

const DEFAULT_THREAD_DISK_WARNING_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const DEFAULT_THREAD_DISK_CHECK_DEBOUNCE_MS: u64 = 30_000;
const DEFAULT_THREAD_DISK_REPORT_DEBOUNCE_MS: u64 = 30 * 60_000;

const OMNE_HARDENING_ENV: &str = "OMNE_HARDENING";
const HARDENING_ITEM_RLIMIT_CORE: &str = "rlimit_core";
const HARDENING_ITEM_UMASK: &str = "umask";
const HARDENING_ITEM_DUMPABLE: &str = "prctl_dumpable";
const HARDENING_ITEM_YAMA_PTRACE_SCOPE: &str = "linux.yama_ptrace_scope";
const HARDENING_ITEM_ENV_PRE_MAIN_SCRUB: &str = "env.pre_main_scrub";
const HARDENING_ITEM_ENV_CI: &str = "env.CI";
const HARDENING_ITEM_ENV_GIT_TERMINAL_PROMPT: &str = "env.GIT_TERMINAL_PROMPT";
const HARDENING_ITEM_ENV_NO_COLOR: &str = "env.NO_COLOR";
const HARDENING_ITEM_ENV_PAGER: &str = "env.PAGER";
const OMNE_HARDENING_SET_CI_ENV: &str = "OMNE_HARDENING_SET_CI";
const OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE_ENV: &str = "OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE";
const LINUX_YAMA_PTRACE_SCOPE_PATH: &str = "/proc/sys/kernel/yama/ptrace_scope";

const PRE_MAIN_ENV_SCRUB_KEYS: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_DEBUG",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_ROOT_PATH",
    "DYLD_SHARED_REGION",
];

fn process_log_max_bytes_per_part() -> u64 {
    std::env::var("OMNE_PROCESS_LOG_MAX_BYTES_PER_PART")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_PROCESS_LOG_MAX_BYTES_PER_PART))
        .unwrap_or(DEFAULT_PROCESS_LOG_MAX_BYTES_PER_PART)
}

fn process_idle_window() -> Option<Duration> {
    let value = std::env::var("OMNE_PROCESS_IDLE_WINDOW_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_PROCESS_IDLE_WINDOW_SECONDS);
    if value == 0 {
        None
    } else {
        Some(Duration::from_secs(value))
    }
}

fn thread_disk_warning_threshold_bytes() -> Option<u64> {
    let value = std::env::var("OMNE_THREAD_DISK_WARNING_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_THREAD_DISK_WARNING_BYTES);
    if value == 0 { None } else { Some(value) }
}

fn thread_disk_check_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("OMNE_THREAD_DISK_CHECK_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_THREAD_DISK_CHECK_DEBOUNCE_MS),
    )
}

fn thread_disk_report_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("OMNE_THREAD_DISK_REPORT_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_THREAD_DISK_REPORT_DEBOUNCE_MS),
    )
}

#[derive(Clone, Debug, Serialize)]
struct EffectiveEnvSummary {
    hardening_mode: String,
    scrubbed_keys: Vec<String>,
    allowlist_dropped_keys: Vec<String>,
    injected_defaults: BTreeMap<String, String>,
    configured_extra_scrub_keys: Vec<String>,
    configured_extra_scrub_patterns: Vec<String>,
    configured_allowed_env_keys: Vec<String>,
    configured_allowed_env_patterns: Vec<String>,
}

const OMNE_HARDENING_EXTRA_SCRUB_KEYS_ENV: &str = "OMNE_HARDENING_EXTRA_SCRUB_KEYS";
const OMNE_HARDENING_EXTRA_SCRUB_PATTERNS_ENV: &str = "OMNE_HARDENING_EXTRA_SCRUB_PATTERNS";
const OMNE_HARDENING_ALLOW_ENV_KEYS_ENV: &str = "OMNE_HARDENING_ALLOW_ENV_KEYS";
const OMNE_HARDENING_ALLOW_ENV_PATTERNS_ENV: &str = "OMNE_HARDENING_ALLOW_ENV_PATTERNS";

#[derive(Clone, Debug)]
struct EnvScrubConfig {
    extra_keys: BTreeSet<String>,
    extra_patterns: Vec<String>,
    extra_pattern_set: Option<GlobSet>,
    allowed_env_keys: BTreeSet<String>,
    allowed_env_patterns: Vec<String>,
    allowed_env_pattern_set: Option<GlobSet>,
}

impl EnvScrubConfig {
    fn parse_csv(raw: Option<&str>) -> Vec<String> {
        raw.unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    }

    fn build_pattern_set(patterns: &[String], kind: &str) -> anyhow::Result<Option<GlobSet>> {
        if patterns.is_empty() {
            return Ok(None);
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .with_context(|| format!("invalid hardening {kind} glob pattern `{pattern}`"))?;
            builder.add(glob);
        }
        Ok(Some(
            builder
                .build()
                .with_context(|| format!("build hardening {kind} glob set"))?,
        ))
    }

    fn from_raw(
        extra_keys_raw: Option<&str>,
        extra_patterns_raw: Option<&str>,
        allowed_env_keys_raw: Option<&str>,
        allowed_env_patterns_raw: Option<&str>,
    ) -> anyhow::Result<Self> {
        let extra_keys = Self::parse_csv(extra_keys_raw)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let extra_patterns = Self::parse_csv(extra_patterns_raw);
        let extra_pattern_set = Self::build_pattern_set(&extra_patterns, "scrub")?;
        let allowed_env_keys = Self::parse_csv(allowed_env_keys_raw)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let allowed_env_patterns = Self::parse_csv(allowed_env_patterns_raw);
        let allowed_env_pattern_set = Self::build_pattern_set(&allowed_env_patterns, "allow")?;
        Ok(Self {
            extra_keys,
            extra_patterns,
            extra_pattern_set,
            allowed_env_keys,
            allowed_env_patterns,
            allowed_env_pattern_set,
        })
    }

    fn from_env() -> anyhow::Result<Self> {
        Self::from_raw(
            std::env::var(OMNE_HARDENING_EXTRA_SCRUB_KEYS_ENV).ok().as_deref(),
            std::env::var(OMNE_HARDENING_EXTRA_SCRUB_PATTERNS_ENV)
                .ok()
                .as_deref(),
            std::env::var(OMNE_HARDENING_ALLOW_ENV_KEYS_ENV).ok().as_deref(),
            std::env::var(OMNE_HARDENING_ALLOW_ENV_PATTERNS_ENV)
                .ok()
                .as_deref(),
        )
        .with_context(|| {
            format!(
                "parse {OMNE_HARDENING_EXTRA_SCRUB_KEYS_ENV}/{OMNE_HARDENING_EXTRA_SCRUB_PATTERNS_ENV}/{OMNE_HARDENING_ALLOW_ENV_KEYS_ENV}/{OMNE_HARDENING_ALLOW_ENV_PATTERNS_ENV}"
            )
        })
    }

    fn matches_scrub_pattern(&self, key: &str) -> bool {
        self.extra_pattern_set
            .as_ref()
            .is_some_and(|set| set.is_match(key))
    }

    fn matches_allowed_env_pattern(&self, key: &str) -> bool {
        self.allowed_env_pattern_set
            .as_ref()
            .is_some_and(|set| set.is_match(key))
    }

    fn allowlist_enabled(&self) -> bool {
        !self.allowed_env_keys.is_empty() || !self.allowed_env_patterns.is_empty()
    }

    fn is_allowlisted_env_key(&self, key: &str) -> bool {
        self.allowed_env_keys.contains(key) || self.matches_allowed_env_pattern(key)
    }
}

fn scrub_child_process_env_with_config(
    cmd: &mut Command,
    extra_env: Option<&BTreeMap<String, String>>,
    config: &EnvScrubConfig,
) -> Vec<String> {
    let mut scrubbed_keys = BTreeSet::new();
    let present_in_env =
        |key: &str| std::env::var_os(key).is_some() || extra_env.is_some_and(|env| env.contains_key(key));

    for key in CHILD_PROCESS_ENV_SCRUB_KEYS {
        cmd.env_remove(key);
        if present_in_env(key) {
            scrubbed_keys.insert((*key).to_string());
        }
    }

    for key in &config.extra_keys {
        cmd.env_remove(key);
        if present_in_env(key) {
            scrubbed_keys.insert(key.clone());
        }
    }

    if config.extra_pattern_set.is_some() {
        let mut candidates = BTreeSet::new();
        candidates.extend(std::env::vars_os().filter_map(|(key, _)| key.into_string().ok()));
        if let Some(extra_env) = extra_env {
            candidates.extend(extra_env.keys().cloned());
        }
        for candidate in candidates {
            if config.matches_scrub_pattern(&candidate) {
                cmd.env_remove(&candidate);
                if present_in_env(&candidate) {
                    scrubbed_keys.insert(candidate);
                }
            }
        }
    }

    scrubbed_keys.into_iter().collect()
}

fn apply_allowlist_to_child_process_env(
    cmd: &mut Command,
    extra_env: Option<&BTreeMap<String, String>>,
    config: &EnvScrubConfig,
) -> Vec<String> {
    if !config.allowlist_enabled() {
        return Vec::new();
    }

    let mut dropped_keys = BTreeSet::new();
    for key in std::env::vars_os().filter_map(|(key, _)| key.into_string().ok()) {
        if extra_env.is_some_and(|env| env.contains_key(&key)) {
            continue;
        }
        if config.is_allowlisted_env_key(&key) {
            continue;
        }
        cmd.env_remove(&key);
        dropped_keys.insert(key);
    }
    dropped_keys.into_iter().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HardeningMode {
    Off,
    BestEffort,
}

static HARDENING_MODE: OnceLock<HardeningMode> = OnceLock::new();

fn hardening_mode() -> HardeningMode {
    HARDENING_MODE
        .get()
        .copied()
        .unwrap_or(HardeningMode::BestEffort)
}

impl HardeningMode {
    fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        let Some(raw) = value else {
            return Ok(Self::BestEffort);
        };
        let raw = raw.trim();
        if raw.is_empty() {
            anyhow::bail!(
                "{OMNE_HARDENING_ENV} must be `off` or `best_effort` (got empty string)"
            );
        }
        match raw.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "best_effort" => Ok(Self::BestEffort),
            _ => anyhow::bail!(
                "{OMNE_HARDENING_ENV} must be `off` or `best_effort` (got `{raw}`)"
            ),
        }
    }

    fn from_env() -> anyhow::Result<Self> {
        match std::env::var(OMNE_HARDENING_ENV) {
            Ok(value) => Self::parse(Some(&value)),
            Err(std::env::VarError::NotPresent) => Ok(Self::BestEffort),
            Err(err) => Err(err).context(format!("read {OMNE_HARDENING_ENV}")),
        }
    }
}

impl std::fmt::Display for HardeningMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HardeningMode::Off => write!(f, "off"),
            HardeningMode::BestEffort => write!(f, "best_effort"),
        }
    }
}

fn log_hardening_applied(item: &str) {
    tracing::info!(hardening.item = item, hardening.status = "applied", "hardening applied");
}

fn log_hardening_enabled(item: &str) {
    tracing::info!(hardening.item = item, hardening.status = "enabled", "hardening enabled");
}

fn log_hardening_skipped(item: &str, reason: &str) {
    tracing::info!(
        hardening.item = item,
        hardening.status = "skipped",
        hardening.reason = reason,
        "hardening skipped"
    );
}

fn log_hardening_failed(item: &str, err: &anyhow::Error) {
    tracing::warn!(
        hardening.item = item,
        hardening.status = "failed",
        error = %err,
        "hardening failed"
    );
}

fn collect_pre_main_env_scrub_targets<I>(env_keys: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let env_keys = env_keys.into_iter().collect::<HashSet<_>>();
    PRE_MAIN_ENV_SCRUB_KEYS
        .iter()
        .filter_map(|key| env_keys.contains(*key).then_some((*key).to_string()))
        .collect()
}

fn remove_process_env_pre_main(key: &str) {
    // SAFETY:
    // - Rust 2024 marks process-environment mutation unsafe because the environment is shared
    //   process-global state.
    // - This helper is only used during pre-main hardening, before the runtime starts worker
    //   threads or any background tasks that could concurrently read/write the environment.
    // - We must mutate the current process environment here because subsequent child-process
    //   launches inherit from this process unless scrubbed up front.
    unsafe { std::env::remove_var(key) };
}

fn set_process_env_pre_main(key: &str, value: &str) {
    // SAFETY:
    // - Same pre-main boundary and invariants as `remove_process_env_pre_main`: this runs before
    //   any concurrency exists in the process, so there is no concurrent environment access.
    // - We intentionally keep the unsafe boundary narrow and only use it for startup hardening
    //   defaults that must affect the current process and its future children.
    unsafe { std::env::set_var(key, value) };
}

fn apply_pre_main_env_scrub() -> Vec<String> {
    let scrubbed_keys = collect_pre_main_env_scrub_targets(
        std::env::vars_os().filter_map(|(key, _)| key.into_string().ok()),
    );
    for key in &scrubbed_keys {
        remove_process_env_pre_main(key);
    }
    scrubbed_keys
}

fn parse_bool_env(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn pre_main_set_ci_enabled() -> anyhow::Result<bool> {
    let Some(raw) = std::env::var(OMNE_HARDENING_SET_CI_ENV).ok() else {
        return Ok(false);
    };
    let Some(value) = parse_bool_env(&raw) else {
        anyhow::bail!("{OMNE_HARDENING_SET_CI_ENV} must be a boolean-like value");
    };
    Ok(value)
}

fn parse_linux_yama_ptrace_scope(raw: Option<&str>) -> anyhow::Result<Option<u8>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("{OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE_ENV} must be 0..3 (got empty string)");
    }
    let scope: u8 = raw
        .parse()
        .with_context(|| format!("parse {OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE_ENV}"))?;
    if scope > 3 {
        anyhow::bail!("{OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE_ENV} must be 0..3 (got {scope})");
    }
    Ok(Some(scope))
}

fn write_linux_yama_ptrace_scope(path: &Path, scope: u8) -> anyhow::Result<()> {
    std::fs::write(path, scope.to_string())
        .with_context(|| format!("write yama ptrace scope {} to {}", scope, path.display()))
}

fn skip_hardening_items(reason: &str) {
    log_hardening_skipped(HARDENING_ITEM_RLIMIT_CORE, reason);
    log_hardening_skipped(HARDENING_ITEM_UMASK, reason);
    log_hardening_skipped(HARDENING_ITEM_DUMPABLE, reason);
    log_hardening_skipped(HARDENING_ITEM_YAMA_PTRACE_SCOPE, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_PRE_MAIN_SCRUB, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_CI, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_GIT_TERMINAL_PROMPT, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_NO_COLOR, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_PAGER, reason);
}

fn apply_pre_main_hardening() -> anyhow::Result<()> {
    let mode = HardeningMode::from_env()?;
    if HARDENING_MODE.set(mode).is_err() {
        tracing::warn!("hardening mode already initialized; keeping the first value");
    }
    tracing::info!(hardening.mode = %mode, "hardening configured");

    if mode == HardeningMode::Off {
        skip_hardening_items("hardening disabled");
        return Ok(());
    }

    log_hardening_enabled(HARDENING_ITEM_ENV_GIT_TERMINAL_PROMPT);
    log_hardening_enabled(HARDENING_ITEM_ENV_NO_COLOR);
    log_hardening_enabled(HARDENING_ITEM_ENV_PAGER);
    log_hardening_enabled(HARDENING_ITEM_ENV_PRE_MAIN_SCRUB);

    let scrubbed_keys = apply_pre_main_env_scrub();
    log_hardening_applied(HARDENING_ITEM_ENV_PRE_MAIN_SCRUB);
    tracing::info!(
        hardening.item = HARDENING_ITEM_ENV_PRE_MAIN_SCRUB,
        hardening.scrubbed_count = scrubbed_keys.len(),
        hardening.scrubbed_keys = ?scrubbed_keys,
        "hardening pre-main env scrub result"
    );

    if pre_main_set_ci_enabled()? {
        log_hardening_enabled(HARDENING_ITEM_ENV_CI);
        if std::env::var_os("CI").is_none() {
            set_process_env_pre_main("CI", "1");
            log_hardening_applied(HARDENING_ITEM_ENV_CI);
        } else {
            log_hardening_skipped(HARDENING_ITEM_ENV_CI, "already set");
        }
    } else {
        log_hardening_skipped(HARDENING_ITEM_ENV_CI, "disabled by config");
    }

    #[cfg(unix)]
    {
        let result = {
            use nix::sys::resource::{Resource, setrlimit};
            setrlimit(Resource::RLIMIT_CORE, 0, 0).context("set RLIMIT_CORE=0")
        };
        match result {
            Ok(()) => log_hardening_applied(HARDENING_ITEM_RLIMIT_CORE),
            Err(err) => log_hardening_failed(HARDENING_ITEM_RLIMIT_CORE, &err),
        }
    }
    #[cfg(not(unix))]
    log_hardening_skipped(HARDENING_ITEM_RLIMIT_CORE, "unsupported platform");

    #[cfg(unix)]
    {
        use nix::sys::stat::{umask, Mode};
        umask(Mode::from_bits_truncate(0o077));
        log_hardening_applied(HARDENING_ITEM_UMASK);
    }
    #[cfg(not(unix))]
    log_hardening_skipped(HARDENING_ITEM_UMASK, "unsupported platform");

    #[cfg(target_os = "linux")]
    {
        let result = nix::sys::prctl::set_dumpable(false)
            .context("set PR_SET_DUMPABLE=false");
        match result {
            Ok(()) => log_hardening_applied(HARDENING_ITEM_DUMPABLE),
            Err(err) => log_hardening_failed(HARDENING_ITEM_DUMPABLE, &err),
        }

        let yama_scope = parse_linux_yama_ptrace_scope(
            std::env::var(OMNE_HARDENING_LINUX_YAMA_PTRACE_SCOPE_ENV)
                .ok()
                .as_deref(),
        )?;
        match yama_scope {
            Some(scope) => {
                let path = Path::new(LINUX_YAMA_PTRACE_SCOPE_PATH);
                if !path.exists() {
                    log_hardening_skipped(
                        HARDENING_ITEM_YAMA_PTRACE_SCOPE,
                        "unsupported kernel (missing /proc/sys/kernel/yama/ptrace_scope)",
                    );
                } else {
                    match write_linux_yama_ptrace_scope(path, scope) {
                        Ok(()) => log_hardening_applied(HARDENING_ITEM_YAMA_PTRACE_SCOPE),
                        Err(err) => log_hardening_failed(HARDENING_ITEM_YAMA_PTRACE_SCOPE, &err),
                    }
                }
            }
            None => log_hardening_skipped(HARDENING_ITEM_YAMA_PTRACE_SCOPE, "disabled by config"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    log_hardening_skipped(HARDENING_ITEM_DUMPABLE, "unsupported platform");
    #[cfg(not(target_os = "linux"))]
    log_hardening_skipped(HARDENING_ITEM_YAMA_PTRACE_SCOPE, "unsupported platform");

    Ok(())
}

fn apply_child_process_env_defaults(
    cmd: &mut Command,
    extra_env: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let mut injected_defaults = BTreeMap::new();
    if hardening_mode() == HardeningMode::Off {
        return injected_defaults;
    }

    fn apply_default(
        cmd: &mut Command,
        extra_env: Option<&BTreeMap<String, String>>,
        injected_defaults: &mut BTreeMap<String, String>,
        key: &'static str,
        value: &'static str,
    ) {
        if extra_env.is_some_and(|env| env.contains_key(key)) {
            return;
        }
        if std::env::var_os(key).is_some() {
            return;
        }
        cmd.env(key, value);
        injected_defaults.insert(key.to_string(), value.to_string());
    }

    apply_default(
        cmd,
        extra_env,
        &mut injected_defaults,
        "GIT_TERMINAL_PROMPT",
        "0",
    );
    apply_default(cmd, extra_env, &mut injected_defaults, "NO_COLOR", "1");
    apply_default(cmd, extra_env, &mut injected_defaults, "PAGER", "cat");
    injected_defaults
}

fn apply_child_process_hardening_with_config(
    cmd: &mut Command,
    extra_env: Option<&BTreeMap<String, String>>,
    config: &EnvScrubConfig,
) -> EffectiveEnvSummary {
    let mut configured_extra_scrub_keys = config.extra_keys.iter().cloned().collect::<Vec<_>>();
    configured_extra_scrub_keys.sort();
    let configured_extra_scrub_patterns = config.extra_patterns.clone();
    let mut configured_allowed_env_keys = config
        .allowed_env_keys
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    configured_allowed_env_keys.sort();
    let configured_allowed_env_patterns = config.allowed_env_patterns.clone();

    let scrubbed_keys = scrub_child_process_env_with_config(cmd, extra_env, config);
    let allowlist_dropped_keys = apply_allowlist_to_child_process_env(cmd, extra_env, config);
    let injected_defaults = apply_child_process_env_defaults(cmd, extra_env);
    EffectiveEnvSummary {
        hardening_mode: hardening_mode().to_string(),
        scrubbed_keys,
        allowlist_dropped_keys,
        injected_defaults,
        configured_extra_scrub_keys,
        configured_extra_scrub_patterns,
        configured_allowed_env_keys,
        configured_allowed_env_patterns,
    }
}

fn apply_child_process_hardening(
    cmd: &mut Command,
    extra_env: Option<&BTreeMap<String, String>>,
) -> anyhow::Result<EffectiveEnvSummary> {
    let config = EnvScrubConfig::from_env()?;
    Ok(apply_child_process_hardening_with_config(
        cmd, extra_env, &config,
    ))
}

#[cfg(test)]
mod hardening_env_scrub_tests {
    use super::*;

    #[test]
    fn env_scrub_config_parses_extra_keys_and_patterns() -> anyhow::Result<()> {
        let config = EnvScrubConfig::from_raw(
            Some("OPENAI_API_KEY, APP_SECRET , OPENAI_API_KEY"),
            Some("*_TOKEN,*secret*"),
            Some("PATH,HOME"),
            Some("OMNE_*"),
        )?;
        assert!(config.extra_keys.contains("OPENAI_API_KEY"));
        assert!(config.extra_keys.contains("APP_SECRET"));
        assert_eq!(config.extra_keys.len(), 2);
        assert_eq!(config.extra_patterns.len(), 2);
        assert!(config.matches_scrub_pattern("MY_TOKEN"));
        assert!(config.matches_scrub_pattern("mysecretname"));
        assert!(config.allowed_env_keys.contains("PATH"));
        assert!(config.allowed_env_keys.contains("HOME"));
        assert!(config.matches_allowed_env_pattern("OMNE_EXECVE_TOKEN"));
        Ok(())
    }

    #[test]
    fn env_scrub_config_rejects_invalid_pattern() {
        let err = EnvScrubConfig::from_raw(None, Some("[invalid"), None, None)
            .expect_err("invalid pattern should fail");
        assert!(err.to_string().contains("invalid hardening scrub glob pattern"));
    }

    #[test]
    fn env_scrub_config_rejects_invalid_allow_pattern() {
        let err = EnvScrubConfig::from_raw(None, None, None, Some("[invalid"))
            .expect_err("invalid allow pattern should fail");
        assert!(err.to_string().contains("invalid hardening allow glob pattern"));
    }

    #[test]
    fn scrub_child_process_env_with_config_reports_scrubbed_keys() -> anyhow::Result<()> {
        let config = EnvScrubConfig::from_raw(Some("APP_SECRET"), Some("*_TOKEN"), None, None)?;
        let mut cmd = Command::new("echo");
        let mut extra_env = BTreeMap::new();
        extra_env.insert("APP_SECRET".to_string(), "1".to_string());
        extra_env.insert("SESSION_TOKEN".to_string(), "2".to_string());

        let summary = apply_child_process_hardening_with_config(&mut cmd, Some(&extra_env), &config);
        assert!(
            summary
                .scrubbed_keys
                .iter()
                .any(|key| key == "APP_SECRET")
        );
        assert!(
            summary
                .scrubbed_keys
                .iter()
                .any(|key| key == "SESSION_TOKEN")
        );
        assert_eq!(
            summary.configured_extra_scrub_keys,
            vec!["APP_SECRET".to_string()]
        );
        assert_eq!(
            summary.configured_extra_scrub_patterns,
            vec!["*_TOKEN".to_string()]
        );
        assert!(summary.allowlist_dropped_keys.is_empty());
        assert!(summary.configured_allowed_env_keys.is_empty());
        assert!(summary.configured_allowed_env_patterns.is_empty());
        Ok(())
    }

    #[test]
    fn allowlist_drops_inherited_keys_and_keeps_explicit_env() -> anyhow::Result<()> {
        let config = EnvScrubConfig::from_raw(None, None, Some("PATH"), None)?;
        let mut cmd = Command::new("echo");
        let mut extra_env = BTreeMap::new();
        extra_env.insert("OMNE_TEST_EXPLICIT".to_string(), "1".to_string());

        let summary = apply_child_process_hardening_with_config(&mut cmd, Some(&extra_env), &config);
        assert_eq!(
            summary.configured_allowed_env_keys,
            vec!["PATH".to_string()]
        );
        assert!(summary.configured_allowed_env_patterns.is_empty());
        assert!(summary.allowlist_dropped_keys.iter().all(|key| key != "PATH"));
        assert!(
            summary
                .allowlist_dropped_keys
                .iter()
                .all(|key| key != "OMNE_TEST_EXPLICIT")
        );
        let has_non_allowlisted_inherited = std::env::vars_os()
            .filter_map(|(key, _)| key.into_string().ok())
            .any(|key| key != "PATH");
        if has_non_allowlisted_inherited {
            assert!(!summary.allowlist_dropped_keys.is_empty());
        }
        Ok(())
    }
}

#[cfg(test)]
mod pre_main_hardening_tests {
    use super::*;

    #[test]
    fn collect_pre_main_env_scrub_targets_keeps_known_keys_in_stable_order() {
        let targets = collect_pre_main_env_scrub_targets(vec![
            "PATH".to_string(),
            "DYLD_INSERT_LIBRARIES".to_string(),
            "LD_PRELOAD".to_string(),
            "LD_DEBUG".to_string(),
        ]);
        assert_eq!(
            targets,
            vec![
                "LD_PRELOAD".to_string(),
                "LD_DEBUG".to_string(),
                "DYLD_INSERT_LIBRARIES".to_string(),
            ]
        );
    }

    #[test]
    fn collect_pre_main_env_scrub_targets_ignores_unknown_keys() {
        let targets = collect_pre_main_env_scrub_targets(vec![
            "PATH".to_string(),
            "HOME".to_string(),
            "CUSTOM_DEBUG".to_string(),
        ]);
        assert!(targets.is_empty());
    }

    #[test]
    fn parse_bool_env_accepts_common_values() {
        for value in ["1", "true", "yes", "on", " TRUE "] {
            assert_eq!(parse_bool_env(value), Some(true));
        }
        for value in ["0", "false", "no", "off", " Off "] {
            assert_eq!(parse_bool_env(value), Some(false));
        }
    }

    #[test]
    fn parse_bool_env_rejects_invalid_values() {
        for value in ["", "2", "enable", "disabled"] {
            assert_eq!(parse_bool_env(value), None);
        }
    }

    #[test]
    fn parse_linux_yama_ptrace_scope_accepts_valid_values() -> anyhow::Result<()> {
        for value in ["0", "1", "2", "3"] {
            assert_eq!(
                parse_linux_yama_ptrace_scope(Some(value))?,
                Some(value.parse::<u8>()?)
            );
        }
        assert_eq!(parse_linux_yama_ptrace_scope(None)?, None);
        Ok(())
    }

    #[test]
    fn parse_linux_yama_ptrace_scope_rejects_invalid_values() {
        for value in ["", "4", "abc", "-1"] {
            assert!(parse_linux_yama_ptrace_scope(Some(value)).is_err());
        }
    }

    #[test]
    fn write_linux_yama_ptrace_scope_writes_expected_value() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let target = tmp.path().join("ptrace_scope");
        std::fs::write(&target, "0")?;
        write_linux_yama_ptrace_scope(&target, 3)?;
        assert_eq!(std::fs::read_to_string(&target)?, "3");
        Ok(())
    }
}
