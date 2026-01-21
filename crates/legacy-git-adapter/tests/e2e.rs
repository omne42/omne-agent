use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use time::OffsetDateTime;

use pm_core::{
    CheckSummary, EventBus, FsStorage, Merger, NoopHookRunner, Orchestrator, PmPaths, PrName,
    PullRequest, PullRequestStatus, RepositoryName, Session, SessionId, SessionPaths, TaskId,
};
use pm_git::{GitCoder, GitMerger, RepoManager};

fn run_git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git").current_dir(repo).args(args).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {:?} failed (exit {:?}): {}",
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_clone(tmp_root: &Path, bare_repo: &Path, work_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(tmp_root)
        .arg("clone")
        .arg(bare_repo)
        .arg(work_dir)
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git clone failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

fn read_file(path: &Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn init_source_repo(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    run_git(dir, &["init", "-b", "main"])?;
    run_git(dir, &["config", "user.email", "test@example.com"])?;
    run_git(dir, &["config", "user.name", "Test"])?;
    run_git(dir, &["config", "commit.gpgsign", "false"])?;

    write_file(&dir.join("hello.txt"), "hello\n")?;
    run_git(dir, &["add", "hello.txt"])?;
    run_git(dir, &["commit", "-m", "chore(test): init"])?;
    Ok(())
}

fn init_rust_source_repo(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    run_git(dir, &["init", "-b", "main"])?;
    run_git(dir, &["config", "user.email", "test@example.com"])?;
    run_git(dir, &["config", "user.name", "Test"])?;
    run_git(dir, &["config", "commit.gpgsign", "false"])?;

    write_file(
        &dir.join("Cargo.toml"),
        r#"[package]
name = "source_rust"
version = "0.1.0"
edition = "2021"
"#,
    )?;
    write_file(
        &dir.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )?;

    let cargo_target_dir = dir.join(".cargo-target");
    let output = Command::new("cargo")
        .current_dir(dir)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", &cargo_target_dir)
        .args(["generate-lockfile"])
        .output()?;
    if !output.status.success() {
        anyhow::bail!(
            "cargo generate-lockfile failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(cargo_target_dir);

    run_git(dir, &["add", "Cargo.toml", "Cargo.lock", "src/lib.rs"])?;
    run_git(dir, &["commit", "-m", "chore(test): init rust"])?;
    Ok(())
}

fn init_rust_source_repo_without_lock(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    run_git(dir, &["init", "-b", "main"])?;
    run_git(dir, &["config", "user.email", "test@example.com"])?;
    run_git(dir, &["config", "user.name", "Test"])?;
    run_git(dir, &["config", "commit.gpgsign", "false"])?;

    write_file(
        &dir.join("Cargo.toml"),
        r#"[package]
name = "source_rust_nolock"
version = "0.1.0"
edition = "2021"
"#,
    )?;
    write_file(
        &dir.join("src/lib.rs"),
        r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    )?;

    run_git(dir, &["add", "Cargo.toml", "src/lib.rs"])?;
    run_git(dir, &["commit", "-m", "chore(test): init rust"])?;
    Ok(())
}

fn make_patch(dir: &Path) -> anyhow::Result<PathBuf> {
    write_file(&dir.join("hello.txt"), "hello world\n")?;
    let patch = run_git(dir, &["diff"])?;
    run_git(dir, &["checkout", "--", "hello.txt"])?;

    let patch_path = dir.join("change.patch");
    write_file(&patch_path, &patch)?;
    Ok(patch_path)
}

struct SingleTaskArchitect;

#[async_trait::async_trait]
impl pm_core::Architect for SingleTaskArchitect {
    async fn split(&self, session: &pm_core::Session) -> anyhow::Result<Vec<pm_core::TaskSpec>> {
        Ok(vec![pm_core::TaskSpec {
            id: pm_core::TaskId::sanitize("main"),
            title: format!("Implement {}", session.pr_name.as_str()),
            description: None,
        }])
    }
}

#[tokio::test]
async fn repo_inject_accepts_relative_source_paths() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let tmp = tempfile::tempdir_in(&cwd)?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");

    let source_repo_rel = source_repo
        .strip_prefix(&cwd)
        .map_err(|_| anyhow::anyhow!("tempdir not under current dir"))?
        .to_string_lossy()
        .to_string();
    let repo = repo_manager.inject(&repo_name, &source_repo_rel).await?;
    assert!(repo.bare_path.exists());
    Ok(())
}

#[tokio::test]
async fn repo_inject_updates_origin_when_repo_exists() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo_1 = tmp.path().join("source1");
    init_source_repo(&source_repo_1)?;

    let source_repo_2 = tmp.path().join("source2");
    init_source_repo(&source_repo_2)?;
    write_file(&source_repo_2.join("hello.txt"), "hello from source2\n")?;
    run_git(&source_repo_2, &["add", "hello.txt"])?;
    run_git(&source_repo_2, &["commit", "-m", "chore(test): update"])?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");

    let repo = repo_manager
        .inject(&repo_name, source_repo_1.to_string_lossy().as_ref())
        .await?;
    repo_manager
        .inject(&repo_name, source_repo_2.to_string_lossy().as_ref())
        .await?;

    let verify_dir = tmp.path().join("verify");
    git_clone(tmp.path(), &repo.bare_path, &verify_dir)?;
    let contents = read_file(&verify_dir.join("hello.txt"))?;
    assert_eq!(contents, "hello from source2\n");
    Ok(())
}

#[tokio::test]
async fn repo_inject_populates_heads_for_preexisting_bare_repo() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    repo_manager.ensure_layout().await?;

    let repo_name = RepositoryName::sanitize("source");
    let bare_repo = pm_paths.repo_bare_path(&repo_name);
    run_git(
        tmp.path(),
        &["init", "--bare", bare_repo.to_string_lossy().as_ref()],
    )?;

    repo_manager
        .inject(&repo_name, source_repo.to_string_lossy().as_ref())
        .await?;

    run_git(&bare_repo, &["show-ref", "--verify", "refs/heads/main"])?;

    Ok(())
}

#[tokio::test]
async fn inject_run_merge_updates_base_branch() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo)?;
    let patch_path = make_patch(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let storage = FsStorage::new(pm_paths.data_dir());

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "update hello".to_string(),
                base_branch: "main".to_string(),
                tasks: None,
                apply_patch: Some(patch_path),
                hook: None,
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert!(result.merge.merged);
    assert_eq!(result.merge.merged_prs.len(), 1);

    let verify_dir = tmp.path().join("verify");
    git_clone(tmp.path(), &repo.bare_path, &verify_dir)?;
    let contents = read_file(&verify_dir.join("hello.txt"))?;
    assert_eq!(contents, "hello world\n");

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn inject_run_merge_uses_concurrent_path() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo)?;
    let patch_path = make_patch(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let storage = FsStorage::new(pm_paths.data_dir());

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let tasks = vec![
        pm_core::TaskSpec {
            id: pm_core::TaskId::sanitize("a"),
            title: "Apply patch A".to_string(),
            description: None,
        },
        pm_core::TaskSpec {
            id: pm_core::TaskId::sanitize("b"),
            title: "Apply patch B".to_string(),
            description: None,
        },
    ];

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "update hello".to_string(),
                base_branch: "main".to_string(),
                tasks: Some(tasks),
                apply_patch: Some(patch_path),
                hook: None,
                max_concurrency: 2,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert!(result.merge.merged);
    assert_eq!(result.merge.merged_prs.len(), 2);

    let verify_dir = tmp.path().join("verify");
    git_clone(tmp.path(), &repo.bare_path, &verify_dir)?;
    let contents = read_file(&verify_dir.join("hello.txt"))?;
    assert_eq!(contents, "hello world\n");

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn inject_run_records_failed_pr_on_patch_error() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo)?;

    let patch_path = tmp.path().join("bad.patch");
    write_file(
        &patch_path,
        r#"diff --git a/missing.txt b/missing.txt
index 1111111..2222222 100644
--- a/missing.txt
+++ b/missing.txt
@@ -1 +1 @@
-old
+new
"#,
    )?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let storage = FsStorage::new(pm_paths.data_dir());

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "invalid patch".to_string(),
                base_branch: "main".to_string(),
                tasks: None,
                apply_patch: Some(patch_path),
                hook: None,
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 1);
    let pr = &result.prs[0];
    assert_eq!(pr.status, PullRequestStatus::Failed);
    assert!(
        pr.checks
            .steps
            .iter()
            .any(|step| step.name == "git_apply" && !step.ok)
    );
    assert!(
        pr.checks
            .steps
            .iter()
            .any(|step| step.name == "error" && !step.ok)
    );
    assert!(!result.merge.merged);
    assert!(result.merge.error.is_none());

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn merger_reports_conflict_as_error_result() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    fn create_branch(
        tmp_root: &Path,
        bare_repo: &Path,
        work_dir: &Path,
        branch: &str,
        contents: &str,
    ) -> anyhow::Result<()> {
        git_clone(tmp_root, bare_repo, work_dir)?;
        run_git(work_dir, &["config", "user.email", "test@example.com"])?;
        run_git(work_dir, &["config", "user.name", "Test"])?;
        run_git(work_dir, &["config", "commit.gpgsign", "false"])?;
        run_git(work_dir, &["checkout", "-B", branch, "origin/main"])?;
        write_file(&work_dir.join("hello.txt"), contents)?;
        run_git(work_dir, &["add", "hello.txt"])?;
        run_git(work_dir, &["commit", "-m", "chore(test): conflict"])?;
        run_git(work_dir, &["push", "origin", branch])?;
        Ok(())
    }

    let branch_a = "ai/test/conflict/a";
    let branch_b = "ai/test/conflict/b";
    create_branch(
        tmp.path(),
        &repo.bare_path,
        &tmp.path().join("work-a"),
        branch_a,
        "one\n",
    )?;
    create_branch(
        tmp.path(),
        &repo.bare_path,
        &tmp.path().join("work-b"),
        branch_b,
        "two\n",
    )?;

    let session = Session {
        id: SessionId::new(),
        repo: repo.name.clone(),
        pr_name: PrName::sanitize("test"),
        prompt: "merge".to_string(),
        base_branch: "main".to_string(),
        created_at: OffsetDateTime::now_utc(),
    };
    let session_paths = SessionPaths::new(&repo.name, session.id);

    let prs = vec![
        PullRequest {
            id: TaskId::sanitize("a"),
            head_branch: branch_a.to_string(),
            base_branch: "main".to_string(),
            status: PullRequestStatus::Ready,
            checks: CheckSummary::default(),
            head_commit: None,
        },
        PullRequest {
            id: TaskId::sanitize("b"),
            head_branch: branch_b.to_string(),
            base_branch: "main".to_string(),
            status: PullRequestStatus::Ready,
            checks: CheckSummary::default(),
            head_commit: None,
        },
    ];

    let merger = GitMerger::default();
    let merge = merger.merge(&repo, &session, &session_paths, &prs).await?;

    assert!(!merge.merged);
    assert!(merge.error.is_some());
    assert!(
        merge
            .checks
            .steps
            .iter()
            .any(|step| step.name == "git_merge_b" && !step.ok)
    );
    assert!(
        merge
            .checks
            .steps
            .iter()
            .any(|step| step.name == "merge_error" && !step.ok)
    );

    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn run_with_missing_bare_repo_records_clone_failure() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let storage = FsStorage::new(pm_paths.data_dir());

    let repo = pm_core::Repository {
        name: RepositoryName::sanitize("missing"),
        bare_path: tmp.path().join("does-not-exist.git"),
        lock_path: tmp.path().join("missing.lock"),
    };

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "clone should fail".to_string(),
                base_branch: "main".to_string(),
                tasks: None,
                apply_patch: None,
                hook: None,
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 1);
    let pr = &result.prs[0];
    assert_eq!(pr.status, PullRequestStatus::Failed);
    assert!(
        pr.checks
            .steps
            .iter()
            .any(|step| step.name == "git_clone" && !step.ok)
    );
    assert!(
        !pr.checks
            .steps
            .iter()
            .any(|step| step.name == "git_checkout_branch")
    );
    assert!(
        pr.checks
            .steps
            .iter()
            .any(|step| step.name == "error" && !step.ok)
    );

    assert!(!result.merge.merged);
    assert!(result.merge.error.is_none());

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn rust_repo_does_not_pollute_task_repo_with_target_dir() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source-rust");
    init_rust_source_repo(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source-rust");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let storage = FsStorage::new(pm_paths.data_dir());

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "no changes".to_string(),
                base_branch: "main".to_string(),
                tasks: None,
                apply_patch: None,
                hook: None,
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 1);
    let pr = &result.prs[0];
    assert_eq!(pr.status, PullRequestStatus::NoChanges);
    assert!(
        pr.checks
            .steps
            .iter()
            .any(|step| step.name == "cargo_check")
    );

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let task_paths = session_paths.task_paths(&TaskId::sanitize("main"));
    assert!(
        !task_paths.repo_dir().join("target").exists(),
        "cargo should not write target/ into the task repo"
    );

    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn rust_repo_without_lockfile_does_not_create_noise_pr() -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source-rust-nolock");
    init_rust_source_repo_without_lock(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source-rust-nolock");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let storage = FsStorage::new(pm_paths.data_dir());

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "no changes".to_string(),
                base_branch: "main".to_string(),
                tasks: None,
                apply_patch: None,
                hook: None,
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 1);
    let pr = &result.prs[0];
    assert_eq!(pr.status, PullRequestStatus::NoChanges);
    assert!(
        pr.checks
            .steps
            .iter()
            .any(|step| step.name == "cargo_check")
    );

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let task_paths = session_paths.task_paths(&TaskId::sanitize("main"));
    assert!(!task_paths.repo_dir().join("target").exists());
    assert!(!task_paths.repo_dir().join("Cargo.lock").exists());

    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn rust_repo_without_lockfile_does_not_create_noise_pr_with_cargo_test() -> anyhow::Result<()>
{
    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source-rust-nolock");
    init_rust_source_repo_without_lock(&source_repo)?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source-rust-nolock");
    let source_repo_arg = source_repo.to_string_lossy();
    let repo = repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let storage = FsStorage::new(pm_paths.data_dir());

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(SingleTaskArchitect),
        coder: Arc::new(GitCoder::default()),
        merger: Arc::new(GitMerger::default()),
    };

    let result = orchestrator
        .run(
            &pm_paths,
            repo.clone(),
            pm_core::RunRequest {
                pr_name: PrName::sanitize("test"),
                prompt: "no changes".to_string(),
                base_branch: "main".to_string(),
                tasks: None,
                apply_patch: None,
                hook: None,
                max_concurrency: 1,
                cargo_test: true,
                auto_merge: true,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 1);
    let pr = &result.prs[0];
    assert_eq!(pr.status, PullRequestStatus::NoChanges);
    assert!(pr.checks.steps.iter().any(|step| step.name == "cargo_test"));

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let task_paths = session_paths.task_paths(&TaskId::sanitize("main"));
    assert!(!task_paths.repo_dir().join("Cargo.lock").exists());

    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}
