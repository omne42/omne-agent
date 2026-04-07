use std::path::PathBuf;

use anyhow::Context;
use globset::Glob;
use regex::Regex;
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIndexOutcome {
    pub paths: Vec<String>,
    pub truncated: bool,
    pub files_scanned: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoGrepMatch {
    pub path: String,
    pub line_number: u64,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoGrepOutcome {
    pub matches: Vec<RepoGrepMatch>,
    pub truncated: bool,
    pub files_scanned: usize,
    pub files_skipped_too_large: usize,
    pub files_skipped_binary: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoGrepRequest {
    pub root: PathBuf,
    pub query: String,
    pub is_regex: bool,
    pub include_glob: Option<String>,
    pub max_matches: usize,
    pub max_bytes_per_file: u64,
    pub max_files: usize,
}

const MAX_LISTED_PATHS: usize = 2000;

pub fn scan_repo_index(
    root: PathBuf,
    include_glob: Option<String>,
    max_files: usize,
) -> anyhow::Result<RepoIndexOutcome> {
    let include_matcher = match include_glob.as_deref() {
        Some(glob) => Some(
            Glob::new(glob)
                .with_context(|| format!("invalid glob pattern: {glob}"))?
                .compile_matcher(),
        ),
        None => None,
    };

    let mut paths = Vec::<String>::new();
    let mut truncated = false;
    let mut files_scanned = 0usize;
    let mut size_bytes = 0u64;

    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(omne_fs_policy::should_walk_entry)
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if files_scanned >= max_files {
            truncated = true;
            break;
        }

        let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        if omne_fs_policy::is_repo_tool_hidden_rel_path(rel) {
            continue;
        }
        if let Some(ref matcher) = include_matcher
            && !matcher.is_match(rel)
        {
            continue;
        }

        files_scanned += 1;
        let meta = entry.metadata()?;
        size_bytes = size_bytes.saturating_add(meta.len());

        if paths.len() < MAX_LISTED_PATHS {
            paths.push(rel.to_string_lossy().to_string());
        }
    }

    paths.sort();

    Ok(RepoIndexOutcome {
        paths,
        truncated,
        files_scanned,
        size_bytes,
    })
}

pub fn search_repo(req: RepoGrepRequest) -> anyhow::Result<RepoGrepOutcome> {
    let pattern = if req.is_regex {
        req.query.clone()
    } else {
        regex::escape(&req.query)
    };
    let re = Regex::new(&pattern).with_context(|| format!("invalid regex: {}", req.query))?;

    let include_matcher = match req.include_glob.as_deref() {
        Some(glob) => Some(
            Glob::new(glob)
                .with_context(|| format!("invalid glob pattern: {glob}"))?
                .compile_matcher(),
        ),
        None => None,
    };

    let mut matches = Vec::new();
    let mut truncated = false;
    let mut files_scanned = 0usize;
    let mut files_skipped_too_large = 0usize;
    let mut files_skipped_binary = 0usize;

    for entry in WalkDir::new(&req.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(omne_fs_policy::should_walk_entry)
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if files_scanned >= req.max_files {
            truncated = true;
            break;
        }

        let rel = entry.path().strip_prefix(&req.root).unwrap_or(entry.path());
        if omne_fs_policy::is_repo_tool_hidden_rel_path(rel) {
            continue;
        }
        if let Some(ref matcher) = include_matcher
            && !matcher.is_match(rel)
        {
            continue;
        }

        files_scanned += 1;

        let meta = entry.metadata()?;
        if meta.len() > req.max_bytes_per_file {
            files_skipped_too_large += 1;
            continue;
        }

        let bytes = match std::fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if bytes.contains(&0) {
            files_skipped_binary += 1;
            continue;
        }

        let text = String::from_utf8_lossy(&bytes);
        for (idx, line) in text.lines().enumerate() {
            if !re.is_match(line) {
                continue;
            }
            matches.push(RepoGrepMatch {
                path: rel.to_string_lossy().to_string(),
                line_number: (idx + 1) as u64,
                line: truncate_line(line, 4000),
            });
            if matches.len() >= req.max_matches {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    Ok(RepoGrepOutcome {
        matches,
        truncated,
        files_scanned,
        files_skipped_too_large,
        files_skipped_binary,
    })
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in line.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_skips_secret_files() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("a.txt"), "ok")?;
        std::fs::write(root.join(".env"), "secret")?;
        std::fs::write(root.join(".env.local"), "secret-local")?;
        std::fs::write(root.join(".env.production"), "secret-production")?;
        std::fs::write(root.join(".env.example"), "example")?;

        let out = scan_repo_index(root, None, 100)?;
        assert!(out.paths.iter().any(|p| p == "a.txt"));
        assert!(!out.paths.iter().any(|p| p == ".env"));
        assert!(!out.paths.iter().any(|p| p == ".env.local"));
        assert!(!out.paths.iter().any(|p| p == ".env.production"));
        assert!(out.paths.iter().any(|p| p == ".env.example"));
        Ok(())
    }

    #[test]
    fn grep_skips_secret_file_variants() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("visible.txt"), "needle\n")?;
        std::fs::write(root.join(".env.local"), "needle\n")?;
        std::fs::write(root.join(".env.production"), "needle\n")?;
        std::fs::write(root.join(".env.example"), "needle\n")?;

        let out = search_repo(RepoGrepRequest {
            root,
            query: "needle".to_string(),
            is_regex: false,
            include_glob: None,
            max_matches: 10,
            max_bytes_per_file: 1024 * 1024,
            max_files: 100,
        })?;

        assert!(out.matches.iter().any(|m| m.path == "visible.txt"));
        assert!(!out.matches.iter().any(|m| m.path == ".env.local"));
        assert!(!out.matches.iter().any(|m| m.path == ".env.production"));
        assert!(out.matches.iter().any(|m| m.path == ".env.example"));
        Ok(())
    }

    #[test]
    fn repo_tools_skip_omne_data_runtime_files() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".omne_data"))?;
        std::fs::write(root.join("visible.txt"), "needle\n")?;
        std::fs::write(root.join(".omne_data").join("AGENTS.md"), "needle\n")?;

        let index = scan_repo_index(root.clone(), None, 100)?;
        assert!(index.paths.iter().any(|path| path == "visible.txt"));
        assert!(
            !index
                .paths
                .iter()
                .any(|path| path == ".omne_data/AGENTS.md")
        );

        let grep = search_repo(RepoGrepRequest {
            root,
            query: "needle".to_string(),
            is_regex: false,
            include_glob: None,
            max_matches: 10,
            max_bytes_per_file: 1024 * 1024,
            max_files: 100,
        })?;
        assert!(grep.matches.iter().any(|m| m.path == "visible.txt"));
        assert!(
            !grep
                .matches
                .iter()
                .any(|m| m.path == ".omne_data/AGENTS.md")
        );
        Ok(())
    }

    #[test]
    fn grep_honors_max_matches() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("a.rs"), "hello\nhello\nhello\n")?;
        let out = search_repo(RepoGrepRequest {
            root,
            query: "hello".to_string(),
            is_regex: false,
            include_glob: None,
            max_matches: 2,
            max_bytes_per_file: 1024 * 1024,
            max_files: 100,
        })?;
        assert_eq!(out.matches.len(), 2);
        assert!(out.truncated);
        Ok(())
    }

    #[test]
    fn grep_marks_truncated_when_max_files_is_hit() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("a.rs"), "nope\n")?;
        std::fs::write(root.join("b.rs"), "needle\n")?;

        let out = search_repo(RepoGrepRequest {
            root,
            query: "needle".to_string(),
            is_regex: false,
            include_glob: None,
            max_matches: 10,
            max_bytes_per_file: 1024 * 1024,
            max_files: 1,
        })?;

        assert!(out.truncated);
        assert!(out.matches.is_empty());
        assert_eq!(out.files_scanned, 1);
        Ok(())
    }
}
