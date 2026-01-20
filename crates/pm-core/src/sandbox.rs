use std::path::{Component, Path, PathBuf};

use anyhow::Context;

#[derive(Clone, Copy, Debug)]
pub enum PathAccess {
    Read,
    Write,
}

pub async fn resolve_dir(root: &Path, input: &Path) -> anyhow::Result<PathBuf> {
    let root = tokio::fs::canonicalize(root)
        .await
        .with_context(|| format!("canonicalize root {}", root.display()))?;

    reject_parent_components(input)?;

    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        root.join(input)
    };

    let candidate = strip_cur_dir_components(&candidate);
    let canon = tokio::fs::canonicalize(&candidate)
        .await
        .with_context(|| format!("canonicalize {}", candidate.display()))?;
    if !canon.starts_with(&root) {
        anyhow::bail!(
            "path escapes root: root={}, path={}",
            root.display(),
            canon.display()
        );
    }

    let meta = tokio::fs::metadata(&canon)
        .await
        .with_context(|| format!("stat {}", canon.display()))?;
    if !meta.is_dir() {
        anyhow::bail!("not a directory: {}", canon.display());
    }

    Ok(canon)
}

pub async fn resolve_file(
    root: &Path,
    input: &Path,
    access: PathAccess,
    create_parent_dirs: bool,
) -> anyhow::Result<PathBuf> {
    let root = tokio::fs::canonicalize(root)
        .await
        .with_context(|| format!("canonicalize root {}", root.display()))?;

    reject_parent_components(input)?;

    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        root.join(input)
    };
    let candidate = strip_cur_dir_components(&candidate);

    if !candidate.starts_with(&root) {
        anyhow::bail!(
            "path escapes root: root={}, path={}",
            root.display(),
            candidate.display()
        );
    }

    let Some(parent) = candidate.parent() else {
        anyhow::bail!("path has no parent: {}", candidate.display());
    };

    if create_parent_dirs {
        ensure_dir_tree_no_symlink(&root, parent).await?;
    }

    let canon_parent = tokio::fs::canonicalize(parent)
        .await
        .with_context(|| format!("canonicalize {}", parent.display()))?;
    if !canon_parent.starts_with(&root) {
        anyhow::bail!(
            "path escapes root via symlink: root={}, path={}",
            root.display(),
            canon_parent.display()
        );
    }

    match access {
        PathAccess::Read => {
            let canon = tokio::fs::canonicalize(&candidate)
                .await
                .with_context(|| format!("canonicalize {}", candidate.display()))?;
            if !canon.starts_with(&root) {
                anyhow::bail!(
                    "path escapes root via symlink: root={}, path={}",
                    root.display(),
                    canon.display()
                );
            }
            Ok(canon)
        }
        PathAccess::Write => {
            if let Ok(meta) = tokio::fs::symlink_metadata(&candidate).await {
                if meta.file_type().is_symlink() {
                    anyhow::bail!("refusing to write through symlink: {}", candidate.display());
                }
            }
            Ok(candidate)
        }
    }
}

fn reject_parent_components(path: &Path) -> anyhow::Result<()> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        anyhow::bail!("parent traversal is not allowed: {}", path.display());
    }
    Ok(())
}

fn strip_cur_dir_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            _ => out.push(comp.as_os_str()),
        }
    }
    out
}

async fn ensure_dir_tree_no_symlink(root: &Path, dir: &Path) -> anyhow::Result<()> {
    if !dir.starts_with(root) {
        anyhow::bail!(
            "path escapes root: root={}, dir={}",
            root.display(),
            dir.display()
        );
    }

    let relative = dir
        .strip_prefix(root)
        .with_context(|| format!("strip prefix root={} dir={}", root.display(), dir.display()))?;

    let mut current = root.to_path_buf();
    for comp in relative.components() {
        let Component::Normal(name) = comp else {
            anyhow::bail!("unexpected path component in {}", dir.display());
        };
        current.push(name);

        match tokio::fs::symlink_metadata(&current).await {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    anyhow::bail!("symlink not allowed in path: {}", current.display());
                }
                if !meta.is_dir() {
                    anyhow::bail!("not a directory: {}", current.display());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir(&current)
                    .await
                    .with_context(|| format!("create dir {}", current.display()))?;
            }
            Err(err) => {
                return Err(err).with_context(|| format!("stat {}", current.display()));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[tokio::test]
    async fn resolve_file_rejects_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let err = resolve_file(root, Path::new("../x"), PathAccess::Read, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("parent traversal"));
    }

    #[tokio::test]
    async fn resolve_file_rejects_symlink_escape_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();
        tokio::fs::write(outside.join("secret.txt"), "nope")
            .await
            .unwrap();

        #[cfg(unix)]
        {
            symlink(&outside, &root.join("link")).unwrap();
            let err = resolve_file(&root, Path::new("link/secret.txt"), PathAccess::Read, false)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("escapes root"));
        }
    }

    #[tokio::test]
    async fn resolve_file_rejects_symlink_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside.txt");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(&outside, "x").await.unwrap();

        #[cfg(unix)]
        {
            symlink(&outside, &root.join("file.txt")).unwrap();
            let err = resolve_file(&root, Path::new("file.txt"), PathAccess::Write, false)
                .await
                .unwrap_err();
            assert!(err.to_string().contains("symlink"));
        }
    }
}
