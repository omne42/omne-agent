fn build_tools() -> Vec<Value> {
    vec![
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
            "process_start",
            "Start a background process (non-interactive).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "argv": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                },
                "required": ["argv"],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
            "artifact_list",
            "List user-facing artifacts for this thread.",
            serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false,
            }),
        ),
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
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
        pm_openai::tool_function(
            "agent_spawn",
            "Fork the current thread and start a background agent turn in the forked thread.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" },
                    "task_id": { "type": "string" },
                    "expected_artifact_type": { "type": "string" },
                    "mode": { "type": "string" },
                    "workspace_mode": { "type": "string", "enum": ["read_only", "isolated_write"] },
                    "model": { "type": "string" },
                    "openai_base_url": { "type": "string" },
                },
                "required": ["input"],
                "additionalProperties": false,
            }),
        ),
    ]
}
