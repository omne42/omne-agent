// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("core/preamble.rs");
include!("core/role_prompts.rs");
include!("core/run_turn.rs");
include!("core/auto_compact_and_config.rs");
include!("core/conversation.rs");
include!("core/tests.rs");
