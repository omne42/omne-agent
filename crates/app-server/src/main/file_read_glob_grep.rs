// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

include!("file_read_glob_grep/read.rs");
include!("file_read_glob_grep/shared.rs");
include!("file_read_glob_grep/glob.rs");
include!("file_read_glob_grep/grep.rs");
