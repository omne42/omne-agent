use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;

pub async fn lock_exclusive(path: &Path) -> anyhow::Result<std::fs::File> {
    let path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create lock dir {}", parent.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open lock {}", path.display()))?;
        FileExt::lock_exclusive(&file).with_context(|| format!("lock {}", path.display()))?;
        Ok(file)
    })
    .await
    .context("join lock task")?
}

pub async fn lock_shared(path: &Path) -> anyhow::Result<std::fs::File> {
    let path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create lock dir {}", parent.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open lock {}", path.display()))?;
        FileExt::lock_shared(&file).with_context(|| format!("lock {}", path.display()))?;
        Ok(file)
    })
    .await
    .context("join lock task")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lock_exclusive_creates_parent_dir() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let lock_path = tmp.path().join("locks").join("repo.lock");
        assert!(!lock_path.exists());
        assert!(!lock_path.parent().unwrap().exists());

        let _lock = lock_exclusive(&lock_path).await?;
        assert!(lock_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn lock_shared_creates_parent_dir() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let lock_path = tmp.path().join("locks").join("repo.lock");
        assert!(!lock_path.exists());
        assert!(!lock_path.parent().unwrap().exists());

        let _lock = lock_shared(&lock_path).await?;
        assert!(lock_path.exists());
        Ok(())
    }
}
