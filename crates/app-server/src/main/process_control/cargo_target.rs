fn is_cargo_command(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };

    let mut name = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program.as_str())
        .to_ascii_lowercase();
    if let Some(stripped) = name.strip_suffix(".exe") {
        name = stripped.to_string();
    }

    name == "cargo"
}

fn fnv1a_64(input: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

async fn find_git_common_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let git_path = dir.join(".git");
        let meta = match tokio::fs::metadata(&git_path).await {
            Ok(meta) => Some(meta),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => None,
        };

        if let Some(meta) = meta {
            if meta.is_dir() {
                let resolved = tokio::fs::canonicalize(&git_path).await.unwrap_or(git_path);
                return Some(resolved);
            }

            if meta.is_file() {
                let contents = tokio::fs::read_to_string(&git_path).await.ok()?;
                let line = contents
                    .lines()
                    .find(|line| line.trim_start().starts_with("gitdir:"))?;
                let gitdir_raw = line.trim_start().strip_prefix("gitdir:")?.trim();
                if gitdir_raw.is_empty() {
                    return None;
                }

                let gitdir_path = {
                    let raw = Path::new(gitdir_raw);
                    if raw.is_absolute() {
                        raw.to_path_buf()
                    } else {
                        dir.join(raw)
                    }
                };
                let gitdir_path =
                    tokio::fs::canonicalize(&gitdir_path).await.unwrap_or(gitdir_path);

                let commondir_path = gitdir_path.join("commondir");
                let commondir = tokio::fs::read_to_string(&commondir_path).await.ok();
                let commondir = commondir
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if let Some(commondir) = commondir {
                    let rel = Path::new(commondir);
                    let common = if rel.is_absolute() {
                        rel.to_path_buf()
                    } else {
                        gitdir_path.join(rel)
                    };
                    let common = tokio::fs::canonicalize(&common).await.unwrap_or(common);
                    return Some(common);
                }

                return Some(gitdir_path);
            }
        }

        let parent = dir.parent()?;
        dir = parent.to_path_buf();
    }
}

async fn resolve_shared_cargo_target_dir(server: &Server, cwd: &Path) -> Option<PathBuf> {
    let common_git_dir = find_git_common_dir(cwd).await?;
    let common_git_dir_str = common_git_dir.to_string_lossy();
    let key = format!("{:016x}", fnv1a_64(common_git_dir_str.as_ref()));

    let root = server.thread_store.agent_paths().data_dir().join("cargo-target");
    let target_dir = root.join(key);
    if tokio::fs::create_dir_all(&root).await.is_err() {
        return None;
    }
    Some(target_dir)
}

