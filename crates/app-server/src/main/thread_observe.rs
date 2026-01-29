// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("thread_observe/attention_and_subscribe.rs");
include!("thread_observe/disk_git_diff.rs");
