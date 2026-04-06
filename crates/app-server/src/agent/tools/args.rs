#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileReadArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileGlobArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    pattern: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct RepoIndexArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct RepoGotoDefinitionArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    symbol: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    max_files: Option<usize>,
    #[serde(default)]
    max_bytes_per_file: Option<u64>,
    #[serde(default)]
    max_symbols: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoFindReferencesArgs {
    #[serde(default)]
    root: Option<crate::FileRoot>,
    symbol: String,
    #[serde(default)]
    path: Option<String>,
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
#[serde(deny_unknown_fields)]
struct McpListToolsArgs {
    server: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpListResourcesArgs {
    server: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpCallArgs {
    server: String,
    tool: String,
    // Keep MCP payload nested and required to avoid root-level flattening regressions.
    arguments: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileWriteArgs {
    path: String,
    text: String,
    #[serde(default)]
    create_parent_dirs: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePatchArgs {
    path: String,
    patch: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEditArgs {
    path: String,
    edits: Vec<FileEditOpArgs>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEditOpArgs {
    old: String,
    new: String,
    #[serde(default)]
    expected_replacements: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDeleteArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FsMkdirArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessStartArgs {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessInspectArgs {
    process_id: String,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessTailArgs {
    process_id: String,
    stream: super::ProcessStream,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessFollowArgs {
    process_id: String,
    stream: super::ProcessStream,
    #[serde(default)]
    since_offset: u64,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessKillArgs {
    process_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactWriteArgs {
    artifact_type: String,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReadArgs {
    artifact_id: String,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactDeleteArgs {
    artifact_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlanStepArgs {
    step: String,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlanArgs {
    #[serde(default)]
    explanation: Option<String>,
    plan: Vec<UpdatePlanStepArgs>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestUserInputOptionArgs {
    label: String,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestUserInputQuestionArgs {
    header: String,
    id: String,
    question: String,
    options: Vec<RequestUserInputOptionArgs>,
}

#[derive(Debug, Deserialize)]
struct RequestUserInputArgs {
    questions: Vec<RequestUserInputQuestionArgs>,
}

#[derive(Debug, Deserialize)]
struct WebSearchArgs {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WebFetchArgs {
    url: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ViewImageArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

const FACADE_ERROR_INVALID_PARAMS: &str = "facade_invalid_params";
const FACADE_ERROR_UNSUPPORTED_OP: &str = "facade_unsupported_op";
const FACADE_ERROR_POLICY_DENIED: &str = "facade_policy_denied";

#[derive(Debug, Deserialize)]
struct FacadeToolArgs {
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    help: Option<bool>,
    #[serde(default)]
    topic: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
struct FacadeQuickstartExample {
    op: &'static str,
    example: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct FacadeAdvancedTopic {
    topic: &'static str,
    summary: &'static str,
    args_schema_hint: serde_json::Value,
    error_examples: Vec<serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
struct FacadeHelpResponse {
    facade_tool: &'static str,
    op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    quickstart: Vec<FacadeQuickstartExample>,
    advanced: Vec<FacadeAdvancedTopic>,
}

#[derive(Debug, serde::Serialize)]
struct FacadeErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug, serde::Serialize)]
struct FacadeErrorResponse {
    facade_tool: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    op: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mapped_action: Option<String>,
    error: FacadeErrorBody,
}

#[derive(Debug, Deserialize)]
struct ThreadStateArgs {
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadUsageArgs {
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

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum AgentSpawnMode {
    Fork,
    New,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AgentSpawnTaskPriority {
    High,
    Normal,
    Low,
}

impl AgentSpawnTaskPriority {
    fn rank(self) -> usize {
        match self {
            Self::High => 0,
            Self::Normal => 1,
            Self::Low => 2,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentSpawnTaskArgs {
    id: String,
    input: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    spawn_mode: Option<AgentSpawnMode>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    workspace_mode: Option<AgentSpawnWorkspaceMode>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
    #[serde(default)]
    expected_artifact_type: Option<String>,
    #[serde(default)]
    priority: Option<AgentSpawnTaskPriority>,
}

#[derive(Debug, Deserialize)]
struct AgentSpawnArgs {
    #[serde(default)]
    spawn_mode: Option<AgentSpawnMode>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    workspace_mode: Option<AgentSpawnWorkspaceMode>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
    #[serde(default)]
    expected_artifact_type: Option<String>,
    #[serde(default)]
    priority: Option<AgentSpawnTaskPriority>,
    tasks: Vec<AgentSpawnTaskArgs>,
}

#[derive(Debug, Deserialize)]
struct SubagentSendInputArgs {
    id: String,
    message: String,
    #[serde(default)]
    interrupt: bool,
}

#[derive(Debug, Deserialize)]
struct SubagentWaitArgs {
    ids: Vec<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SubagentCloseArgs {
    id: String,
    #[serde(default)]
    reason: Option<String>,
}
