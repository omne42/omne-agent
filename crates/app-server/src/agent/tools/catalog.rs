fn tool_function(name: &str, description: &str, parameters: Value) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

const OMNE_ENABLE_MCP_ENV: &str = "OMNE_ENABLE_MCP";
const OMNE_TOOL_EXPOSE_SUBAGENT_ENV: &str = "OMNE_TOOL_EXPOSE_SUBAGENT";
const OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION_ENV: &str = "OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION";
const OMNE_TOOL_EXPOSE_THREAD_HOOK_ENV: &str = "OMNE_TOOL_EXPOSE_THREAD_HOOK";
const OMNE_TOOL_EXPOSE_REPO_SYMBOLS_ENV: &str = "OMNE_TOOL_EXPOSE_REPO_SYMBOLS";
const OMNE_TOOL_EXPOSE_WEB_ENV: &str = "OMNE_TOOL_EXPOSE_WEB";
const OMNE_TOOL_MODEL_PROFILE_ENV: &str = "OMNE_TOOL_MODEL_PROFILE";
const OMNE_TOOL_FACADE_ENABLED_ENV: &str = "OMNE_TOOL_FACADE_ENABLED";
const OMNE_TOOL_FACADE_EXPOSE_LEGACY_ENV: &str = "OMNE_TOOL_FACADE_EXPOSE_LEGACY";

fn parse_bool_env_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn bool_env_or_default(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|raw| parse_bool_env_value(&raw))
        .unwrap_or(default)
}

fn tool_spec_name(spec: &Value) -> Option<&str> {
    spec.get("function")
        .and_then(Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
}

fn tool_function_from_dynamic(spec: &DynamicToolSpec) -> Value {
    tool_function(&spec.name, &spec.description, spec.parameters.clone())
}

fn is_mcp_tool_name(name: &str) -> bool {
    matches!(
        name,
        "mcp_list_servers" | "mcp_list_tools" | "mcp_list_resources" | "mcp_call"
    )
}

#[derive(Debug, Clone, Copy)]
enum ToolModelProfile {
    Full,
    Compact,
}

impl ToolModelProfile {
    fn infer(model: Option<&str>) -> Self {
        let Some(model) = model else {
            return Self::Full;
        };
        let model = model.to_ascii_lowercase();
        if model.contains("mini") || model.contains("flash") || model.contains("haiku") {
            Self::Compact
        } else {
            Self::Full
        }
    }

    fn from_env_or_model(model: Option<&str>) -> Self {
        let from_env = std::env::var(OMNE_TOOL_MODEL_PROFILE_ENV)
            .ok()
            .map(|raw| raw.trim().to_ascii_lowercase());
        match from_env.as_deref() {
            Some("full") => Self::Full,
            Some("compact") => Self::Compact,
            _ => Self::infer(model),
        }
    }

    fn allows_tool_name(self, name: &str) -> bool {
        match self {
            Self::Full => true,
            Self::Compact => !matches!(name, "repo_index" | "process_follow"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolRoleProfile {
    Legacy,
    Chatter,
    Default,
    Codder,
}

impl ToolRoleProfile {
    fn from_role(role: Option<&str>) -> Self {
        let Some(role) = role else {
            return Self::Legacy;
        };
        let trimmed = role.trim();
        if matches!(trimmed, "作者" | "文档整理者") {
            return Self::Default;
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "chatter" | "chat" | "roleplay" => Self::Chatter,
            "default" | "author" | "doc_organizer" | "doc-organizer" => Self::Default,
            "codder" | "coder" | "code" => Self::Codder,
            _ => Self::Legacy,
        }
    }

    fn allows_tool_name(self, name: &str) -> bool {
        if is_facade_tool_name(name) {
            return match self {
                Self::Legacy | Self::Codder | Self::Default | Self::Chatter => true,
            };
        }
        match self {
            Self::Legacy | Self::Codder => true,
            Self::Chatter => matches!(
                name,
                "file_read"
                    | "file_glob"
                    | "file_grep"
                    | "repo_search"
                    | "repo_index"
                    | "repo_symbols"
                    | "repo_goto_definition"
                    | "repo_find_references"
                    | "mcp_list_servers"
                    | "mcp_list_tools"
                    | "mcp_list_resources"
                    | "update_plan"
                    | "request_user_input"
                    | "web_search"
                    | "webfetch"
                    | "view_image"
                    | "artifact_list"
                    | "artifact_read"
                    | "thread_diff"
                    | "thread_state"
                    | "thread_usage"
                    | "thread_events"
            ),
            Self::Default => matches!(
                name,
                "file_read"
                    | "file_glob"
                    | "file_grep"
                    | "repo_search"
                    | "repo_index"
                    | "repo_symbols"
                    | "repo_goto_definition"
                    | "repo_find_references"
                    | "mcp_list_servers"
                    | "mcp_list_tools"
                    | "mcp_list_resources"
                    | "update_plan"
                    | "request_user_input"
                    | "web_search"
                    | "webfetch"
                    | "view_image"
                    | "artifact_write"
                    | "artifact_list"
                    | "artifact_read"
                    | "process_inspect"
                    | "process_tail"
                    | "process_follow"
                    | "thread_diff"
                    | "thread_state"
                    | "thread_usage"
                    | "thread_events"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolExposurePolicy {
    mcp_enabled: bool,
    expose_subagent: bool,
    expose_thread_introspection: bool,
    expose_thread_hook: bool,
    expose_repo_symbols: bool,
    expose_web: bool,
    facade_enabled: bool,
    facade_expose_legacy: bool,
    model_profile: ToolModelProfile,
    role_profile: ToolRoleProfile,
}

impl ToolExposurePolicy {
    fn from_env(model: Option<&str>, role: Option<&str>) -> Self {
        Self {
            mcp_enabled: bool_env_or_default(OMNE_ENABLE_MCP_ENV, false),
            expose_subagent: bool_env_or_default(OMNE_TOOL_EXPOSE_SUBAGENT_ENV, false),
            expose_thread_introspection: bool_env_or_default(
                OMNE_TOOL_EXPOSE_THREAD_INTROSPECTION_ENV,
                false,
            ),
            expose_thread_hook: bool_env_or_default(OMNE_TOOL_EXPOSE_THREAD_HOOK_ENV, false),
            expose_repo_symbols: bool_env_or_default(OMNE_TOOL_EXPOSE_REPO_SYMBOLS_ENV, false),
            expose_web: bool_env_or_default(OMNE_TOOL_EXPOSE_WEB_ENV, false),
            facade_enabled: bool_env_or_default(OMNE_TOOL_FACADE_ENABLED_ENV, true),
            facade_expose_legacy: bool_env_or_default(OMNE_TOOL_FACADE_EXPOSE_LEGACY_ENV, false),
            model_profile: ToolModelProfile::from_env_or_model(model),
            role_profile: ToolRoleProfile::from_role(role),
        }
    }

    fn allows_non_mcp_tool_name(self, name: &str) -> bool {
        match name {
            "workspace" | "process" | "thread" | "artifact" => self.facade_enabled,
            "integration" => self.facade_enabled && (self.mcp_enabled || self.expose_web),
            "agent_spawn" => self.expose_subagent,
            "thread_diff" | "thread_state" | "thread_usage" | "thread_events" => {
                self.expose_thread_introspection
            }
            "thread_hook_run" => self.expose_thread_hook,
            "repo_symbols" | "repo_goto_definition" | "repo_find_references" => {
                self.expose_repo_symbols
            }
            "web_search" | "webfetch" | "view_image" => self.expose_web,
            _ => true,
        }
    }

    fn allows_tool_name(self, name: &str) -> bool {
        if is_mcp_tool_name(name) {
            return self.mcp_enabled && self.role_profile.allows_tool_name(name);
        }
        self.allows_non_mcp_tool_name(name)
            && self.model_profile.allows_tool_name(name)
            && self.role_profile.allows_tool_name(name)
    }
}

fn select_tools_for_turn(
    tools: Vec<Value>,
    allowed_tools: Option<&[String]>,
    policy: ToolExposurePolicy,
) -> Vec<Value> {
    let allowed_actions = allowed_tools.map(|actions| {
        actions
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>()
    });

    tools
        .into_iter()
        .filter(|spec| {
            let Some(name) = tool_spec_name(spec) else {
                return true;
            };
            if !policy.allows_tool_name(name) {
                return false;
            }
            let Some(allowed_actions) = allowed_actions.as_ref() else {
                return true;
            };
            tool_allowed_by_action_filter(name, allowed_actions)
        })
        .collect()
}

fn tool_allowed_by_action_filter(
    tool_name: &str,
    allowed_actions: &std::collections::HashSet<&str>,
) -> bool {
    if let Some(action) = agent_tool_action(tool_name)
        && allowed_actions.contains(action)
    {
        return true;
    }
    facade_tool_internal_actions(tool_name)
        .map(|actions| actions.iter().any(|action| allowed_actions.contains(*action)))
        .unwrap_or(false)
}

fn build_tools() -> Vec<Value> {
    vec![
        tool_function(
            "file_read",
            "Read a UTF-8 text file from the project (or from the reference repo when root=reference).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "path": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "file_glob",
            "Find files by glob pattern (e.g. **/*.rs). Use root=reference to search the reference repo.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "pattern": { "type": "string" },
                    "max_results": { "type": "integer", "minimum": 1 },
                },
                "required": ["pattern"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "file_grep",
            "Search text across files. Use root=reference to search the reference repo.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "query": { "type": "string" },
                    "is_regex": { "type": "boolean" },
                    "include_glob": { "type": "string" },
                    "max_matches": { "type": "integer", "minimum": 1 },
                },
                "required": ["query"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "repo_search",
            "Search text across the repo and write a user-facing artifact (repo_search).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "query": { "type": "string" },
                    "is_regex": { "type": "boolean" },
                    "include_glob": { "type": "string" },
                    "max_matches": { "type": "integer", "minimum": 1 },
                    "max_bytes_per_file": { "type": "integer", "minimum": 1 },
                    "max_files": { "type": "integer", "minimum": 1 },
                },
                "required": ["query"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "repo_index",
            "Generate a lightweight repo index artifact (repo_index).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "include_glob": { "type": "string" },
                    "max_files": { "type": "integer", "minimum": 1 },
                },
                "required": [],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "repo_symbols",
            "Extract Rust symbols with tree-sitter and write a user-facing artifact (repo_symbols).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "include_glob": { "type": "string" },
                    "max_files": { "type": "integer", "minimum": 1 },
                    "max_bytes_per_file": { "type": "integer", "minimum": 1 },
                    "max_symbols": { "type": "integer", "minimum": 1 },
                },
                "required": [],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "repo_goto_definition",
            "Resolve likely definition locations for a symbol and write a user-facing artifact (repo_goto_definition).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "symbol": { "type": "string" },
                    "path": { "type": "string" },
                    "include_glob": { "type": "string" },
                    "max_results": { "type": "integer", "minimum": 1 },
                    "max_files": { "type": "integer", "minimum": 1 },
                    "max_bytes_per_file": { "type": "integer", "minimum": 1 },
                    "max_symbols": { "type": "integer", "minimum": 1 },
                },
                "required": ["symbol"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "repo_find_references",
            "Find text references for a symbol and write a user-facing artifact (repo_find_references).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "symbol": { "type": "string" },
                    "path": { "type": "string" },
                    "include_glob": { "type": "string" },
                    "max_matches": { "type": "integer", "minimum": 1 },
                    "max_bytes_per_file": { "type": "integer", "minimum": 1 },
                    "max_files": { "type": "integer", "minimum": 1 },
                },
                "required": ["symbol"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "mcp_list_servers",
            "List configured MCP servers (from .omne_data/spec/mcp.json).",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "mcp_list_tools",
            "List tools exposed by an MCP server.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" },
                },
                "required": ["server"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "mcp_list_resources",
            "List resources exposed by an MCP server.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" },
                },
                "required": ["server"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "mcp_call",
            "Call a tool exposed by an MCP server (requires prompt_strict approval).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" },
                    "tool": { "type": "string" },
                    "arguments": {
                        "type": "object",
                        "description": "Required. The exact payload for the underlying MCP tool. Must be a JSON object containing required fields for that MCP tool. Even when empty, provide {}. Do not flatten nested MCP arguments to root.",
                        "additionalProperties": true
                    },
                },
                "required": ["server", "tool", "arguments"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "file_write",
            "Write a UTF-8 text file (overwrites).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "text": { "type": "string" },
                    "create_parent_dirs": { "type": "boolean" },
                },
                "required": ["path", "text"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "file_patch",
            "Apply a unified diff patch to a file.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "patch": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["path", "patch"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "file_edit",
            "Edit a UTF-8 file by applying exact string replacements.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old": { "type": "string" },
                                "new": { "type": "string" },
                                "expected_replacements": { "type": "integer", "minimum": 0 }
                            },
                            "required": ["old", "new"],
                            "additionalProperties": false
                        }
                    },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["path", "edits"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "file_delete",
            "Delete a file (or a directory if recursive=true).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "fs_mkdir",
            "Create a directory.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "recursive": { "type": "boolean" },
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "process_start",
            "Start a background process (non-interactive).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "timeout_ms": { "type": "integer", "minimum": 1 },
                },
                "required": ["argv"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "process_inspect",
            "Inspect a process and read recent stdout/stderr.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "max_lines": { "type": "integer", "minimum": 1 },
                },
                "required": ["process_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "process_tail",
            "Read the last N lines from a process log (stdout/stderr).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "stream": { "type": "string", "enum": ["stdout", "stderr"] },
                    "max_lines": { "type": "integer", "minimum": 1 },
                },
                "required": ["process_id", "stream"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "process_follow",
            "Read the next chunk from a process log (stdout/stderr) starting at since_offset.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "stream": { "type": "string", "enum": ["stdout", "stderr"] },
                    "since_offset": { "type": "integer", "minimum": 0 },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["process_id", "stream"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "process_kill",
            "Kill a running process.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string" },
                    "reason": { "type": "string" },
                },
                "required": ["process_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "artifact_write",
            "Write a user-facing markdown artifact for this thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_type": { "type": "string" },
                    "summary": { "type": "string" },
                    "text": { "type": "string" },
                },
                "required": ["artifact_type", "summary", "text"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "update_plan",
            "Write a structured execution plan artifact from ordered steps and statuses.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "explanation": { "type": "string" },
                    "plan": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "step": { "type": "string" },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
                            },
                            "required": ["step", "status"],
                            "additionalProperties": false,
                        },
                    },
                },
                "required": ["plan"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "request_user_input",
            "Request structured user input with 1-3 short multiple-choice questions.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 3,
                        "items": {
                            "type": "object",
                            "properties": {
                                "header": { "type": "string" },
                                "id": { "type": "string" },
                                "question": { "type": "string" },
                                "options": {
                                    "type": "array",
                                    "minItems": 2,
                                    "maxItems": 3,
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": { "type": "string" },
                                            "description": { "type": "string" },
                                        },
                                        "required": ["label", "description"],
                                        "additionalProperties": false,
                                    },
                                },
                            },
                            "required": ["header", "id", "question", "options"],
                            "additionalProperties": false,
                        },
                    },
                },
                "required": ["questions"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "web_search",
            "Search the web and return concise top results.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 10 },
                },
                "required": ["query"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "webfetch",
            "Fetch a web page URL and return extracted text content.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["url"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "view_image",
            "Read local or remote image bytes and return metadata.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string", "enum": ["workspace", "reference"] },
                    "path": { "type": "string" },
                    "url": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": [],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "artifact_list",
            "List user-facing artifacts for this thread.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "artifact_read",
            "Read a user-facing artifact by id.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                    "max_bytes": { "type": "integer", "minimum": 1 },
                },
                "required": ["artifact_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "artifact_delete",
            "Delete a user-facing artifact by id.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string" },
                },
                "required": ["artifact_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "thread_diff",
            "Read incremental thread diff output and recent snapshot metadata.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "max_bytes": { "type": "integer", "minimum": 1 },
                    "wait_seconds": { "type": "integer", "minimum": 0 },
                },
                "required": [],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "thread_state",
            "Read the derived state for a thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                },
                "required": ["thread_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "thread_usage",
            "Read aggregated token usage and cache ratios for a thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                },
                "required": ["thread_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "thread_events",
            "Read thread events since a given seq.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": "string" },
                    "since_seq": { "type": "integer", "minimum": 0 },
                    "max_events": { "type": "integer", "minimum": 1 },
                },
                "required": ["thread_id"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "thread_hook_run",
            "Run a configured workspace hook for this thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "hook": { "type": "string", "enum": ["setup", "run", "archive"] },
                },
                "required": ["hook"],
                "additionalProperties": false,
            }),
        ),
        tool_function(
            "agent_spawn",
            "Spawn subagent tasks (fork or new) with optional dependencies.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "spawn_mode": { "type": "string", "enum": ["fork", "new"] },
                    "mode": { "type": "string" },
                    "workspace_mode": { "type": "string", "enum": ["read_only", "isolated_write"] },
                    "priority": { "type": "string", "enum": ["high", "normal", "low"] },
                    "model": { "type": "string" },
                    "openai_base_url": { "type": "string" },
                    "expected_artifact_type": { "type": "string" },
                    "tasks": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "title": { "type": "string" },
                                "input": { "type": "string" },
                                "depends_on": { "type": "array", "items": { "type": "string" } },
                                "spawn_mode": { "type": "string", "enum": ["fork", "new"] },
                                "mode": { "type": "string" },
                                "workspace_mode": { "type": "string", "enum": ["read_only", "isolated_write"] },
                                "priority": { "type": "string", "enum": ["high", "normal", "low"] },
                                "model": { "type": "string" },
                                "openai_base_url": { "type": "string" },
                                "expected_artifact_type": { "type": "string" },
                            },
                            "required": ["id", "input"],
                            "additionalProperties": false,
                        },
                    },
                },
                "required": ["tasks"],
                "additionalProperties": false,
            }),
        ),
    ]
}

fn facade_tool_parameters(ops: &[&str]) -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "op": { "type": "string", "enum": ops },
            "help": { "type": "boolean" },
            "topic": { "type": "string" },
        },
        "required": ["op"],
        "additionalProperties": true,
    })
}

fn build_facade_tools() -> Vec<Value> {
    vec![
        tool_function(
            "workspace",
            "Workspace facade: files + repo operations. For op!=help, all operation parameters MUST be root-level fields (alongside op). Do not use args wrapper. Use op=help for quickstart and advanced topics.",
            facade_tool_parameters(&[
                "help",
                "read",
                "glob",
                "grep",
                "repo_search",
                "repo_index",
                "repo_symbols",
                "repo_goto_definition",
                "repo_find_references",
                "write",
                "patch",
                "edit",
                "delete",
                "mkdir",
            ]),
        ),
        tool_function(
            "process",
            "Process facade: start/inspect/tail/follow/kill. For op!=help, all operation parameters MUST be root-level fields (alongside op). Do not use args wrapper. Use op=help for quickstart and advanced topics.",
            facade_tool_parameters(&["help", "start", "inspect", "tail", "follow", "kill"]),
        ),
        tool_function(
            "thread",
            "Thread facade: diff/state/usage/events/hooks plus subagent lifecycle (spawn/send_input/wait/close). For op!=help, all operation parameters MUST be root-level fields (alongside op). Do not use args wrapper. Use op=help for details.",
            facade_tool_parameters(&[
                "help",
                "diff",
                "state",
                "usage",
                "events",
                "hook_run",
                "request_input",
                "spawn_agent",
                "send_input",
                "wait",
                "close",
                "close_agent",
            ]),
        ),
        tool_function(
            "artifact",
            "Artifact facade: write/update_plan/list/read/delete. For op!=help, all operation parameters MUST be root-level fields (alongside op). Do not use args wrapper. Use op=help for quickstart and advanced topics.",
            facade_tool_parameters(&["help", "write", "update_plan", "list", "read", "delete"]),
        ),
        tool_function(
            "integration",
            "Integration facade: MCP + web tools. For op!=help, all operation parameters MUST be root-level fields (alongside op). Do not use args wrapper. Optional by default; use op=help for capabilities.",
            facade_tool_parameters(&[
                "help",
                "mcp_list_servers",
                "mcp_list_tools",
                "mcp_list_resources",
                "mcp_call",
                "web_search",
                "web_fetch",
                "view_image",
            ]),
        ),
    ]
}

fn build_tools_for_turn_with_policy(
    allowed_tools: Option<&[String]>,
    policy: ToolExposurePolicy,
    thread_root: Option<&std::path::Path>,
) -> Vec<Value> {
    let mut tools = Vec::<Value>::new();

    if policy.facade_enabled {
        tools.extend(select_tools_for_turn(
            build_facade_tools(),
            allowed_tools,
            policy,
        ));
    }

    if !policy.facade_enabled || policy.facade_expose_legacy {
        tools.extend(select_tools_for_turn(build_tools(), allowed_tools, policy));
    }

    if dynamic_tool_registry_enabled() {
        let allowed_actions = allowed_tools.map(|actions| {
            actions
                .iter()
                .map(String::as_str)
                .collect::<std::collections::HashSet<_>>()
        });
        tools.extend(
            load_dynamic_tool_specs(thread_root)
                .into_iter()
                .filter(|spec| {
                    if !policy.allows_tool_name(&spec.mapped_tool) {
                        return false;
                    }
                    allowed_actions
                        .as_ref()
                        .is_none_or(|actions| actions.contains(spec.mapped_action.as_str()))
                })
                .map(|spec| tool_function_from_dynamic(&spec)),
        );
    }

    tools
}

fn build_tools_for_turn(
    allowed_tools: Option<&[String]>,
    model: Option<&str>,
    role: Option<&str>,
    thread_root: Option<&std::path::Path>,
) -> Vec<Value> {
    build_tools_for_turn_with_policy(
        allowed_tools,
        ToolExposurePolicy::from_env(model, role),
        thread_root,
    )
}

#[cfg(test)]
mod tool_catalog_tests {
    use super::*;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            crate::set_locked_process_env(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            crate::remove_locked_process_env(key);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            crate::restore_locked_process_env(self.key, self.previous.as_deref());
        }
    }

    fn permissive_policy() -> ToolExposurePolicy {
        ToolExposurePolicy {
            mcp_enabled: true,
            expose_subagent: true,
            expose_thread_introspection: true,
            expose_thread_hook: true,
            expose_repo_symbols: true,
            expose_web: true,
            facade_enabled: false,
            facade_expose_legacy: true,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Legacy,
        }
    }

    fn default_policy() -> ToolExposurePolicy {
        ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: false,
            facade_expose_legacy: true,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Legacy,
        }
    }

    fn tool_names(tools: &[Value]) -> Vec<String> {
        tools
            .iter()
            .filter_map(tool_spec_name)
            .map(ToString::to_string)
            .collect()
    }

    fn tool_schema_bytes(tools: &[Value]) -> usize {
        tools
            .iter()
            .map(|tool| serde_json::to_vec(tool).map(|bytes| bytes.len()).unwrap_or(0))
            .sum()
    }

    #[test]
    fn full_catalog_includes_thread_diff() {
        let names = tool_names(&build_tools());
        assert!(names.iter().any(|name| name == "thread_diff"));
    }

    #[test]
    fn full_catalog_includes_update_plan() {
        let names = tool_names(&build_tools());
        assert!(names.iter().any(|name| name == "update_plan"));
    }

    #[test]
    fn full_catalog_includes_request_user_input() {
        let names = tool_names(&build_tools());
        assert!(names.iter().any(|name| name == "request_user_input"));
    }

    #[test]
    fn full_catalog_includes_web_tools() {
        let names = tool_names(&build_tools());
        assert!(names.iter().any(|name| name == "web_search"));
        assert!(names.iter().any(|name| name == "webfetch"));
        assert!(names.iter().any(|name| name == "view_image"));
    }

    #[test]
    fn select_tools_hides_mcp_when_disabled() {
        let mut policy = permissive_policy();
        policy.mcp_enabled = false;
        let names = tool_names(&select_tools_for_turn(build_tools(), None, policy));
        assert!(!names.iter().any(|name| name.starts_with("mcp_")));
    }

    #[test]
    fn select_tools_respects_allowed_action_list() {
        let allowed = vec!["file/read".to_string(), "thread/diff".to_string()];
        let mut names = tool_names(&select_tools_for_turn(
            build_tools(),
            Some(&allowed),
            permissive_policy(),
        ));
        names.sort();
        assert_eq!(names, vec!["file_read".to_string(), "thread_diff".to_string()]);
    }

    #[test]
    fn select_tools_respects_allowed_action_list_for_artifact_write() {
        let allowed = vec!["artifact/write".to_string()];
        let mut names = tool_names(&select_tools_for_turn(
            build_tools(),
            Some(&allowed),
            permissive_policy(),
        ));
        names.sort();
        assert_eq!(
            names,
            vec!["artifact_write".to_string(), "update_plan".to_string()]
        );
    }

    #[test]
    fn select_tools_respects_allowed_action_list_for_request_user_input() {
        let allowed = vec!["thread/request_user_input".to_string()];
        let names = tool_names(&select_tools_for_turn(
            build_tools(),
            Some(&allowed),
            permissive_policy(),
        ));
        assert_eq!(names, vec!["request_user_input".to_string()]);
    }

    #[test]
    fn select_tools_respects_allowed_action_list_for_web_fetch() {
        let allowed = vec!["web/fetch".to_string()];
        let names = tool_names(&select_tools_for_turn(
            build_tools(),
            Some(&allowed),
            permissive_policy(),
        ));
        assert_eq!(names, vec!["webfetch".to_string()]);
    }

    #[test]
    fn default_policy_hides_optional_groups() {
        let names = tool_names(&select_tools_for_turn(build_tools(), None, default_policy()));
        assert!(!names.iter().any(|name| name == "repo_symbols"));
        assert!(!names.iter().any(|name| name == "repo_goto_definition"));
        assert!(!names.iter().any(|name| name == "repo_find_references"));
        assert!(!names.iter().any(|name| name == "agent_spawn"));
        assert!(!names.iter().any(|name| name == "thread_state"));
        assert!(!names.iter().any(|name| name == "thread_usage"));
        assert!(!names.iter().any(|name| name == "thread_events"));
        assert!(!names.iter().any(|name| name == "thread_diff"));
        assert!(!names.iter().any(|name| name == "thread_hook_run"));
        assert!(!names.iter().any(|name| name == "web_search"));
        assert!(!names.iter().any(|name| name == "webfetch"));
        assert!(!names.iter().any(|name| name == "view_image"));
    }

    #[test]
    fn permissive_policy_shows_optional_groups() {
        let names = tool_names(&select_tools_for_turn(build_tools(), None, permissive_policy()));
        assert!(names.iter().any(|name| name == "repo_symbols"));
        assert!(names.iter().any(|name| name == "repo_goto_definition"));
        assert!(names.iter().any(|name| name == "repo_find_references"));
        assert!(names.iter().any(|name| name == "agent_spawn"));
        assert!(names.iter().any(|name| name == "thread_state"));
        assert!(names.iter().any(|name| name == "thread_usage"));
        assert!(names.iter().any(|name| name == "thread_events"));
        assert!(names.iter().any(|name| name == "thread_diff"));
        assert!(names.iter().any(|name| name == "thread_hook_run"));
        assert!(names.iter().any(|name| name == "web_search"));
        assert!(names.iter().any(|name| name == "webfetch"));
        assert!(names.iter().any(|name| name == "view_image"));
    }

    #[test]
    fn compact_profile_hides_heavy_tools() {
        let mut policy = permissive_policy();
        policy.model_profile = ToolModelProfile::Compact;
        let names = tool_names(&select_tools_for_turn(build_tools(), None, policy));
        assert!(!names.iter().any(|name| name == "repo_index"));
        assert!(!names.iter().any(|name| name == "process_follow"));
    }

    #[test]
    fn infer_model_profile_uses_compact_for_mini_family() {
        assert!(matches!(
            ToolModelProfile::infer(Some("gpt-5-mini")),
            ToolModelProfile::Compact
        ));
        assert!(matches!(
            ToolModelProfile::infer(Some("claude-3.5-haiku")),
            ToolModelProfile::Compact
        ));
        assert!(matches!(
            ToolModelProfile::infer(Some("gpt-5-codex")),
            ToolModelProfile::Full
        ));
    }

    #[test]
    fn chatter_role_hides_mutating_tools() {
        let mut policy = permissive_policy();
        policy.role_profile = ToolRoleProfile::Chatter;
        let names = tool_names(&select_tools_for_turn(build_tools(), None, policy));

        assert!(names.iter().any(|name| name == "request_user_input"));
        assert!(names.iter().any(|name| name == "web_search"));
        assert!(!names.iter().any(|name| name == "file_write"));
        assert!(!names.iter().any(|name| name == "process_start"));
        assert!(!names.iter().any(|name| name == "artifact_write"));
        assert!(!names.iter().any(|name| name == "mcp_call"));
        assert!(!names.iter().any(|name| name == "thread_hook_run"));
        assert!(!names.iter().any(|name| name == "agent_spawn"));
    }

    #[test]
    fn default_role_allows_read_only_process_tools_but_not_mutating_tools() {
        let mut policy = permissive_policy();
        policy.role_profile = ToolRoleProfile::Default;
        let names = tool_names(&select_tools_for_turn(build_tools(), None, policy));

        assert!(names.iter().any(|name| name == "artifact_write"));
        assert!(names.iter().any(|name| name == "process_inspect"));
        assert!(names.iter().any(|name| name == "process_tail"));
        assert!(names.iter().any(|name| name == "process_follow"));
        assert!(!names.iter().any(|name| name == "file_write"));
        assert!(!names.iter().any(|name| name == "process_start"));
        assert!(!names.iter().any(|name| name == "process_kill"));
        assert!(!names.iter().any(|name| name == "artifact_delete"));
        assert!(!names.iter().any(|name| name == "mcp_call"));
        assert!(!names.iter().any(|name| name == "thread_hook_run"));
        assert!(!names.iter().any(|name| name == "agent_spawn"));
    }

    #[test]
    fn codder_role_keeps_coding_tools() {
        let mut policy = permissive_policy();
        policy.role_profile = ToolRoleProfile::Codder;
        let names = tool_names(&select_tools_for_turn(build_tools(), None, policy));

        assert!(names.iter().any(|name| name == "file_write"));
        assert!(names.iter().any(|name| name == "process_start"));
        assert!(names.iter().any(|name| name == "process_kill"));
        assert!(names.iter().any(|name| name == "agent_spawn"));
    }

    #[test]
    fn facade_default_tool_surface_is_at_most_five() {
        let policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: true,
            facade_expose_legacy: false,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };
        let names = tool_names(&build_tools_for_turn_with_policy(None, policy, None));
        assert!(names.len() <= 5, "facade tools should be <= 5, got {names:?}");
        assert_eq!(
            names,
            vec![
                "workspace".to_string(),
                "process".to_string(),
                "thread".to_string(),
                "artifact".to_string(),
            ]
        );
    }

    #[test]
    fn facade_filter_keeps_workspace_when_allowed_tools_overlap_mapped_actions() {
        let policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: true,
            facade_expose_legacy: false,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };
        let allowed = vec!["file/read".to_string()];
        let names = tool_names(&build_tools_for_turn_with_policy(Some(&allowed), policy, None));
        assert_eq!(names, vec!["workspace".to_string()]);
    }

    #[test]
    fn facade_integration_is_exposed_only_when_optional_capabilities_are_enabled() {
        let mut policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: true,
            facade_expose_legacy: false,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };
        let names = tool_names(&build_tools_for_turn_with_policy(None, policy, None));
        assert!(!names.iter().any(|name| name == "integration"));

        policy.expose_web = true;
        let names = tool_names(&build_tools_for_turn_with_policy(None, policy, None));
        assert!(names.iter().any(|name| name == "integration"));
    }

    #[test]
    fn facade_tool_surface_reduces_schema_bytes_vs_legacy_default() {
        let legacy_policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: false,
            facade_expose_legacy: true,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };
        let facade_policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: true,
            facade_expose_legacy: false,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };

        let legacy_tools = build_tools_for_turn_with_policy(None, legacy_policy, None);
        let facade_tools = build_tools_for_turn_with_policy(None, facade_policy, None);
        let legacy_bytes = tool_schema_bytes(&legacy_tools);
        let facade_bytes = tool_schema_bytes(&facade_tools);

        eprintln!(
            "tool-surface-baseline legacy_count={} legacy_bytes={} facade_count={} facade_bytes={}",
            legacy_tools.len(),
            legacy_bytes,
            facade_tools.len(),
            facade_bytes
        );

        assert!(facade_tools.len() <= 5);
        assert!(legacy_bytes > 0);
        assert!(facade_bytes < legacy_bytes);

        let reduction_pct = 100.0 * (1.0 - (facade_bytes as f64 / legacy_bytes as f64));
        assert!(
            reduction_pct >= 50.0,
            "expected schema reduction >= 50%, got {reduction_pct:.2}%"
        );
    }

    #[test]
    fn dynamic_registry_adds_tools_when_enabled() {
        let _lock = crate::app_server_process_env_lock().blocking_lock();
        let _enabled = ScopedEnvVar::set("OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED", "1");
        let _path = ScopedEnvVar::unset("OMNE_TOOL_DYNAMIC_REGISTRY_PATH");

        let tmp = tempfile::tempdir().expect("tempdir");
        let spec_dir = tmp.path().join(".omne_data/spec");
        std::fs::create_dir_all(&spec_dir).expect("create registry dir");
        std::fs::write(
            spec_dir.join("tool_registry.json"),
            serde_json::json!({
                "version": 1,
                "tools": [
                    {
                        "name": "dyn_readme_reader",
                        "description": "Read README via dynamic registry",
                        "mapped_tool": "file_read",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" }
                            },
                            "required": ["path"],
                            "additionalProperties": false
                        },
                        "fixed_args": {
                            "root": "workspace"
                        },
                        "read_only": true
                    }
                ]
            })
            .to_string(),
        )
        .expect("write registry");

        let policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: true,
            facade_expose_legacy: false,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };
        let names = tool_names(&build_tools_for_turn_with_policy(None, policy, Some(tmp.path())));
        assert!(names.iter().any(|name| name == "dyn_readme_reader"));
    }

    #[test]
    fn dynamic_registry_respects_allowed_tool_filter() {
        let _lock = crate::app_server_process_env_lock().blocking_lock();
        let _enabled = ScopedEnvVar::set("OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED", "1");
        let _path = ScopedEnvVar::unset("OMNE_TOOL_DYNAMIC_REGISTRY_PATH");

        let tmp = tempfile::tempdir().expect("tempdir");
        let spec_dir = tmp.path().join(".omne_data/spec");
        std::fs::create_dir_all(&spec_dir).expect("create registry dir");
        std::fs::write(
            spec_dir.join("tool_registry.json"),
            serde_json::json!({
                "version": 1,
                "tools": [
                    {
                        "name": "dyn_repo_scan",
                        "mapped_tool": "repo_search",
                        "read_only": true
                    }
                ]
            })
            .to_string(),
        )
        .expect("write registry");

        let policy = ToolExposurePolicy {
            mcp_enabled: false,
            expose_subagent: false,
            expose_thread_introspection: false,
            expose_thread_hook: false,
            expose_repo_symbols: false,
            expose_web: false,
            facade_enabled: true,
            facade_expose_legacy: false,
            model_profile: ToolModelProfile::Full,
            role_profile: ToolRoleProfile::Codder,
        };
        let allowed = vec!["file/read".to_string()];
        let names = tool_names(&build_tools_for_turn_with_policy(
            Some(&allowed),
            policy,
            Some(tmp.path()),
        ));
        assert!(!names.iter().any(|name| name == "dyn_repo_scan"));
    }
}
