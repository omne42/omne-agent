#[derive(Debug, Deserialize)]
struct ThreadStartParams {
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadResumeParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadForkParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadArchiveParams {
    thread_id: ThreadId,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadUnarchiveParams {
    thread_id: ThreadId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadPauseParams {
    thread_id: ThreadId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadUnpauseParams {
    thread_id: ThreadId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadClearArtifactsParams {
    thread_id: ThreadId,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadStateParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadAttentionParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadListMetaParams {
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
struct ThreadDiskUsageParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadDiskReportParams {
    thread_id: ThreadId,
    #[serde(default)]
    top_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ThreadDiffParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ThreadPatchParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    max_bytes: Option<u64>,
    #[serde(default)]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ThreadCheckpointCreateParams {
    thread_id: ThreadId,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadCheckpointListParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadCheckpointRestoreParams {
    thread_id: ThreadId,
    checkpoint_id: omne_agent_protocol::CheckpointId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum WorkspaceHookName {
    Setup,
    Run,
    Archive,
}

#[derive(Debug, Deserialize)]
struct ThreadHookRunParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    hook: WorkspaceHookName,
}

#[derive(Debug, Deserialize)]
struct ThreadConfigureParams {
    thread_id: ThreadId,
    #[serde(default)]
    approval_policy: Option<omne_agent_protocol::ApprovalPolicy>,
    #[serde(default)]
    sandbox_policy: Option<omne_agent_protocol::SandboxPolicy>,
    #[serde(default)]
    sandbox_writable_roots: Option<Vec<String>>,
    #[serde(default)]
    sandbox_network_access: Option<omne_agent_protocol::SandboxNetworkAccess>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    openai_provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    openai_base_url: Option<String>,
    #[serde(default)]
    allowed_tools: Option<Option<Vec<String>>>,
}

#[derive(Debug, Deserialize)]
struct ThreadConfigExplainParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadModelsParams {
    thread_id: ThreadId,
}

#[derive(Debug, Deserialize)]
struct ThreadEventsParams {
    thread_id: ThreadId,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ThreadSubscribeParams {
    thread_id: ThreadId,
    #[serde(default)]
    since_seq: u64,
    #[serde(default)]
    max_events: Option<usize>,
    /// Long-poll timeout in milliseconds. When set to 0, returns immediately.
    #[serde(default)]
    wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TurnStartParams {
    thread_id: ThreadId,
    input: String,
    #[serde(default)]
    context_refs: Option<Vec<omne_agent_protocol::ContextRef>>,
    #[serde(default)]
    attachments: Option<Vec<omne_agent_protocol::TurnAttachment>>,
    #[serde(default)]
    priority: Option<omne_agent_protocol::TurnPriority>,
}

#[derive(Debug, Deserialize)]
struct TurnInterruptParams {
    thread_id: ThreadId,
    turn_id: TurnId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessStartParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessListParams {
    #[serde(default)]
    thread_id: Option<ThreadId>,
}

#[derive(Debug, Deserialize)]
struct ProcessKillParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessInterruptParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ProcessStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Deserialize)]
struct ProcessTailParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    stream: ProcessStream,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ProcessFollowParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    stream: ProcessStream,
    #[serde(default)]
    since_offset: u64,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ProcessInspectParams {
    process_id: ProcessId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    max_lines: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum FileRoot {
    Workspace,
    Reference,
}

impl FileRoot {
    fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Reference => "reference",
        }
    }
}

#[derive(Debug, Deserialize)]
struct FileReadParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    path: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileGlobParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    #[serde(default)]
    path_prefix: Option<String>,
    pattern: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileGrepParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    #[serde(default)]
    path_prefix: Option<String>,
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
struct RepoSearchParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
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
struct RepoIndexParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
    #[serde(default)]
    include_glob: Option<String>,
    #[serde(default)]
    max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RepoSymbolsParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    root: Option<FileRoot>,
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
struct FileWriteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    path: String,
    text: String,
    #[serde(default)]
    create_parent_dirs: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FilePatchParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    path: String,
    patch: String,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    path: String,
    edits: Vec<FileEditOp>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FileEditOp {
    old: String,
    new: String,
    #[serde(default)]
    expected_replacements: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FileDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct FsMkdirParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Debug, Deserialize)]
struct ArtifactWriteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    #[serde(default)]
    artifact_id: Option<ArtifactId>,
    artifact_type: String,
    summary: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactListParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
}

#[derive(Debug, Deserialize)]
struct ArtifactReadParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    artifact_id: ArtifactId,
    #[serde(default)]
    max_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ArtifactDeleteParams {
    thread_id: ThreadId,
    #[serde(default)]
    turn_id: Option<TurnId>,
    #[serde(default)]
    approval_id: Option<omne_agent_protocol::ApprovalId>,
    artifact_id: ArtifactId,
}

#[derive(Debug, Deserialize)]
struct ApprovalDecideParams {
    thread_id: ThreadId,
    approval_id: omne_agent_protocol::ApprovalId,
    decision: omne_agent_protocol::ApprovalDecision,
    #[serde(default)]
    remember: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApprovalListParams {
    thread_id: ThreadId,
    #[serde(default)]
    include_decided: bool,
}
