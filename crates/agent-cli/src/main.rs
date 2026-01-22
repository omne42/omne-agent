// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("main/preamble.rs");
include!("main/app.rs");
include!("main/ask_exec.rs");
include!("main/repl.rs");
include!("main/watch_inbox.rs");
include!("main/process_and_utils.rs");
