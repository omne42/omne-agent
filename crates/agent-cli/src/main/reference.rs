use walkdir::WalkDir;
use time::OffsetDateTime;

const DEFAULT_MAX_REFERENCE_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_MAX_REFERENCE_FILE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct ReferenceRepoManifest {
    version: u32,
    created_at: String,
    source: ReferenceRepoSource,
    max_file_bytes: u64,
    removed: Vec<ReferenceRepoRemovedEntry>,
    stats: ReferenceRepoStats,
}

#[derive(Debug, Serialize)]
struct ReferenceRepoSource {
    path: String,
}

#[derive(Debug, Serialize)]
struct ReferenceRepoRemovedEntry {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    reason: String,
}

#[derive(Debug, Serialize)]
struct ReferenceRepoStats {
    dirs_created: u64,
    files_copied: u64,
    bytes_copied: u64,
    entries_skipped: u64,
}

fn resolve_pm_root(cli: &Cli) -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    Ok(cli
        .omne_root
        .clone()
        .or_else(|| std::env::var_os("OMNE_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| cwd.join(".omne_data")))
}

async fn run_reference(cli: &Cli, command: ReferenceCommand) -> anyhow::Result<()> {
    let omne_root = resolve_pm_root(cli)?;

    match command {
        ReferenceCommand::Import {
            from,
            force,
            max_file_bytes,
            json,
        } => {
            let max_file_bytes = max_file_bytes
                .unwrap_or(DEFAULT_MAX_REFERENCE_FILE_BYTES)
                .clamp(1, MAX_MAX_REFERENCE_FILE_BYTES);

            let report = reference_repo_import(&omne_root, &from, force, max_file_bytes).await?;
            print_json_or_pretty(json, &serde_json::to_value(report)?)?;
        }
        ReferenceCommand::Status { json } => {
            let status = reference_repo_status(&omne_root).await?;
            print_json_or_pretty(json, &status)?;
        }
    }

    Ok(())
}

async fn reference_repo_status(omne_root: &Path) -> anyhow::Result<Value> {
    let ref_dir = omne_root.join("reference");
    let repo_dir = ref_dir.join("repo");
    let manifest_path = ref_dir.join("manifest.json");

    let repo_present = tokio::fs::try_exists(&repo_dir).await?;
    let manifest_present = tokio::fs::try_exists(&manifest_path).await?;

    let manifest = if manifest_present {
        let raw = tokio::fs::read_to_string(&manifest_path)
            .await
            .with_context(|| format!("read {}", manifest_path.display()))?;
        serde_json::from_str::<Value>(&raw).ok()
    } else {
        None
    };

    Ok(serde_json::json!({
        "omne_root": omne_root.display().to_string(),
        "repo_dir": repo_dir.display().to_string(),
        "repo_present": repo_present,
        "manifest_path": manifest_path.display().to_string(),
        "manifest_present": manifest_present,
        "manifest": manifest,
    }))
}

async fn reference_repo_import(
    omne_root: &Path,
    from: &Path,
    force: bool,
    max_file_bytes: u64,
) -> anyhow::Result<ReferenceRepoManifest> {
    let omne_root = omne_root.to_path_buf();
    let from = from.to_path_buf();

    tokio::task::spawn_blocking(move || {
        do_reference_repo_import(&omne_root, &from, force, max_file_bytes)
    })
    .await
    .context("join reference repo import task")?
}

fn do_reference_repo_import(
    omne_root: &Path,
    from: &Path,
    force: bool,
    max_file_bytes: u64,
) -> anyhow::Result<ReferenceRepoManifest> {
    if !from.is_dir() {
        anyhow::bail!("source is not a directory: {}", from.display());
    }

    let ref_dir = omne_root.join("reference");
    let repo_dir = ref_dir.join("repo");
    let manifest_path = ref_dir.join("manifest.json");

    std::fs::create_dir_all(&ref_dir).with_context(|| format!("create {}", ref_dir.display()))?;

    if repo_dir.exists() {
        if force {
            std::fs::remove_dir_all(&repo_dir)
                .with_context(|| format!("remove {}", repo_dir.display()))?;
        } else {
            anyhow::bail!(
                "reference repo already exists at {}; pass --force to overwrite",
                repo_dir.display()
            );
        }
    }
    std::fs::create_dir_all(&repo_dir).with_context(|| format!("create {}", repo_dir.display()))?;

    fn should_copy_entry(entry: &walkdir::DirEntry) -> bool {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name();
        if name == std::ffi::OsStr::new(".git")
            || name == std::ffi::OsStr::new(".ssh")
            || name == std::ffi::OsStr::new(".aws")
            || name == std::ffi::OsStr::new(".kube")
        {
            return false;
        }
        true
    }

    let mut removed = Vec::<ReferenceRepoRemovedEntry>::new();
    let mut stats = ReferenceRepoStats {
        dirs_created: 0,
        files_copied: 0,
        bytes_copied: 0,
        entries_skipped: 0,
    };

    for entry in WalkDir::new(from)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_copy_entry)
    {
        let entry = entry?;
        if entry.depth() == 0 {
            continue;
        }
        let rel = entry.path().strip_prefix(from).unwrap_or(entry.path());
        let dest = repo_dir.join(rel);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest)
                .with_context(|| format!("create {}", dest.display()))?;
            stats.dirs_created += 1;
            continue;
        }

        if entry.file_type().is_symlink() {
            removed.push(ReferenceRepoRemovedEntry {
                path: rel.to_string_lossy().to_string(),
                bytes: None,
                reason: "symlink".to_string(),
            });
            stats.entries_skipped += 1;
            continue;
        }

        if !entry.file_type().is_file() {
            removed.push(ReferenceRepoRemovedEntry {
                path: rel.to_string_lossy().to_string(),
                bytes: None,
                reason: "not_regular_file".to_string(),
            });
            stats.entries_skipped += 1;
            continue;
        }

        if omne_fs_policy::is_secret_rel_path(rel) {
            removed.push(ReferenceRepoRemovedEntry {
                path: rel.to_string_lossy().to_string(),
                bytes: None,
                reason: "sensitive".to_string(),
            });
            stats.entries_skipped += 1;
            continue;
        }
        if matches!(
            entry.path().extension().and_then(|s| s.to_str()),
            Some("pem" | "key")
        ) {
            removed.push(ReferenceRepoRemovedEntry {
                path: rel.to_string_lossy().to_string(),
                bytes: None,
                reason: "sensitive".to_string(),
            });
            stats.entries_skipped += 1;
            continue;
        }

        let meta = entry.metadata()?;
        let size = meta.len();
        if size > max_file_bytes {
            removed.push(ReferenceRepoRemovedEntry {
                path: rel.to_string_lossy().to_string(),
                bytes: Some(size),
                reason: "too_large".to_string(),
            });
            stats.entries_skipped += 1;
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::copy(entry.path(), &dest).with_context(|| {
            format!(
                "copy {} -> {}",
                entry.path().display(),
                dest.display()
            )
        })?;

        stats.files_copied += 1;
        stats.bytes_copied += size;
    }

    let created_at = OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    let manifest = ReferenceRepoManifest {
        version: 1,
        created_at,
        source: ReferenceRepoSource {
            path: from.display().to_string(),
        },
        max_file_bytes,
        removed,
        stats,
    };

    let bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(&manifest_path, bytes)
        .with_context(|| format!("write {}", manifest_path.display()))?;

    Ok(manifest)
}

#[cfg(test)]
mod reference_tests {
    use super::*;

    #[test]
    fn reference_import_skips_all_env_style_secret_variants() -> anyhow::Result<()> {
        let omne_root = tempfile::tempdir()?;
        let source = tempfile::tempdir()?;

        std::fs::write(source.path().join(".env.local"), "SECRET=1\n")?;
        std::fs::write(source.path().join(".env_prod"), "SECRET=1\n")?;
        std::fs::write(source.path().join(".env-staging"), "SECRET=1\n")?;
        std::fs::write(source.path().join(".env.example"), "SAFE=1\n")?;

        let manifest = do_reference_repo_import(omne_root.path(), source.path(), true, 1024)?;
        let removed = manifest
            .removed
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();

        assert!(removed.contains(&".env.local"));
        assert!(removed.contains(&".env_prod"));
        assert!(removed.contains(&".env-staging"));
        assert!(!removed.contains(&".env.example"));
        assert!(omne_root.path().join("reference/repo/.env.example").exists());
        Ok(())
    }
}
