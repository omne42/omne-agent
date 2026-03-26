type ThreadStartParams = omne_app_server_protocol::ThreadStartParams;
type ThreadResumeParams = omne_app_server_protocol::ThreadResumeParams;
type ThreadForkParams = omne_app_server_protocol::ThreadForkParams;
type ThreadArchiveParams = omne_app_server_protocol::ThreadArchiveParams;
type ThreadUnarchiveParams = omne_app_server_protocol::ThreadUnarchiveParams;
type ThreadPauseParams = omne_app_server_protocol::ThreadPauseParams;
type ThreadUnpauseParams = omne_app_server_protocol::ThreadUnpauseParams;
type ThreadDeleteParams = omne_app_server_protocol::ThreadDeleteParams;
type ThreadClearArtifactsParams = omne_app_server_protocol::ThreadClearArtifactsParams;
type ThreadStateParams = omne_app_server_protocol::ThreadStateParams;
type ThreadUsageParams = omne_app_server_protocol::ThreadUsageParams;
type ThreadAttentionParams = omne_app_server_protocol::ThreadAttentionParams;
type ThreadListParams = omne_app_server_protocol::ThreadListParams;
type ThreadLoadedParams = omne_app_server_protocol::ThreadLoadedParams;
type ThreadListMetaParams = omne_app_server_protocol::ThreadListMetaParams;
type ThreadDiskUsageParams = omne_app_server_protocol::ThreadDiskUsageParams;
type ThreadDiskReportParams = omne_app_server_protocol::ThreadDiskReportParams;
type ThreadDiffParams = omne_app_server_protocol::ThreadDiffParams;
type ThreadPatchParams = omne_app_server_protocol::ThreadPatchParams;
type ThreadCheckpointCreateParams = omne_app_server_protocol::ThreadCheckpointCreateParams;
type ThreadCheckpointListParams = omne_app_server_protocol::ThreadCheckpointListParams;
type ThreadCheckpointRestoreParams = omne_app_server_protocol::ThreadCheckpointRestoreParams;
type WorkspaceHookName = omne_app_server_protocol::WorkspaceHookName;
type ThreadHookRunParams = omne_app_server_protocol::ThreadHookRunParams;

type ThreadConfigureParams = omne_app_server_protocol::ThreadConfigureParams;

type ThreadConfigExplainParams = omne_app_server_protocol::ThreadConfigExplainParams;
type ThreadModelsParams = omne_app_server_protocol::ThreadModelsParams;
type ThreadEventsParams = omne_app_server_protocol::ThreadEventsParams;

type ThreadSubscribeParams = omne_app_server_protocol::ThreadSubscribeParams;

type TurnStartParams = omne_app_server_protocol::TurnStartParams;
type TurnInterruptParams = omne_app_server_protocol::TurnInterruptParams;

type ProcessStartParams = omne_app_server_protocol::ProcessStartParams;
type ProcessListParams = omne_app_server_protocol::ProcessListParams;
type ProcessKillParams = omne_app_server_protocol::ProcessKillParams;
type ProcessInterruptParams = omne_app_server_protocol::ProcessInterruptParams;
type ProcessStream = omne_app_server_protocol::ProcessStream;
type ProcessTailParams = omne_app_server_protocol::ProcessTailParams;
type ProcessFollowParams = omne_app_server_protocol::ProcessFollowParams;
type ProcessInspectParams = omne_app_server_protocol::ProcessInspectParams;

type FileRoot = omne_app_server_protocol::FileRoot;

type FileReadParams = omne_app_server_protocol::FileReadParams;
type FileGlobParams = omne_app_server_protocol::FileGlobParams;
type FileGrepParams = omne_app_server_protocol::FileGrepParams;

type RepoSearchParams = omne_app_server_protocol::RepoSearchParams;
type RepoIndexParams = omne_app_server_protocol::RepoIndexParams;
type RepoSymbolsParams = omne_app_server_protocol::RepoSymbolsParams;
type RepoGotoDefinitionParams = omne_app_server_protocol::RepoGotoDefinitionParams;
type RepoFindReferencesParams = omne_app_server_protocol::RepoFindReferencesParams;

type FileWriteParams = omne_app_server_protocol::FileWriteParams;
type FilePatchParams = omne_app_server_protocol::FilePatchParams;
type FileEditParams = omne_app_server_protocol::FileEditParams;
type FileEditOp = omne_app_server_protocol::FileEditOp;
type FileDeleteParams = omne_app_server_protocol::FileDeleteParams;
type FsMkdirParams = omne_app_server_protocol::FsMkdirParams;

type ArtifactWriteParams = omne_app_server_protocol::ArtifactWriteParams;
type ArtifactListParams = omne_app_server_protocol::ArtifactListParams;
type ArtifactReadParams = omne_app_server_protocol::ArtifactReadParams;
type ArtifactVersionsParams = omne_app_server_protocol::ArtifactVersionsParams;
type ArtifactDeleteParams = omne_app_server_protocol::ArtifactDeleteParams;

type ApprovalDecideParams = omne_app_server_protocol::ApprovalDecideParams;
type ApprovalListParams = omne_app_server_protocol::ApprovalListParams;
