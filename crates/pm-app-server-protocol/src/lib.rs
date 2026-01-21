use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod export;

pub use export::{generate_json_schema, generate_ts};

pub const JSONRPC_VERSION: &str = "2.0";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    #[ts(type = "number")]
    Integer(i64),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct JsonRpcRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub jsonrpc: Option<String>,
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct JsonRpcResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub jsonrpc: Option<String>,
    pub id: RequestId,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct JsonRpcErrorResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub jsonrpc: Option<String>,
    pub id: RequestId,
    pub error: JsonRpcError,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStartParams {
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadResumeParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadForkParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadArchiveParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnarchiveParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadPauseParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadUnpauseParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDeleteParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadClearArtifactsParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadStateParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadAttentionParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadListMetaParams {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskUsageParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadDiskReportParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub top_files: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigureParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub approval_policy: Option<pm_protocol::ApprovalPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub sandbox_policy: Option<pm_protocol::SandboxPolicy>,
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub openai_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadConfigExplainParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadEventsParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_events: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ThreadSubscribeParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_events: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct TurnStartParams {
    pub thread_id: pm_protocol::ThreadId,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct TurnInterruptParams {
    pub thread_id: pm_protocol::ThreadId,
    pub turn_id: pm_protocol::TurnId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessStartParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub argv: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessListParams {
    #[serde(default)]
    #[ts(optional)]
    pub thread_id: Option<pm_protocol::ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessKillParams {
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessTailParams {
    pub process_id: pm_protocol::ProcessId,
    pub stream: ProcessStream,
    #[serde(default)]
    #[ts(optional)]
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessFollowParams {
    pub process_id: pm_protocol::ProcessId,
    pub stream: ProcessStream,
    #[serde(default)]
    pub since_offset: u64,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ProcessInspectParams {
    pub process_id: pm_protocol::ProcessId,
    #[serde(default)]
    #[ts(optional)]
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileReadParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    pub path: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileGlobParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    pub pattern: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileGrepParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    pub query: String,
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    #[ts(optional)]
    pub include_glob: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub max_matches: Option<usize>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes_per_file: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileWriteParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub path: String,
    pub text: String,
    #[serde(default)]
    #[ts(optional)]
    pub create_parent_dirs: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FilePatchParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub path: String,
    pub patch: String,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileEditParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub path: String,
    pub edits: Vec<FileEditOp>,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileEditOp {
    pub old: String,
    pub new: String,
    #[serde(default)]
    #[ts(optional)]
    pub expected_replacements: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FileDeleteParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct FsMkdirParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub approval_id: Option<pm_protocol::ApprovalId>,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactWriteParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    #[serde(default)]
    #[ts(optional)]
    pub artifact_id: Option<pm_protocol::ArtifactId>,
    pub artifact_type: String,
    pub summary: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactListParams {
    pub thread_id: pm_protocol::ThreadId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactReadParams {
    pub thread_id: pm_protocol::ThreadId,
    pub artifact_id: pm_protocol::ArtifactId,
    #[serde(default)]
    #[ts(optional)]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ArtifactDeleteParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    #[ts(optional)]
    pub turn_id: Option<pm_protocol::TurnId>,
    pub artifact_id: pm_protocol::ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalDecideParams {
    pub thread_id: pm_protocol::ThreadId,
    pub approval_id: pm_protocol::ApprovalId,
    pub decision: pm_protocol::ApprovalDecision,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    #[ts(optional)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
pub struct ApprovalListParams {
    pub thread_id: pm_protocol::ThreadId,
    #[serde(default)]
    pub include_decided: bool,
}

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "undefined")]
        params: Option<()>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(type = "undefined")]
        params: Option<()>,
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
