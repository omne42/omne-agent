use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use serde::Serialize;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, Default)]
pub struct SnapshotOutcome {
    pub file_count: u64,
    pub total_bytes: u64,
    pub symlink_count: u64,
    pub oversize_count: u64,
    pub secret_count: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct RestorePlan {
    pub create: u64,
    pub modify: u64,
    pub delete: u64,
}

pub fn checkpoint_ignored_globs() -> Vec<String> {
    vec![
        ".git/**".to_string(),
        ".omne/**".to_string(),
        "target/**".to_string(),
        "node_modules/**".to_string(),
        "example/**".to_string(),
        ".omne_data/tmp/**".to_string(),
        ".omne_data/threads/**".to_string(),
        ".omne_data/locks/**".to_string(),
        ".omne_data/logs/**".to_string(),
        ".omne_data/data/**".to_string(),
        ".omne_data/repos/**".to_string(),
        ".omne_data/reference/**".to_string(),
        "**/.env".to_string(),
        "**/.env.*".to_string(),
        "**/.envrc".to_string(),
        "**/*.pem".to_string(),
        "**/*.key".to_string(),
        "**/.ssh/**".to_string(),
        "**/.aws/**".to_string(),
        "**/.kube/**".to_string(),
    ]
}

pub async fn snapshot_workspace_to_dir(
    thread_root: &Path,
    snapshot_root: &Path,
    max_file_bytes: u64,
    max_total_bytes: u64,
) -> anyhow::Result<SnapshotOutcome> {
    let thread_root = thread_root.to_path_buf();
    let snapshot_root = snapshot_root.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<SnapshotOutcome> {
        std::fs::create_dir_all(&snapshot_root)
            .with_context(|| format!("create {}", snapshot_root.display()))?;

        let mut out = SnapshotOutcome::default();

        for entry in WalkDir::new(&thread_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(omne_fs_policy::should_walk_entry)
        {
            let entry = entry?;

            if entry.file_type().is_symlink() {
                out.symlink_count += 1;
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }

            let rel = entry
                .path()
                .strip_prefix(&thread_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            if rel_path_is_checkpoint_secret(rel) {
                out.secret_count += 1;
                continue;
            }

            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > max_file_bytes {
                out.oversize_count += 1;
                continue;
            }

            out.file_count += 1;
            out.total_bytes = out
                .total_bytes
                .checked_add(meta.len())
                .ok_or_else(|| anyhow::anyhow!("checkpoint size overflow"))?;
            if out.total_bytes > max_total_bytes {
                anyhow::bail!(
                    "checkpoint exceeds max_total_bytes={} (current={})",
                    max_total_bytes,
                    out.total_bytes
                );
            }

            let dest = snapshot_root.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(entry.path(), &dest).with_context(|| {
                format!("copy {} -> {}", entry.path().display(), dest.display())
            })?;
        }

        Ok(out)
    })
    .await
    .context("join checkpoint snapshot task")?
}

pub async fn compute_restore_plan(
    thread_root: &Path,
    snapshot_root: &Path,
    max_file_bytes: u64,
) -> anyhow::Result<RestorePlan> {
    let thread_root = thread_root.to_path_buf();
    let snapshot_root = snapshot_root.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<RestorePlan> {
        let mut snapshot_sizes = BTreeMap::<String, u64>::new();
        for entry in WalkDir::new(&snapshot_root).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&snapshot_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            snapshot_sizes.insert(rel.to_string_lossy().to_string(), meta.len());
        }

        let mut current_sizes = BTreeMap::<String, u64>::new();
        for entry in WalkDir::new(&thread_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(omne_fs_policy::should_walk_entry)
        {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&thread_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            if rel_path_is_checkpoint_secret(rel) {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > max_file_bytes {
                continue;
            }
            current_sizes.insert(rel.to_string_lossy().to_string(), meta.len());
        }

        let snapshot_paths = snapshot_sizes.keys().cloned().collect::<BTreeSet<_>>();
        let current_paths = current_sizes.keys().cloned().collect::<BTreeSet<_>>();

        let create = snapshot_paths.difference(&current_paths).count() as u64;
        let delete = current_paths.difference(&snapshot_paths).count() as u64;

        let mut modify = 0u64;
        for path in snapshot_paths.intersection(&current_paths) {
            let Some(snap_len) = snapshot_sizes.get(path) else {
                continue;
            };
            let Some(cur_len) = current_sizes.get(path) else {
                continue;
            };
            if snap_len != cur_len {
                modify += 1;
            }
        }

        Ok(RestorePlan {
            create,
            modify,
            delete,
        })
    })
    .await
    .context("join checkpoint plan task")?
}

pub async fn restore_workspace_from_snapshot(
    thread_root: &Path,
    snapshot_root: &Path,
    max_file_bytes: u64,
) -> anyhow::Result<()> {
    let thread_root = thread_root.to_path_buf();
    let snapshot_root = snapshot_root.to_path_buf();

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut snapshot_paths = BTreeSet::<String>::new();
        for entry in WalkDir::new(&snapshot_root).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&snapshot_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            snapshot_paths.insert(rel.to_string_lossy().to_string());
        }

        let mut current_paths = Vec::<String>::new();
        for entry in WalkDir::new(&thread_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(omne_fs_policy::should_walk_entry)
        {
            let entry = entry?;
            if entry.file_type().is_symlink() || !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&thread_root)
                .unwrap_or(entry.path());
            if rel.as_os_str().is_empty() {
                continue;
            }
            if rel_path_is_checkpoint_secret(rel) {
                continue;
            }
            let meta = entry
                .metadata()
                .with_context(|| format!("stat {}", entry.path().display()))?;
            if meta.len() > max_file_bytes {
                continue;
            }
            current_paths.push(rel.to_string_lossy().to_string());
        }

        for rel in current_paths {
            if snapshot_paths.contains(&rel) {
                continue;
            }
            let path = thread_root.join(&rel);
            std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
        }

        for rel in &snapshot_paths {
            let src = snapshot_root.join(rel);
            let dst = thread_root.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(&src, &dst)
                .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))?;
        }

        Ok(())
    })
    .await
    .context("join checkpoint restore task")?
}

fn rel_path_is_checkpoint_secret(rel_path: &Path) -> bool {
    if rel_path.components().any(
        |c| matches!(c, std::path::Component::Normal(os) if os == ".ssh" || os == ".aws" || os == ".kube"),
    ) {
        return true;
    }

    let Some(file_name) = rel_path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };

    if file_name == ".env" || file_name == ".envrc" || file_name.starts_with(".env.") {
        return true;
    }

    file_name.ends_with(".pem") || file_name.ends_with(".key")
}
