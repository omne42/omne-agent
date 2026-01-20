mod checks;
pub mod coder;
pub mod git;
mod identity;
mod lock;
pub mod merger;
pub mod repo;

pub use crate::coder::GitCoder;
pub use crate::git::GitCli;
pub use crate::lock::{lock_exclusive, lock_shared};
pub use crate::merger::GitMerger;
pub use crate::repo::{RepoManager, find_repo_root};
