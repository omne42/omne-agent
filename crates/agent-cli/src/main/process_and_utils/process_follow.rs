async fn run_process_follow(
    app: &mut App,
    process_id: ProcessId,
    stderr: bool,
    mut offset: u64,
    max_bytes: Option<u64>,
    poll_ms: u64,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(poll_ms.max(50));
    loop {
        let (text, next_offset, eof) = app
            .process_follow(process_id, stderr, offset, max_bytes, approval_id)
            .await?;
        offset = next_offset;
        if !text.is_empty() {
            print!("{text}");
            std::io::stdout().flush().ok();
        }

        if eof {
            let status = app.process_status(process_id).await?;
            if status != "running" {
                return Ok(());
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}
