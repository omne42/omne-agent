use omne_core::modes::Decision;

pub fn effective_decision(base: Decision, tool_override: Option<Decision>) -> Decision {
    match tool_override {
        Some(override_decision) => base.combine(override_decision),
        None => base,
    }
}

pub fn is_denied(decision: Decision) -> bool {
    decision == Decision::Deny
}

pub fn is_prompt(decision: Decision) -> bool {
    decision == Decision::Prompt
}
