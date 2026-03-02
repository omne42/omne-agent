use crate::modes::{Decision, ModeDef};

const KNOWN_ALLOWED_TOOLS: &[&str] = &[
    "facade/workspace",
    "facade/process",
    "facade/thread",
    "facade/artifact",
    "facade/integration",
    "file/read",
    "file/glob",
    "file/grep",
    "file/write",
    "file/patch",
    "file/edit",
    "file/delete",
    "fs/mkdir",
    "repo/search",
    "repo/index",
    "repo/symbols",
    "mcp/list_servers",
    "mcp/list_tools",
    "mcp/list_resources",
    "mcp/call",
    "artifact/write",
    "artifact/list",
    "artifact/read",
    "artifact/delete",
    "thread/request_user_input",
    "thread/diff",
    "thread/state",
    "thread/usage",
    "thread/events",
    "thread/hook_run",
    "web/search",
    "web/fetch",
    "web/view_image",
    "subagent/spawn",
    "subagent/send_input",
    "subagent/wait",
    "subagent/close",
    "process/start",
    "process/list",
    "process/inspect",
    "process/kill",
    "process/interrupt",
    "process/tail",
    "process/follow",
];

pub fn known_allowed_tools() -> &'static [&'static str] {
    KNOWN_ALLOWED_TOOLS
}

pub fn is_known_allowed_tool(tool: &str) -> bool {
    KNOWN_ALLOWED_TOOLS.contains(&tool)
}

pub fn normalize_allowed_tools(tools: Vec<String>) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::<String>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for tool in tools {
        let trimmed = tool.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !is_known_allowed_tool(trimmed) {
            let known = known_allowed_tools().join(", ");
            anyhow::bail!("unknown tool: {trimmed} (known tools: {known})");
        }
        if seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

fn base_mode_decision_for_tool(mode: &ModeDef, tool: &str) -> Option<Decision> {
    match tool {
        "facade/workspace" => Some(
            mode.permissions
                .read
                .combine(mode.permissions.edit.decision)
                .combine(mode.permissions.artifact),
        ),
        "facade/process" => Some(
            mode.permissions
                .command
                .combine(mode.permissions.process.inspect)
                .combine(mode.permissions.process.kill),
        ),
        "facade/thread" => Some(
            mode.permissions
                .read
                .combine(mode.permissions.command)
                .combine(mode.permissions.process.inspect)
                .combine(mode.permissions.subagent.spawn.decision),
        ),
        "facade/artifact" => Some(mode.permissions.artifact),
        "facade/integration" => Some(
            mode.permissions
                .browser
                .combine(mode.permissions.command)
                .combine(mode.permissions.artifact)
                .combine(mode.permissions.read),
        ),
        "file/read" | "file/glob" | "file/grep" => Some(mode.permissions.read),
        "file/write" | "file/patch" | "file/edit" | "file/delete" | "fs/mkdir" => {
            Some(mode.permissions.edit.decision)
        }
        "repo/search" | "repo/index" | "repo/symbols" => {
            Some(mode.permissions.read.combine(mode.permissions.artifact))
        }
        "mcp/list_servers" => Some(mode.permissions.read),
        "mcp/list_tools" | "mcp/list_resources" | "mcp/call" => {
            Some(mode.permissions.command.combine(mode.permissions.artifact))
        }
        "artifact/write" | "artifact/list" | "artifact/read" | "artifact/delete" => {
            Some(mode.permissions.artifact)
        }
        "thread/request_user_input" | "thread/state" | "thread/usage" | "thread/events" => {
            Some(mode.permissions.read)
        }
        "thread/diff" => Some(mode.permissions.command.combine(mode.permissions.artifact)),
        "thread/hook_run" => Some(
            mode.permissions
                .command
                .combine(mode.permissions.process.inspect),
        ),
        "web/search" | "web/fetch" | "web/view_image" => Some(mode.permissions.browser),
        "subagent/spawn" | "subagent/send_input" => Some(mode.permissions.subagent.spawn.decision),
        "subagent/wait" => Some(mode.permissions.read),
        "subagent/close" => Some(
            mode.permissions
                .subagent
                .spawn
                .decision
                .combine(mode.permissions.process.kill),
        ),
        "process/start" => Some(mode.permissions.command),
        "process/list" | "process/inspect" | "process/tail" | "process/follow" => {
            Some(mode.permissions.process.inspect)
        }
        "process/kill" | "process/interrupt" => Some(mode.permissions.process.kill),
        _ => None,
    }
}

pub fn effective_mode_decision_for_tool(mode: &ModeDef, tool: &str) -> Option<Decision> {
    let base = base_mode_decision_for_tool(mode, tool)?;
    let decision = match mode.tool_overrides.get(tool).copied() {
        Some(override_decision) => base.combine(override_decision),
        None => base,
    };
    Some(decision)
}

pub fn effective_mode_and_role_decision_for_tool(
    mode: &ModeDef,
    role_permission_mode: &ModeDef,
    tool: &str,
) -> Option<Decision> {
    let mode_decision = effective_mode_decision_for_tool(mode, tool)?;
    let role_decision = effective_mode_decision_for_tool(role_permission_mode, tool)?;
    Some(mode_decision.combine(role_decision))
}

pub fn effective_permissions_for_mode_and_role(
    mode: &ModeDef,
    role_permission_mode: &ModeDef,
    allowed_tools: Option<&[String]>,
) -> Vec<String> {
    let candidate_tools: Vec<&str> = match allowed_tools {
        Some(tools) => tools.iter().map(String::as_str).collect(),
        None => known_allowed_tools().to_vec(),
    };

    candidate_tools
        .into_iter()
        .filter(|tool| {
            effective_mode_and_role_decision_for_tool(mode, role_permission_mode, tool)
                .is_some_and(|decision| decision != Decision::Deny)
        })
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::ModeCatalog;

    #[test]
    fn normalize_allowed_tools_trims_dedups_and_rejects_unknown() {
        let out = normalize_allowed_tools(vec![
            " file/read ".to_string(),
            "file/read".to_string(),
            "".to_string(),
            "process/start".to_string(),
        ])
        .expect("normalize allowed tools");
        assert_eq!(
            out,
            vec!["file/read".to_string(), "process/start".to_string()]
        );

        let err = normalize_allowed_tools(vec!["bad/tool".to_string()])
            .expect_err("unknown tool should fail");
        assert!(err.to_string().contains("unknown tool: bad/tool"));
    }

    #[test]
    fn every_known_allowed_tool_has_mode_decision_mapping() {
        let catalog = ModeCatalog::builtin();
        let mode = catalog.mode("coder").expect("builtin coder mode");
        for tool in known_allowed_tools() {
            assert!(
                effective_mode_decision_for_tool(mode, tool).is_some(),
                "missing mode decision mapping for tool: {tool}"
            );
        }
    }

    #[test]
    fn effective_mode_decision_respects_tool_overrides() {
        let catalog = ModeCatalog::builtin();
        let mut mode = catalog.mode("coder").expect("builtin coder mode").clone();
        mode.tool_overrides
            .insert("process/start".to_string(), Decision::Deny);

        assert_eq!(
            effective_mode_decision_for_tool(&mode, "process/start"),
            Some(Decision::Deny)
        );
    }

    #[test]
    fn effective_permissions_for_mode_and_role_intersects_both_dimensions() {
        let catalog = ModeCatalog::builtin();
        let mode = catalog.mode("coder").expect("builtin coder mode");
        let role_mode = catalog.mode("chatter").expect("builtin chatter mode");
        let effective = effective_permissions_for_mode_and_role(mode, role_mode, None);

        assert!(effective.iter().any(|tool| tool == "file/read"));
        assert!(!effective.iter().any(|tool| tool == "process/start"));
        assert!(!effective.iter().any(|tool| tool == "file/write"));
    }

    #[test]
    fn effective_permissions_respects_allowed_tools_subset() {
        let catalog = ModeCatalog::builtin();
        let mode = catalog.mode("coder").expect("builtin coder mode");
        let role_mode = catalog.mode("chatter").expect("builtin chatter mode");
        let allowed = vec![
            "file/read".to_string(),
            "process/start".to_string(),
            "file/write".to_string(),
        ];
        let effective = effective_permissions_for_mode_and_role(mode, role_mode, Some(&allowed));

        assert!(effective.iter().any(|tool| tool == "file/read"));
        assert!(!effective.iter().any(|tool| tool == "file/write"));
        assert!(!effective.iter().any(|tool| tool == "process/start"));
    }
}
