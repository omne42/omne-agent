use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::SessionId;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn put_json(&self, key: &str, value: &Value) -> anyhow::Result<()>;
    async fn get_json(&self, key: &str) -> anyhow::Result<Option<Value>>;
    async fn list_prefix(&self, prefix: &str) -> anyhow::Result<Vec<String>>;
}

#[derive(Clone, Debug)]
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub async fn list_session_ids(&self) -> anyhow::Result<Vec<SessionId>> {
        let dir = self.root.join("sessions");
        let mut read_dir = match tokio::fs::read_dir(&dir).await {
            Ok(read_dir) => read_dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
        };

        let mut ids = Vec::new();
        while let Some(entry) = read_dir.next_entry().await? {
            let file_type = entry.file_type().await?;
            if !file_type.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Ok(id) = name.parse::<SessionId>() else {
                continue;
            };
            ids.push(id);
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn key_to_path(&self, key: &str) -> anyhow::Result<PathBuf> {
        let mut path = self.root.clone();
        for segment in key.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                anyhow::bail!("invalid storage key segment: {segment:?}");
            }
            if segment.contains('\\') {
                anyhow::bail!("invalid storage key segment: {segment:?}");
            }
            path = path.join(segment);
        }
        let Some(file_name) = path.file_name() else {
            anyhow::bail!("invalid storage key (missing file name): {key:?}");
        };
        let mut file_name = file_name.to_os_string();
        file_name.push(".json");
        path.set_file_name(file_name);
        Ok(path)
    }

    fn path_to_key(&self, path: &Path) -> anyhow::Result<String> {
        let rel = path
            .strip_prefix(&self.root)
            .context("path is outside storage root")?;
        let Some(stem) = rel.to_str() else {
            anyhow::bail!("non-utf8 storage path: {rel:?}");
        };
        let Some(key) = stem.strip_suffix(".json") else {
            anyhow::bail!("storage path missing .json extension: {rel:?}");
        };
        Ok(key.to_string())
    }

    async fn list_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut read_dir = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = read_dir.next_entry().await? {
                let entry_path = entry.path();
                let ty = entry.file_type().await?;
                if ty.is_dir() {
                    stack.push(entry_path);
                    continue;
                }
                if ty.is_file() && entry_path.extension().and_then(|s| s.to_str()) == Some("json") {
                    out.push(entry_path);
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Storage for FsStorage {
    async fn put_json(&self, key: &str, value: &Value) -> anyhow::Result<()> {
        let path = self.key_to_path(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(value)?;
        let tmp_path = path.with_extension(format!("json.tmp.{}", Uuid::new_v4()));
        tokio::fs::write(&tmp_path, &bytes)
            .await
            .with_context(|| format!("write json to {}", tmp_path.display()))?;

        match tokio::fs::rename(&tmp_path, &path).await {
            Ok(()) => {}
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                let remove = tokio::fs::remove_file(&path).await;
                if let Err(remove_err) = remove {
                    if remove_err.kind() != std::io::ErrorKind::NotFound {
                        return Err(remove_err)
                            .with_context(|| format!("remove old json {}", path.display()));
                    }
                }
                tokio::fs::rename(&tmp_path, &path).await.with_context(|| {
                    format!("rename {} -> {}", tmp_path.display(), path.display())
                })?;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("rename {} -> {}", tmp_path.display(), path.display())
                });
            }
        };
        Ok(())
    }

    async fn get_json(&self, key: &str) -> anyhow::Result<Option<Value>> {
        let path = self.key_to_path(key)?;
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context(format!("read json from {}", path.display())),
        };
        let value = serde_json::from_slice(&bytes)?;
        Ok(Some(value))
    }

    async fn list_prefix(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let mut dir = self.root.clone();
        for segment in prefix.split('/') {
            if segment.is_empty() {
                continue;
            }
            if segment == "." || segment == ".." {
                anyhow::bail!("invalid storage key segment: {segment:?}");
            }
            if segment.contains('\\') {
                anyhow::bail!("invalid storage key segment: {segment:?}");
            }
            dir = dir.join(segment);
        }
        let mut files = Vec::new();
        match tokio::fs::metadata(&dir).await {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Ok(Vec::new()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).context("stat prefix directory"),
        }
        Self::list_json_files(&dir, &mut files).await?;
        let mut keys = Vec::with_capacity(files.len());
        for file in files {
            keys.push(self.path_to_key(&file)?);
        }
        keys.sort();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SessionId;

    #[tokio::test]
    async fn roundtrip_put_get_list() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        storage
            .put_json("sessions/abc/session", &serde_json::json!({"ok": true}))
            .await?;

        let value = storage.get_json("sessions/abc/session").await?;
        assert_eq!(value, Some(serde_json::json!({"ok": true})));

        let keys = storage.list_prefix("sessions/").await?;
        assert_eq!(keys, vec!["sessions/abc/session"]);
        Ok(())
    }

    #[tokio::test]
    async fn list_prefix_rejects_parent_traversal() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let err = storage.list_prefix("../").await.unwrap_err();
        assert!(err.to_string().contains("invalid storage key segment"));
        Ok(())
    }

    #[tokio::test]
    async fn keys_with_json_suffix_roundtrip() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        storage
            .put_json("weird/key.json", &serde_json::json!({"ok": true}))
            .await?;

        let keys = storage.list_prefix("weird/").await?;
        assert_eq!(keys, vec!["weird/key.json"]);

        let value = storage.get_json("weird/key.json").await?;
        assert_eq!(value, Some(serde_json::json!({"ok": true})));

        Ok(())
    }

    #[tokio::test]
    async fn list_session_ids_reads_directories() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
        let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

        storage
            .put_json(&format!("sessions/{id2}/tasks"), &serde_json::json!([]))
            .await?;
        storage
            .put_json(&format!("sessions/{id1}/session"), &serde_json::json!({}))
            .await?;
        storage
            .put_json("sessions/not-a-uuid/session", &serde_json::json!({}))
            .await?;

        assert_eq!(storage.list_session_ids().await?, vec![id1, id2]);
        Ok(())
    }
}
