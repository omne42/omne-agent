async fn capture_rotating_log<R>(
    mut reader: R,
    base_path: PathBuf,
    max_bytes_per_part: u64,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    let max_bytes_per_part = max_bytes_per_part.max(1);
    if let Some(parent) = base_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create dir {}", parent.display()))?;
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&base_path)
        .await
        .with_context(|| format!("open {}", base_path.display()))?;
    let mut current_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let mut next_part = next_rotating_log_part(&base_path).await?;

    let mut buf = vec![0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .await
            .with_context(|| format!("read process output into {}", base_path.display()))?;
        if n == 0 {
            break;
        }
        let mut offset = 0usize;
        while offset < n {
            let remaining = max_bytes_per_part.saturating_sub(current_len);
            if remaining == 0 {
                file.flush()
                    .await
                    .with_context(|| format!("flush {}", base_path.display()))?;
                drop(file);
                next_part = rotate_log_file(&base_path, next_part).await?;
                file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&base_path)
                    .await
                    .with_context(|| format!("open {}", base_path.display()))?;
                current_len = 0;
                continue;
            }

            let take = usize::try_from(remaining.min((n - offset) as u64)).unwrap_or(n - offset);
            file.write_all(&buf[offset..(offset + take)])
                .await
                .with_context(|| format!("write {}", base_path.display()))?;
            current_len = current_len.saturating_add(take as u64);
            offset = offset.saturating_add(take);
        }
    }

    file.flush()
        .await
        .with_context(|| format!("flush {}", base_path.display()))?;
    Ok(())
}

async fn rotate_log_file(base_path: &Path, mut part: u32) -> anyhow::Result<u32> {
    let Some(parent) = base_path.parent() else {
        return Ok(part);
    };
    let Some(stem) = base_path.file_stem().and_then(|s| s.to_str()) else {
        return Ok(part);
    };

    loop {
        let rotated = parent.join(format!("{stem}.segment-{part:04}.log"));
        match tokio::fs::rename(base_path, &rotated).await {
            Ok(()) => return Ok(part.saturating_add(1)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(part),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                part = part.saturating_add(1);
                continue;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("rename {} -> {}", base_path.display(), rotated.display())
                });
            }
        }
    }
}
