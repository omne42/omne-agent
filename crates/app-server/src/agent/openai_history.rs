use tokio::io::AsyncWriteExt;

const OPENAI_RESPONSES_HISTORY_FILE_NAME: &str = "openai_responses_history.jsonl";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpenAiResponsesHistoryRecord {
    Item { item: serde_json::Value },
    Compacted {
        replacement_history: Vec<serde_json::Value>,
    },
}

fn openai_responses_history_path(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> std::path::PathBuf {
    thread_store
        .thread_dir(thread_id)
        .join(OPENAI_RESPONSES_HISTORY_FILE_NAME)
}

async fn append_openai_responses_history_records(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    records: &[OpenAiResponsesHistoryRecord],
) -> anyhow::Result<()> {
    if records.is_empty() {
        return Ok(());
    }

    let path = openai_responses_history_path(thread_store, thread_id);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("open {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        let _ = tokio::fs::set_permissions(&path, perm).await;
    }

    for record in records {
        let line = serde_json::to_string(record).context("serialize openai history record")?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }

    Ok(())
}

async fn append_openai_responses_history_items(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    items: &[serde_json::Value],
) -> anyhow::Result<()> {
    let records = items
        .iter()
        .cloned()
        .map(|item| OpenAiResponsesHistoryRecord::Item { item })
        .collect::<Vec<_>>();
    append_openai_responses_history_records(thread_store, thread_id, &records).await
}

async fn append_openai_responses_history_compacted(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    replacement_history: Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    append_openai_responses_history_records(
        thread_store,
        thread_id,
        &[OpenAiResponsesHistoryRecord::Compacted { replacement_history }],
    )
    .await
}

async fn read_openai_responses_history(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let path = openai_responses_history_path(thread_store, thread_id);
    let data = match tokio::fs::read_to_string(&path).await {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };

    let mut history = Vec::<serde_json::Value>::new();
    for (idx, line) in data.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record = serde_json::from_str::<OpenAiResponsesHistoryRecord>(line).with_context(
            || format!("parse openai history record: {} (line={})", path.display(), idx + 1),
        )?;

        match record {
            OpenAiResponsesHistoryRecord::Item { item } => history.push(item),
            OpenAiResponsesHistoryRecord::Compacted {
                replacement_history,
            } => history = replacement_history,
        }
    }

    Ok(history)
}

async fn compact_openai_responses_history(
    thread_store: &omne_core::ThreadStore,
    thread_id: omne_protocol::ThreadId,
    client: &ditto_llm::OpenAI,
    model: &str,
    instructions: &str,
    input: &[serde_json::Value],
) -> anyhow::Result<Vec<serde_json::Value>> {
    let replacement_history = client
        .compact_responses_history_raw(&ditto_llm::providers::openai::OpenAIResponsesCompactionRequest {
            model,
            input,
            instructions,
        })
        .await
        .map_err(anyhow::Error::new)?;

    append_openai_responses_history_compacted(thread_store, thread_id, replacement_history.clone())
        .await?;

    Ok(replacement_history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn openai_history_replays_compaction_replacement() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let thread_store = omne_core::ThreadStore::new(omne_core::PmPaths::new(
            dir.path().join(".omne_data"),
        ));

        let handle = thread_store
            .create_thread(std::path::PathBuf::from("/tmp"))
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        append_openai_responses_history_items(
            &thread_store,
            thread_id,
            &[serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"a"}]})],
        )
        .await?;

        append_openai_responses_history_compacted(
            &thread_store,
            thread_id,
            vec![serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"b"}]})],
        )
        .await?;

        append_openai_responses_history_items(
            &thread_store,
            thread_id,
            &[serde_json::json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"c"}]})],
        )
        .await?;

        let history = read_openai_responses_history(&thread_store, thread_id).await?;
        assert_eq!(
            history,
            vec![
                serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"b"}]}),
                serde_json::json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"c"}]}),
            ]
        );

        Ok(())
    }
}
