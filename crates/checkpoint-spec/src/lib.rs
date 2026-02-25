use std::path::Path;

use anyhow::Context;
use omne_protocol::{CheckpointId, ThreadId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const CHECKPOINT_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointManifestV1 {
    pub version: u32,
    pub checkpoint_id: CheckpointId,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub source: CheckpointSource,
    pub snapshot_ref: String,
    pub stats: CheckpointStats,
    pub excluded: CheckpointExcluded,
    pub size_limits: CheckpointSizeLimits,
    pub ignored_globs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSource {
    pub thread_id: ThreadId,
    pub cwd: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CheckpointStats {
    pub file_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CheckpointExcluded {
    pub symlink_count: u64,
    pub oversize_count: u64,
    pub secret_count: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CheckpointSizeLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

pub fn parse_manifest(raw: &str, manifest_path: &Path) -> anyhow::Result<CheckpointManifestV1> {
    serde_json::from_str(raw).with_context(|| format!("parse {}", manifest_path.display()))
}

pub async fn read_manifest(manifest_path: &Path) -> anyhow::Result<CheckpointManifestV1> {
    let raw = tokio::fs::read_to_string(manifest_path)
        .await
        .with_context(|| format!("read {}", manifest_path.display()))?;
    parse_manifest(&raw, manifest_path)
}

pub async fn write_manifest(
    manifest_path: &Path,
    manifest: &CheckpointManifestV1,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest).context("serialize checkpoint manifest")?;
    if let Some(parent) = manifest_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    tokio::fs::write(manifest_path, bytes)
        .await
        .with_context(|| format!("write {}", manifest_path.display()))?;
    Ok(())
}
