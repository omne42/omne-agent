#[cfg(test)]
mod thread_fork_tests {
    use super::*;

    #[tokio::test]
    async fn thread_fork_projects_readable_history_from_copied_events() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let rt = server.get_or_load_thread(thread_id).await?;
        let turn_id = TurnId::new();
        rt.append_event(omne_protocol::ThreadEventKind::TurnStarted {
            turn_id,
            input: "fork user".to_string(),
            context_refs: None,
            attachments: None,
            directives: None,
            priority: omne_protocol::TurnPriority::Foreground,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::AssistantMessage {
            turn_id: Some(turn_id),
            text: "fork assistant".to_string(),
            model: Some("test".to_string()),
            response_id: Some("resp_fork_readable".to_string()),
            token_usage: None,
        })
        .await?;
        rt.append_event(omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status: omne_protocol::TurnStatus::Completed,
            reason: None,
        })
        .await?;

        let forked = handle_thread_fork(
            &server,
            ThreadForkParams {
                thread_id,
                cwd: None,
            },
        )
        .await?;
        let forked_thread_id = forked.thread_id;

        let readable_history_path = server.thread_store.readable_history_path(forked_thread_id);
        let raw = tokio::fs::read_to_string(&readable_history_path)
            .await
            .with_context(|| format!("read {}", readable_history_path.display()))?;
        assert!(raw.contains("\"text\":\"fork user\""));
        assert!(raw.contains("\"text\":\"fork assistant\""));
        Ok(())
    }
}

