use std::path::Path;

const DEFAULT_IGNORED_DIRS: &[&str] = &[".git", ".omne", "target", "node_modules", "example"];

fn is_blocked_env_style_name(file_name: &str) -> bool {
    let file_name = file_name.to_ascii_lowercase();
    if !file_name.starts_with(".env") {
        return false;
    }

    let suffix = &file_name[".env".len()..];
    if !suffix.is_empty()
        && !suffix.starts_with('.')
        && !suffix.starts_with('_')
        && !suffix.starts_with('-')
    {
        return false;
    }

    !(file_name.contains("example") || file_name.contains("template"))
}

pub fn is_secret_rel_path(rel_path: &Path) -> bool {
    rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(is_blocked_env_style_name)
        .unwrap_or(false)
}

pub fn is_read_blocked_rel_path(rel_path: &Path) -> bool {
    rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(is_blocked_env_style_name)
        .unwrap_or(false)
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
    fn secret_path_matches_env_style_sensitive_files() {
        assert!(is_secret_rel_path(Path::new(".env")));
        assert!(is_secret_rel_path(Path::new(".env.local")));
        assert!(is_secret_rel_path(Path::new(".env.production")));
        assert!(!is_secret_rel_path(Path::new(".env.example")));
        assert!(!is_secret_rel_path(Path::new(".env.template")));
        assert!(!is_secret_rel_path(Path::new("src/main.rs")));
    }

    #[test]
    fn read_blocked_path_matches_sensitive_env_variants() {
        assert!(is_read_blocked_rel_path(Path::new(".env")));
        assert!(is_read_blocked_rel_path(Path::new(".env.local")));
        assert!(is_read_blocked_rel_path(Path::new(".env.production")));
        assert!(!is_read_blocked_rel_path(Path::new(".env.example")));
        assert!(!is_read_blocked_rel_path(Path::new(".env.template")));
        assert!(!is_read_blocked_rel_path(Path::new(".environment")));
        assert!(!is_read_blocked_rel_path(Path::new("config.env")));
    }
}
