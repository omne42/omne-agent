// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("tool_loop/core.rs");
include!("tool_loop/doom_loop.rs");
include!("tool_loop/llm_stream.rs");
