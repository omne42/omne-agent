use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Context;
use pm_core::{PmPaths, Repository, RepositoryName};
use tracing::info;

use crate::checks::os_arg;
use crate::git::GitCli;
use crate::lock::lock_exclusive;

async fn normalize_git_source_arg(source: &str) -> anyhow::Result<OsString> {
    let path = Path::new(source);
    if tokio::fs::try_exists(path).await? {
        let abs_path = match tokio::fs::canonicalize(path).await {
            Ok(path) => path,
            Err(_) if path.is_absolute() => path.to_path_buf(),
            Err(_) => std::env::current_dir()?.join(path),
        };
        Ok(abs_path.into_os_string())
    } else {
        Ok(OsString::from(source))
    }
}

#[derive(Clone)]
pub struct RepoManager {
    paths: PmPaths,
    git: GitCli,
}

impl RepoManager {
    pub fn new(paths: PmPaths) -> Self {
        Self { paths, git: GitCli }
    }

    pub fn paths(&self) -> &PmPaths {
        &self.paths
    }

    pub async fn ensure_layout(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(self.paths.root()).await?;
        tokio::fs::create_dir_all(self.paths.repos_dir()).await?;
        tokio::fs::create_dir_all(self.paths.data_dir()).await?;
        tokio::fs::create_dir_all(self.paths.locks_dir()).await?;
        Ok(())
    }

    pub async fn repo_exists(&self, name: &RepositoryName) -> anyhow::Result<bool> {
        let path = self.paths.repo_bare_path(name);
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.is_dir() => is_valid_bare_repo_dir(&path).await,
            Ok(_) => Ok(false),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err).with_context(|| format!("stat {}", path.display())),
        }
    }

    pub async fn inject(&self, name: &RepositoryName, source: &str) -> anyhow::Result<Repository> {
        self.ensure_layout().await?;

        let lock_path = self.paths.repo_lock_path(name);
        let _lock_file = lock_exclusive(&lock_path)
            .await
            .context("lock repo injection")?;

        let bare_path = self.paths.repo_bare_path(name);
        let bare_exists = match tokio::fs::metadata(&bare_path).await {
            Ok(meta) => {
                if !meta.is_dir() {
                    anyhow::bail!(
                        "invalid repo {name} (expected bare repo directory at {})",
                        bare_path.display()
                    );
                }
                if !is_valid_bare_repo_dir(&bare_path).await? {
                    anyhow::bail!(
                        "invalid repo {name} (expected bare git repository at {})",
                        bare_path.display()
                    );
                }
                true
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => return Err(err).with_context(|| format!("stat {}", bare_path.display())),
        };

        if bare_exists {
            info!(repo = %name, "updating injected repo");
            let source_arg = normalize_git_source_arg(source).await?;

            let has_origin_args = vec![os_arg("remote"), os_arg("get-url"), os_arg("origin")];
            let has_origin = self.git.run(&bare_path, &has_origin_args, None).await?;
            if has_origin.ok {
                let set_url_args = vec![
                    os_arg("remote"),
                    os_arg("set-url"),
                    os_arg("origin"),
                    source_arg.clone(),
                ];
                let output = self.git.run(&bare_path, &set_url_args, None).await?;
                if !output.ok {
                    anyhow::bail!(
                        "git remote set-url origin failed (exit {:?}): {}",
                        output.exit_code,
                        output.stderr
                    );
                }
            } else {
                let add_remote_args = vec![
                    os_arg("remote"),
                    os_arg("add"),
                    os_arg("origin"),
                    source_arg.clone(),
                ];
                let output = self.git.run(&bare_path, &add_remote_args, None).await?;
                if !output.ok {
                    anyhow::bail!(
                        "git remote add origin failed (exit {:?}): {}",
                        output.exit_code,
                        output.stderr
                    );
                }
            }

            let fetch_args = vec![
                os_arg("fetch"),
                os_arg("--prune"),
                os_arg("origin"),
                os_arg("+refs/*:refs/*"),
            ];
            let output = self.git.run(&bare_path, &fetch_args, None).await?;
            if !output.ok {
                anyhow::bail!(
                    "git fetch failed (exit {:?}): {}",
                    output.exit_code,
                    output.stderr
                );
            }
        } else {
            info!(repo = %name, "injecting repo via mirror clone");
            let parent = bare_path.parent().context("bare path has no parent")?;
            tokio::fs::create_dir_all(parent).await?;
            let source_arg = normalize_git_source_arg(source).await?;
            let clone_args = vec![
                os_arg("clone"),
                os_arg("--mirror"),
                source_arg,
                os_arg(bare_path.as_path()),
            ];
            let output = self.git.run(parent, &clone_args, None).await?;
            if !output.ok {
                anyhow::bail!(
                    "git clone --mirror failed (exit {:?}): {}",
                    output.exit_code,
                    output.stderr
                );
            }
        }

        for (key, value) in [("http.receivepack", "true"), ("http.uploadpack", "true")] {
            let config_args = vec![os_arg("config"), os_arg(key), os_arg(value)];
            let output = self.git.run(&bare_path, &config_args, None).await?;
            if !output.ok {
                anyhow::bail!(
                    "git config {key} failed (exit {:?}): {}",
                    output.exit_code,
                    output.stderr
                );
            }
        }

        Ok(Repository {
            name: name.clone(),
            bare_path,
            lock_path,
        })
    }

    pub async fn load(&self, name: &RepositoryName) -> anyhow::Result<Repository> {
        let bare_path = self.paths.repo_bare_path(name);
        match tokio::fs::metadata(&bare_path).await {
            Ok(meta) => {
                if !meta.is_dir() {
                    anyhow::bail!(
                        "invalid repo {name} (expected bare repo directory at {})",
                        bare_path.display()
                    );
                }
                if !is_valid_bare_repo_dir(&bare_path).await? {
                    anyhow::bail!(
                        "invalid repo {name} (expected bare git repository at {})",
                        bare_path.display()
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                anyhow::bail!("unknown repo {name} (missing {})", bare_path.display());
            }
            Err(err) => return Err(err).with_context(|| format!("stat {}", bare_path.display())),
        }
        Ok(Repository {
            name: name.clone(),
            bare_path,
            lock_path: self.paths.repo_lock_path(name),
        })
    }

    pub async fn list_repos(&self) -> anyhow::Result<Vec<RepositoryName>> {
        self.ensure_layout().await?;
        let mut names = Vec::new();
        let mut read_dir = tokio::fs::read_dir(self.paths.repos_dir()).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("git") {
                continue;
            }
            let meta = entry.metadata().await?;
            if !meta.is_dir() {
                continue;
            }
            if !is_valid_bare_repo_dir(&path).await? {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(name) = RepositoryName::new(stem.to_string()) else {
                continue;
            };
            names.push(name);
        }
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(names)
    }

    pub fn default_repo_name_from_source(source: &str) -> RepositoryName {
        let mut base = source.trim();
        base = base.trim_end_matches(['/', '\\']);

        if base.contains('/') || base.contains('\\') {
            base = base.rsplit(['/', '\\']).next().unwrap_or(base);
        } else if base.contains('@') {
            base = base.rsplit(':').next().unwrap_or(base);
        }

        RepositoryName::sanitize(base.trim_end_matches(".git"))
    }
}

pub async fn is_valid_bare_repo_dir(path: &Path) -> anyhow::Result<bool> {
    let head_path = path.join("HEAD");
    let config_path = path.join("config");
    let objects_path = path.join("objects");

    let head = match tokio::fs::metadata(&head_path).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("stat {}", head_path.display())),
    };
    if !head.is_file() {
        return Ok(false);
    }

    let config = match tokio::fs::metadata(&config_path).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("stat {}", config_path.display())),
    };
    if !config.is_file() {
        return Ok(false);
    }

    let objects = match tokio::fs::metadata(&objects_path).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("stat {}", objects_path.display())),
    };
    Ok(objects.is_dir())
}

pub fn is_rust_repo(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
}

#[derive(Clone, Debug)]
pub struct RepoRoot {
    pub root: PathBuf,
    pub is_git_repo: bool,
}

pub fn find_repo_root(cwd: &Path) -> anyhow::Result<RepoRoot> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            Ok(RepoRoot {
                root: PathBuf::from(text.trim()),
                is_git_repo: true,
            })
        }
        _ => Ok(RepoRoot {
            root: cwd.to_path_buf(),
            is_git_repo: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    #[test]
    fn default_repo_name_handles_urls_and_paths() {
        assert_eq!(
            RepoManager::default_repo_name_from_source("https://example.com/foo/bar.git").as_str(),
            "bar"
        );
        assert_eq!(
            RepoManager::default_repo_name_from_source("git@github.com:foo/bar.git").as_str(),
            "bar"
        );
        assert_eq!(
            RepoManager::default_repo_name_from_source("git@github.com:foo").as_str(),
            "foo"
        );
        assert_eq!(
            RepoManager::default_repo_name_from_source("/tmp/MyRepo").as_str(),
            "myrepo"
        );
        assert_eq!(
            RepoManager::default_repo_name_from_source(r"C:\Users\me\Repo.git").as_str(),
            "repo"
        );
        assert_eq!(
            RepoManager::default_repo_name_from_source(" ").as_str(),
            "repo"
        );
    }

    #[tokio::test]
    async fn list_repos_ignores_invalid_entries() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let repo_manager = RepoManager::new(pm_paths.clone());
        repo_manager.ensure_layout().await?;

        let good_repo = pm_paths.repos_dir().join("good.git");
        let output = Command::new("git")
            .current_dir(tmp.path())
            .arg("init")
            .arg("--bare")
            .arg(&good_repo)
            .output()
            .await?;
        if !output.status.success() {
            anyhow::bail!(
                "git init --bare failed (exit {:?}): {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        tokio::fs::create_dir_all(pm_paths.repos_dir().join("empty.git")).await?;
        tokio::fs::write(pm_paths.repos_dir().join("bad.git"), b"not a repo").await?;

        let repos = repo_manager.list_repos().await?;
        assert_eq!(
            repos.iter().map(|repo| repo.as_str()).collect::<Vec<_>>(),
            vec!["good"]
        );
        Ok(())
    }
}
