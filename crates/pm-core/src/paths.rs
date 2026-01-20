use std::path::{Path, PathBuf};

use crate::domain::{RepositoryName, SessionId, TaskId};

#[derive(Clone, Debug)]
pub struct PmPaths {
    root: PathBuf,
}

impl PmPaths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.root.join("repos")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    pub fn repo_bare_path(&self, name: &RepositoryName) -> PathBuf {
        self.repos_dir().join(format!("{}.git", name.as_str()))
    }

    pub fn repo_lock_path(&self, name: &RepositoryName) -> PathBuf {
        self.locks_dir().join(format!("{}.lock", name.as_str()))
    }

    pub fn session_dir(&self, session_id: SessionId) -> PathBuf {
        self.data_dir()
            .join("sessions")
            .join(session_id.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct SessionPaths {
    root: PathBuf,
}

impl SessionPaths {
    pub fn new(repo: &RepositoryName, session_id: SessionId) -> Self {
        let tmp_root = match std::env::var_os("CODE_PM_TMP_ROOT").map(PathBuf::from) {
            Some(path) => path,
            None => {
                let path = PathBuf::from("/tmp");
                if path.is_dir() {
                    path
                } else {
                    std::env::temp_dir()
                }
            }
        };
        let root = tmp_root.join(format!("{}_{}", repo.as_str(), session_id));
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn tasks_dir(&self) -> PathBuf {
        self.root.join("tasks")
    }

    pub fn task_paths(&self, task_id: &TaskId) -> TaskPaths {
        TaskPaths::new(self.tasks_dir().join(task_id.as_str()))
    }

    pub fn merge_dir(&self) -> PathBuf {
        self.root.join("merge")
    }
}

#[derive(Clone, Debug)]
pub struct TaskPaths {
    root: PathBuf,
}

impl TaskPaths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn repo_dir(&self) -> PathBuf {
        self.root.join("repo")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.root.join("artifacts")
    }

    pub fn cargo_target_dir(&self) -> PathBuf {
        self.artifacts_dir().join("cargo-target")
    }
}
