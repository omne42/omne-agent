#[derive(Debug, Deserialize)]
struct FileReadArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileGlobArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    pattern: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileGrepArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    query: String,
    #[serde(default)]
    is_regex: bool,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_matches: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RepoSearchArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    query: String,
    #[serde(default)]
    is_regex: bool,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_matches: Option<usize>,
    #[serde(default)]
    max_bytes_per_file: Option<u64>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RepoIndexArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RepoSymbolsArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_files: Option<usize>,
    #[serde(default)]
    max_bytes_per_file: Option<u64>,
    #[serde(default)]
    max_symbols: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct McpListToolsArgs {
    server: String,
}

#[derive(Debug, Deserialize)]
struct McpListResourcesArgs {
    server: String,
}

#[derive(Debug, Deserialize)]
struct McpCallArgs {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct FileWriteArgs {
    path: String,
    text: String,
    #[serde(default)]
    create_parent_dirs: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FilePatchArgs {
    path: String,
    patch: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditArgs {
    path: String,
    edits: Vec<FileEditOpArgs>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditOpArgs {
    old: String,
    new: String,
    #[serde(default)]
    expected_replacements: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileDeleteArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct FsMkdirArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct ProcessStartArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessInspectArgs {
    process_id: String,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessTailArgs {
    process_id: String,
    stream: super::ProcessStream,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessFollowArgs {
    process_id: String,
    stream: super::ProcessStream,
    #[serde(default)]
    since_offset: u64,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ProcessKillArgs {
    process_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtifactWriteArgs {
    artifact_type: String,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactReadArgs {
    artifact_id: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ArtifactDeleteArgs {
    artifact_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadStateArgs {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadEventsArgs {
    thread_id: String,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ThreadHookRunArgs {
    hook: super::WorkspaceHookName,
}

#[derive(Debug, Deserialize)]
struct ThreadDiffArgs {
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum AgentSpawnWorkspaceMode {
    ReadOnly,
    IsolatedWrite,
}

#[derive(Debug, Deserialize)]
struct AgentSpawnArgs {
    input: String,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    expected_artifact_type: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    workspace_mode: Option<AgentSpawnWorkspaceMode>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
}
