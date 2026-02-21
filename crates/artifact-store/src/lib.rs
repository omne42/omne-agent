use std::path::{Path, PathBuf};

use anyhow::Context;
use omne_protocol::{ArtifactId, ArtifactMetadata};
use time::OffsetDateTime;

pub fn user_artifacts_dir_for_thread(thread_dir: &Path) -> PathBuf {
    thread_dir.join("artifacts").join("user")
}

pub fn user_artifact_paths(thread_dir: &Path, artifact_id: ArtifactId) -> (PathBuf, PathBuf) {
    let dir = user_artifacts_dir_for_thread(thread_dir);
    (
        dir.join(format!("{artifact_id}.md")),
        dir.join(format!("{artifact_id}.metadata.json")),
    )
}

pub fn user_artifact_history_dir_for_thread(thread_dir: &Path, artifact_id: ArtifactId) -> PathBuf {
    user_artifacts_dir_for_thread(thread_dir)
        .join("history")
        .join(artifact_id.to_string())
}

pub fn user_artifact_history_path(
    thread_dir: &Path,
    artifact_id: ArtifactId,
    version: u32,
) -> PathBuf {
    user_artifact_history_dir_for_thread(thread_dir, artifact_id).join(format!("v{version:04}.md"))
}

pub async fn read_artifact_metadata(path: &Path) -> anyhow::Result<ArtifactMetadata> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let meta = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse artifact metadata {}", path.display()))?;
    Ok(meta)
}

pub async fn write_file_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create dir {}", parent.display()))?;

    let pid = std::process::id();
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let tmp_path = path.with_extension(format!("tmp.{pid}.{nanos}"));

    tokio::fs::write(&tmp_path, bytes)
        .await
        .with_context(|| format!("write {}", tmp_path.display()))?;

    if let Err(err) = tokio::fs::rename(&tmp_path, path).await {
        if matches!(
            err.kind(),
            std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
        ) {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(remove_err) if remove_err.kind() == std::io::ErrorKind::NotFound => {}
                Err(remove_err) => {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                    return Err(remove_err)
                        .with_context(|| format!("remove old {}", path.display()));
                }
            }
            if let Err(rename_err) = tokio::fs::rename(&tmp_path, path).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(rename_err).with_context(|| {
                    format!("rename {} -> {}", tmp_path.display(), path.display())
                });
            }
        } else {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(err)
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
        }
    }

    Ok(())
}
