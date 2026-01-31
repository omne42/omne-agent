// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("tool_loop/core.rs");
include!("tool_loop/doom_loop.rs");
include!("tool_loop/openai_responses_stream.rs");
include!("tool_loop/openai_responses_helpers.rs");
include!("tool_loop/openai_responses_reasoning.rs");
include!("tool_loop/openai_responses_loop.rs");
include!("tool_loop/llm_stream.rs");
