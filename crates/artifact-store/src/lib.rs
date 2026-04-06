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

pub fn user_artifact_history_metadata_path(
    thread_dir: &Path,
    artifact_id: ArtifactId,
    version: u32,
) -> PathBuf {
    user_artifact_history_dir_for_thread(thread_dir, artifact_id)
        .join(format!("v{version:04}.metadata.json"))
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
    tighten_dir_permissions_best_effort(parent).await;

    let pid = std::process::id();
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    let tmp_path = path.with_extension(format!("tmp.{pid}.{nanos}"));

    tokio::fs::write(&tmp_path, bytes)
        .await
        .with_context(|| format!("write {}", tmp_path.display()))?;
    tighten_file_permissions_best_effort(&tmp_path).await;

    if let Err(err) = tokio::fs::rename(&tmp_path, path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(err)
            .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
    }

    tighten_file_permissions_best_effort(path).await;
    Ok(())
}

#[cfg(unix)]
async fn tighten_dir_permissions_best_effort(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let perm = std::fs::Permissions::from_mode(0o700);
    let _ = tokio::fs::set_permissions(path, perm).await;
}

#[cfg(not(unix))]
async fn tighten_dir_permissions_best_effort(_path: &Path) {}

#[cfg(unix)]
async fn tighten_file_permissions_best_effort(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let perm = std::fs::Permissions::from_mode(0o600);
    let _ = tokio::fs::set_permissions(path, perm).await;
}

#[cfg(not(unix))]
async fn tighten_file_permissions_best_effort(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn write_file_atomic_tightens_permissions() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("artifacts/user/demo.md");
        write_file_atomic(&path, b"hello").await?;

        let file_mode = tokio::fs::metadata(&path).await?.permissions().mode() & 0o777u32;
        assert_eq!(file_mode, 0o600);
        let parent_mode = tokio::fs::metadata(path.parent().expect("parent"))
            .await?
            .permissions()
            .mode()
            & 0o777u32;
        assert_eq!(parent_mode, 0o700);
        Ok(())
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn write_file_atomic_replaces_existing_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("artifacts/user/demo.md");

        write_file_atomic(&path, b"old").await?;
        write_file_atomic(&path, b"new").await?;

        assert_eq!(tokio::fs::read(&path).await?, b"new");
        Ok(())
    }
}
