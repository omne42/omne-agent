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

fn resolve_mode_and_role_decision_audit<F>(
    catalog: &omne_core::modes::ModeCatalog,
    mode: &omne_core::modes::ModeDef,
    role_name: Option<&str>,
    action: &str,
    base_decision_for_mode: F,
) -> ModeDecisionAudit
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
{
    let mode_decision = resolve_mode_decision_audit(mode, action, base_decision_for_mode(mode));
    let Some(role_name) = role_name.map(str::trim).filter(|role| !role.is_empty()) else {
        return mode_decision;
    };

    let role_catalog = omne_core::roles::RoleCatalog::builtin();
    let Some(permission_mode_name) = role_catalog.permission_mode_name(role_name) else {
        return mode_decision;
    };
    let Some(role_permission_mode) = catalog.mode(permission_mode_name) else {
        return mode_decision;
    };

    let role_decision = resolve_mode_decision_audit(
        role_permission_mode,
        action,
        base_decision_for_mode(role_permission_mode),
    );
    let combined = mode_decision.decision.combine(role_decision.decision);
    let role_tightened = combined != mode_decision.decision;

    ModeDecisionAudit {
        decision: combined,
        decision_source: if role_tightened {
            "role_permission_mode"
        } else {
            mode_decision.decision_source
        },
        tool_override_hit: mode_decision.tool_override_hit || role_decision.tool_override_hit,
    }
}

macro_rules! map_mode_decision_for_protocol {
    ($decision:expr, $enum:ty) => {
        match $decision {
            omne_core::modes::Decision::Allow => <$enum>::Allow,
            omne_core::modes::Decision::Prompt => <$enum>::Prompt,
            omne_core::modes::Decision::Deny => <$enum>::Deny,
        }
    };
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

    #[test]
    fn mode_gate_role_permission_mode_can_tighten_decision() {
        let catalog = omne_core::modes::ModeCatalog::builtin();
        let mode = catalog.mode("coder").expect("builtin coder mode must exist");

        let audit = resolve_mode_and_role_decision_audit(
            &catalog,
            mode,
            Some("chat"),
            "process/start",
            |mode| mode.permissions.command,
        );

        assert_eq!(audit.decision, omne_core::modes::Decision::Deny);
        assert_eq!(audit.decision_source, "role_permission_mode");
    }

    #[test]
    fn mode_gate_unknown_role_falls_back_to_mode_decision() {
        let catalog = omne_core::modes::ModeCatalog::builtin();
        let mode = catalog.mode("coder").expect("builtin coder mode must exist");

        let audit = resolve_mode_and_role_decision_audit(
            &catalog,
            mode,
            Some("legacy-role"),
            "process/start",
            |mode| mode.permissions.command,
        );

        assert_eq!(audit.decision, omne_core::modes::Decision::Prompt);
        assert_eq!(audit.decision_source, "mode_permission");
    }
}
