use std::path::{Path, PathBuf};

use omne_fs::ops::{
    Context, DeleteKind, DeleteRequest, GlobRequest, GrepRequest, MkdirRequest, PatchRequest,
    ReadRequest, WriteFileRequest, apply_unified_patch, delete, glob_paths, grep, mkdir, read_file,
    write_file,
};
use omne_fs::policy::{Limits, Permissions, Root, SandboxPolicy, SecretRules};
use policy_meta::WriteScope;

const EXTRA_DENY_GLOBS: &[&str] = &[
    ".omne_data/**",
    "**/.omne_data/**",
    ".omne/**",
    "**/.omne/**",
    "target/**",
    "**/target/**",
    "node_modules/**",
    "**/node_modules/**",
    "example/**",
    "**/example/**",
];

const EXTRA_SKIP_GLOBS: &[&str] = &[
    ".omne_data/**",
    "**/.omne_data/**",
    ".omne/**",
    "**/.omne/**",
    "target/**",
    "**/target/**",
    "node_modules/**",
    "**/node_modules/**",
    "example/**",
    "**/example/**",
];

const MAX_WRITE_BYTES_HARD_CAP: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOutcome {
    pub path: PathBuf,
    pub bytes_read: u64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobOutcome {
    pub paths: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepMatchOutcome {
    pub path: String,
    pub line_number: u64,
    pub line: String,
    pub line_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepOutcome {
    pub matches: Vec<GrepMatchOutcome>,
    pub truncated: bool,
    pub scanned_files: u64,
    pub skipped_too_large_files: u64,
    pub skipped_non_utf8_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub path: PathBuf,
    pub bytes_written: u64,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOutcome {
    pub path: PathBuf,
    pub bytes_written: u64,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteOutcome {
    pub path: PathBuf,
    pub deleted: bool,
    pub kind: DeleteKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkdirOutcome {
    pub path: PathBuf,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditReplaceOp {
    pub old: String,
    pub new: String,
    pub expected_replacements: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditReplaceOutcome {
    pub path: PathBuf,
    pub changed: bool,
    pub replacements: usize,
    pub bytes_written: u64,
}

fn rel_path_is_secret(rel_path: &Path) -> bool {
    rel_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(is_blocked_env_style_name)
        .unwrap_or(false)
}

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

pub fn read_text_read_only(
    root_id: String,
    root: PathBuf,
    path: PathBuf,
    max_read_bytes: u64,
) -> anyhow::Result<ReadOutcome> {
    let limits = Limits {
        max_read_bytes,
        ..Default::default()
    };
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::ReadOnly,
        Permissions {
            read: true,
            ..Default::default()
        },
        limits,
        true,
    )?;
    let response = read_file(
        &ctx,
        ReadRequest {
            root_id,
            path,
            start_line: None,
            end_line: None,
        },
    )
    .map_err(anyhow::Error::new)?;

    Ok(ReadOutcome {
        path: response.path,
        bytes_read: response.bytes_read,
        content: response.content,
    })
}

pub fn glob_read_only_paths(
    root_id: String,
    root: PathBuf,
    pattern: String,
    max_results: usize,
) -> anyhow::Result<GlobOutcome> {
    let limits = Limits {
        max_results,
        // `max_results * max_line_bytes` has a hard validation cap; keep this stable
        // for large `max_results` calls (up to 20k in app-server).
        max_line_bytes: 1024,
        ..Default::default()
    };
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::ReadOnly,
        Permissions {
            glob: true,
            ..Default::default()
        },
        limits,
        true,
    )?;
    let response =
        glob_paths(&ctx, GlobRequest { root_id, pattern }).map_err(anyhow::Error::new)?;

    Ok(GlobOutcome {
        paths: response
            .matches
            .into_iter()
            .filter(|path| !rel_path_is_secret(path))
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
        truncated: response.truncated,
    })
}

pub fn grep_read_only_paths(
    root_id: String,
    root: PathBuf,
    query: String,
    regex: bool,
    glob: Option<String>,
    max_results: usize,
    max_read_bytes: u64,
    max_files: usize,
) -> anyhow::Result<GrepOutcome> {
    let limits = Limits {
        max_results,
        max_read_bytes,
        max_walk_files: max_files,
        max_walk_entries: max_files.saturating_mul(8).max(max_files),
        ..Default::default()
    };
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::ReadOnly,
        Permissions {
            grep: true,
            ..Default::default()
        },
        limits,
        true,
    )?;
    let response = grep(
        &ctx,
        GrepRequest {
            root_id,
            query,
            regex,
            glob,
        },
    )
    .map_err(anyhow::Error::new)?;

    Ok(GrepOutcome {
        matches: response
            .matches
            .into_iter()
            .filter(|item| !rel_path_is_secret(&item.path))
            .map(|item| GrepMatchOutcome {
                path: item.path.to_string_lossy().to_string(),
                line_number: item.line,
                line: item.text,
                line_truncated: item.line_truncated,
            })
            .collect::<Vec<_>>(),
        truncated: response.truncated,
        scanned_files: response.scanned_files,
        skipped_too_large_files: response.skipped_too_large_files,
        skipped_non_utf8_files: response.skipped_non_utf8_files,
    })
}

pub fn write_text_workspace(
    root_id: String,
    root: PathBuf,
    path: PathBuf,
    content: String,
    create_parents: bool,
) -> anyhow::Result<WriteOutcome> {
    let max_write_bytes = u64::try_from(content.len())
        .unwrap_or(u64::MAX)
        .max(Limits::default().max_write_bytes)
        .min(MAX_WRITE_BYTES_HARD_CAP);
    let limits = Limits {
        max_write_bytes,
        ..Default::default()
    };
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::WorkspaceWrite,
        Permissions {
            write: true,
            ..Default::default()
        },
        limits,
        false,
    )?;
    let response = write_file(
        &ctx,
        WriteFileRequest {
            root_id,
            path,
            content,
            overwrite: true,
            create_parents,
        },
    )
    .map_err(anyhow::Error::new)?;

    Ok(WriteOutcome {
        path: response.path,
        bytes_written: response.bytes_written,
        created: response.created,
    })
}

pub fn patch_text_workspace(
    root_id: String,
    root: PathBuf,
    path: PathBuf,
    patch: String,
    max_bytes: u64,
) -> anyhow::Result<PatchOutcome> {
    let limits = Limits {
        max_read_bytes: max_bytes,
        max_patch_bytes: Some(max_bytes),
        max_write_bytes: max_bytes,
        ..Default::default()
    };
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::WorkspaceWrite,
        Permissions {
            patch: true,
            ..Default::default()
        },
        limits,
        false,
    )?;
    let response = apply_unified_patch(
        &ctx,
        PatchRequest {
            root_id,
            path,
            patch,
        },
    )
    .map_err(anyhow::Error::new)?;

    Ok(PatchOutcome {
        path: response.path,
        bytes_written: response.bytes_written,
        changed: response.bytes_written > 0,
    })
}

pub fn delete_path_workspace(
    root_id: String,
    root: PathBuf,
    path: PathBuf,
    recursive: bool,
    ignore_missing: bool,
) -> anyhow::Result<DeleteOutcome> {
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::WorkspaceWrite,
        Permissions {
            delete: true,
            ..Default::default()
        },
        Limits::default(),
        false,
    )?;
    let response = delete(
        &ctx,
        DeleteRequest {
            root_id,
            path,
            recursive,
            ignore_missing,
        },
    )
    .map_err(anyhow::Error::new)?;

    Ok(DeleteOutcome {
        path: response.path,
        deleted: response.deleted,
        kind: response.kind,
    })
}

pub fn mkdir_workspace(
    root_id: String,
    root: PathBuf,
    path: PathBuf,
    create_parents: bool,
    ignore_existing: bool,
) -> anyhow::Result<MkdirOutcome> {
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::WorkspaceWrite,
        Permissions {
            mkdir: true,
            ..Default::default()
        },
        Limits::default(),
        false,
    )?;
    let response = mkdir(
        &ctx,
        MkdirRequest {
            root_id,
            path,
            create_parents,
            ignore_existing,
        },
    )
    .map_err(anyhow::Error::new)?;

    Ok(MkdirOutcome {
        path: response.path,
        created: response.created,
    })
}

pub fn edit_replace_workspace(
    root_id: String,
    root: PathBuf,
    path: PathBuf,
    edits: Vec<EditReplaceOp>,
    max_bytes: u64,
) -> anyhow::Result<EditReplaceOutcome> {
    if edits.is_empty() {
        anyhow::bail!("edits must not be empty");
    }
    if edits.iter().any(|edit| edit.old.is_empty()) {
        anyhow::bail!("edit.old must not be empty");
    }

    let limits = Limits {
        max_read_bytes: max_bytes,
        max_write_bytes: max_bytes,
        ..Default::default()
    };
    let ctx = build_context(
        root_id.clone(),
        root,
        WriteScope::WorkspaceWrite,
        Permissions {
            read: true,
            write: true,
            ..Default::default()
        },
        limits,
        false,
    )?;
    let read_response = read_file(
        &ctx,
        ReadRequest {
            root_id: root_id.clone(),
            path: path.clone(),
            start_line: None,
            end_line: None,
        },
    )
    .map_err(anyhow::Error::new)?;

    let mut text = read_response.content;
    let mut replacements = 0usize;
    let mut changed = false;

    for edit in edits {
        let expected = edit.expected_replacements.unwrap_or(1);
        let found = count_non_overlapping(&text, &edit.old);
        if found != expected {
            anyhow::bail!(
                "edit mismatch for {}: expected {} replacements, found {}",
                read_response.path.display(),
                expected,
                found
            );
        }
        if edit.old != edit.new {
            changed = true;
        }
        replacements += expected;
        text = text.replacen(&edit.old, &edit.new, expected);
    }

    let bytes_written = if changed {
        write_file(
            &ctx,
            WriteFileRequest {
                root_id,
                path,
                content: text,
                overwrite: true,
                create_parents: false,
            },
        )
        .map_err(anyhow::Error::new)?
        .bytes_written
    } else {
        0
    };

    Ok(EditReplaceOutcome {
        path: read_response.path,
        changed,
        replacements,
        bytes_written,
    })
}

fn build_context(
    root_id: String,
    root: PathBuf,
    write_scope: WriteScope,
    permissions: Permissions,
    limits: Limits,
    allow_reading_env_examples: bool,
) -> anyhow::Result<Context> {
    let mut secrets = SecretRules::default();
    if allow_reading_env_examples {
        secrets
            .deny_globs
            .retain(|pattern| pattern != ".env.*" && pattern != "**/.env.*");
    }
    secrets
        .deny_globs
        .extend(EXTRA_DENY_GLOBS.iter().copied().map(str::to_string));

    let mut traversal = omne_fs::policy::TraversalRules::default();
    traversal
        .skip_globs
        .extend(EXTRA_SKIP_GLOBS.iter().copied().map(str::to_string));

    let policy = SandboxPolicy {
        roots: vec![Root {
            id: root_id,
            path: root,
            write_scope,
        }],
        permissions,
        limits,
        secrets,
        traversal,
        paths: Default::default(),
        metadata: Default::default(),
    };

    Context::new(policy).map_err(anyhow::Error::new)
}

fn count_non_overlapping(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }

    let mut count = 0usize;
    let mut rest = haystack;
    while let Some(index) = rest.find(needle) {
        count += 1;
        rest = &rest[(index + needle.len())..];
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_allows_env_examples_but_hides_real_env_secrets() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join(".env.local"), "SECRET=1\n")?;
        std::fs::write(root.join(".env.example"), "EXAMPLE=1\n")?;
        std::fs::write(root.join("visible.txt"), "ok\n")?;

        let outcome = glob_read_only_paths("workspace".to_string(), root, "**/*".to_string(), 100)?;

        assert!(outcome.paths.iter().any(|path| path == ".env.example"));
        assert!(outcome.paths.iter().any(|path| path == "visible.txt"));
        assert!(!outcome.paths.iter().any(|path| path == ".env.local"));
        Ok(())
    }

    #[test]
    fn grep_allows_env_examples_but_hides_real_env_secrets() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join(".env.local"), "needle\n")?;
        std::fs::write(root.join(".env.example"), "needle\n")?;
        std::fs::write(root.join("visible.txt"), "needle\n")?;

        let outcome = grep_read_only_paths(
            "workspace".to_string(),
            root,
            "needle".to_string(),
            false,
            None,
            100,
            1024 * 1024,
            100,
        )?;

        assert!(
            outcome
                .matches
                .iter()
                .any(|item| item.path == ".env.example")
        );
        assert!(
            outcome
                .matches
                .iter()
                .any(|item| item.path == "visible.txt")
        );
        assert!(!outcome.matches.iter().any(|item| item.path == ".env.local"));
        Ok(())
    }
}
