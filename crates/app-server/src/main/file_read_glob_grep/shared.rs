fn rel_path_is_secret(rel_path: &Path) -> bool {
    rel_path.file_name() == Some(std::ffi::OsStr::new(".env"))
}

fn should_walk_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name();
    if DEFAULT_IGNORED_DIRS
        .iter()
        .any(|dir| name == std::ffi::OsStr::new(*dir))
    {
        return false;
    }

    if (name == std::ffi::OsStr::new("tmp")
        || name == std::ffi::OsStr::new("threads")
        || name == std::ffi::OsStr::new("locks")
        || name == std::ffi::OsStr::new("logs")
        || name == std::ffi::OsStr::new("data")
        || name == std::ffi::OsStr::new("repos")
        || name == std::ffi::OsStr::new("reference"))
        && entry
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|parent| {
                parent == std::ffi::OsStr::new(".codepm_data")
                    || parent == std::ffi::OsStr::new("codepm_data")
            })
    {
        return false;
    }

    true
}

async fn resolve_reference_repo_root(thread_root: &Path) -> anyhow::Result<PathBuf> {
    let rel = Path::new(".codepm_data/reference/repo");
    pm_core::resolve_dir(thread_root, rel)
        .await
        .with_context(|| format!("resolve reference repo root {}", thread_root.join(rel).display()))
}

