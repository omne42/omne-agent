use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use diffy::{Patch, apply};
use globset::Glob;
use omne_agent_core::{AgentPaths, ThreadStore};
use omne_agent_execpolicy::{Decision as ExecDecision, RuleMatch as ExecRuleMatch};
use omne_agent_protocol::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, EventSeq, ProcessId, ThreadEvent, ThreadId,
    TurnId, TurnStatus,
};
use regex::Regex;
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
    "OMNE_AGENT_OPENAI_API_KEY",
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

const DEFAULT_CARGO_TARGET_WARNING_BYTES: u64 = 200 * 1024 * 1024 * 1024;
const DEFAULT_CARGO_TARGET_CHECK_DEBOUNCE_MS: u64 = 60_000;
const DEFAULT_CARGO_TARGET_REPORT_DEBOUNCE_MS: u64 = 30 * 60_000;

const OMNE_AGENT_HARDENING_ENV: &str = "OMNE_AGENT_HARDENING";
const HARDENING_ITEM_RLIMIT_CORE: &str = "rlimit_core";
const HARDENING_ITEM_UMASK: &str = "umask";
const HARDENING_ITEM_DUMPABLE: &str = "prctl_dumpable";
const HARDENING_ITEM_ENV_GIT_TERMINAL_PROMPT: &str = "env.GIT_TERMINAL_PROMPT";
const HARDENING_ITEM_ENV_NO_COLOR: &str = "env.NO_COLOR";
const HARDENING_ITEM_ENV_PAGER: &str = "env.PAGER";

fn process_log_max_bytes_per_part() -> u64 {
    std::env::var("OMNE_AGENT_PROCESS_LOG_MAX_BYTES_PER_PART")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_PROCESS_LOG_MAX_BYTES_PER_PART))
        .unwrap_or(DEFAULT_PROCESS_LOG_MAX_BYTES_PER_PART)
}

fn process_idle_window() -> Option<Duration> {
    let value = std::env::var("OMNE_AGENT_PROCESS_IDLE_WINDOW_SECONDS")
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
    let value = std::env::var("OMNE_AGENT_THREAD_DISK_WARNING_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_THREAD_DISK_WARNING_BYTES);
    if value == 0 { None } else { Some(value) }
}

fn thread_disk_check_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("OMNE_AGENT_THREAD_DISK_CHECK_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_THREAD_DISK_CHECK_DEBOUNCE_MS),
    )
}

fn thread_disk_report_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("OMNE_AGENT_THREAD_DISK_REPORT_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_THREAD_DISK_REPORT_DEBOUNCE_MS),
    )
}

fn cargo_target_warning_threshold_bytes() -> Option<u64> {
    let value = std::env::var("OMNE_AGENT_CARGO_TARGET_WARNING_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_CARGO_TARGET_WARNING_BYTES);
    if value == 0 { None } else { Some(value) }
}

fn cargo_target_check_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("OMNE_AGENT_CARGO_TARGET_CHECK_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_CARGO_TARGET_CHECK_DEBOUNCE_MS),
    )
}

fn cargo_target_report_debounce() -> Duration {
    Duration::from_millis(
        std::env::var("OMNE_AGENT_CARGO_TARGET_REPORT_DEBOUNCE_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_CARGO_TARGET_REPORT_DEBOUNCE_MS),
    )
}

fn scrub_child_process_env(cmd: &mut Command) {
    for key in CHILD_PROCESS_ENV_SCRUB_KEYS {
        cmd.env_remove(key);
    }
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
                "{OMNE_AGENT_HARDENING_ENV} must be `off` or `best_effort` (got empty string)"
            );
        }
        match raw.to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "best_effort" => Ok(Self::BestEffort),
            _ => anyhow::bail!(
                "{OMNE_AGENT_HARDENING_ENV} must be `off` or `best_effort` (got `{raw}`)"
            ),
        }
    }

    fn from_env() -> anyhow::Result<Self> {
        match std::env::var(OMNE_AGENT_HARDENING_ENV) {
            Ok(value) => Self::parse(Some(&value)),
            Err(std::env::VarError::NotPresent) => Ok(Self::BestEffort),
            Err(err) => Err(err).context(format!("read {OMNE_AGENT_HARDENING_ENV}")),
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

fn skip_hardening_items(reason: &str) {
    log_hardening_skipped(HARDENING_ITEM_RLIMIT_CORE, reason);
    log_hardening_skipped(HARDENING_ITEM_UMASK, reason);
    log_hardening_skipped(HARDENING_ITEM_DUMPABLE, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_GIT_TERMINAL_PROMPT, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_NO_COLOR, reason);
    log_hardening_skipped(HARDENING_ITEM_ENV_PAGER, reason);
}

fn apply_pre_main_hardening() -> anyhow::Result<()> {
    let mode = HardeningMode::from_env()?;
    let _ = HARDENING_MODE.set(mode);
    tracing::info!(hardening.mode = %mode, "hardening configured");

    if mode == HardeningMode::Off {
        skip_hardening_items("hardening disabled");
        return Ok(());
    }

    log_hardening_enabled(HARDENING_ITEM_ENV_GIT_TERMINAL_PROMPT);
    log_hardening_enabled(HARDENING_ITEM_ENV_NO_COLOR);
    log_hardening_enabled(HARDENING_ITEM_ENV_PAGER);

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
    }
    #[cfg(not(target_os = "linux"))]
    log_hardening_skipped(HARDENING_ITEM_DUMPABLE, "unsupported platform");

    Ok(())
}

fn apply_child_process_env_defaults(
    cmd: &mut Command,
    extra_env: Option<&BTreeMap<String, String>>,
) {
    if hardening_mode() == HardeningMode::Off {
        return;
    }

    fn apply_default(
        cmd: &mut Command,
        extra_env: Option<&BTreeMap<String, String>>,
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
    }

    apply_default(cmd, extra_env, "GIT_TERMINAL_PROMPT", "0");
    apply_default(cmd, extra_env, "NO_COLOR", "1");
    apply_default(cmd, extra_env, "PAGER", "cat");
}
