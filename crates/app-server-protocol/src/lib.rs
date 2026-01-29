// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod export;

pub use export::{generate_json_schema, generate_ts};

include!("lib/jsonrpc.rs");
include!("lib/thread.rs");
include!("lib/turn.rs");
include!("lib/process.rs");
include!("lib/file_read_glob_grep.rs");
include!("lib/repo_index_search.rs");
include!("lib/file_write_patch.rs");
include!("lib/file_edit_delete.rs");
include!("lib/fs.rs");
include!("lib/artifact.rs");
include!("lib/approval.rs");
include!("lib/client_request.rs");
include!("lib/server_notification.rs");
