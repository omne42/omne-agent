#[derive(Debug, Clone, Copy)]
enum DoomLoopDecision {
    Approved,
    Denied { remembered: bool },
}

#[allow(clippy::too_many_arguments)]
async fn gate_doom_loop(
    server: &super::Server,
    thread_rt: &Arc<super::ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: TurnId,
    approval_policy: omne_agent_protocol::ApprovalPolicy,
    kind: &'static str,
    signature: u64,
    tool_calls_for_event: &[omne_agent_protocol::AgentStepToolCall],
    cancel: CancellationToken,
) -> anyhow::Result<DoomLoopDecision> {
    let tool_calls_for_prompt = tool_calls_for_event
        .iter()
        .map(|call| {
            serde_json::json!({
                "name": call.name,
                "call_id": call.call_id,
                "arguments": truncate_chars(call.arguments.as_str(), 500),
            })
        })
        .collect::<Vec<_>>();

    let approval_params = serde_json::json!({
        "approval": { "requirement": "prompt_strict" },
        "kind": kind,
        "signature": signature,
        "tool_calls": tool_calls_for_prompt,
    });

    match super::gate_approval(
        server,
        thread_rt,
        thread_id,
        Some(turn_id),
        approval_policy,
        super::ApprovalRequest {
            approval_id: None,
            action: "doom_loop",
            params: &approval_params,
        },
    )
    .await?
    {
        super::ApprovalGate::Approved => Ok(DoomLoopDecision::Approved),
        super::ApprovalGate::Denied { remembered } => Ok(DoomLoopDecision::Denied { remembered }),
        super::ApprovalGate::NeedsApproval { approval_id } => {
            let outcome =
                wait_for_approval_outcome(server, thread_id, approval_id, cancel).await?;
            match outcome.decision {
                omne_agent_protocol::ApprovalDecision::Approved => Ok(DoomLoopDecision::Approved),
                omne_agent_protocol::ApprovalDecision::Denied => Ok(DoomLoopDecision::Denied {
                    remembered: outcome.remember,
                }),
            }
        }
    }
}
