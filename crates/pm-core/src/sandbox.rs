use std::path::{Component, Path, PathBuf};

use anyhow::Context;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

pub async fn resolve_file_with_writable_roots(
    root: &Path,
    writable_roots: &[PathBuf],
    input: &Path,
    access: PathAccess,
    create_parent_dirs: bool,
) -> anyhow::Result<PathBuf> {
    if access == PathAccess::Read || writable_roots.is_empty() || !input.is_absolute() {
        return resolve_file(root, input, access, create_parent_dirs).await;
    }

    let root = tokio::fs::canonicalize(root)
        .await
        .with_context(|| format!("canonicalize root {}", root.display()))?;

    let mut allowed_roots = Vec::with_capacity(1 + writable_roots.len());
    allowed_roots.push(root);
    for writable_root in writable_roots {
        let canon = tokio::fs::canonicalize(writable_root)
            .await
            .with_context(|| format!("canonicalize root {}", writable_root.display()))?;
        let meta = tokio::fs::metadata(&canon)
            .await
            .with_context(|| format!("stat {}", canon.display()))?;
        if !meta.is_dir() {
            anyhow::bail!("not a directory: {}", canon.display());
        }
        allowed_roots.push(canon);
    }

    reject_parent_components(input)?;
    let input = strip_cur_dir_components(input);
    let candidate = canonicalize_nonexistent_file_path(&input).await?;

    let Some(selected_root) = allowed_roots
        .iter()
        .filter(|root| candidate.starts_with(root))
        .max_by_key(|root| root.components().count())
    else {
        anyhow::bail!("path escapes roots: {}", candidate.display());
    };

    resolve_file(selected_root, &candidate, access, create_parent_dirs).await
}

pub async fn resolve_dir_unrestricted(base: &Path, input: &Path) -> anyhow::Result<PathBuf> {
    let base = tokio::fs::canonicalize(base)
        .await
        .with_context(|| format!("canonicalize base {}", base.display()))?;

    reject_parent_components(input)?;

    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        base.join(input)
    };
    let candidate = strip_cur_dir_components(&candidate);

    let canon = tokio::fs::canonicalize(&candidate)
        .await
        .with_context(|| format!("canonicalize {}", candidate.display()))?;
    let meta = tokio::fs::metadata(&canon)
        .await
        .with_context(|| format!("stat {}", canon.display()))?;
    if !meta.is_dir() {
        anyhow::bail!("not a directory: {}", canon.display());
    }
    Ok(canon)
}

pub async fn resolve_file_unrestricted(
    base: &Path,
    input: &Path,
    access: PathAccess,
    create_parent_dirs: bool,
) -> anyhow::Result<PathBuf> {
    let base = tokio::fs::canonicalize(base)
        .await
        .with_context(|| format!("canonicalize base {}", base.display()))?;

    reject_parent_components(input)?;

    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        base.join(input)
    };
    let candidate = strip_cur_dir_components(&candidate);

    let Some(parent) = candidate.parent() else {
        anyhow::bail!("path has no parent: {}", candidate.display());
    };

    if create_parent_dirs {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create dir {}", parent.display()))?;
    }

    match access {
        PathAccess::Read => {
            let canon = tokio::fs::canonicalize(&candidate)
                .await
                .with_context(|| format!("canonicalize {}", candidate.display()))?;
            Ok(canon)
        }
        PathAccess::Write => Ok(candidate),
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

async fn canonicalize_nonexistent_file_path(path: &Path) -> anyhow::Result<PathBuf> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("path has no parent: {}", path.display());
    };
    let Some(file_name) = path.file_name() else {
        anyhow::bail!("path has no file name: {}", path.display());
    };
    let parent = canonicalize_nonexistent_path(parent).await?;
    Ok(parent.join(file_name))
}

async fn canonicalize_nonexistent_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::<std::ffi::OsString>::new();

    loop {
        match tokio::fs::canonicalize(&cursor).await {
            Ok(canon) => {
                let mut out = canon;
                for comp in suffix.into_iter().rev() {
                    out.push(comp);
                }
                return Ok(out);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let Some(file_name) = cursor.file_name() else {
                    return Err(err).with_context(|| {
                        format!("canonicalize {} (path has no file name)", cursor.display())
                    });
                };
                suffix.push(file_name.to_os_string());
                if !cursor.pop() {
                    return Err(err).with_context(|| {
                        format!("canonicalize {} (path has no parent)", cursor.display())
                    });
                }
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("canonicalize existing prefix {}", cursor.display()));
            }
        }
    }
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
    async fn resolve_file_unrestricted_rejects_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let err = resolve_file_unrestricted(root, Path::new("../x"), PathAccess::Read, false)
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
    async fn resolve_file_unrestricted_allows_symlink_escape_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();
        tokio::fs::write(outside.join("secret.txt"), "ok")
            .await
            .unwrap();

        #[cfg(unix)]
        {
            symlink(&outside, &root.join("link")).unwrap();
            let resolved = resolve_file_unrestricted(
                &root,
                Path::new("link/secret.txt"),
                PathAccess::Read,
                false,
            )
            .await
            .unwrap();
            assert!(resolved.ends_with("outside/secret.txt"));
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

    #[tokio::test]
    async fn resolve_file_with_writable_roots_allows_write_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let extra = dir.path().join("extra");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&extra).await.unwrap();

        let input = extra.join("out.txt");
        let resolved = resolve_file_with_writable_roots(
            &workspace,
            std::slice::from_ref(&extra),
            &input,
            PathAccess::Write,
            true,
        )
        .await
        .unwrap();
        let extra_canon = tokio::fs::canonicalize(&extra).await.unwrap();
        assert!(resolved.starts_with(extra_canon));
    }

    #[tokio::test]
    async fn resolve_file_with_writable_roots_rejects_read_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let extra = dir.path().join("extra");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&extra).await.unwrap();
        tokio::fs::write(extra.join("secret.txt"), "nope")
            .await
            .unwrap();

        let err = resolve_file_with_writable_roots(
            &workspace,
            std::slice::from_ref(&extra),
            &extra.join("secret.txt"),
            PathAccess::Read,
            false,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("escapes root"));
    }

    #[tokio::test]
    async fn resolve_file_with_writable_roots_accepts_symlinked_root_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let real_root = dir.path().join("real");
        let alias_root = dir.path().join("alias");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&real_root).await.unwrap();

        #[cfg(unix)]
        {
            symlink(&real_root, &alias_root).unwrap();
            let input = alias_root.join("nested/file.txt");
            let resolved = resolve_file_with_writable_roots(
                &workspace,
                std::slice::from_ref(&alias_root),
                &input,
                PathAccess::Write,
                true,
            )
            .await
            .unwrap();
            let real_canon = tokio::fs::canonicalize(&real_root).await.unwrap();
            assert!(resolved.starts_with(real_canon));
            assert!(
                tokio::fs::metadata(real_root.join("nested"))
                    .await
                    .unwrap()
                    .is_dir()
            );
        }
    }
}
