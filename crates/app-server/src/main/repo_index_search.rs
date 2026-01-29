// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("repo_index_search/search.rs");
include!("repo_index_search/index.rs");
include!("repo_index_search/symbols.rs");
include!("repo_index_search/format.rs");
