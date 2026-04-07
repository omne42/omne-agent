use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::events::{EventBus, RunEvent};
use crate::hooks::{HookRunner, NoopHookRunner};
use crate::storage::FsStorage;

struct PanicArchitect;

#[async_trait]
impl Architect for PanicArchitect {
    async fn split(&self, _session: &Session) -> anyhow::Result<Vec<TaskSpec>> {
        anyhow::bail!("architect should not be called in this test")
    }
}

struct DelayedCoder;

#[async_trait]
impl Coder for DelayedCoder {
    async fn execute(
        &self,
        _repo: &Repository,
        session: &Session,
        _session_paths: &SessionPaths,
        request: &RunRequest,
        task: &TaskSpec,
    ) -> anyhow::Result<PullRequest> {
        assert_eq!(request.prompt, "x");
        let delay = match task.id.as_str() {
            "a" => Duration::from_millis(50),
            "b" => Duration::from_millis(10),
            _ => Duration::from_millis(0),
        };
        tokio::time::sleep(delay).await;

        Ok(PullRequest {
            id: task.id.clone(),
            head_branch: format!(
                "ai/{}/{}/{}",
                session.pr_name.as_str(),
                session.id,
                task.id.as_str()
            ),
            base_branch: session.base_branch.clone(),
            status: PullRequestStatus::Ready,
            checks: Default::default(),
            head_commit: None,
        })
    }
}

struct NoopMerger;

#[async_trait]
impl Merger for NoopMerger {
    async fn merge(
        &self,
        _repo: &Repository,
        session: &Session,
        _session_paths: &SessionPaths,
        _prs: &[PullRequest],
    ) -> anyhow::Result<MergeResult> {
        Ok(MergeResult {
            merged: false,
            base_branch: session.base_branch.clone(),
            merge_commit: None,
            merged_prs: Vec::new(),
            checks: Default::default(),
            error: None,
            error_log_path: None,
        })
    }
}

struct PanicMerger;

#[async_trait]
impl Merger for PanicMerger {
    async fn merge(
        &self,
        _repo: &Repository,
        _session: &Session,
        _session_paths: &SessionPaths,
        _prs: &[PullRequest],
    ) -> anyhow::Result<MergeResult> {
        panic!("merger should not be called")
    }
}

struct FailingHookRunner;

#[async_trait]
impl HookRunner for FailingHookRunner {
    async fn run(
        &self,
        _hook: &HookSpec,
        _omne_paths: &PmPaths,
        _session_paths: &SessionPaths,
        _result: &RunResult,
    ) -> anyhow::Result<()> {
        anyhow::bail!("hook boom")
    }
}

struct ResultFileCheckingHookRunner;

#[async_trait]
impl HookRunner for ResultFileCheckingHookRunner {
    async fn run(
        &self,
        _hook: &HookSpec,
        _omne_paths: &PmPaths,
        session_paths: &SessionPaths,
        _result: &RunResult,
    ) -> anyhow::Result<()> {
        let result_path = session_paths.root().join("result.json");
        anyhow::ensure!(
            tokio::fs::try_exists(&result_path).await?,
            "hook should receive a prewritten result artifact: {}",
            result_path.display()
        );
        Ok(())
    }
}

#[tokio::test]
async fn rule_based_architect_parses_checklist_tasks() -> anyhow::Result<()> {
    let architect = RuleBasedArchitect::new(8);
    let session = Session {
        id: SessionId::new(),
        repo: crate::domain::RepositoryName::sanitize("repo"),
        pr_name: crate::domain::PrName::sanitize("demo"),
        prompt: r#"
Some intro.

- [ ] implement foo
- [x] fix bar
"#
        .to_string(),
        base_branch: "main".to_string(),
        created_at: OffsetDateTime::now_utc(),
    };

    let tasks = architect.split(&session).await?;
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id.as_str(), "t1");
    assert_eq!(tasks[0].title, "implement foo");
    assert_eq!(tasks[1].id.as_str(), "t2");
    assert_eq!(tasks[1].title, "fix bar");
    Ok(())
}

#[tokio::test]
async fn rule_based_architect_falls_back_to_single_task() -> anyhow::Result<()> {
    let architect = RuleBasedArchitect::new(8);
    let session = Session {
        id: SessionId::new(),
        repo: crate::domain::RepositoryName::sanitize("repo"),
        pr_name: crate::domain::PrName::sanitize("demo"),
        prompt: "no list here".to_string(),
        base_branch: "main".to_string(),
        created_at: OffsetDateTime::now_utc(),
    };

    let tasks = architect.split(&session).await?;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id.as_str(), "main");
    assert_eq!(tasks[0].title, "Implement demo");
    Ok(())
}

#[tokio::test]
async fn concurrent_tasks_preserve_input_order() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(dir.path().join(".omne_data"));
    let storage = FsStorage::new(omne_paths.data_dir());

    let events = EventBus::new(256);

    let repo = Repository {
        name: crate::domain::RepositoryName::sanitize("repo"),
        bare_path: PathBuf::from("/nonexistent/bare.git"),
        lock_path: dir.path().join("repo.lock"),
    };

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: events.clone(),
        architect: Arc::new(PanicArchitect),
        coder: Arc::new(DelayedCoder),
        merger: Arc::new(NoopMerger),
    };

    let tasks = vec![
        TaskSpec {
            id: crate::domain::TaskId::sanitize("a"),
            title: "A".to_string(),
            description: None,
        },
        TaskSpec {
            id: crate::domain::TaskId::sanitize("b"),
            title: "B".to_string(),
            description: None,
        },
        TaskSpec {
            id: crate::domain::TaskId::sanitize("c"),
            title: "C".to_string(),
            description: None,
        },
    ];

    let result = orchestrator
        .run(
            &omne_paths,
            repo.clone(),
            RunRequest {
                pr_name: crate::domain::PrName::sanitize("test"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                tasks: Some(tasks.clone()),
                apply_patch: None,
                hook: None,
                max_concurrency: 8,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    let ids: Vec<String> = result
        .prs
        .iter()
        .map(|pr| pr.id.as_str().to_string())
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"]);

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn run_emits_basic_events() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(dir.path().join(".omne_data"));
    let storage = FsStorage::new(omne_paths.data_dir());

    let events = EventBus::new(256);
    let mut rx = events.subscribe();

    let repo = Repository {
        name: crate::domain::RepositoryName::sanitize("repo"),
        bare_path: PathBuf::from("/nonexistent/bare.git"),
        lock_path: dir.path().join("repo.lock"),
    };

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: events.clone(),
        architect: Arc::new(PanicArchitect),
        coder: Arc::new(DelayedCoder),
        merger: Arc::new(NoopMerger),
    };

    let tasks = vec![
        TaskSpec {
            id: crate::domain::TaskId::sanitize("a"),
            title: "A".to_string(),
            description: None,
        },
        TaskSpec {
            id: crate::domain::TaskId::sanitize("b"),
            title: "B".to_string(),
            description: None,
        },
    ];

    let omne_paths_run = omne_paths.clone();
    let repo_run = repo.clone();
    let request = RunRequest {
        pr_name: crate::domain::PrName::sanitize("test"),
        prompt: "x".to_string(),
        base_branch: "main".to_string(),
        tasks: Some(tasks.clone()),
        apply_patch: None,
        hook: None,
        max_concurrency: 8,
        cargo_test: false,
        auto_merge: true,
    };
    let run =
        tokio::spawn(async move { orchestrator.run(&omne_paths_run, repo_run, request).await });

    let mut saw_session = false;
    let mut saw_tasks_planned = false;
    let mut started = std::collections::HashSet::new();
    let mut finished = std::collections::HashSet::new();
    let mut saw_merge_started = false;
    let mut saw_merge_finished = false;

    while !saw_merge_finished {
        let event = match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Ok(event)) => event,
            Ok(Err(err)) => anyhow::bail!("event recv failed: {err}"),
            Err(_) => anyhow::bail!("timed out waiting for events"),
        };

        match event {
            RunEvent::SessionCreated { .. } => saw_session = true,
            RunEvent::TasksPlanned { tasks } => {
                saw_tasks_planned = true;
                assert_eq!(tasks.len(), 2);
            }
            RunEvent::TaskStarted { task } => {
                started.insert(task.id);
            }
            RunEvent::TaskFinished { pr } => {
                finished.insert(pr.id);
            }
            RunEvent::MergeStarted { .. } => saw_merge_started = true,
            RunEvent::MergeFinished { .. } => saw_merge_finished = true,
            RunEvent::HookStarted | RunEvent::HookFinished { .. } => {}
        }
    }

    let result = run.await??;
    assert!(saw_session);
    assert!(saw_tasks_planned);
    assert!(saw_merge_started);
    assert_eq!(started, finished);
    assert_eq!(finished.len(), 2);

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

struct PanicOnTaskCoder;

#[async_trait]
impl Coder for PanicOnTaskCoder {
    async fn execute(
        &self,
        _repo: &Repository,
        session: &Session,
        _session_paths: &SessionPaths,
        _request: &RunRequest,
        task: &TaskSpec,
    ) -> anyhow::Result<PullRequest> {
        if task.id.as_str() == "b" {
            panic!("boom");
        }
        Ok(PullRequest {
            id: task.id.clone(),
            head_branch: format!(
                "ai/{}/{}/{}",
                session.pr_name.as_str(),
                session.id,
                task.id.as_str()
            ),
            base_branch: session.base_branch.clone(),
            status: PullRequestStatus::Ready,
            checks: Default::default(),
            head_commit: None,
        })
    }
}

#[tokio::test]
async fn run_skips_merge_when_auto_merge_disabled() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(dir.path().join(".omne_data"));
    let storage = FsStorage::new(omne_paths.data_dir());

    let events = EventBus::default();

    let repo = Repository {
        name: crate::domain::RepositoryName::sanitize("repo"),
        bare_path: PathBuf::from("/nonexistent/bare.git"),
        lock_path: dir.path().join("repo.lock"),
    };

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: events.clone(),
        architect: Arc::new(PanicArchitect),
        coder: Arc::new(DelayedCoder),
        merger: Arc::new(PanicMerger),
    };

    let tasks = vec![
        TaskSpec {
            id: crate::domain::TaskId::sanitize("a"),
            title: "A".to_string(),
            description: None,
        },
        TaskSpec {
            id: crate::domain::TaskId::sanitize("b"),
            title: "B".to_string(),
            description: None,
        },
    ];

    let result = orchestrator
        .run(
            &omne_paths,
            repo.clone(),
            RunRequest {
                pr_name: crate::domain::PrName::sanitize("test"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                tasks: Some(tasks),
                apply_patch: None,
                hook: None,
                max_concurrency: 2,
                cargo_test: false,
                auto_merge: false,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 2);
    assert!(
        result
            .prs
            .iter()
            .all(|pr| pr.status == PullRequestStatus::Ready)
    );
    assert!(!result.merge.merged);
    assert!(result.merge.error.is_none());

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn concurrent_tasks_convert_panics_to_failed_prs() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(dir.path().join(".omne_data"));
    let storage = FsStorage::new(omne_paths.data_dir());

    let repo = Repository {
        name: crate::domain::RepositoryName::sanitize("repo"),
        bare_path: PathBuf::from("/nonexistent/bare.git"),
        lock_path: dir.path().join("repo.lock"),
    };

    let tasks = vec![
        TaskSpec {
            id: crate::domain::TaskId::sanitize("a"),
            title: "A".to_string(),
            description: None,
        },
        TaskSpec {
            id: crate::domain::TaskId::sanitize("b"),
            title: "B".to_string(),
            description: None,
        },
        TaskSpec {
            id: crate::domain::TaskId::sanitize("c"),
            title: "C".to_string(),
            description: None,
        },
    ];

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(NoopHookRunner),
        events: EventBus::default(),
        architect: Arc::new(PanicArchitect),
        coder: Arc::new(PanicOnTaskCoder),
        merger: Arc::new(NoopMerger),
    };

    let result = orchestrator
        .run(
            &omne_paths,
            repo.clone(),
            RunRequest {
                pr_name: crate::domain::PrName::sanitize("test"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                tasks: Some(tasks.clone()),
                apply_patch: None,
                hook: None,
                max_concurrency: 8,
                cargo_test: false,
                auto_merge: true,
            },
        )
        .await?;

    assert_eq!(result.prs.len(), 3);
    assert_eq!(result.prs[0].id.as_str(), "a");
    assert_eq!(result.prs[1].id.as_str(), "b");
    assert_eq!(result.prs[2].id.as_str(), "c");
    assert_eq!(result.prs[1].status, PullRequestStatus::Failed);

    let session_paths = SessionPaths::new(&repo.name, result.session.id);
    let _ = tokio::fs::remove_dir_all(session_paths.root()).await;

    Ok(())
}

#[tokio::test]
async fn run_does_not_persist_result_when_hook_fails() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(dir.path().join(".omne_data"));
    let storage = FsStorage::new(omne_paths.data_dir());

    let repo = Repository {
        name: crate::domain::RepositoryName::sanitize("repo"),
        bare_path: PathBuf::from("/nonexistent/bare.git"),
        lock_path: dir.path().join("repo.lock"),
    };

    let orchestrator = Orchestrator {
        storage: Arc::new(storage),
        hook_runner: Arc::new(FailingHookRunner),
        events: EventBus::default(),
        architect: Arc::new(PanicArchitect),
        coder: Arc::new(DelayedCoder),
        merger: Arc::new(NoopMerger),
    };

    let err = orchestrator
        .run(
            &omne_paths,
            repo.clone(),
            RunRequest {
                pr_name: crate::domain::PrName::sanitize("test"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                tasks: Some(vec![TaskSpec {
                    id: crate::domain::TaskId::sanitize("a"),
                    title: "A".to_string(),
                    description: None,
                }]),
                apply_patch: None,
                hook: Some(HookSpec::Webhook {
                    url: "https://example.invalid/hook".to_string(),
                }),
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: false,
            },
        )
        .await
        .expect_err("hook failure should abort run");
    assert!(err.to_string().contains("hook failed"));

    let sessions_dir = omne_paths.data_dir().join("sessions");
    let mut entries = tokio::fs::read_dir(&sessions_dir).await?;
    let session_dir = entries
        .next_entry()
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing stored session"))?
        .path();
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("missing session id"))?
        .parse::<SessionId>()?;

    assert!(
        !tokio::fs::try_exists(session_dir.join("result.json"))
            .await
            .unwrap_or(false),
        "storage should not persist result.json when hook fails"
    );
    assert!(
        !tokio::fs::try_exists(
            SessionPaths::new(&repo.name, session_id)
                .root()
                .join("result.json")
        )
        .await
        .unwrap_or(false),
        "tmp result.json should be cleaned up when hook fails"
    );

    Ok(())
}

#[tokio::test]
async fn run_prewrites_result_artifact_before_running_hook() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let omne_paths = PmPaths::new(dir.path().join(".omne_data"));
    let storage = FsStorage::new(omne_paths.data_dir());

    let repo = Repository {
        name: crate::domain::RepositoryName::sanitize("repo"),
        bare_path: PathBuf::from("/nonexistent/bare.git"),
        lock_path: dir.path().join("repo.lock"),
    };

    let orchestrator = Orchestrator {
        storage: Arc::new(storage.clone()),
        hook_runner: Arc::new(ResultFileCheckingHookRunner),
        events: EventBus::default(),
        architect: Arc::new(PanicArchitect),
        coder: Arc::new(DelayedCoder),
        merger: Arc::new(NoopMerger),
    };

    let result = orchestrator
        .run(
            &omne_paths,
            repo,
            RunRequest {
                pr_name: crate::domain::PrName::sanitize("test"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                tasks: Some(vec![TaskSpec {
                    id: crate::domain::TaskId::sanitize("a"),
                    title: "A".to_string(),
                    description: None,
                }]),
                apply_patch: None,
                hook: Some(HookSpec::Webhook {
                    url: "https://example.invalid/hook".to_string(),
                }),
                max_concurrency: 1,
                cargo_test: false,
                auto_merge: false,
            },
        )
        .await?;

    assert!(
        tokio::fs::try_exists(
            SessionPaths::new(&result.session.repo, result.session.id)
                .root()
                .join("result.json")
        )
        .await?
    );
    assert!(
        storage
            .get_json(&format!("sessions/{}/result", result.session.id))
            .await?
            .is_some(),
        "successful hook run should persist the final result"
    );

    Ok(())
}
