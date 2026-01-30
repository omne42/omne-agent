// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

mod agent;
mod model_limits;
#[cfg(feature = "notify")]
mod notify_integration;
mod project_config;

include!("main/preamble.rs");
include!("main/app.rs");
include!("main/thread_observe.rs");
include!("main/thread_manage.rs");
include!("main/approval.rs");
include!("main/file_read_glob_grep.rs");
include!("main/repo_index_search.rs");
include!("main/mcp.rs");
include!("main/file_write_patch.rs");
include!("main/file_edit_delete.rs");
include!("main/fs.rs");
include!("main/artifact.rs");
include!("main/hooks_dispatch.rs");
include!("main/process_control.rs");
include!("main/process_stream.rs");

#[cfg(test)]
include!("main/artifact_history_tests.rs");

#[cfg(test)]
include!("main/thread_diff_tests.rs");

#[cfg(test)]
include!("main/repo_index_search_tests.rs");

#[cfg(test)]
include!("main/mcp_tests.rs");

#[cfg(test)]
include!("main/fs_tools_security_tests.rs");
