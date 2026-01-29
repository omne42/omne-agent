// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("process_and_utils/process_follow.rs");
include!("process_and_utils/event_render.rs");
include!("process_and_utils/app_types.rs");
include!("process_and_utils/app_impl.rs");
include!("process_and_utils/utils.rs");
include!("process_and_utils/tests.rs");
