#[derive(Debug)]
struct ApprovalOutcome {
    decision: ApprovalDecision,
    remember: bool,
    reason: Option<String>,
}

async fn wait_for_approval_outcome(
    server: &super::Server,
    thread_id: omne_agent_protocol::ThreadId,
    approval_id: ApprovalId,
    cancel: CancellationToken,
) -> anyhow::Result<ApprovalOutcome> {
    let mut since = EventSeq::ZERO;
    loop {
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled waiting for approval");
        }

        let events = server
            .thread_store
            .read_events_since(thread_id, since)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        since = events.last().map(|e| e.seq).unwrap_or(since);

        for event in events {
            if let ThreadEventKind::ApprovalDecided {
                approval_id: id,
                decision,
                remember,
                reason,
                ..
            } = event.kind
                && id == approval_id
            {
                return Ok(ApprovalOutcome {
                    decision,
                    remember,
                    reason,
                });
            }
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn parse_needs_approval(value: &Value) -> anyhow::Result<Option<ApprovalId>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let Some(needs_approval) = obj.get("needs_approval").and_then(|v| v.as_bool()) else {
        return Ok(None);
    };
    if !needs_approval {
        return Ok(None);
    }
    let Some(approval_id) = obj.get("approval_id").and_then(|v| v.as_str()) else {
        anyhow::bail!("tool returned needs_approval without approval_id");
    };
    Ok(Some(approval_id.parse()?))
}
