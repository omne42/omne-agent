async fn list_rotating_log_files(base_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut parts = scan_rotating_log_parts(base_path).await?;
    parts.sort_by_key(|(part, _)| *part);
    let mut files = parts.into_iter().map(|(_, path)| path).collect::<Vec<_>>();
    if tokio::fs::metadata(base_path).await.is_ok() {
        files.push(base_path.to_path_buf());
    }
    Ok(files)
}

async fn next_rotating_log_part(base_path: &Path) -> anyhow::Result<u32> {
    let parts = scan_rotating_log_parts(base_path).await?;
    let next_part = parts
        .iter()
        .map(|(part, _)| *part)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    Ok(next_part.max(1))
}

async fn scan_rotating_log_parts(base_path: &Path) -> anyhow::Result<Vec<(u32, PathBuf)>> {
    let Some(parent) = base_path.parent() else {
        return Ok(Vec::new());
    };
    let Some(stem) = base_path.file_stem().and_then(|s| s.to_str()) else {
        return Ok(Vec::new());
    };

    let mut parts = Vec::<(u32, PathBuf)>::new();
    let mut read_dir = match tokio::fs::read_dir(parent).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", parent.display())),
    };

    let prefixes = [format!("{stem}.segment-"), format!("{stem}.part-")];
    while let Some(entry) = read_dir.next_entry().await? {
        let ty = entry.file_type().await?;
        if !ty.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(rest) = prefixes.iter().find_map(|prefix| name.strip_prefix(prefix)) else {
            continue;
        };
        let Some(part_str) = rest.strip_suffix(".log") else {
            continue;
        };
        let Ok(part) = part_str.parse::<u32>() else {
            continue;
        };
        parts.push((part, entry.path()));
    }

    Ok(parts)
}
