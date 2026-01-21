#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn process_logs_rotate_and_follow_reads_across_segments() -> anyhow::Result<()> {
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

        let part1 = dir.path().join("stdout.segment-0001.log");
        let part2 = dir.path().join("stdout.segment-0002.log");

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
