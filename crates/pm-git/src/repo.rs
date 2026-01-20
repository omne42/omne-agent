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
        Ok(tokio::fs::try_exists(self.paths.repo_bare_path(name)).await?)
    }

    pub async fn inject(&self, name: &RepositoryName, source: &str) -> anyhow::Result<Repository> {
        self.ensure_layout().await?;

        let lock_path = self.paths.repo_lock_path(name);
        let _lock_file = lock_exclusive(&lock_path)
            .await
            .context("lock repo injection")?;

        let bare_path = self.paths.repo_bare_path(name);
        if tokio::fs::try_exists(&bare_path).await? {
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
        if !tokio::fs::try_exists(&bare_path).await? {
            anyhow::bail!("unknown repo {name} (missing {})", bare_path.display());
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

pub fn is_rust_repo(path: &Path) -> bool {
    path.join("Cargo.toml").is_file()
}

pub fn find_repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            Ok(PathBuf::from(text.trim()))
        }
        _ => Ok(cwd.to_path_buf()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
