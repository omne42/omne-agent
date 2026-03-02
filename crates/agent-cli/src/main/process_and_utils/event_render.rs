fn format_approval_requested_details(action: &str, params: &serde_json::Value) -> String {
    let action_label = approval_action_label_from_action(action);
    let mut details = format!("action={action_label}");
    if let Some(summary) = approval_summary_from_params(params) {
        if let Some(display) = approval_summary_display_from_summary(&summary) {
            details.push_str(&format!(" summary={display}"));
        }
    }
    details
}

fn render_event(event: &ThreadEvent) {
    let ts = event
        .timestamp
        .format(&time::format_description::well_known::Rfc3339);
    let ts = ts.unwrap_or_else(|_| "<time>".to_string());
    match &event.kind {
        omne_protocol::ThreadEventKind::ThreadCreated { cwd } => {
            println!("[{ts}] thread created cwd={cwd}");
        }
        omne_protocol::ThreadEventKind::ThreadArchived { reason } => {
            println!(
                "[{ts}] thread archived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadUnarchived { reason } => {
            println!(
                "[{ts}] thread unarchived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadPaused { reason } => {
            println!(
                "[{ts}] thread paused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadUnpaused { reason } => {
            println!(
                "[{ts}] thread unpaused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::TurnStarted { turn_id, input, .. } => {
            println!("[{ts}] turn started {turn_id}");
            println!("user: {input}");
        }
        omne_protocol::ThreadEventKind::ModelRouted {
            turn_id,
            selected_model,
            rule_source,
            reason,
            rule_id,
        } => {
            println!(
                "[{ts}] model routed {turn_id} model={selected_model} source={rule_source:?} rule_id={} reason={}",
                rule_id.as_deref().unwrap_or(""),
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
            println!(
                "[{ts}] turn interrupt requested {turn_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } => {
            println!(
                "[{ts}] turn completed {turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            role,
            model,
            thinking,
            show_thinking,
            openai_base_url,
            allowed_tools,
            execpolicy_rules,
        } => {
            println!(
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} sandbox_writable_roots={sandbox_writable_roots:?} sandbox_network_access={sandbox_network_access:?} mode={} role={} model={} thinking={} show_thinking={} openai_base_url={} allowed_tools={allowed_tools:?} execpolicy_rules={execpolicy_rules:?}",
                mode.as_deref().unwrap_or(""),
                role.as_deref().unwrap_or(""),
                model.as_deref().unwrap_or(""),
                thinking.as_deref().unwrap_or(""),
                show_thinking
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                openai_base_url.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ApprovalRequested {
            approval_id,
            action,
            params,
            ..
        } => {
            println!(
                "[{ts}] approval requested {approval_id} {}",
                format_approval_requested_details(action, params)
            );
        }
        omne_protocol::ThreadEventKind::ApprovalDecided {
            approval_id,
            decision,
            remember,
            reason,
        } => {
            println!(
                "[{ts}] approval decided {approval_id} decision={decision:?} remember={remember} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ToolStarted { tool, params, .. } => {
            let mapping = format_facade_mapping_suffix(tool, params.as_ref()).unwrap_or_default();
            println!("[{ts}] tool started {tool}{mapping}");
        }
        omne_protocol::ThreadEventKind::ToolCompleted {
            status,
            error,
            result,
            ..
        } => {
            let mapping = format_facade_mapping_suffix("", result.as_ref()).unwrap_or_default();
            println!(
                "[{ts}] tool completed status={status:?} error={}{}",
                error.as_deref().unwrap_or(""),
                mapping
            );
        }
        omne_protocol::ThreadEventKind::AgentStep {
            turn_id,
            step,
            model,
            response_id,
            text,
            tool_calls,
            tool_results,
            ..
        } => {
            println!(
                "[{ts}] step {step} turn_id={turn_id} model={model} response_id={response_id} tool_calls={} tool_results={}",
                tool_calls.len(),
                tool_results.len()
            );
            if let Some(text) = text.as_deref().filter(|s| !s.trim().is_empty()) {
                println!("{text}");
            }
        }
        omne_protocol::ThreadEventKind::AssistantMessage { text, model, .. } => {
            if let Some(model) = model {
                println!("[{ts}] assistant (model={model}):");
            } else {
                println!("[{ts}] assistant:");
            }
            println!("{text}");
        }
        omne_protocol::ThreadEventKind::ProcessStarted {
            process_id, argv, ..
        } => {
            println!("[{ts}] process started {process_id} argv={argv:?}");
        }
        omne_protocol::ThreadEventKind::ProcessInterruptRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process interrupt requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ProcessKillRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process kill requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => {
            println!(
                "[{ts}] process exited {process_id} exit_code={} reason={}",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::AttentionMarkerSet {
            marker,
            turn_id,
            artifact_id,
            artifact_type,
            process_id,
            exit_code,
            command,
        } => {
            println!(
                "[{ts}] attention marker set marker={marker:?} turn_id={} artifact_id={} artifact_type={} process_id={} exit_code={} command={}",
                turn_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                artifact_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                artifact_type.as_deref().unwrap_or(""),
                process_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                command.as_deref().unwrap_or(""),
            );
        }
        omne_protocol::ThreadEventKind::AttentionMarkerCleared {
            marker,
            turn_id,
            reason,
        } => {
            println!(
                "[{ts}] attention marker cleared marker={marker:?} turn_id={} reason={}",
                turn_id
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                reason.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::CheckpointCreated {
            checkpoint_id,
            label,
            snapshot_ref,
            ..
        } => {
            println!(
                "[{ts}] checkpoint created {checkpoint_id} label={} snapshot_ref={snapshot_ref}",
                label.as_deref().unwrap_or("")
            );
        }
        omne_protocol::ThreadEventKind::CheckpointRestored {
            checkpoint_id,
            status,
            reason,
            ..
        } => {
            println!(
                "[{ts}] checkpoint restored {checkpoint_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
    }
}

#[cfg(test)]
mod event_render_tests {
    use super::*;

    #[test]
    fn format_approval_requested_details_includes_subagent_child_and_path() {
        let child_thread_id = omne_protocol::ThreadId::new();
        let child_approval_id = omne_protocol::ApprovalId::new();
        let params = serde_json::json!({
            "subagent_proxy": {
                "child_thread_id": child_thread_id,
                "child_approval_id": child_approval_id
            },
            "child_request": {
                "action": "file/write",
                "params": {
                    "path": "/tmp/ws/src/main.rs"
                }
            }
        });
        let details = format_approval_requested_details("subagent/proxy_approval", &params);
        assert!(details.contains("action=subagent/proxy_approval"));
        assert!(details.contains("child_thread_id="));
        assert!(details.contains(&child_thread_id.to_string()));
        assert!(details.contains("child_approval_id="));
        assert!(details.contains(&child_approval_id.to_string()));
        assert!(details.contains("path=/tmp/ws/src/main.rs"));
    }

    #[test]
    fn format_facade_mapping_suffix_extracts_fields() {
        let suffix = format_facade_mapping_suffix(
            "facade/thread",
            Some(&serde_json::json!({
                "facade_tool": "thread",
                "op": "wait",
                "mapped_action": "subagent/wait"
            })),
        )
        .expect("facade mapping");
        assert!(suffix.contains("facade=thread"));
        assert!(suffix.contains("op=wait"));
        assert!(suffix.contains("mapped_action=subagent/wait"));
    }
}
