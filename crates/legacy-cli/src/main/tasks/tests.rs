#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::Json;
    use axum::Router;
    use pm_core::Storage;
    use serde_json::Value;
    use time::OffsetDateTime;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn list_sessions_returns_sorted_unique_ids() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id1: SessionId = "00000000-0000-0000-0000-000000000001".parse()?;
        let id2: SessionId = "00000000-0000-0000-0000-000000000002".parse()?;

        storage
            .put_json(
                &format!("sessions/{id2}/tasks"),
                &serde_json::json!([{"id":"t1","title":"x"}]),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id1}/session"),
                &serde_json::json!({"ok": true}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id1}/merge"),
                &serde_json::json!({"merged": false}),
            )
            .await?;

        assert_eq!(list_sessions(&storage).await?, vec![id1, id2]);
        Ok(())
    }

    fn make_run_result(
        prs: Vec<pm_core::PullRequest>,
        merge: pm_core::MergeResult,
    ) -> pm_core::RunResult {
        let session_id: SessionId = "00000000-0000-0000-0000-000000000123"
            .parse()
            .expect("valid uuid");
        pm_core::RunResult {
            session: pm_core::Session {
                id: session_id,
                repo: pm_core::RepositoryName::sanitize("repo"),
                pr_name: pm_core::PrName::sanitize("demo"),
                prompt: "x".to_string(),
                base_branch: "main".to_string(),
                created_at: OffsetDateTime::from_unix_timestamp(0).unwrap(),
            },
            tasks: Vec::new(),
            prs,
            merge,
        }
    }

    #[test]
    fn strict_validation_allows_no_changes_sessions() {
        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::NoChanges,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: false,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: pm_core::CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        );
        assert!(validate_strict_run_result(&result).is_ok());
    }

    #[test]
    fn strict_validation_fails_on_task_failure() {
        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::Failed,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: false,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: pm_core::CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        );
        assert!(validate_strict_run_result(&result).is_err());
    }

    #[test]
    fn strict_validation_fails_on_merge_error() {
        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::Ready,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: false,
                base_branch: "main".to_string(),
                merge_commit: None,
                merged_prs: Vec::new(),
                checks: pm_core::CheckSummary::default(),
                error: Some("boom".to_string()),
                error_log_path: None,
            },
        );
        assert!(validate_strict_run_result(&result).is_err());
    }

    #[test]
    fn resolve_stream_events_mode_rejects_conflicts() {
        assert!(resolve_stream_events_mode(true, true).is_err());
    }

    #[test]
    fn cli_rejects_zero_max_concurrency() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--max-concurrency",
            "0",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_run_parses_cargo_test_flag() {
        let cli = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--cargo-test",
        ])
        .unwrap();

        let Command::Run(args) = cli.command else {
            panic!("expected run subcommand");
        };
        assert!(args.cargo_test);
    }

    #[test]
    fn resolve_run_repo_errors_when_missing_flags_outside_git_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = RepoRoot {
            root: tmp.path().to_path_buf(),
            is_git_repo: false,
        };

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some("x".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = resolve_run_repo(&repo_root, &args).unwrap_err();
        assert!(err.to_string().contains("missing --repo or --repo-src"));
    }

    #[test]
    fn resolve_run_repo_defaults_to_repo_root_inside_git_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = RepoRoot {
            root: tmp.path().to_path_buf(),
            is_git_repo: true,
        };

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some("x".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let ResolvedRunRepo::Inject { repo_name, source } =
            resolve_run_repo(&repo_root, &args).expect("resolve")
        else {
            panic!("expected inject");
        };
        assert_eq!(source, repo_root.root.to_string_lossy());
        let expected = sanitize_repo_name_input(
            repo_root
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .expect("tmp dir has file name"),
        );
        assert_eq!(repo_name, expected);
    }

    #[test]
    fn resolve_run_repo_strips_dot_git_suffix_from_repo_root_dir_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = RepoRoot {
            root: tmp.path().join("demo.git"),
            is_git_repo: true,
        };

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some("x".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let ResolvedRunRepo::Inject { repo_name, source } =
            resolve_run_repo(&repo_root, &args).expect("resolve")
        else {
            panic!("expected inject");
        };
        assert_eq!(source, repo_root.root.to_string_lossy());
        assert_eq!(repo_name.as_str(), "demo");
    }

    #[test]
    fn cli_rejects_auto_tasks_with_task_override() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--auto-tasks",
            "--task",
            "t1:foo",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn cli_rejects_auto_tasks_with_tasks_file_override() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--auto-tasks",
            "--tasks-file",
            "tasks.json",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn cli_rejects_empty_pr_name() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            " ",
            "--prompt",
            "x",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_base_branch() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            "repo",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
            "--base",
            " ",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_repo_name() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo",
            " ",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_repo_source() {
        let err = Cli::try_parse_from([
            "code-pm",
            "run",
            "--repo-src",
            " ",
            "--pr-name",
            "demo",
            "--prompt",
            "x",
        ])
        .err()
        .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_inject_source() {
        let err = Cli::try_parse_from(["code-pm", "repo", "inject", " "])
            .err()
            .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_rejects_empty_inject_name() {
        let err = Cli::try_parse_from(["code-pm", "repo", "inject", "src", "--name", " "])
            .err()
            .expect("cli parse must fail");
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn cli_repo_list_parses_json_and_verbose_flags() {
        let cli = Cli::try_parse_from(["code-pm", "repo", "list", "--json", "--verbose"]).unwrap();

        let Command::Repo { command } = cli.command else {
            panic!("expected repo subcommand");
        };
        let RepoCommand::List(args) = command else {
            panic!("expected repo list");
        };
        assert!(args.json);
        assert!(args.verbose);
    }

    #[test]
    fn cli_init_parses_json_flag() {
        let cli = Cli::try_parse_from(["code-pm", "init", "--json"]).unwrap();

        let Command::Init(args) = cli.command else {
            panic!("expected init subcommand");
        };
        assert!(args.json);
    }

    #[test]
    fn cli_repo_inject_parses_json_flag() {
        let cli = Cli::try_parse_from(["code-pm", "repo", "inject", "src", "--json"]).unwrap();

        let Command::Repo { command } = cli.command else {
            panic!("expected repo subcommand");
        };
        let RepoCommand::Inject { json, .. } = command else {
            panic!("expected repo inject");
        };
        assert!(json);
    }

    #[test]
    fn sanitize_repo_name_input_strips_dot_git_suffix() {
        assert_eq!(sanitize_repo_name_input("demo.git").as_str(), "demo");
        assert_eq!(sanitize_repo_name_input("demo.git/").as_str(), "demo");
        assert_eq!(sanitize_repo_name_input(" demo.git/ ").as_str(), "demo");
        assert_eq!(sanitize_repo_name_input("demo").as_str(), "demo");
    }

    #[tokio::test]
    async fn read_prompt_rejects_blank_prompt_arg() {
        let args = RunArgs {
            repo: Some("repo".to_string()),
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: Some(" \n\t".to_string()),
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = read_prompt(&args).await.unwrap_err();
        assert!(err.to_string().contains("--prompt must not be empty"));
    }

    #[tokio::test]
    async fn read_prompt_rejects_blank_prompt_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("prompt.txt");
        tokio::fs::write(&path, " \n\t").await?;

        let args = RunArgs {
            repo: Some("repo".to_string()),
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: None,
            prompt_file: Some(path),
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: None,
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = read_prompt(&args).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("--prompt-file content must not be empty")
        );
        Ok(())
    }

    #[tokio::test]
    async fn webhook_hook_runner_posts_expected_payload() -> anyhow::Result<()> {
        #[derive(Clone)]
        struct Capture {
            payload: Arc<Mutex<Option<Value>>>,
        }

        async fn handler(State(state): State<Capture>, Json(payload): Json<Value>) -> StatusCode {
            *state.payload.lock().await = Some(payload);
            StatusCode::NO_CONTENT
        }

        let captured = Arc::new(Mutex::new(None));
        let state = Capture {
            payload: Arc::clone(&captured),
        };

        let app = Router::new().route("/hook", post(handler)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));

        let result = make_run_result(
            vec![pm_core::PullRequest {
                id: pm_core::TaskId::sanitize("t1"),
                head_branch: "ai/demo/123/t1".to_string(),
                base_branch: "main".to_string(),
                status: pm_core::PullRequestStatus::Ready,
                checks: pm_core::CheckSummary::default(),
                head_commit: None,
            }],
            pm_core::MergeResult {
                merged: true,
                base_branch: "main".to_string(),
                merge_commit: Some("deadbeef".to_string()),
                merged_prs: vec![pm_core::TaskId::sanitize("t1")],
                checks: pm_core::CheckSummary::default(),
                error: None,
                error_log_path: None,
            },
        );
        let repo = pm_core::RepositoryName::sanitize("repo");
        let session_paths =
            pm_core::SessionPaths::new_in(tmp.path().join("tmp"), &repo, result.session.id);

        let hook = HookSpec::Webhook {
            url: format!("http://{addr}/hook"),
        };
        let runner = WebhookHookRunner::new()?;
        runner.run(&hook, &pm_paths, &session_paths, &result).await?;

        let payload = captured
            .lock()
            .await
            .take()
            .expect("webhook handler must capture payload");

        assert_eq!(payload["session_id"], result.session.id.to_string());
        assert_eq!(payload["repo"], result.session.repo.as_str());
        assert_eq!(payload["pr_name"], result.session.pr_name.as_str());
        assert_eq!(payload["base_branch"], result.session.base_branch.as_str());
        assert_eq!(payload["merged"], result.merge.merged);
        assert_eq!(payload["merge_error"], Value::Null);

        assert_eq!(payload["pm_root"], pm_paths.root().display().to_string());
        assert_eq!(
            payload["session_dir"],
            pm_paths.session_dir(result.session.id).display().to_string()
        );
        assert_eq!(payload["tmp_dir"], session_paths.root().display().to_string());
        assert_eq!(
            payload["result_json"],
            session_paths
                .root()
                .join("result.json")
                .display()
                .to_string()
        );

        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn show_session_prefers_result_by_default() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id: SessionId = "00000000-0000-0000-0000-000000000123".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id, "stage": "session"}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/result"),
                &serde_json::json!({"id": id, "stage": "result"}),
            )
            .await?;

        let json = show_session_json(&storage, id, false).await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(value["result"]["stage"], "result");
        assert!(value.get("session").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn show_session_falls_back_when_result_missing() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id: SessionId = "00000000-0000-0000-0000-000000000456".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id, "stage": "session"}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/tasks"),
                &serde_json::json!([{"id":"t1","title":"x"}]),
            )
            .await?;

        let json = show_session_json(&storage, id, false).await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(value["session"]["stage"], "session");
        assert_eq!(value["tasks"][0]["id"], "t1");
        assert!(value.get("result").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn show_session_all_includes_all_keys() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let pm_paths = PmPaths::new(tmp.path().join(".code_pm"));
        let storage = FsStorage::new(pm_paths.data_dir());

        let id: SessionId = "00000000-0000-0000-0000-000000000789".parse()?;
        storage
            .put_json(
                &format!("sessions/{id}/session"),
                &serde_json::json!({"id": id}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/tasks"),
                &serde_json::json!([{"id":"t1"}]),
            )
            .await?;
        storage
            .put_json(&format!("sessions/{id}/prs"), &serde_json::json!([]))
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/merge"),
                &serde_json::json!({"merged": true}),
            )
            .await?;
        storage
            .put_json(
                &format!("sessions/{id}/result"),
                &serde_json::json!({"id": id}),
            )
            .await?;

        let json = show_session_json(&storage, id, true).await?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        for key in ["session", "tasks", "prs", "merge", "result"] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
        Ok(())
    }

    #[test]
    fn resolve_pm_root_defaults_to_repo_root_dot_code_pm() {
        let repo_root = PathBuf::from("/repo");
        assert_eq!(
            resolve_pm_root(&repo_root, None, None),
            (repo_root.join(".code_pm"), PmRootSource::Default)
        );
    }

    #[test]
    fn resolve_pm_root_prefers_cli_override() {
        let repo_root = PathBuf::from("/repo");
        let cli = PathBuf::from("cli-root");
        let env = std::ffi::OsString::from("env-root");
        assert_eq!(
            resolve_pm_root(&repo_root, Some(&cli), Some(env.as_os_str())),
            (repo_root.join("cli-root"), PmRootSource::Override)
        );
    }

    #[test]
    fn resolve_pm_root_resolves_relative_env_to_repo_root() {
        let repo_root = PathBuf::from("/repo");
        let env = std::ffi::OsString::from("state");
        assert_eq!(
            resolve_pm_root(&repo_root, None, Some(env.as_os_str())),
            (repo_root.join("state"), PmRootSource::Override)
        );
    }

    #[tokio::test]
    async fn parse_tasks_override_rejects_empty_id_in_tasks_file() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("tasks.json");
        tokio::fs::write(&path, r#"[{"id":"","title":"x"}]"#).await?;

        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: None,
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: Some(path),
            task: Vec::new(),
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = parse_tasks_override(&args).await.unwrap_err();
        assert!(err.to_string().contains("task id must not be empty"));
        Ok(())
    }

    #[tokio::test]
    async fn parse_tasks_override_rejects_empty_id_in_task_arg() -> anyhow::Result<()> {
        let args = RunArgs {
            repo: None,
            repo_src: None,
            pr_name: "demo".to_string(),
            prompt: None,
            prompt_file: None,
            base: "main".to_string(),
            apply_patch: None,
            max_concurrency: 1,
            stream_events: false,
            stream_events_json: false,
            strict: false,
            json: false,
            cargo_test: false,
            no_merge: false,
            auto_tasks: false,
            tasks_file: None,
            task: vec![":x".to_string()],
            hook_cmd: None,
            hook_arg: Vec::new(),
            hook_url: None,
        };

        let err = parse_tasks_override(&args).await.unwrap_err();
        assert!(err.to_string().contains("--task id must not be empty"));
        Ok(())
    }

    #[test]
    fn legacy_pm_root_warning_emits_notice_for_default_root_when_legacy_dir_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".codex_pm")).expect("create legacy dir");

        let (pm_root, source) = resolve_pm_root(repo_root, None, None);
        assert_eq!(source, PmRootSource::Default);
        assert!(legacy_pm_root_warning(repo_root, &pm_root, source).is_some());
    }

    #[test]
    fn legacy_pm_root_warning_skips_when_new_dir_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".codex_pm")).expect("create legacy dir");
        std::fs::create_dir_all(repo_root.join(".code_pm")).expect("create new dir");

        let (pm_root, source) = resolve_pm_root(repo_root, None, None);
        assert_eq!(source, PmRootSource::Default);
        assert!(legacy_pm_root_warning(repo_root, &pm_root, source).is_none());
    }

    #[test]
    fn legacy_pm_root_warning_skips_when_override_root_used() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo_root = tmp.path();
        std::fs::create_dir_all(repo_root.join(".codex_pm")).expect("create legacy dir");

        let override_root = repo_root.join("custom-root");
        let (pm_root, source) = resolve_pm_root(repo_root, Some(&override_root), None);
        assert_eq!(source, PmRootSource::Override);
        assert!(legacy_pm_root_warning(repo_root, &pm_root, source).is_none());
    }
}
