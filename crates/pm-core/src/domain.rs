use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum NameError {
    #[error("name must not be empty")]
    Empty,
    #[error("name contains forbidden path segment: {0}")]
    ForbiddenSegment(String),
    #[error("name contains invalid character: {0:?}")]
    InvalidChar(char),
}

fn is_allowed_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-'
}

fn validate_name(value: &str) -> Result<(), NameError> {
    if value.is_empty() {
        return Err(NameError::Empty);
    }
    if value == "." || value == ".." {
        return Err(NameError::ForbiddenSegment(value.to_string()));
    }
    for ch in value.chars() {
        if !is_allowed_char(ch) {
            return Err(NameError::InvalidChar(ch));
        }
    }
    Ok(())
}

fn sanitize_name(input: &str, fallback: &str) -> String {
    let trimmed = input.trim();
    let mut out = String::with_capacity(trimmed.len());

    let mut last_dash = false;
    for ch in trimmed.chars() {
        let mapped = if is_allowed_char(ch) {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(mapped);
    }

    let sanitized = out.trim_matches('-').to_string();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        fallback.to_string()
    } else {
        sanitized
    }
}

macro_rules! name_type {
    ($ty:ident, $fallback:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $ty(String);

        impl $ty {
            pub fn new(value: impl Into<String>) -> Result<Self, NameError> {
                let value = value.into();
                validate_name(&value)?;
                Ok(Self(value))
            }

            pub fn sanitize(input: &str) -> Self {
                Self(sanitize_name(input, $fallback))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

name_type!(RepositoryName, "repo");
name_type!(PrName, "pr");
name_type!(TaskId, "task");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    pub name: RepositoryName,
    pub bare_path: PathBuf,
    pub lock_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub repo: RepositoryName,
    pub pr_name: PrName,
    pub prompt: String,
    pub base_branch: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: TaskId,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StepSummary {
    pub name: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub log_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CheckSummary {
    pub steps: Vec<StepSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullRequestStatus {
    Draft,
    Ready,
    NoChanges,
    Merged,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: TaskId,
    pub head_branch: String,
    pub base_branch: String,
    pub status: PullRequestStatus,
    pub checks: CheckSummary,
    pub head_commit: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MergeResult {
    pub merged: bool,
    pub base_branch: String,
    pub merge_commit: Option<String>,
    pub merged_prs: Vec<TaskId>,
    #[serde(default)]
    pub checks: CheckSummary,
    pub error: Option<String>,
    pub error_log_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HookSpec {
    Command { program: PathBuf, args: Vec<String> },
}

#[derive(Clone, Debug)]
pub struct RunRequest {
    pub pr_name: PrName,
    pub prompt: String,
    pub base_branch: String,
    pub tasks: Option<Vec<TaskSpec>>,
    pub apply_patch: Option<PathBuf>,
    pub hook: Option<HookSpec>,
    pub max_concurrency: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub session: Session,
    pub tasks: Vec<TaskSpec>,
    pub prs: Vec<PullRequest>,
    pub merge: MergeResult,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_produces_path_safe_names() {
        assert_eq!(RepositoryName::sanitize(" Foo/Bar ").as_str(), "foo-bar");
        assert_eq!(PrName::sanitize("..").as_str(), "pr");
        assert_eq!(TaskId::sanitize("").as_str(), "task");
    }

    #[test]
    fn validate_rejects_invalid_chars() {
        let err = RepositoryName::new("no spaces".to_string()).unwrap_err();
        let NameError::InvalidChar(ch) = err else {
            panic!("unexpected error: {err:?}");
        };
        assert_eq!(ch, ' ');
    }
}
