// Split from a formerly-large file. Kept as `include!()` blocks to preserve scope and minimize refactor risk.
// Follow-up refactors can convert these into proper modules.

use std::collections::{BTreeMap, BTreeSet};

#[path = "command/types.rs"]
mod types;
use types::*;
include!("command/fan_out.rs");
include!("command/workflow.rs");
include!("command/tasks.rs");

#[path = "command/utils.rs"]
mod utils;
use utils::*;

include!("command/tests.rs");
