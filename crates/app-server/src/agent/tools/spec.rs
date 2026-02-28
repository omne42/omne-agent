#[derive(Debug, Clone, Copy)]
pub(crate) struct AgentToolSpec {
    pub name: &'static str,
    pub action: &'static str,
    pub plan_read_only: bool,
}

const AGENT_TOOL_SPECS: &[AgentToolSpec] = &[
    AgentToolSpec { name: "file_read", action: "file/read", plan_read_only: true },
    AgentToolSpec { name: "file_glob", action: "file/glob", plan_read_only: true },
    AgentToolSpec { name: "file_grep", action: "file/grep", plan_read_only: true },
    AgentToolSpec { name: "repo_search", action: "repo/search", plan_read_only: true },
    AgentToolSpec { name: "repo_index", action: "repo/index", plan_read_only: true },
    AgentToolSpec { name: "repo_symbols", action: "repo/symbols", plan_read_only: true },
    AgentToolSpec { name: "mcp_list_servers", action: "mcp/list_servers", plan_read_only: true },
    AgentToolSpec { name: "mcp_list_tools", action: "mcp/list_tools", plan_read_only: true },
    AgentToolSpec { name: "mcp_list_resources", action: "mcp/list_resources", plan_read_only: true },
    AgentToolSpec { name: "mcp_call", action: "mcp/call", plan_read_only: false },
    AgentToolSpec { name: "file_write", action: "file/write", plan_read_only: false },
    AgentToolSpec { name: "file_patch", action: "file/patch", plan_read_only: false },
    AgentToolSpec { name: "file_edit", action: "file/edit", plan_read_only: false },
    AgentToolSpec { name: "file_delete", action: "file/delete", plan_read_only: false },
    AgentToolSpec { name: "fs_mkdir", action: "fs/mkdir", plan_read_only: false },
    AgentToolSpec { name: "process_start", action: "process/start", plan_read_only: false },
    AgentToolSpec { name: "process_inspect", action: "process/inspect", plan_read_only: true },
    AgentToolSpec { name: "process_tail", action: "process/tail", plan_read_only: true },
    AgentToolSpec { name: "process_follow", action: "process/follow", plan_read_only: true },
    AgentToolSpec { name: "process_kill", action: "process/kill", plan_read_only: false },
    AgentToolSpec { name: "artifact_write", action: "artifact/write", plan_read_only: false },
    AgentToolSpec { name: "artifact_list", action: "artifact/list", plan_read_only: true },
    AgentToolSpec { name: "artifact_read", action: "artifact/read", plan_read_only: true },
    AgentToolSpec { name: "artifact_delete", action: "artifact/delete", plan_read_only: false },
    AgentToolSpec { name: "thread_diff", action: "thread/diff", plan_read_only: true },
    AgentToolSpec { name: "thread_state", action: "thread/state", plan_read_only: true },
    AgentToolSpec { name: "thread_usage", action: "thread/usage", plan_read_only: true },
    AgentToolSpec { name: "thread_events", action: "thread/events", plan_read_only: true },
    AgentToolSpec { name: "thread_hook_run", action: "thread/hook_run", plan_read_only: false },
    AgentToolSpec { name: "agent_spawn", action: "subagent/spawn", plan_read_only: false },
];

pub(crate) fn agent_tool_spec(tool_name: &str) -> Option<&'static AgentToolSpec> {
    AGENT_TOOL_SPECS.iter().find(|spec| spec.name == tool_name)
}

pub(crate) fn is_known_agent_tool_name(tool_name: &str) -> bool {
    agent_tool_spec(tool_name).is_some()
}

pub(crate) fn is_plan_read_only_agent_tool(tool_name: &str) -> bool {
    agent_tool_spec(tool_name)
        .map(|spec| spec.plan_read_only)
        .unwrap_or(false)
}

pub(crate) fn agent_tool_action(tool_name: &str) -> Option<&'static str> {
    agent_tool_spec(tool_name).map(|spec| spec.action)
}
