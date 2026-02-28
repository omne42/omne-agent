// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod export;

pub use export::{generate_json_schema, generate_ts};

macro_rules! define_tool_denied_response {
    ($name:ident { $($extra_fields:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
        pub struct $name {
            pub tool_id: omne_protocol::ToolId,
            pub denied: bool,
            $($extra_fields)*
            #[serde(default)]
            #[ts(optional)]
            pub remembered: Option<bool>,
            #[serde(default)]
            #[ts(optional)]
            pub error_code: Option<String>,
        }
    };
}

macro_rules! define_tool_needs_approval_response {
    ($name:ident { $($extra_fields:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
        pub struct $name {
            pub needs_approval: bool,
            $($extra_fields)*
            pub approval_id: omne_protocol::ApprovalId,
        }
    };
}

macro_rules! define_tool_denied_response_skip_none {
    ($name:ident { $($extra_fields:tt)* }) => {
        #[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema, TS)]
        pub struct $name {
            pub tool_id: omne_protocol::ToolId,
            pub denied: bool,
            $($extra_fields)*
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            pub remembered: Option<bool>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            #[ts(optional)]
            pub error_code: Option<String>,
        }
    };
}

#[path = "lib/approval.rs"]
mod approval;
#[path = "lib/artifact.rs"]
mod artifact;
#[path = "lib/client_request.rs"]
mod client_request;
#[path = "lib/file_edit_delete.rs"]
mod file_edit_delete;
#[path = "lib/file_read_glob_grep.rs"]
mod file_read_glob_grep;
#[path = "lib/file_write_patch.rs"]
mod file_write_patch;
#[path = "lib/fs.rs"]
mod fs;
#[path = "lib/jsonrpc.rs"]
mod jsonrpc;
#[path = "lib/mcp.rs"]
mod mcp;
#[path = "lib/process.rs"]
mod process;
#[path = "lib/repo_index_search.rs"]
mod repo_index_search;
#[path = "lib/server_notification.rs"]
mod server_notification;
#[path = "lib/thread.rs"]
mod thread;
#[path = "lib/turn.rs"]
mod turn;

pub use approval::*;
pub use artifact::*;
pub use client_request::*;
pub use file_edit_delete::*;
pub use file_read_glob_grep::*;
pub use file_write_patch::*;
pub use fs::*;
pub use jsonrpc::*;
pub use mcp::*;
pub use process::*;
pub use repo_index_search::*;
pub use server_notification::*;
pub use thread::*;
pub use turn::*;
