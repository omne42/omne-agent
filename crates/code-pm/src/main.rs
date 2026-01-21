// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("main/preamble.rs");
include!("main/app.rs");
include!("main/helpers1.rs");
include!("main/helpers2.rs");
include!("main/tasks.rs");
