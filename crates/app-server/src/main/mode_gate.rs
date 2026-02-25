#[derive(Clone, Copy, Debug)]
struct ModeDecisionAudit {
    decision: omne_core::modes::Decision,
    decision_source: &'static str,
    tool_override_hit: bool,
}

fn resolve_mode_decision_audit(
    mode: &omne_core::modes::ModeDef,
    action: &str,
    base_decision: omne_core::modes::Decision,
) -> ModeDecisionAudit {
    let tool_override_decision = mode.tool_overrides.get(action).copied();
    let tool_override_hit = tool_override_decision.is_some();
    let decision_source = if tool_override_hit {
        "tool_override"
    } else {
        "mode_permission"
    };
    let decision = match tool_override_decision {
        Some(override_decision) => base_decision.combine(override_decision),
        None => base_decision,
    };

    ModeDecisionAudit {
        decision,
        decision_source,
        tool_override_hit,
    }
}

#[cfg(test)]
mod mode_gate_tests {
    use super::*;

    #[test]
    fn mode_gate_without_override_uses_base_decision() {
        let catalog = omne_core::modes::ModeCatalog::builtin();
        let mode = catalog.mode("coder").expect("builtin coder mode must exist");

        let audit = resolve_mode_decision_audit(
            mode,
            "nonexistent/tool",
            omne_core::modes::Decision::Prompt,
        );

        assert_eq!(audit.decision, omne_core::modes::Decision::Prompt);
        assert_eq!(audit.decision_source, "mode_permission");
        assert!(!audit.tool_override_hit);
    }

    #[test]
    fn mode_gate_with_override_reports_override_source() {
        let catalog = omne_core::modes::ModeCatalog::builtin();
        let mut mode = catalog
            .mode("coder")
            .expect("builtin coder mode must exist")
            .clone();
        mode.tool_overrides
            .insert("file/read".to_string(), omne_core::modes::Decision::Deny);

        let audit = resolve_mode_decision_audit(
            &mode,
            "file/read",
            omne_core::modes::Decision::Allow,
        );

        assert_eq!(audit.decision, omne_core::modes::Decision::Deny);
        assert_eq!(audit.decision_source, "tool_override");
        assert!(audit.tool_override_hit);
    }

    #[test]
    fn mode_gate_override_allow_does_not_weaken_base_deny() {
        let catalog = omne_core::modes::ModeCatalog::builtin();
        let mut mode = catalog
            .mode("coder")
            .expect("builtin coder mode must exist")
            .clone();
        mode.tool_overrides
            .insert("file/read".to_string(), omne_core::modes::Decision::Allow);

        let audit = resolve_mode_decision_audit(
            &mode,
            "file/read",
            omne_core::modes::Decision::Deny,
        );

        assert_eq!(audit.decision, omne_core::modes::Decision::Deny);
        assert_eq!(audit.decision_source, "tool_override");
        assert!(audit.tool_override_hit);
    }
}
