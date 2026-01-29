fn render_event(event: &ThreadEvent) {
    let ts = event
        .timestamp
        .format(&time::format_description::well_known::Rfc3339);
    let ts = ts.unwrap_or_else(|_| "<time>".to_string());
    match &event.kind {
        pm_protocol::ThreadEventKind::ThreadCreated { cwd } => {
            println!("[{ts}] thread created cwd={cwd}");
        }
        pm_protocol::ThreadEventKind::ThreadArchived { reason } => {
            println!(
                "[{ts}] thread archived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnarchived { reason } => {
            println!(
                "[{ts}] thread unarchived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadPaused { reason } => {
            println!(
                "[{ts}] thread paused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnpaused { reason } => {
            println!(
                "[{ts}] thread unpaused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnStarted { turn_id, input, .. } => {
            println!("[{ts}] turn started {turn_id}");
            println!("user: {input}");
        }
        pm_protocol::ThreadEventKind::ModelRouted {
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
        pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
            println!(
                "[{ts}] turn interrupt requested {turn_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } => {
            println!(
                "[{ts}] turn completed {turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            openai_provider,
            model,
            thinking,
            openai_base_url,
            allowed_tools,
        } => {
            println!(
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} sandbox_writable_roots={sandbox_writable_roots:?} sandbox_network_access={sandbox_network_access:?} mode={} openai_provider={} model={} thinking={} openai_base_url={} allowed_tools={allowed_tools:?}",
                mode.as_deref().unwrap_or(""),
                openai_provider.as_deref().unwrap_or(""),
                model.as_deref().unwrap_or(""),
                thinking.as_deref().unwrap_or(""),
                openai_base_url.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ApprovalRequested {
            approval_id,
            action,
            ..
        } => {
            println!("[{ts}] approval requested {approval_id} action={action}");
        }
        pm_protocol::ThreadEventKind::ApprovalDecided {
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
        pm_protocol::ThreadEventKind::ToolStarted { tool, .. } => {
            println!("[{ts}] tool started {tool}");
        }
        pm_protocol::ThreadEventKind::ToolCompleted { status, error, .. } => {
            println!(
                "[{ts}] tool completed status={status:?} error={}",
                error.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::AgentStep {
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
        pm_protocol::ThreadEventKind::AssistantMessage { text, model, .. } => {
            if let Some(model) = model {
                println!("[{ts}] assistant (model={model}):");
            } else {
                println!("[{ts}] assistant:");
            }
            println!("{text}");
        }
        pm_protocol::ThreadEventKind::ProcessStarted {
            process_id, argv, ..
        } => {
            println!("[{ts}] process started {process_id} argv={argv:?}");
        }
        pm_protocol::ThreadEventKind::ProcessInterruptRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process interrupt requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessKillRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process kill requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessExited {
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
        pm_protocol::ThreadEventKind::CheckpointCreated {
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
        pm_protocol::ThreadEventKind::CheckpointRestored {
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
