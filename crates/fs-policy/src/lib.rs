use std::path::Path;

const DEFAULT_IGNORED_DIRS: &[&str] = &[".git", ".omne", "target", "node_modules", "example"];

pub fn is_secret_rel_path(rel_path: &Path) -> bool {
    rel_path.file_name() == Some(std::ffi::OsStr::new(".env"))
}

pub fn should_walk_entry(entry: &walkdir::DirEntry) -> bool {
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
                parent == std::ffi::OsStr::new(".omne_data")
                    || parent == std::ffi::OsStr::new("omne_data")
            })
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_path_matches_env_file() {
        assert!(is_secret_rel_path(Path::new(".env")));
        assert!(!is_secret_rel_path(Path::new("src/main.rs")));
    }
}
