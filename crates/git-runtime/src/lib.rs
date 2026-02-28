use anyhow::Context;
use std::path::Path;
use std::time::Duration;

pub const DEFAULT_MAX_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_WAIT_SECONDS: u64 = 30;
pub const MAX_WAIT_SECONDS: u64 = 10 * 60;
pub const POLL_INTERVAL_MS: u64 = 50;
pub const MAX_STDERR_BYTES: u64 = 32 * 1024;

pub const DEFAULT_ISOLATED_PATCH_MAX_BYTES: u64 = 2 * 1024 * 1024;
pub const DEFAULT_ISOLATED_PATCH_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotLimits {
    pub max_bytes: u64,
    pub wait_seconds: u64,
}

pub fn normalize_limits(max_bytes: Option<u64>, wait_seconds: Option<u64>) -> SnapshotLimits {
    SnapshotLimits {
        max_bytes: max_bytes.unwrap_or(DEFAULT_MAX_BYTES).min(MAX_MAX_BYTES),
        wait_seconds: wait_seconds
            .unwrap_or(DEFAULT_WAIT_SECONDS)
            .min(MAX_WAIT_SECONDS),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    Diff,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRecipe {
    pub argv: Vec<String>,
    pub artifact_type: &'static str,
    pub summary_clean: &'static str,
    pub summary_dirty: &'static str,
}

pub fn recipe(kind: SnapshotKind) -> SnapshotRecipe {
    match kind {
        SnapshotKind::Diff => SnapshotRecipe {
            argv: vec![
                "git".to_string(),
                "--no-pager".to_string(),
                "diff".to_string(),
                "--no-ext-diff".to_string(),
                "--no-textconv".to_string(),
                "--no-color".to_string(),
            ],
            artifact_type: "diff",
            summary_clean: "git diff (clean)",
            summary_dirty: "git diff",
        },
        SnapshotKind::Patch => SnapshotRecipe {
            argv: vec![
                "git".to_string(),
                "--no-pager".to_string(),
                "diff".to_string(),
                "--no-ext-diff".to_string(),
                "--no-textconv".to_string(),
                "--no-color".to_string(),
                "--binary".to_string(),
                "--patch".to_string(),
            ],
            artifact_type: "patch",
            summary_clean: "git patch (clean)",
            summary_dirty: "git patch",
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchCaptureConfig {
    pub max_patch_bytes: usize,
    pub timeout: Duration,
}

impl PatchCaptureConfig {
    pub fn new(max_patch_bytes: usize, timeout: Duration) -> Self {
        Self {
            max_patch_bytes,
            timeout,
        }
    }
}

impl Default for PatchCaptureConfig {
    fn default() -> Self {
        Self {
            max_patch_bytes: DEFAULT_ISOLATED_PATCH_MAX_BYTES as usize,
            timeout: Duration::from_millis(DEFAULT_ISOLATED_PATCH_TIMEOUT_MS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPatch {
    pub text: String,
    pub truncated: bool,
}

pub async fn capture_workspace_patch(
    cwd: &str,
    config: PatchCaptureConfig,
) -> anyhow::Result<Option<CapturedPatch>> {
    let max_patch_bytes = config.max_patch_bytes.max(1);

    // Best-effort: include untracked files in generated patch without staging content.
    let _ = tokio::time::timeout(
        config.timeout,
        tokio::process::Command::new("git")
            .args(["add", "--intent-to-add", "--", "."])
            .current_dir(cwd)
            .output(),
    )
    .await;

    let output = tokio::time::timeout(
        config.timeout,
        tokio::process::Command::new("git")
            .args([
                "--no-pager",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--binary",
                "--patch",
            ])
            .current_dir(cwd)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("git diff timed out after {}ms", config.timeout.as_millis()))?
    .with_context(|| format!("spawn git diff in {cwd}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git diff --binary --patch failed in {} (exit {:?}): {}",
            cwd,
            output.status.code(),
            stderr
        );
    }

    if output.stdout.is_empty() {
        return Ok(None);
    }

    let mut bytes = output.stdout;
    let truncated = bytes.len() > max_patch_bytes;
    if truncated {
        bytes.truncate(max_patch_bytes);
    }
    let mut text = String::from_utf8_lossy(&bytes).to_string();
    if truncated {
        text.push_str("\n# <...truncated...>\n");
    }

    Ok(Some(CapturedPatch { text, truncated }))
}

pub async fn run_git_apply_with_patch_stdin(
    cwd: &str,
    args: &[&str],
    patch_text: &str,
) -> anyhow::Result<()> {
    let mut child = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn git {} in {}", args.join(" "), cwd))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(patch_text.as_bytes())
            .await
            .with_context(|| format!("write patch stdin for git {} in {}", args.join(" "), cwd))?;
    }

    let output = child
        .wait_with_output()
        .await
        .with_context(|| format!("wait git {} in {}", args.join(" "), cwd))?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "git {} failed in {} (exit {:?}): stdout={}, stderr={}",
        args.join(" "),
        cwd,
        output.status.code(),
        stdout,
        stderr
    );
}

pub async fn create_detached_worktree(
    source_repo_cwd: &str,
    worktree_path: &str,
    reference: Option<&str>,
) -> anyhow::Result<()> {
    let reference = reference
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("HEAD");

    let output = tokio::process::Command::new("git")
        .args(["worktree", "add", "--detach", worktree_path, reference])
        .current_dir(source_repo_cwd)
        .output()
        .await
        .with_context(|| format!("spawn git worktree add in {}", source_repo_cwd))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    anyhow::bail!(
        "git worktree add --detach failed in {} (exit {:?}): stdout={}, stderr={}",
        source_repo_cwd,
        output.status.code(),
        stdout,
        stderr
    );
}

fn parse_dot_git_marker(marker: &str) -> Option<String> {
    let line = marker.lines().next()?.trim();
    let value = line.strip_prefix("gitdir:")?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

fn looks_like_linked_worktree_gitdir(gitdir: &str) -> bool {
    gitdir.replace('\\', "/").contains("/worktrees/")
}

pub async fn remove_detached_worktree_and_prune(worktree_path: &str) -> anyhow::Result<bool> {
    let worktree_path = worktree_path.trim();
    if worktree_path.is_empty() {
        return Ok(false);
    }

    let git_marker = Path::new(worktree_path).join(".git");
    let marker_meta = match tokio::fs::metadata(&git_marker).await {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("stat {}", git_marker.display())),
    };
    if !marker_meta.is_file() {
        return Ok(false);
    }

    let marker_text = tokio::fs::read_to_string(&git_marker)
        .await
        .with_context(|| format!("read {}", git_marker.display()))?;
    let Some(gitdir) = parse_dot_git_marker(&marker_text) else {
        return Ok(false);
    };
    if !looks_like_linked_worktree_gitdir(&gitdir) {
        return Ok(false);
    }

    let common_dir_output = tokio::process::Command::new("git")
        .args([
            "-C",
            worktree_path,
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ])
        .output()
        .await
        .with_context(|| format!("spawn git rev-parse in {}", worktree_path))?;
    if !common_dir_output.status.success() {
        let stdout = String::from_utf8_lossy(&common_dir_output.stdout)
            .trim()
            .to_string();
        let stderr = String::from_utf8_lossy(&common_dir_output.stderr)
            .trim()
            .to_string();
        anyhow::bail!(
            "git rev-parse --git-common-dir failed in {} (exit {:?}): stdout={}, stderr={}",
            worktree_path,
            common_dir_output.status.code(),
            stdout,
            stderr
        );
    }
    let common_dir = String::from_utf8_lossy(&common_dir_output.stdout)
        .trim()
        .to_string();
    if common_dir.is_empty() {
        anyhow::bail!("git rev-parse --git-common-dir returned empty output");
    }

    let remove_output = tokio::process::Command::new("git")
        .arg(format!("--git-dir={common_dir}"))
        .args(["worktree", "remove", "--force", worktree_path])
        .output()
        .await
        .with_context(|| format!("spawn git worktree remove for {}", worktree_path))?;
    if !remove_output.status.success() {
        let stdout = String::from_utf8_lossy(&remove_output.stdout)
            .trim()
            .to_string();
        let stderr = String::from_utf8_lossy(&remove_output.stderr)
            .trim()
            .to_string();
        anyhow::bail!(
            "git worktree remove --force failed for {} (exit {:?}): stdout={}, stderr={}",
            worktree_path,
            remove_output.status.code(),
            stdout,
            stderr
        );
    }

    let prune_output = tokio::process::Command::new("git")
        .arg(format!("--git-dir={common_dir}"))
        .args(["worktree", "prune"])
        .output()
        .await
        .with_context(|| format!("spawn git worktree prune for {}", common_dir))?;
    if !prune_output.status.success() {
        let stdout = String::from_utf8_lossy(&prune_output.stdout)
            .trim()
            .to_string();
        let stderr = String::from_utf8_lossy(&prune_output.stderr)
            .trim()
            .to_string();
        anyhow::bail!(
            "git worktree prune failed for {} (exit {:?}): stdout={}, stderr={}",
            common_dir,
            prune_output.status.code(),
            stdout,
            stderr
        );
    }

    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoApplyFailureStage {
    Precondition,
    CapturePatch,
    CheckPatch,
    ApplyPatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoApplyFailureReason {
    TurnNotCompleted,
    MissingTargetWorkspace,
    NoPatchToApply,
    CapturePatchFailed,
    PatchTruncated,
    CheckPatchFailed,
    ApplyPatchFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoApplyFailure {
    pub stage: AutoApplyFailureStage,
    pub reason: AutoApplyFailureReason,
    pub hint: &'static str,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoApplyWorkspacePatchResult {
    pub attempted: bool,
    pub applied: bool,
    pub check_argv: Option<Vec<String>>,
    pub apply_argv: Option<Vec<String>>,
    pub failure: Option<AutoApplyFailure>,
}

fn auto_apply_failure(
    stage: AutoApplyFailureStage,
    reason: AutoApplyFailureReason,
    hint: &'static str,
    error: impl Into<String>,
) -> AutoApplyWorkspacePatchResult {
    AutoApplyWorkspacePatchResult {
        attempted: false,
        applied: false,
        check_argv: None,
        apply_argv: None,
        failure: Some(AutoApplyFailure {
            stage,
            reason,
            hint,
            error: error.into(),
        }),
    }
}

pub async fn auto_apply_workspace_patch(
    workspace_cwd: &str,
    target_workspace_cwd: Option<&str>,
    turn_completed: bool,
    capture_config: PatchCaptureConfig,
) -> AutoApplyWorkspacePatchResult {
    if !turn_completed {
        return auto_apply_failure(
            AutoApplyFailureStage::Precondition,
            AutoApplyFailureReason::TurnNotCompleted,
            "retry auto-apply after the child turn reaches completed status",
            "turn status is not completed",
        );
    }

    let Some(target_workspace_cwd) = target_workspace_cwd
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return auto_apply_failure(
            AutoApplyFailureStage::Precondition,
            AutoApplyFailureReason::MissingTargetWorkspace,
            "ensure parent workspace cwd is available for auto-apply",
            "target workspace cwd is missing",
        );
    };

    let patch = match capture_workspace_patch(workspace_cwd, capture_config).await {
        Ok(Some(patch)) => patch,
        Ok(None) => {
            return auto_apply_failure(
                AutoApplyFailureStage::CapturePatch,
                AutoApplyFailureReason::NoPatchToApply,
                "collect patch manually from isolated workspace and apply it in parent workspace",
                "isolated workspace has no patch to apply",
            );
        }
        Err(err) => {
            return auto_apply_failure(
                AutoApplyFailureStage::CapturePatch,
                AutoApplyFailureReason::CapturePatchFailed,
                "collect patch manually from isolated workspace and apply it in parent workspace",
                format!("capture isolated patch for auto-apply failed: {err}"),
            );
        }
    };

    if patch.truncated {
        return auto_apply_failure(
            AutoApplyFailureStage::CapturePatch,
            AutoApplyFailureReason::PatchTruncated,
            "patch is truncated; use the patch artifact or manual git diff/apply workflow",
            "isolated patch is truncated; refusing to auto-apply",
        );
    }

    let check_argv = vec![
        "git".to_string(),
        "-C".to_string(),
        target_workspace_cwd.to_string(),
        "apply".to_string(),
        "--check".to_string(),
        "--whitespace=nowarn".to_string(),
        "-".to_string(),
    ];
    let apply_argv = vec![
        "git".to_string(),
        "-C".to_string(),
        target_workspace_cwd.to_string(),
        "apply".to_string(),
        "--whitespace=nowarn".to_string(),
        "-".to_string(),
    ];

    if let Err(err) = run_git_apply_with_patch_stdin(
        target_workspace_cwd,
        &["apply", "--check", "--whitespace=nowarn", "-"],
        &patch.text,
    )
    .await
    {
        return AutoApplyWorkspacePatchResult {
            attempted: true,
            applied: false,
            check_argv: Some(check_argv),
            apply_argv: Some(apply_argv),
            failure: Some(AutoApplyFailure {
                stage: AutoApplyFailureStage::CheckPatch,
                reason: AutoApplyFailureReason::CheckPatchFailed,
                hint: "resolve apply-check conflicts in parent workspace, then apply patch manually",
                error: format!("git apply --check failed: {err}"),
            }),
        };
    }

    if let Err(err) = run_git_apply_with_patch_stdin(
        target_workspace_cwd,
        &["apply", "--whitespace=nowarn", "-"],
        &patch.text,
    )
    .await
    {
        return AutoApplyWorkspacePatchResult {
            attempted: true,
            applied: false,
            check_argv: Some(check_argv),
            apply_argv: Some(apply_argv),
            failure: Some(AutoApplyFailure {
                stage: AutoApplyFailureStage::ApplyPatch,
                reason: AutoApplyFailureReason::ApplyPatchFailed,
                hint: "inspect git apply output and apply patch manually if needed",
                error: format!("git apply failed: {err}"),
            }),
        };
    }

    AutoApplyWorkspacePatchResult {
        attempted: true,
        applied: true,
        check_argv: Some(check_argv),
        apply_argv: Some(apply_argv),
        failure: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn normalize_limits_applies_defaults() {
        let limits = normalize_limits(None, None);
        assert_eq!(
            limits,
            SnapshotLimits {
                max_bytes: DEFAULT_MAX_BYTES,
                wait_seconds: DEFAULT_WAIT_SECONDS,
            }
        );
    }

    #[test]
    fn normalize_limits_clamps_large_values() {
        let limits = normalize_limits(Some(u64::MAX), Some(u64::MAX));
        assert_eq!(
            limits,
            SnapshotLimits {
                max_bytes: MAX_MAX_BYTES,
                wait_seconds: MAX_WAIT_SECONDS,
            }
        );
    }

    #[test]
    fn recipe_diff_and_patch_have_expected_identity() {
        let diff = recipe(SnapshotKind::Diff);
        assert_eq!(diff.artifact_type, "diff");
        assert_eq!(diff.summary_dirty, "git diff");
        assert!(diff.argv.iter().all(|arg| arg != "--binary"));

        let patch = recipe(SnapshotKind::Patch);
        assert_eq!(patch.artifact_type, "patch");
        assert_eq!(patch.summary_dirty, "git patch");
        assert!(patch.argv.iter().any(|arg| arg == "--binary"));
        assert!(patch.argv.iter().any(|arg| arg == "--patch"));
    }

    async fn run_git(cwd: &Path, args: &[&str]) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .await
            .with_context(|| format!("spawn git {} in {}", args.join(" "), cwd.display()))?;
        if output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git {} failed in {} (exit {:?}): stdout={}, stderr={}",
            args.join(" "),
            cwd.display(),
            output.status.code(),
            stdout,
            stderr
        );
    }

    async fn init_repo(repo_dir: &Path) -> anyhow::Result<()> {
        run_git(repo_dir, &["init"]).await?;
        run_git(repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(repo_dir, &["config", "user.name", "Test User"]).await?;
        tokio::fs::write(repo_dir.join("hello.txt"), "hello\n").await?;
        run_git(repo_dir, &["add", "hello.txt"]).await?;
        run_git(repo_dir, &["commit", "-m", "init"]).await?;
        Ok(())
    }

    #[tokio::test]
    async fn capture_workspace_patch_returns_none_when_clean() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        init_repo(&repo_dir).await?;

        let patch = capture_workspace_patch(
            &repo_dir.display().to_string(),
            PatchCaptureConfig::default(),
        )
        .await?;
        assert!(patch.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn capture_workspace_patch_includes_untracked_file_diff() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        init_repo(&repo_dir).await?;

        tokio::fs::write(repo_dir.join("new_file.txt"), "new content\n").await?;

        let patch = capture_workspace_patch(
            &repo_dir.display().to_string(),
            PatchCaptureConfig::default(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("expected patch output"))?;
        assert!(!patch.truncated);
        assert!(patch.text.contains("new_file.txt"));
        Ok(())
    }

    #[tokio::test]
    async fn run_git_apply_with_patch_stdin_applies_patch() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;

        let source_repo = tmp.path().join("source");
        let target_repo = tmp.path().join("target");
        tokio::fs::create_dir_all(&source_repo).await?;
        init_repo(&source_repo).await?;

        let source_repo_text = source_repo.display().to_string();
        let target_repo_text = target_repo.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                source_repo_text.as_str(),
                target_repo_text.as_str(),
            ],
        )
        .await?;

        tokio::fs::write(source_repo.join("hello.txt"), "hello\nworld\n").await?;
        let patch = capture_workspace_patch(
            &source_repo.display().to_string(),
            PatchCaptureConfig::default(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("expected patch output"))?;

        run_git_apply_with_patch_stdin(
            &target_repo.display().to_string(),
            &["apply", "--check", "--whitespace=nowarn", "-"],
            &patch.text,
        )
        .await?;
        run_git_apply_with_patch_stdin(
            &target_repo.display().to_string(),
            &["apply", "--whitespace=nowarn", "-"],
            &patch.text,
        )
        .await?;

        let applied = tokio::fs::read_to_string(target_repo.join("hello.txt")).await?;
        assert_eq!(applied, "hello\nworld\n");
        Ok(())
    }

    #[tokio::test]
    async fn auto_apply_workspace_patch_succeeds_on_clean_target() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;

        let parent_repo = tmp.path().join("parent");
        let isolated_repo = tmp.path().join("isolated");
        tokio::fs::create_dir_all(&parent_repo).await?;
        init_repo(&parent_repo).await?;

        let parent_repo_text = parent_repo.display().to_string();
        let isolated_repo_text = isolated_repo.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_text.as_str(),
                isolated_repo_text.as_str(),
            ],
        )
        .await?;
        tokio::fs::write(isolated_repo.join("hello.txt"), "hello\nchild\n").await?;

        let result = auto_apply_workspace_patch(
            &isolated_repo.display().to_string(),
            Some(parent_repo.display().to_string().as_str()),
            true,
            PatchCaptureConfig::default(),
        )
        .await;

        assert!(result.attempted);
        assert!(result.applied);
        assert!(result.failure.is_none());

        let applied = tokio::fs::read_to_string(parent_repo.join("hello.txt")).await?;
        assert_eq!(applied, "hello\nchild\n");
        Ok(())
    }

    #[tokio::test]
    async fn auto_apply_workspace_patch_reports_precondition_failure() -> anyhow::Result<()> {
        let result = auto_apply_workspace_patch(
            "/tmp/not-used",
            Some("/tmp/target"),
            false,
            PatchCaptureConfig::default(),
        )
        .await;
        assert!(!result.attempted);
        assert!(!result.applied);
        let failure = result
            .failure
            .ok_or_else(|| anyhow::anyhow!("expected failure"))?;
        assert_eq!(failure.stage, AutoApplyFailureStage::Precondition);
        assert_eq!(failure.reason, AutoApplyFailureReason::TurnNotCompleted);
        Ok(())
    }

    #[tokio::test]
    async fn auto_apply_workspace_patch_reports_check_failure_on_conflict() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;

        let parent_repo = tmp.path().join("parent");
        let isolated_repo = tmp.path().join("isolated");
        tokio::fs::create_dir_all(&parent_repo).await?;
        init_repo(&parent_repo).await?;

        let parent_repo_text = parent_repo.display().to_string();
        let isolated_repo_text = isolated_repo.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_text.as_str(),
                isolated_repo_text.as_str(),
            ],
        )
        .await?;
        tokio::fs::write(isolated_repo.join("hello.txt"), "hello\nchild-change\n").await?;
        tokio::fs::write(parent_repo.join("hello.txt"), "hello\nparent-change\n").await?;

        let result = auto_apply_workspace_patch(
            &isolated_repo.display().to_string(),
            Some(parent_repo.display().to_string().as_str()),
            true,
            PatchCaptureConfig::default(),
        )
        .await;
        assert!(result.attempted);
        assert!(!result.applied);
        let failure = result
            .failure
            .ok_or_else(|| anyhow::anyhow!("expected failure"))?;
        assert_eq!(failure.stage, AutoApplyFailureStage::CheckPatch);
        assert_eq!(failure.reason, AutoApplyFailureReason::CheckPatchFailed);
        assert!(failure.error.contains("git apply --check failed"));

        let parent_text = tokio::fs::read_to_string(parent_repo.join("hello.txt")).await?;
        assert_eq!(parent_text, "hello\nparent-change\n");
        Ok(())
    }

    #[tokio::test]
    async fn create_detached_worktree_succeeds_for_git_repo() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        init_repo(&repo_dir).await?;

        let worktree_dir = tmp.path().join("wt-1");
        create_detached_worktree(
            &repo_dir.display().to_string(),
            &worktree_dir.display().to_string(),
            None,
        )
        .await?;

        assert!(worktree_dir.join("hello.txt").exists());
        let status = tokio::process::Command::new("git")
            .args(["status", "--short", "--"])
            .current_dir(&worktree_dir)
            .output()
            .await?;
        assert!(status.status.success());
        Ok(())
    }

    #[tokio::test]
    async fn create_detached_worktree_fails_for_non_git_directory() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let non_repo_dir = tmp.path().join("non-repo");
        tokio::fs::create_dir_all(&non_repo_dir).await?;
        let worktree_dir = tmp.path().join("wt-fail");

        let err = create_detached_worktree(
            &non_repo_dir.display().to_string(),
            &worktree_dir.display().to_string(),
            None,
        )
        .await
        .expect_err("expected non-git worktree creation to fail");
        let err = err.to_string();
        assert!(err.contains("git worktree add --detach failed"));
        Ok(())
    }

    #[tokio::test]
    async fn remove_detached_worktree_and_prune_succeeds_for_linked_worktree() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        init_repo(&repo_dir).await?;

        let worktree_dir = tmp.path().join("wt-remove");
        create_detached_worktree(
            &repo_dir.display().to_string(),
            &worktree_dir.display().to_string(),
            None,
        )
        .await?;
        assert!(worktree_dir.exists());

        let removed =
            remove_detached_worktree_and_prune(worktree_dir.display().to_string().as_str()).await?;
        assert!(removed);
        assert!(!worktree_dir.exists());

        let output = tokio::process::Command::new("git")
            .args(["worktree", "list"])
            .current_dir(&repo_dir)
            .output()
            .await?;
        assert!(output.status.success());
        let listed = String::from_utf8_lossy(&output.stdout);
        assert!(!listed.contains(worktree_dir.display().to_string().as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn remove_detached_worktree_and_prune_returns_false_for_primary_worktree()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;
        init_repo(&repo_dir).await?;

        let removed =
            remove_detached_worktree_and_prune(repo_dir.display().to_string().as_str()).await?;
        assert!(!removed);
        assert!(repo_dir.exists());
        Ok(())
    }

    #[tokio::test]
    async fn remove_detached_worktree_and_prune_returns_false_for_non_git_directory()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("plain");
        tokio::fs::create_dir_all(&dir).await?;

        let removed =
            remove_detached_worktree_and_prune(dir.display().to_string().as_str()).await?;
        assert!(!removed);
        assert!(dir.exists());
        Ok(())
    }
}
