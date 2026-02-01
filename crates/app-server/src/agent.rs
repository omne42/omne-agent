// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("agent/core.rs");
include!("agent/tool_loop.rs");
#[cfg(test)]
include!("agent/openai_history.rs");
include!("agent/tools.rs");
