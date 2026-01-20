use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;

pub async fn lock_exclusive(path: &Path) -> anyhow::Result<std::fs::File> {
    let path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || {
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
