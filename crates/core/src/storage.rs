use std::path::{Path, PathBuf};

use anyhow::Context;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{SessionId, SessionMeta};

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
        ids.sort_unstable();
        Ok(ids)
    }

    pub async fn get_session_bundle(
        &self,
        id: SessionId,
        all: bool,
    ) -> anyhow::Result<Option<Value>> {
        let mut out = serde_json::Map::new();

        let result_key = format!("sessions/{id}/result");
        if !all {
            if let Some(value) = self.get_json(&result_key).await? {
                out.insert("result".to_string(), value);
                return Ok(Some(Value::Object(out)));
            }
        }

        for (name, key) in [
            ("session", format!("sessions/{id}/session")),
            ("tasks", format!("sessions/{id}/tasks")),
            ("prs", format!("sessions/{id}/prs")),
            ("merge", format!("sessions/{id}/merge")),
            ("result", result_key),
        ] {
            if let Some(value) = self.get_json(&key).await? {
                if !all && name == "result" {
                    continue;
                }
                out.insert(name.to_string(), value);
            }
        }

        if out.is_empty() {
            return Ok(None);
        }
        Ok(Some(Value::Object(out)))
    }

    pub async fn get_session_meta(&self, id: SessionId) -> anyhow::Result<Option<SessionMeta>> {
        if let Some(meta) = self.get_typed_json(&format!("sessions/{id}/meta")).await? {
            return Ok(Some(meta));
        }
        self.get_typed_json(&format!("sessions/{id}/session")).await
    }

    pub async fn list_session_meta(&self) -> anyhow::Result<Vec<SessionMeta>> {
        let mut sessions = Vec::new();
        for id in self.list_session_ids().await? {
            if let Some(meta) = self.get_session_meta(id).await? {
                sessions.push(meta);
            }
        }

        sessions.sort_by(|a, b| {
            b.created_at
                .unix_timestamp()
                .cmp(&a.created_at.unix_timestamp())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(sessions)
    }

    pub async fn get_typed_json<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> anyhow::Result<Option<T>> {
        let path = self.key_to_path(key)?;
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context(format!("read json from {}", path.display())),
        };
        let value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse json from {}", path.display()))?;
        Ok(Some(value))
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

    async fn cleanup_tmp_file(tmp_path: &Path) {
        if let Err(err) = tokio::fs::remove_file(tmp_path).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(path = %tmp_path.display(), error = %err, "failed to remove temp file");
        }
    }
}

#[async_trait]
impl Storage for FsStorage {
    async fn put_json(&self, key: &str, value: &Value) -> anyhow::Result<()> {
        let path = self.key_to_path(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(value)?;
        let tmp_path = path.with_extension(format!("json.tmp.{}", Uuid::new_v4()));
        tokio::fs::write(&tmp_path, &bytes)
            .await
            .with_context(|| format!("write json to {}", tmp_path.display()))?;

        if let Err(err) = tokio::fs::rename(&tmp_path, &path).await {
            Self::cleanup_tmp_file(&tmp_path).await;
            return Err(err)
                .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()));
        }
        Ok(())
    }

    async fn get_json(&self, key: &str) -> anyhow::Result<Option<Value>> {
        let path = self.key_to_path(key)?;
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context(format!("read json from {}", path.display())),
        };
        let value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse json from {}", path.display()))?;
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
    use crate::domain::{PrName, RepositoryName, Session, SessionId};
    use time::OffsetDateTime;

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

    #[cfg(not(windows))]
    #[tokio::test]
    async fn put_json_replaces_existing_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());
        let key = "sessions/demo/result";

        storage
            .put_json(key, &serde_json::json!({ "value": "old" }))
            .await?;
        storage
            .put_json(key, &serde_json::json!({ "value": "new" }))
            .await?;

        let written = storage
            .get_json(key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing stored json"))?;
        assert_eq!(written["value"], "new");
        Ok(())
    }

    #[tokio::test]
    async fn get_session_bundle_prefers_result_by_default() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id: SessionId = "00000000-0000-0000-0000-000000000123".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id, "stage": "session"}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/result"),
                &serde_json::json!({"id": id, "stage": "result"}),
            )
            .await?;

        let bundle = storage.get_session_bundle(id, false).await?.unwrap();
        assert_eq!(bundle["result"]["stage"], "result");
        assert!(bundle.get("session").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn get_session_bundle_falls_back_when_result_missing() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id: SessionId = "00000000-0000-0000-0000-000000000456".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id, "stage": "session"}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/tasks"),
                &serde_json::json!([{"id":"t1"}]),
            )
            .await?;

        let bundle = storage.get_session_bundle(id, false).await?.unwrap();
        assert_eq!(bundle["session"]["stage"], "session");
        assert_eq!(bundle["tasks"][0]["id"], "t1");
        assert!(bundle.get("result").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn get_session_bundle_all_includes_all_keys() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id: SessionId = "00000000-0000-0000-0000-000000000789".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/tasks"),
                &serde_json::json!([{"id":"t1"}]),
            )
            .await?;
        storage
            .put_json(&format!("sessions/{id}/prs"), &serde_json::json!([]))
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/merge"),
                &serde_json::json!({"merged": true}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/result"),
                &serde_json::json!({"id": id}),
            )
            .await?;

        let bundle = storage.get_session_bundle(id, true).await?.unwrap();
        for key in ["session", "tasks", "prs", "merge", "result"] {
            assert!(bundle.get(key).is_some(), "missing key {key}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn get_session_meta_prefers_meta_file_when_present() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id: SessionId = "00000000-0000-0000-0000-000000000999".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/meta"),
                &serde_json::json!({
                    "id": id,
                    "repo": "repo-meta",
                    "pr_name": "pr-meta",
                    "base_branch": "main",
                    "created_at": "2026-01-20T00:00:00Z",
                }),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({
                    "id": id,
                    "repo": "repo-session",
                    "pr_name": "pr-session",
                    "prompt": "big prompt",
                    "base_branch": "dev",
                    "created_at": "2026-01-20T00:00:00Z",
                }),
            )
            .await?;

        let meta = storage.get_session_meta(id).await?.unwrap();
        assert_eq!(meta.repo.as_str(), "repo-meta");
        assert_eq!(meta.pr_name.as_str(), "pr-meta");
        assert_eq!(meta.base_branch, "main");
        Ok(())
    }

    #[tokio::test]
    async fn get_session_meta_ignores_prompt() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id: SessionId = "00000000-0000-0000-0000-000000000111".parse()?;
        let session = Session {
            id,
            repo: RepositoryName::sanitize("repo"),
            pr_name: PrName::sanitize("pr"),
            prompt: "big prompt".repeat(10),
            base_branch: "main".to_string(),
            created_at: OffsetDateTime::from_unix_timestamp(1_700_000_000)?,
        };
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::to_value(session)?,
            )
            .await?;

        let meta = storage.get_session_meta(id).await?.unwrap();
        assert_eq!(meta.id, id);
        assert_eq!(meta.repo.as_str(), "repo");
        assert_eq!(meta.pr_name.as_str(), "pr");
        assert_eq!(meta.base_branch, "main");
        Ok(())
    }

    #[tokio::test]
    async fn list_session_meta_sorts_by_created_at_desc() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = FsStorage::new(dir.path().to_path_buf());

        let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
        let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

        let make_session = |id, ts| Session {
            id,
            repo: RepositoryName::sanitize("repo"),
            pr_name: PrName::sanitize("pr"),
            prompt: "x".to_string(),
            base_branch: "main".to_string(),
            created_at: OffsetDateTime::from_unix_timestamp(ts).unwrap(),
        };

        storage
            .put_json(
                &format!("sessions/{id1}/session"),
                &serde_json::to_value(make_session(id1, 10))?,
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id2}/session"),
                &serde_json::to_value(make_session(id2, 20))?,
            )
            .await?;

        let sessions = storage.list_session_meta().await?;
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, id2);
        assert_eq!(sessions[1].id, id1);
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_tmp_file_ignores_missing_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let missing = dir.path().join("missing.tmp");
        FsStorage::cleanup_tmp_file(&missing).await;
        assert!(!tokio::fs::try_exists(&missing).await?);
        Ok(())
    }

    #[tokio::test]
    async fn cleanup_tmp_file_removes_existing_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let tmp_file = dir.path().join("existing.tmp");
        tokio::fs::write(&tmp_file, b"temp").await?;
        assert!(tokio::fs::try_exists(&tmp_file).await?);

        FsStorage::cleanup_tmp_file(&tmp_file).await;
        assert!(!tokio::fs::try_exists(&tmp_file).await?);
        Ok(())
    }
}
