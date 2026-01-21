async fn handle_process_tail(server: &Server, params: ProcessTailParams) -> anyhow::Result<Value> {
    let (stdout_path, stderr_path) = resolve_process_log_paths(server, params.process_id).await?;

    let path = match params.stream {
        ProcessStream::Stdout => stdout_path,
        ProcessStream::Stderr => stderr_path,
    };

    let max_lines = params.max_lines.unwrap_or(200).min(2000);
    let text = tail_file_lines(PathBuf::from(path), max_lines).await?;
    let text = pm_core::redact_text(&text);
    Ok(serde_json::json!({ "text": text }))
}

async fn tail_file_lines(path: PathBuf, max_lines: usize) -> anyhow::Result<String> {
    let files = list_rotating_log_files(&path).await?;
    if files.is_empty() {
        return Ok(String::new());
    }

    let mut collected = Vec::<String>::new();
    for file in files.into_iter().rev() {
        let lines = tail_single_file_lines(&file).await?;
        for line in lines.into_iter().rev() {
            collected.push(line);
            if collected.len() >= max_lines {
                break;
            }
        }
        if collected.len() >= max_lines {
            break;
        }
    }

    collected.reverse();
    Ok(collected.join("\n"))
}

async fn tail_single_file_lines(path: &Path) -> anyhow::Result<Vec<String>> {
    let max_bytes: u64 = 256 * 1024;
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };
    let len = file
        .metadata()
        .await
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))
        .await
        .with_context(|| format!("seek {}", path.display()))?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .with_context(|| format!("read {}", path.display()))?;

    let mut text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 {
        if let Some(pos) = text.find('\n') {
            text = text[(pos + 1)..].to_string();
        }
    }

    Ok(text.lines().map(ToString::to_string).collect::<Vec<_>>())
}

async fn handle_process_follow(
    server: &Server,
    params: ProcessFollowParams,
) -> anyhow::Result<Value> {
    let (stdout_path, stderr_path) = resolve_process_log_paths(server, params.process_id).await?;

    let path = match params.stream {
        ProcessStream::Stdout => stdout_path,
        ProcessStream::Stderr => stderr_path,
    };

    let max_bytes = params.max_bytes.unwrap_or(64 * 1024).min(1024 * 1024);
    let (text, next_offset, eof) =
        read_file_chunk(PathBuf::from(path), params.since_offset, max_bytes).await?;
    let text = pm_core::redact_text(&text);

    Ok(serde_json::json!({
        "text": text,
        "next_offset": next_offset,
        "eof": eof,
    }))
}

async fn resolve_process_log_paths(
    server: &Server,
    process_id: ProcessId,
) -> anyhow::Result<(String, String)> {
    let entry = server.processes.lock().await.get(&process_id).cloned();

    if let Some(entry) = entry {
        let info = entry.info.lock().await;
        return Ok((info.stdout_path.clone(), info.stderr_path.clone()));
    }

    let mut processes = handle_process_list(server, ProcessListParams { thread_id: None }).await?;
    let info = processes
        .iter_mut()
        .find(|p| p.process_id == process_id)
        .ok_or_else(|| anyhow::anyhow!("process not found: {}", process_id))?;
    Ok((info.stdout_path.clone(), info.stderr_path.clone()))
}

async fn read_file_chunk(
    path: PathBuf,
    since_offset: u64,
    max_bytes: u64,
) -> anyhow::Result<(String, u64, bool)> {
    let files = list_rotating_log_files(&path).await?;
    if files.is_empty() {
        return Ok((String::new(), since_offset, true));
    }

    let max_bytes = max_bytes.min(1024 * 1024);
    let mut lengths = Vec::<u64>::new();
    let mut total = 0u64;
    for file in &files {
        let len = match tokio::fs::metadata(file).await {
            Ok(meta) => meta.len(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
            Err(err) => return Err(err).with_context(|| format!("stat {}", file.display())),
        };
        lengths.push(len);
        total = total.saturating_add(len);
    }

    let start = since_offset.min(total);
    let mut remaining_offset = start;
    let mut remaining_bytes = max_bytes;
    let mut out: Vec<u8> = Vec::new();

    for (idx, file) in files.iter().enumerate() {
        let len = lengths.get(idx).copied().unwrap_or(0);
        if remaining_offset >= len {
            remaining_offset = remaining_offset.saturating_sub(len);
            continue;
        }

        if remaining_bytes == 0 {
            break;
        }

        let mut f = match tokio::fs::File::open(file).await {
            Ok(f) => f,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err).with_context(|| format!("open {}", file.display())),
        };
        f.seek(SeekFrom::Start(remaining_offset))
            .await
            .with_context(|| format!("seek {}", file.display()))?;

        let buf_len = usize::try_from(remaining_bytes).unwrap_or(1024 * 1024);
        let mut buf = vec![0u8; buf_len];
        let mut n_total = 0usize;
        while n_total < buf_len {
            let n = f
                .read(&mut buf[n_total..])
                .await
                .with_context(|| format!("read {}", file.display()))?;
            if n == 0 {
                break;
            }
            n_total = n_total.saturating_add(n);
        }
        let n = n_total;
        buf.truncate(n);
        remaining_offset = 0;
        remaining_bytes = remaining_bytes.saturating_sub(n as u64);
        out.extend_from_slice(&buf);

        if n == 0 {
            continue;
        }
        if remaining_bytes == 0 {
            break;
        }
    }

    let next_offset = start + out.len() as u64;
    let eof = next_offset >= total;
    let text = String::from_utf8_lossy(&out).to_string();
    Ok((text, next_offset, eof))
}

async fn list_rotating_log_files(base_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
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

    let prefix = format!("{stem}.part-");
    while let Some(entry) = read_dir.next_entry().await? {
        let ty = entry.file_type().await?;
        if !ty.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(rest) = name.strip_prefix(&prefix) else {
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

    parts.sort_by_key(|(part, _)| *part);
    let mut files = parts.into_iter().map(|(_, path)| path).collect::<Vec<_>>();
    if tokio::fs::metadata(base_path).await.is_ok() {
        files.push(base_path.to_path_buf());
    }
    Ok(files)
}

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
    let mut next_part = 1u32;

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
        let rotated = parent.join(format!("{stem}.part-{part:04}.log"));
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

async fn handle_process_inspect(
    server: &Server,
    params: ProcessInspectParams,
) -> anyhow::Result<Value> {
    let mut info: Option<ProcessInfo> = None;
    if let Some(entry) = server
        .processes
        .lock()
        .await
        .get(&params.process_id)
        .cloned()
    {
        info = Some(entry.info.lock().await.clone());
    }

    let info = match info {
        Some(info) => info,
        None => {
            let processes =
                handle_process_list(server, ProcessListParams { thread_id: None }).await?;
            processes
                .into_iter()
                .find(|p| p.process_id == params.process_id)
                .ok_or_else(|| anyhow::anyhow!("process not found: {}", params.process_id))?
        }
    };

    let max_lines = params.max_lines.unwrap_or(200).min(2000);
    let stdout_tail =
        pm_core::redact_text(&tail_file_lines(PathBuf::from(&info.stdout_path), max_lines).await?);
    let stderr_tail =
        pm_core::redact_text(&tail_file_lines(PathBuf::from(&info.stderr_path), max_lines).await?);

    Ok(serde_json::json!({
        "process": info,
        "stdout_tail": stdout_tail,
        "stderr_tail": stderr_tail,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn process_logs_rotate_and_follow_reads_across_parts() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let base_path = dir.path().join("stdout.log");

        let payload = "0123456789abcdefghijXXXXX".to_string();
        let payload_bytes = payload.clone().into_bytes();
        let payload_bytes_for_task = payload_bytes.clone();

        let (mut writer, reader) = tokio::io::duplex(64);
        let write_task = tokio::spawn(async move {
            writer.write_all(&payload_bytes_for_task).await?;
            writer.shutdown().await?;
            anyhow::Ok(())
        });

        capture_rotating_log(reader, base_path.clone(), 10).await?;
        write_task.await??;

        let part1 = dir.path().join("stdout.part-0001.log");
        let part2 = dir.path().join("stdout.part-0002.log");

        assert_eq!(tokio::fs::metadata(&part1).await?.len(), 10);
        assert_eq!(tokio::fs::metadata(&part2).await?.len(), 10);
        assert_eq!(tokio::fs::metadata(&base_path).await?.len(), 5);

        let (text, next_offset, eof) = read_file_chunk(base_path.clone(), 0, 1024).await?;
        assert_eq!(text, payload);
        assert_eq!(next_offset, payload_bytes.len() as u64);
        assert!(eof);

        Ok(())
    }
}
