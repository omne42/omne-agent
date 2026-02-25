use crate::modes::{Decision, ModeDef};

const KNOWN_ALLOWED_TOOLS: &[&str] = &[
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
}
