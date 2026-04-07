use std::path::{Component, Path};

const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".omne",
    ".omne_data",
    "target",
    "node_modules",
    "example",
];
const REPO_TOOL_HIDDEN_COMPONENTS: &[&str] = &[
    ".git",
    ".omne",
    ".omne_data",
    "target",
    "node_modules",
    "example",
];

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

pub fn is_repo_tool_hidden_rel_path(rel_path: &Path) -> bool {
    is_read_blocked_rel_path(rel_path)
        || rel_path.components().any(|component| match component {
            Component::Normal(name) => REPO_TOOL_HIDDEN_COMPONENTS
                .iter()
                .any(|blocked| name == std::ffi::OsStr::new(blocked)),
            _ => false,
        })
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
    fn env_style_secret_and_read_protection_share_semantics() {
        for (path, expected) in [
            (".env", true),
            (".env.local", true),
            (".env.production", true),
            (".env.development.local", true),
            (".env_test", true),
            (".env-test", true),
            (".env.example", false),
            (".env.example.local", false),
            (".env.template", false),
            (".ENV.LOCAL", true),
            (".environment", false),
            (".envrc", false),
            ("config.env", false),
            ("src/main.rs", false),
        ] {
            let path = Path::new(path);
            assert_eq!(
                is_secret_rel_path(path),
                expected,
                "secret: {}",
                path.display()
            );
            assert_eq!(
                is_read_blocked_rel_path(path),
                expected,
                "read-blocked: {}",
                path.display()
            );
        }
    }

    #[test]
    fn repo_tool_hidden_paths_cover_runtime_and_generated_dirs() {
        for path in [
            Path::new(".omne_data/AGENTS.md"),
            Path::new("nested/.omne_data/threads/state.json"),
            Path::new(".omne/config.toml"),
            Path::new("target/debug/app"),
            Path::new("node_modules/pkg/index.js"),
            Path::new("example/demo.txt"),
            Path::new(".env.local"),
        ] {
            assert!(
                is_repo_tool_hidden_rel_path(path),
                "expected hidden: {}",
                path.display()
            );
        }

        for path in [
            Path::new("src/main.rs"),
            Path::new(".env.example"),
            Path::new("examples/demo.rs"),
            Path::new("example.txt"),
        ] {
            assert!(
                !is_repo_tool_hidden_rel_path(path),
                "expected visible: {}",
                path.display()
            );
        }
    }
}
