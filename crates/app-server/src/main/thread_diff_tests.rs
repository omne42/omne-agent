#[cfg(test)]
mod thread_diff_tests {
    use super::*;

    fn build_test_server(pm_root: PathBuf) -> Server {
        let (out_tx, _out_rx) = mpsc::unbounded_channel::<String>();
        Server {
            cwd: pm_root.clone(),
            out_tx,
            thread_store: ThreadStore::new(PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
        }
    }

    async fn run_git(repo_dir: &Path, args: &[&str]) -> anyhow::Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_dir)
            .output()
            .await
            .with_context(|| format!("spawn git {}", args.join(" ")))?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "git {} failed (exit {:?}): stdout={}, stderr={}",
                args.join(" "),
                output.status.code(),
                stdout,
                stderr
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn thread_diff_writes_diff_artifact() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        run_git(&repo_dir, &["init"]).await?;
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(&repo_dir, &["config", "user.name", "Test User"]).await?;

        let file_path = repo_dir.join("hello.txt");
        tokio::fs::write(&file_path, "hello\n").await?;
        run_git(&repo_dir, &["add", "hello.txt"]).await?;
        run_git(&repo_dir, &["commit", "-m", "init"]).await?;

        tokio::fs::write(&file_path, "hello\nworld\n").await?;

        let server = build_test_server(tmp.path().join(".code_pm"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let diff = handle_thread_diff(
            &server,
            ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                max_bytes: None,
                wait_seconds: Some(10),
            },
        )
        .await?;

        let artifact_id: ArtifactId = serde_json::from_value(diff["artifact"]["artifact_id"].clone())
            .context("parse artifact_id")?;

        let read = handle_artifact_read(
            &server,
            ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                max_bytes: None,
            },
        )
        .await?;

        let meta: ArtifactMetadata = serde_json::from_value(read["metadata"].clone())?;
        assert_eq!(meta.artifact_type, "diff");
        assert_eq!(
            meta.preview.as_ref().map(|p| p.kind),
            Some(pm_protocol::ArtifactPreviewKind::DiffUnified)
        );

        let text = read["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing text"))?;
        assert!(text.contains("diff --git a/hello.txt b/hello.txt"));
        assert!(text.contains("+world"));
        Ok(())
    }
}
