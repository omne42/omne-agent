use super::*;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(tag = "method")]
pub enum ClientRequest {
    #[serde(rename = "initialize")]
    Initialize {
        #[serde(rename = "id")]
        request_id: RequestId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "undefined")]
        params: Option<()>,
    },
    #[serde(rename = "initialized")]
    Initialized {
        #[serde(rename = "id")]
        request_id: RequestId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "undefined")]
        params: Option<()>,
    },
    #[serde(rename = "thread/start")]
    ThreadStart {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadStartParams,
    },
    #[serde(rename = "thread/resume")]
    ThreadResume {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadResumeParams,
    },
    #[serde(rename = "thread/fork")]
    ThreadFork {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadForkParams,
    },
    #[serde(rename = "thread/archive")]
    ThreadArchive {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadArchiveParams,
    },
    #[serde(rename = "thread/unarchive")]
    ThreadUnarchive {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadUnarchiveParams,
    },
    #[serde(rename = "thread/pause")]
    ThreadPause {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadPauseParams,
    },
    #[serde(rename = "thread/unpause")]
    ThreadUnpause {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadUnpauseParams,
    },
    #[serde(rename = "thread/delete")]
    ThreadDelete {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadDeleteParams,
    },
    #[serde(rename = "thread/clear_artifacts")]
    ThreadClearArtifacts {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadClearArtifactsParams,
    },
    #[serde(rename = "thread/list")]
    ThreadList {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadListParams,
    },
    #[serde(rename = "thread/list_meta")]
    ThreadListMeta {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadListMetaParams,
    },
    #[serde(rename = "thread/loaded")]
    ThreadLoaded {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadLoadedParams,
    },
    #[serde(rename = "thread/events")]
    ThreadEvents {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadEventsParams,
    },
    #[serde(rename = "thread/subscribe")]
    ThreadSubscribe {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadSubscribeParams,
    },
    #[serde(rename = "thread/state")]
    ThreadState {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadStateParams,
    },
    #[serde(rename = "thread/usage")]
    ThreadUsage {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadUsageParams,
    },
    #[serde(rename = "thread/attention")]
    ThreadAttention {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadAttentionParams,
    },
    #[serde(rename = "thread/disk_usage")]
    ThreadDiskUsage {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadDiskUsageParams,
    },
    #[serde(rename = "thread/disk_report")]
    ThreadDiskReport {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadDiskReportParams,
    },
    #[serde(rename = "thread/diff")]
    ThreadDiff {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadDiffParams,
    },
    #[serde(rename = "thread/patch")]
    ThreadPatch {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadPatchParams,
    },
    #[serde(rename = "thread/checkpoint/create")]
    ThreadCheckpointCreate {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadCheckpointCreateParams,
    },
    #[serde(rename = "thread/checkpoint/list")]
    ThreadCheckpointList {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadCheckpointListParams,
    },
    #[serde(rename = "thread/checkpoint/restore")]
    ThreadCheckpointRestore {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadCheckpointRestoreParams,
    },
    #[serde(rename = "thread/hook_run")]
    ThreadHookRun {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadHookRunParams,
    },
    #[serde(rename = "thread/configure")]
    ThreadConfigure {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadConfigureParams,
    },
    #[serde(rename = "thread/config/explain")]
    ThreadConfigExplain {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadConfigExplainParams,
    },
    #[serde(rename = "thread/models")]
    ThreadModels {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ThreadModelsParams,
    },
    #[serde(rename = "turn/start")]
    TurnStart {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: TurnStartParams,
    },
    #[serde(rename = "turn/interrupt")]
    TurnInterrupt {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: TurnInterruptParams,
    },
    #[serde(rename = "process/start")]
    ProcessStart {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessStartParams,
    },
    #[serde(rename = "process/list")]
    ProcessList {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessListParams,
    },
    #[serde(rename = "process/inspect")]
    ProcessInspect {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessInspectParams,
    },
    #[serde(rename = "process/kill")]
    ProcessKill {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessKillParams,
    },
    #[serde(rename = "process/interrupt")]
    ProcessInterrupt {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessInterruptParams,
    },
    #[serde(rename = "process/tail")]
    ProcessTail {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessTailParams,
    },
    #[serde(rename = "process/follow")]
    ProcessFollow {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ProcessFollowParams,
    },
    #[serde(rename = "file/read")]
    FileRead {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FileReadParams,
    },
    #[serde(rename = "file/glob")]
    FileGlob {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FileGlobParams,
    },
    #[serde(rename = "file/grep")]
    FileGrep {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FileGrepParams,
    },
    #[serde(rename = "repo/search")]
    RepoSearch {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: RepoSearchParams,
    },
    #[serde(rename = "repo/index")]
    RepoIndex {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: RepoIndexParams,
    },
    #[serde(rename = "repo/symbols")]
    RepoSymbols {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: RepoSymbolsParams,
    },
    #[serde(rename = "repo/goto_definition")]
    RepoGotoDefinition {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: RepoGotoDefinitionParams,
    },
    #[serde(rename = "repo/find_references")]
    RepoFindReferences {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: RepoFindReferencesParams,
    },
    #[serde(rename = "mcp/list_servers")]
    McpListServers {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: McpListServersParams,
    },
    #[serde(rename = "mcp/list_tools")]
    McpListTools {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: McpListToolsParams,
    },
    #[serde(rename = "mcp/list_resources")]
    McpListResources {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: McpListResourcesParams,
    },
    #[serde(rename = "mcp/call")]
    McpCall {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: McpCallParams,
    },
    #[serde(rename = "file/write")]
    FileWrite {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FileWriteParams,
    },
    #[serde(rename = "file/patch")]
    FilePatch {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FilePatchParams,
    },
    #[serde(rename = "file/edit")]
    FileEdit {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FileEditParams,
    },
    #[serde(rename = "file/delete")]
    FileDelete {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FileDeleteParams,
    },
    #[serde(rename = "fs/mkdir")]
    FsMkdir {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: FsMkdirParams,
    },
    #[serde(rename = "artifact/write")]
    ArtifactWrite {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ArtifactWriteParams,
    },
    #[serde(rename = "artifact/list")]
    ArtifactList {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ArtifactListParams,
    },
    #[serde(rename = "artifact/read")]
    ArtifactRead {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ArtifactReadParams,
    },
    #[serde(rename = "artifact/versions")]
    ArtifactVersions {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ArtifactVersionsParams,
    },
    #[serde(rename = "artifact/delete")]
    ArtifactDelete {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ArtifactDeleteParams,
    },
    #[serde(rename = "approval/decide")]
    ApprovalDecide {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ApprovalDecideParams,
    },
    #[serde(rename = "approval/list")]
    ApprovalList {
        #[serde(rename = "id")]
        request_id: RequestId,
        params: ApprovalListParams,
    },
}
