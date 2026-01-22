use std::path::Path;

use pm_core::{PmPaths, RepositoryName};
use pm_git::RepoManager;
use tokio::process::Command;

async fn run_git(repo: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .await?;
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

fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

async fn init_source_repo(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    run_git(dir, &["init", "-b", "main"]).await?;
    run_git(dir, &["config", "user.email", "test@example.com"]).await?;
    run_git(dir, &["config", "user.name", "Test"]).await?;
    run_git(dir, &["config", "commit.gpgsign", "false"]).await?;

    write_file(&dir.join("hello.txt"), "hello\n")?;
    run_git(dir, &["add", "hello.txt"]).await?;
    run_git(dir, &["commit", "-m", "chore(test): init"]).await?;
    Ok(())
}

#[tokio::test]
async fn smart_http_clone_and_push() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("pm_http=info")
        .with_test_writer()
        .try_init();

    let tmp = tempfile::tempdir()?;
    let source_repo = tmp.path().join("source");
    init_source_repo(&source_repo).await?;

    let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
    let repo_manager = RepoManager::new(pm_paths.clone());
    let repo_name = RepositoryName::sanitize("source");
    let source_repo_arg = source_repo.to_string_lossy();
    repo_manager
        .inject(&repo_name, source_repo_arg.as_ref())
        .await?;

    let app = pm_http::router(pm_paths.clone())?;
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            eprintln!("skipping smart http test: network not permitted");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };
    let addr = listener.local_addr()?;

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await
    });

    let remote = format!("http://{addr}/git/{}.git", repo_name.as_str());
    let work_dir = tmp.path().join("work");
    let work_dir_arg = work_dir.to_string_lossy();
    eprintln!("clone {remote} -> {}", work_dir.display());
    run_git(tmp.path(), &["clone", &remote, work_dir_arg.as_ref()]).await?;

    run_git(&work_dir, &["config", "user.email", "test@example.com"]).await?;
    run_git(&work_dir, &["config", "user.name", "Test"]).await?;
    run_git(&work_dir, &["config", "commit.gpgsign", "false"]).await?;
    run_git(&work_dir, &["checkout", "-b", "feature"]).await?;
    write_file(&work_dir.join("hello.txt"), "hello over http\n")?;
    run_git(&work_dir, &["add", "hello.txt"]).await?;
    run_git(&work_dir, &["commit", "-m", "chore(test): over http"]).await?;
    eprintln!("push feature -> {remote}");
    run_git(&work_dir, &["push", "origin", "feature"]).await?;

    let bare_repo = pm_paths.repo_bare_path(&repo_name);
    run_git(&bare_repo, &["show-ref", "--verify", "refs/heads/feature"]).await?;

    let _ = tx.send(());
    server.await??;

    Ok(())
}
