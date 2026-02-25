#[cfg(test)]
mod tests {
    use super::*;

    fn env_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        let _guard = env_test_lock()
            .lock()
            .expect("env test lock must not be poisoned");
        let prev = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var(key).ok()))
            .collect::<Vec<_>>();
        unsafe {
            for (key, value) in vars {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
        let out = f();
        unsafe {
            for (key, value) in prev {
                match value {
                    Some(v) => std::env::set_var(&key, v),
                    None => std::env::remove_var(&key),
                }
            }
        }
        out
    }

    fn with_env_var<T>(key: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
        with_env_vars(&[(key, value)], f)
    }

    fn test_scheduling() -> FanOutSchedulingParams {
        FanOutSchedulingParams {
            env_max_concurrent_subagents: 4,
            effective_concurrency_limit: 3,
            priority_aging_rounds: 5,
        }
    }

    fn artifact_write_params_json(
        params: omne_app_server_protocol::ArtifactWriteParams,
    ) -> serde_json::Value {
        serde_json::to_value(params).expect("serialize ArtifactWriteParams")
    }

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
    }

    fn thread_start_response_with_auto_hook(
        auto_hook: omne_app_server_protocol::ThreadAutoHookResponse,
    ) -> omne_app_server_protocol::ThreadStartResponse {
        omne_app_server_protocol::ThreadStartResponse {
            thread_id: ThreadId::new(),
            log_path: "/tmp/.omne_data/threads/thread.log".to_string(),
            last_seq: 1,
            auto_hook,
        }
    }

    #[test]
    fn split_frontmatter_handles_crlf() -> anyhow::Result<()> {
        let raw = "---\r\nversion: 1\r\nmode: coder\r\n---\r\nbody\r\n";
        let (yaml, body) = split_frontmatter(raw)?;
        assert!(yaml.contains("version: 1"));
        assert_eq!(body, "body\r\n");
        Ok(())
    }

    #[test]
    fn render_template_replaces_value() -> anyhow::Result<()> {
        let mut declared = BTreeSet::new();
        declared.insert("name".to_string());
        let mut vars = BTreeMap::new();
        vars.insert("name".to_string(), "ok".to_string());
        let rendered = render_template("hello {{name}}", &declared, &vars)?;
        assert_eq!(rendered, "hello ok");
        Ok(())
    }

    #[test]
    fn render_template_rejects_whitespace() {
        let mut declared = BTreeSet::new();
        declared.insert("name".to_string());
        let mut vars = BTreeMap::new();
        vars.insert("name".to_string(), "ok".to_string());
        let err = render_template("{{ name }}", &declared, &vars).unwrap_err();
        assert!(err.to_string().contains("whitespace"));
    }

    #[test]
    fn command_run_error_code_classifies_flag_conflict() {
        let err = anyhow::anyhow!("--fan-out-early-return requires --fan-out");
        assert_eq!(
            command_run_error_code(&err),
            Some("fan_out_early_return_requires_fan_out")
        );
    }

    #[test]
    fn command_run_error_code_classifies_missing_required_var() {
        let err = anyhow::anyhow!("missing required --var: project");
        assert_eq!(
            command_run_error_code(&err),
            Some("command_var_missing_required")
        );
    }

    #[test]
    fn command_run_error_code_classifies_context_step_failure() {
        let err =
            anyhow::anyhow!("context step failed: summary=x process_id=1 exit_code=1 ok_exit_codes=[0]");
        assert_eq!(command_run_error_code(&err), Some("context_step_failed"));
    }

    #[test]
    fn command_run_error_code_reuses_command_spec_error_classification() {
        let err: anyhow::Error = CommandSpecError::UnknownMode {
            mode: "x".to_string(),
            available: "coder".to_string(),
        }
        .into();
        assert_eq!(command_run_error_code(&err), Some("mode_unknown"));
    }

    #[test]
    fn command_run_error_code_classifies_thread_start_auto_hook_denied() {
        let err = anyhow::anyhow!(
            "[rpc error_code] mode_denied; command/fan_out thread/start auto hook denied: <detail>"
        );
        assert_eq!(
            command_run_error_code(&err),
            Some("thread_start_auto_hook_denied")
        );
    }

    #[test]
    fn command_run_error_code_classifies_thread_start_auto_hook_error() {
        let err = anyhow::anyhow!(
            "command/fan_out thread/start auto hook error: hook=setup error=spawn failed"
        );
        assert_eq!(
            command_run_error_code(&err),
            Some("thread_start_auto_hook_error")
        );
    }

    #[test]
    fn ensure_thread_start_auto_hook_ready_denied_returns_error_with_error_code() {
        let thread_id = ThreadId::new();
        let started = thread_start_response_with_auto_hook(
            omne_app_server_protocol::ThreadAutoHookResponse::Denied(
                omne_app_server_protocol::ThreadHookRunDeniedResponse {
                    denied: true,
                    thread_id,
                    hook: "setup".to_string(),
                    error_code: Some("mode_denied".to_string()),
                    config_path: Some("/tmp/.omne_data/hooks/setup".to_string()),
                    detail: omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                        omne_app_server_protocol::ProcessDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            thread_id,
                            remembered: None,
                            error_code: Some("mode_denied".to_string()),
                        },
                    ),
                },
            ),
        );

        let err = ensure_thread_start_auto_hook_ready("command/fan_out", &started)
            .expect_err("denied auto_hook should fail fast");
        let message = err.to_string();
        assert!(message.contains("thread/start auto hook denied"));
        assert!(message.contains("mode_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn ensure_thread_start_auto_hook_ready_error_returns_error() {
        let started = thread_start_response_with_auto_hook(
            omne_app_server_protocol::ThreadAutoHookResponse::Error(
                omne_app_server_protocol::ThreadHookRunErrorResponse {
                    ok: false,
                    hook: "setup".to_string(),
                    error: "spawn failed".to_string(),
                },
            ),
        );

        let err = ensure_thread_start_auto_hook_ready("command/fan_out", &started)
            .expect_err("error auto_hook should fail fast");
        let message = err.to_string();
        assert!(message.contains("thread/start auto hook error"));
        assert!(message.contains("hook=setup"));
        assert!(message.contains("spawn failed"));
    }

    #[test]
    fn ensure_thread_start_auto_hook_ready_needs_approval_allows_continue() {
        let started = thread_start_response_with_auto_hook(
            omne_app_server_protocol::ThreadAutoHookResponse::NeedsApproval(
                omne_app_server_protocol::ThreadHookRunNeedsApprovalResponse {
                    needs_approval: true,
                    thread_id: ThreadId::new(),
                    approval_id: ApprovalId::new(),
                    hook: "setup".to_string(),
                },
            ),
        );

        ensure_thread_start_auto_hook_ready("command/fan_out", &started)
            .expect("needs approval should not fail run start path");
    }

    #[test]
    fn duplicate_command_name_errors_returns_empty_when_all_names_are_unique() {
        let validated = vec![
            CommandValidateItem {
                name: "plan".to_string(),
                version: 1,
                mode: "architect".to_string(),
                file: "/tmp/plan.md".to_string(),
            },
            CommandValidateItem {
                name: "review".to_string(),
                version: 1,
                mode: "reviewer".to_string(),
                file: "/tmp/review.md".to_string(),
            },
        ];

        let errors = duplicate_command_name_errors(&validated);
        assert!(errors.is_empty());
    }

    #[test]
    fn duplicate_command_name_errors_reports_all_conflicting_files() {
        let validated = vec![
            CommandValidateItem {
                name: "plan".to_string(),
                version: 1,
                mode: "architect".to_string(),
                file: "/tmp/a.md".to_string(),
            },
            CommandValidateItem {
                name: "plan".to_string(),
                version: 1,
                mode: "coder".to_string(),
                file: "/tmp/b.md".to_string(),
            },
        ];

        let errors = duplicate_command_name_errors(&validated);
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|item| {
            item.file == "/tmp/a.md"
                && item.error.contains("duplicate command name `plan`")
                && item.error.contains("/tmp/b.md")
        }));
        assert!(errors.iter().any(|item| {
            item.file == "/tmp/b.md"
                && item.error.contains("duplicate command name `plan`")
                && item.error.contains("/tmp/a.md")
        }));
    }

    #[test]
    fn command_list_result_json_flattens_summary_fields() {
        let result = CommandListResult {
            summary: CommandResultSummary::new("/tmp/spec/commands".to_string(), 1, 0),
            command_count: 1,
            commands: vec![CommandListItem {
                name: "plan".to_string(),
                version: 1,
                mode: "architect".to_string(),
                file: "/tmp/spec/commands/plan.md".to_string(),
            }],
            errors: vec![],
            modes_load_error: None,
        };

        let json = serde_json::to_value(result).expect("serialize command list result");
        assert_eq!(json.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            json.get("commands_dir").and_then(|v| v.as_str()),
            Some("/tmp/spec/commands")
        );
        assert_eq!(json.get("item_count").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(json.get("error_count").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(json.get("command_count").and_then(|v| v.as_u64()), Some(1));
        assert!(json.get("summary").is_none());
    }

    #[test]
    fn command_validate_result_json_flattens_summary_fields() {
        let result = CommandValidateResult {
            summary: CommandResultSummary::new("/tmp/spec/commands".to_string(), 1, 0),
            strict: true,
            target: "plan".to_string(),
            validated_count: 1,
            validated: vec![CommandValidateItem {
                name: "plan".to_string(),
                version: 1,
                mode: "architect".to_string(),
                file: "/tmp/spec/commands/plan.md".to_string(),
            }],
            errors: vec![],
            modes_load_error: None,
        };

        let json = serde_json::to_value(result).expect("serialize command validate result");
        assert_eq!(json.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            json.get("commands_dir").and_then(|v| v.as_str()),
            Some("/tmp/spec/commands")
        );
        assert_eq!(json.get("item_count").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(json.get("error_count").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(json.get("validated_count").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(json.get("strict").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(json.get("target").and_then(|v| v.as_str()), Some("plan"));
        assert!(json.get("summary").is_none());
    }

    #[tokio::test]
    async fn run_command_validate_accepts_valid_files() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-ok");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("plan.md"),
            r#"---
version: 1
mode: architect
---
Plan body
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = run_command_validate(&cli, None, false, true).await;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        result
    }

    #[tokio::test]
    async fn collect_command_validate_result_sets_all_target_when_name_absent() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-target-all");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("plan.md"),
            r#"---
version: 1
mode: architect
---
Plan body
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, None, false).await?;
        assert_eq!(result.target, "all");
        assert_eq!(result.summary.commands_dir, commands_dir.display().to_string());
        assert!(result.summary.ok);
        assert_eq!(result.summary.item_count, 1);
        assert_eq!(result.validated_count, 1);
        assert_eq!(result.summary.error_count, 0);
        assert!(result.errors.is_empty());
        assert_eq!(result.validated.len(), 1);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_validate_result_sets_named_target() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-target-name");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("plan.md"),
            r#"---
version: 1
mode: architect
---
Plan body
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, Some("plan".to_string()), false).await?;
        assert_eq!(result.target, "plan");
        assert_eq!(result.summary.commands_dir, commands_dir.display().to_string());
        assert!(result.summary.ok);
        assert_eq!(result.summary.item_count, 1);
        assert_eq!(result.validated_count, 1);
        assert_eq!(result.summary.error_count, 0);
        assert!(result.errors.is_empty());
        assert_eq!(result.validated.len(), 1);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_command_validate_fails_on_invalid_frontmatter() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-invalid");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("broken.md"),
            r#"---
version: 1
---
Broken
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let err = run_command_validate(&cli, None, false, true)
            .await
            .expect_err("invalid command should fail validation");
        assert!(err.to_string().contains("command validation failed"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_validate_result_reports_unknown_allowed_tool() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-unknown-tool");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("broken.md"),
            r#"---
version: 1
mode: coder
allowed_tools:
  - process/start
  - tool/does_not_exist
---
Broken
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
        assert!(!result.summary.ok);
        assert_eq!(result.summary.error_count, 1);
        assert!(result.errors[0].error.contains("unknown tool in allowed_tools: tool/does_not_exist"));
        assert_eq!(
            result.errors[0].error_code.as_deref(),
            Some("allowed_tools_unknown_tool")
        );

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_validate_result_reports_mode_incompatible_allowed_tool()
    -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-mode-denied-tool");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("broken.md"),
            r#"---
version: 1
mode: reviewer
allowed_tools:
  - file/write
---
Broken
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
        assert!(!result.summary.ok);
        assert_eq!(result.summary.error_count, 1);
        assert!(
            result.errors[0]
                .error
                .contains("allowed_tools tool is denied by mode: mode=reviewer tool=file/write")
        );
        assert_eq!(
            result.errors[0].error_code.as_deref(),
            Some("allowed_tools_mode_denied")
        );

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_validate_result_reports_unknown_mode() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-unknown-mode");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("broken.md"),
            r#"---
version: 1
mode: mode-does-not-exist
---
Broken
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
        assert!(!result.summary.ok);
        assert_eq!(result.summary.error_count, 1);
        assert!(
            result.errors[0]
                .error
                .contains("unknown mode: mode-does-not-exist")
        );
        assert_eq!(result.errors[0].error_code.as_deref(), Some("mode_unknown"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_validate_result_respects_custom_mode_tool_override_denies()
    -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-custom-mode-deny");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::create_dir_all(tmp.join(".omne_data").join("spec")).await?;
        tokio::fs::write(
            tmp.join(".omne_data").join("spec").join("modes.yaml"),
            r#"version: 1
modes:
  strict-coder:
    description: "custom mode with process/start denied"
    permissions:
      read: { decision: allow }
      edit:
        decision: allow
      command:
        decision: allow
      process:
        inspect: { decision: allow }
        kill: { decision: allow }
        interact: { decision: deny }
      artifact: { decision: allow }
      browser: { decision: deny }
    tool_overrides:
      - tool: "process/start"
        decision: deny
"#,
        )
        .await?;
        tokio::fs::write(
            commands_dir.join("broken.md"),
            r#"---
version: 1
mode: strict-coder
allowed_tools:
  - process/start
---
Broken
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, Some("broken".to_string()), false).await?;
        assert!(!result.summary.ok);
        assert_eq!(result.summary.error_count, 1);
        assert!(
            result.errors[0]
                .error
                .contains("allowed_tools tool is denied by mode: mode=strict-coder tool=process/start")
        );
        assert_eq!(
            result.errors[0].error_code.as_deref(),
            Some("allowed_tools_mode_denied")
        );

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_validate_result_exposes_modes_load_error_for_json_clients()
    -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-modes-load-error");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::create_dir_all(tmp.join(".omne_data").join("spec")).await?;
        tokio::fs::write(
            tmp.join(".omne_data").join("spec").join("modes.yaml"),
            "version: [not-a-number]\n",
        )
        .await?;
        tokio::fs::write(
            commands_dir.join("ok.md"),
            r#"---
version: 1
mode: coder
---
Ok
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let result = collect_command_validate_result(&cli, Some("ok".to_string()), false).await?;
        assert!(result.summary.ok);
        assert_eq!(result.summary.error_count, 0);
        assert!(result.modes_load_error.is_some());
        assert!(
            result
                .modes_load_error
                .as_deref()
                .is_some_and(|msg| msg.contains("parse modes config"))
        );

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn load_workflow_file_exposes_modes_load_error_when_modes_config_parse_fails()
    -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-load-workflow-modes-load-error");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::create_dir_all(tmp.join(".omne_data").join("spec")).await?;
        tokio::fs::write(
            tmp.join(".omne_data").join("spec").join("modes.yaml"),
            "version: [not-a-number]\n",
        )
        .await?;
        tokio::fs::write(
            commands_dir.join("ok.md"),
            r#"---
version: 1
mode: coder
---
Ok
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let wf = load_workflow_file(&cli, "ok").await?;
        assert_eq!(wf.frontmatter.mode, "coder");
        assert!(
            wf.modes_load_error
                .as_deref()
                .is_some_and(|msg| msg.contains("parse modes config"))
        );

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_command_validate_strict_rejects_duplicate_declared_names() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-strict");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("a.md"),
            r#"---
version: 1
name: duplicate
mode: reviewer
---
A
"#,
        )
        .await?;
        tokio::fs::write(
            commands_dir.join("b.md"),
            r#"---
version: 1
name: duplicate
mode: reviewer
---
B
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        run_command_validate(&cli, None, false, true).await?;
        let err = run_command_validate(&cli, None, true, true)
            .await
            .expect_err("strict validation should reject duplicate names");
        assert!(err.to_string().contains("command validation failed"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_command_validate_named_target_reports_missing_file() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-missing");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let err = run_command_validate(&cli, Some("missing".to_string()), false, true)
            .await
            .expect_err("missing named command should fail");
        let msg = err.to_string();
        assert!(msg.contains("command `missing` validation failed"));
        assert!(msg.contains("No such file or directory"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_command_validate_named_target_rejects_invalid_name() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-invalid-name");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let err = run_command_validate(&cli, Some("../bad".to_string()), false, true)
            .await
            .expect_err("invalid workflow name should be rejected");
        assert!(err.to_string().contains("workflow name must not contain path separators"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_command_validate_named_target_reports_parse_error_with_name() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-named-parse");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        tokio::fs::write(
            commands_dir.join("broken.md"),
            r#"---
version: 1
---
broken
"#,
        )
        .await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        let err = run_command_validate(&cli, Some("broken".to_string()), false, true)
            .await
            .expect_err("invalid named command should fail");
        let msg = err.to_string();
        assert!(msg.contains("command `broken` validation failed"));
        assert!(msg.contains("missing field `mode`"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_command_validate_allows_empty_command_dir() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-validate-empty-dir");
        let commands_dir = tmp.join("spec").join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;

        let cli = Cli {
            omne_root: Some(tmp.clone()),
            app_server: None,
            execpolicy_rules: vec![],
            command: None,
        };

        run_command_validate(&cli, None, false, true).await?;

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_list_result_accumulates_parse_errors() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-list-errors");
        tokio::fs::create_dir_all(&tmp).await?;
        tokio::fs::write(
            tmp.join("ok.md"),
            r#"---
version: 1
mode: coder
---
ok
"#,
        )
        .await?;
        tokio::fs::write(
            tmp.join("broken.md"),
            r#"---
version: 1
---
broken
"#,
        )
        .await?;
        tokio::fs::write(tmp.join("skip.txt"), "x").await?;

        let result = collect_command_list_result(&tmp).await?;
        assert!(!result.summary.ok);
        assert_eq!(result.summary.commands_dir, tmp.display().to_string());
        assert_eq!(result.summary.item_count, 1);
        assert_eq!(result.command_count, 1);
        assert_eq!(result.summary.error_count, 1);
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0].name, "ok");
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].file.ends_with("broken.md"));
        assert!(result.errors[0].error.contains("missing field `mode`"));

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn collect_command_list_result_sets_ok_when_no_errors() -> anyhow::Result<()> {
        let tmp = unique_temp_dir("omne-command-list-ok");
        tokio::fs::create_dir_all(&tmp).await?;
        tokio::fs::write(
            tmp.join("plan.md"),
            r#"---
version: 1
mode: architect
---
ok
"#,
        )
        .await?;

        let result = collect_command_list_result(&tmp).await?;
        assert!(result.summary.ok);
        assert_eq!(result.summary.commands_dir, tmp.display().to_string());
        assert_eq!(result.summary.item_count, 1);
        assert_eq!(result.command_count, 1);
        assert_eq!(result.summary.error_count, 0);
        assert_eq!(result.commands.len(), 1);
        assert!(result.errors.is_empty());

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[test]
    fn parse_workflow_tasks_extracts_task_sections() -> anyhow::Result<()> {
        let body = "Intro\n\n## Task: t1 First\nhello\n\n## Task: t2\nworld\n";
        let tasks = parse_workflow_tasks(body)?;
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "t1");
        assert_eq!(tasks[0].title, "First");
        assert!(tasks[0].depends_on.is_empty());
        assert!(tasks[0].body.contains("hello"));
        assert_eq!(tasks[1].id, "t2");
        assert_eq!(tasks[1].title, "");
        assert!(tasks[1].depends_on.is_empty());
        assert!(tasks[1].body.contains("world"));
        Ok(())
    }

    #[test]
    fn parse_workflow_tasks_parses_depends_on_directive() -> anyhow::Result<()> {
        let body =
            "## Task: t1 First\nintro\n\n## Task: t2 Second\ndepends_on: t1\nrun second\n";
        let tasks = parse_workflow_tasks(body)?;
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].depends_on.is_empty());
        assert_eq!(tasks[0].priority, WorkflowTaskPriority::Normal);
        assert_eq!(tasks[1].depends_on, vec!["t1".to_string()]);
        assert_eq!(tasks[1].priority, WorkflowTaskPriority::Normal);
        assert!(!tasks[1].body.contains("depends_on:"));
        assert!(tasks[1].body.contains("run second"));
        Ok(())
    }

    #[test]
    fn parse_workflow_tasks_parses_priority_directive() -> anyhow::Result<()> {
        let body = "## Task: t1 First\npriority: high\nreview first\n";
        let tasks = parse_workflow_tasks(body)?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].priority, WorkflowTaskPriority::High);
        assert!(!tasks[0].body.contains("priority:"));
        assert!(tasks[0].body.contains("review first"));
        Ok(())
    }

    #[test]
    fn parse_workflow_tasks_parses_depends_on_and_priority_directives() -> anyhow::Result<()> {
        let body =
            "## Task: t1 First\nrun first\n\n## Task: t2 Second\ndepends_on: t1\npriority: low\nrun second\n";
        let tasks = parse_workflow_tasks(body)?;
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].depends_on, vec!["t1".to_string()]);
        assert_eq!(tasks[1].priority, WorkflowTaskPriority::Low);
        assert!(!tasks[1].body.contains("depends_on:"));
        assert!(!tasks[1].body.contains("priority:"));
        Ok(())
    }

    #[test]
    fn parse_workflow_tasks_rejects_invalid_priority() {
        let body = "## Task: t1 First\npriority: urgent\nrun first\n";
        let err = parse_workflow_tasks(body).unwrap_err();
        assert!(err.to_string().contains("invalid priority"));
    }

    #[test]
    fn parse_workflow_tasks_rejects_duplicate_priority_directive() {
        let body = "## Task: t1 First\npriority: high\npriority: low\nrun first\n";
        let err = parse_workflow_tasks(body).unwrap_err();
        assert!(err.to_string().contains("duplicate priority directive"));
    }

    #[test]
    fn parse_workflow_tasks_rejects_unknown_depends_on() {
        let body = "## Task: t1 First\ndepends_on: t2\nrun first\n";
        let err = parse_workflow_tasks(body).unwrap_err();
        assert!(err.to_string().contains("unknown depends_on"));
    }

    #[test]
    fn parse_workflow_tasks_rejects_dependency_cycles() {
        let body = "## Task: t1 First\ndepends_on: t2\nrun first\n\n## Task: t2 Second\ndepends_on: t1\nrun second\n";
        let err = parse_workflow_tasks(body).unwrap_err();
        assert!(err.to_string().contains("task dependencies contain a cycle"));
    }

    #[test]
    fn collect_dependency_blocked_tasks_marks_dependents_of_failed_tasks() {
        let tasks = vec![
            WorkflowTask {
                id: "t1".to_string(),
                title: "first".to_string(),
                body: "first".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::Normal,
            },
            WorkflowTask {
                id: "t2".to_string(),
                title: "second".to_string(),
                body: "second".to_string(),
                depends_on: vec!["t1".to_string()],
                priority: WorkflowTaskPriority::Normal,
            },
        ];
        let started = BTreeSet::<String>::new();
        let mut statuses = BTreeMap::<String, TurnStatus>::new();
        statuses.insert("t1".to_string(), TurnStatus::Failed);
        let blocked = collect_dependency_blocked_task_ids(&tasks, &started, &statuses);
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].0, "t2");
        assert_eq!(blocked[0].1, "t1");
        assert!(matches!(blocked[0].2, TurnStatus::Failed));
    }

    #[test]
    fn dependency_blocker_fields_extracts_task_and_status_from_reason() {
        let (task_id, status) = dependency_blocker_fields(
            true,
            Some("blocked by dependency: task_a status=Failed"),
        );
        assert_eq!(task_id.as_deref(), Some("task_a"));
        assert_eq!(status.as_deref(), Some("Failed"));

        let (task_id, status) =
            dependency_blocker_fields(true, Some("turn finished with status=Failed"));
        assert!(task_id.is_none());
        assert!(status.is_none());

        let (task_id, status) = dependency_blocker_fields(
            false,
            Some("blocked by dependency: task_a status=Failed"),
        );
        assert!(task_id.is_none());
        assert!(status.is_none());
    }

    #[test]
    fn pick_next_runnable_task_prefers_higher_priority() {
        let tasks = vec![
            WorkflowTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                body: "low".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::Low,
            },
            WorkflowTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                body: "high".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::High,
            },
            WorkflowTask {
                id: "t-normal".to_string(),
                title: "normal".to_string(),
                body: "normal".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::Normal,
            },
        ];
        let started = BTreeSet::<String>::new();
        let statuses = BTreeMap::<String, TurnStatus>::new();
        let selected = pick_next_runnable_task(&tasks, &started, &statuses);
        assert_eq!(selected.map(|task| task.id.as_str()), Some("t-high"));
    }

    #[test]
    fn update_ready_wait_rounds_tracks_only_ready_pending_tasks() {
        let tasks = vec![
            WorkflowTask {
                id: "t-ready".to_string(),
                title: "ready".to_string(),
                body: "ready".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::Normal,
            },
            WorkflowTask {
                id: "t-blocked".to_string(),
                title: "blocked".to_string(),
                body: "blocked".to_string(),
                depends_on: vec!["t-ready".to_string()],
                priority: WorkflowTaskPriority::Normal,
            },
        ];
        let started = BTreeSet::<String>::new();
        let statuses = BTreeMap::<String, TurnStatus>::new();
        let mut wait_rounds = BTreeMap::<String, usize>::new();

        update_ready_wait_rounds(&tasks, &started, &statuses, &mut wait_rounds);
        assert_eq!(wait_rounds.get("t-ready").copied(), Some(1));
        assert!(!wait_rounds.contains_key("t-blocked"));

        update_ready_wait_rounds(&tasks, &started, &statuses, &mut wait_rounds);
        assert_eq!(wait_rounds.get("t-ready").copied(), Some(2));
    }

    #[test]
    fn pick_next_runnable_task_fair_can_age_low_priority() {
        let tasks = vec![
            WorkflowTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                body: "low".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::Low,
            },
            WorkflowTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                body: "high".to_string(),
                depends_on: vec![],
                priority: WorkflowTaskPriority::High,
            },
        ];
        let started = BTreeSet::<String>::new();
        let statuses = BTreeMap::<String, TurnStatus>::new();
        let mut wait_rounds = BTreeMap::<String, usize>::new();
        wait_rounds.insert("t-low".to_string(), 6);

        let selected = pick_next_runnable_task_fair(&tasks, &started, &statuses, &wait_rounds, 3);
        assert_eq!(selected.map(|task| task.id.as_str()), Some("t-low"));
    }

    #[test]
    fn parse_env_bool_accepts_common_values() {
        assert_eq!(parse_bool_token("yes"), Some(true));
        assert_eq!(parse_bool_token("off"), Some(false));
        assert_eq!(parse_bool_token("  TRUE "), Some(true));
        assert_eq!(parse_bool_token("maybe"), None);
    }

    #[test]
    fn fan_out_priority_aging_rounds_env_fallback_and_clamp() {
        let default_value = with_env_var("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", None, || {
            fan_out_priority_aging_rounds()
        });
        assert_eq!(default_value, 3);

        let fallback_value = with_env_var(
            "OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS",
            Some("not-a-number"),
            || fan_out_priority_aging_rounds(),
        );
        assert_eq!(fallback_value, 3);

        let clamped_low = with_env_var("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", Some("0"), || {
            fan_out_priority_aging_rounds()
        });
        assert_eq!(clamped_low, 1);

        let clamped_high = with_env_var("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", Some("99999"), || {
            fan_out_priority_aging_rounds()
        });
        assert_eq!(clamped_high, 10_000);
    }

    #[test]
    fn fan_out_scheduling_params_respects_unlimited_and_fixed_env_limit() {
        let unlimited = with_env_vars(
            &[
                ("OMNE_MAX_CONCURRENT_SUBAGENTS", Some("0")),
                ("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", Some("7")),
            ],
            || fan_out_scheduling_params(5),
        );
        assert_eq!(unlimited.env_max_concurrent_subagents, 0);
        assert_eq!(unlimited.effective_concurrency_limit, 5);
        assert_eq!(unlimited.priority_aging_rounds, 7);

        let unlimited_zero_tasks = with_env_var("OMNE_MAX_CONCURRENT_SUBAGENTS", Some("0"), || {
            fan_out_scheduling_params(0)
        });
        assert_eq!(unlimited_zero_tasks.env_max_concurrent_subagents, 0);
        assert_eq!(unlimited_zero_tasks.effective_concurrency_limit, 1);

        let fixed = with_env_var("OMNE_MAX_CONCURRENT_SUBAGENTS", Some("2"), || {
            fan_out_scheduling_params(5)
        });
        assert_eq!(fixed.env_max_concurrent_subagents, 2);
        assert_eq!(fixed.effective_concurrency_limit, 2);
    }

    #[test]
    fn validate_fan_out_results_blocks_non_completed_when_required() {
        let parent_thread_id = ThreadId::new();
        let error_artifact_id = ArtifactId::new();
        let results = vec![WorkflowTaskResult {
            task_id: "t1".to_string(),
            title: "task".to_string(),
            thread_id: None,
            turn_id: None,
            result_artifact_id: None,
            result_artifact_error: Some("result artifact write failed".to_string()),
            result_artifact_error_id: Some(error_artifact_id),
            status: TurnStatus::Failed,
            reason: Some("failed".to_string()),
            dependency_blocked: true,
            assistant_text: None,
            pending_approval: None,
        }];

        let err =
            validate_fan_out_results(&results, parent_thread_id, omne_protocol::ArtifactId::new(), true)
                .unwrap_err();
        assert!(err.to_string().contains("fan-out task is not completed"));
        assert!(err.to_string().contains("thread_id=-"));
        assert!(err.to_string().contains("artifact_error=result artifact write failed"));
        assert!(err.to_string().contains(&format!(
            "artifact_error_read_cmd=omne artifact read {} {}",
            parent_thread_id, error_artifact_id
        )));
        assert!(
            validate_fan_out_results(&results, parent_thread_id, omne_protocol::ArtifactId::new(), false)
                .is_ok()
        );
    }

    #[test]
    fn format_non_completed_fan_out_issue_includes_error_read_command() {
        let parent_thread_id = ThreadId::new();
        let artifact_id = ArtifactId::new();
        let error_artifact_id = ArtifactId::new();
        let result = WorkflowTaskResult {
            task_id: "t1".to_string(),
            title: "task".to_string(),
            thread_id: Some(ThreadId::new()),
            turn_id: Some(TurnId::new()),
            result_artifact_id: None,
            result_artifact_error: Some("write failed".to_string()),
            result_artifact_error_id: Some(error_artifact_id),
            status: TurnStatus::Failed,
            reason: None,
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        };

        let text = format_non_completed_fan_out_issue(
            "fan-out linkage issue",
            &result,
            parent_thread_id,
            artifact_id,
        );
        assert!(text.contains("fan-out linkage issue"));
        assert!(text.contains("artifact_error=write failed"));
        assert!(text.contains(&format!(
            "artifact_error_read_cmd=omne artifact read {} {}",
            parent_thread_id, error_artifact_id
        )));
        assert!(text.contains(&format!(
            "fan_in_summary artifact_id={}",
            artifact_id
        )));
    }

    #[test]
    fn format_non_completed_fan_out_issue_includes_pending_approval_handles() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let turn_id = TurnId::new();
        let artifact_id = ArtifactId::new();
        let approval_id = ApprovalId::new();
        let result = WorkflowTaskResult {
            task_id: "t-approval".to_string(),
            title: "approval task".to_string(),
            thread_id: Some(child_thread_id),
            turn_id: Some(turn_id),
            result_artifact_id: None,
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Interrupted,
            reason: Some("blocked on approval".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: Some(WorkflowPendingApproval {
                approval_id,
                action: "subagent/proxy_approval".to_string(),
                summary: Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
                approve_cmd: Some(format!(
                    "omne approval decide {} {} --approve",
                    child_thread_id, approval_id
                )),
                deny_cmd: Some(format!(
                    "omne approval decide {} {} --deny",
                    child_thread_id, approval_id
                )),
            }),
        };

        let text = format_non_completed_fan_out_issue(
            "fan-out linkage issue",
            &result,
            parent_thread_id,
            artifact_id,
        );
        assert!(text.contains("pending_approval_action=subagent/proxy_approval"));
        assert!(text.contains(&format!("pending_approval_id={approval_id}")));
        assert!(text.contains("pending_approval_summary=child_thread_id=abc"));
        assert!(text.contains(&format!(
            "approve_cmd=omne approval decide {} {} --approve",
            child_thread_id, approval_id
        )));
        assert!(text.contains(&format!(
            "deny_cmd=omne approval decide {} {} --deny",
            child_thread_id, approval_id
        )));
    }

    #[test]
    fn first_non_completed_task_from_fan_in_summary_finds_first_non_completed() {
        let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: "fan_in_summary.v1".to_string(),
            thread_id: "thread-1".to_string(),
            task_count: 3,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            tasks: vec![
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t1".to_string(),
                    title: "done".to_string(),
                    thread_id: Some("child-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    status: "Completed".to_string(),
                    reason: None,
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t2".to_string(),
                    title: "blocked".to_string(),
                    thread_id: Some("child-2".to_string()),
                    turn_id: Some("turn-2".to_string()),
                    status: "NeedUserInput".to_string(),
                    reason: Some("approval needed".to_string()),
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: Some("approval required".to_string()),
                    result_artifact_error_id: Some("artifact-error-2".to_string()),
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t3".to_string(),
                    title: "failed".to_string(),
                    thread_id: Some("child-3".to_string()),
                    turn_id: Some("turn-3".to_string()),
                    status: "Failed".to_string(),
                    reason: None,
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: None,
                },
            ],
        };

        let task = first_non_completed_task_from_fan_in_summary(&payload).expect("task");
        assert_eq!(task.task_id, "t2");
    }

    #[test]
    fn format_non_completed_fan_out_issue_from_structured_task_includes_pending_handles() {
        let parent_thread_id = ThreadId::new().to_string();
        let artifact_id = ArtifactId::new();
        let task = omne_app_server_protocol::ArtifactFanInSummaryTask {
            task_id: "t-approval".to_string(),
            title: "approval task".to_string(),
            thread_id: Some("child-thread-1".to_string()),
            turn_id: Some("child-turn-1".to_string()),
            status: "NeedUserInput".to_string(),
            reason: Some("blocked on approval".to_string()),
            dependency_blocked: true,
            dependency_blocker_task_id: Some("t-upstream".to_string()),
            dependency_blocker_status: Some("Failed".to_string()),
            result_artifact_id: None,
            result_artifact_error: Some("approval required".to_string()),
            result_artifact_error_id: Some("artifact-error-1".to_string()),
            result_artifact_diagnostics: None,
            pending_approval: Some(omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                approval_id: "approval-1".to_string(),
                action: "subagent/proxy_approval".to_string(),
                summary: Some("child_thread_id=abc".to_string()),
                approve_cmd: Some("omne approval decide child-thread-1 approval-1 --approve".to_string()),
                deny_cmd: Some("omne approval decide child-thread-1 approval-1 --deny".to_string()),
            }),
        };

        let text = format_non_completed_fan_out_issue_from_structured_task(
            "fan-out task is not completed",
            &parent_thread_id,
            &task,
            artifact_id,
        );
        assert!(text.contains("fan-out task is not completed"));
        assert!(text.contains("task_id=t-approval"));
        assert!(text.contains("status=NeedUserInput"));
        assert!(text.contains("thread_id=child-thread-1"));
        assert!(text.contains("turn_id=child-turn-1"));
        assert!(text.contains("artifact_error=approval required"));
        assert!(text.contains(&format!(
            "artifact_error_read_cmd=omne artifact read {} artifact-error-1",
            parent_thread_id
        )));
        assert!(text.contains("pending_approval_action=subagent/proxy_approval"));
        assert!(text.contains("pending_approval_id=approval-1"));
        assert!(text.contains("pending_approval_summary=child_thread_id=abc"));
        assert!(text.contains("approve_cmd=omne approval decide child-thread-1 approval-1 --approve"));
        assert!(text.contains("deny_cmd=omne approval decide child-thread-1 approval-1 --deny"));
        assert!(text.contains("dependency_blocked=true"));
        assert!(text.contains("dependency_blocker_task_id=t-upstream"));
        assert!(text.contains("dependency_blocker_status=Failed"));
        assert!(text.contains(&format!(
            "fan_in_summary artifact_id={}",
            artifact_id
        )));
    }

    #[test]
    fn format_fan_out_linkage_issue_from_structured_payload_includes_artifact_handles() {
        let artifact_id = ArtifactId::new();
        let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
            schema_version: "fan_out_linkage_issue.v1".to_string(),
            fan_in_summary_artifact_id: "fan-in-1".to_string(),
            issue: "fan-out linkage issue: blocked".to_string(),
            issue_truncated: true,
        };

        let text = format_fan_out_linkage_issue_from_structured_payload(&payload, artifact_id)
            .expect("linkage issue text");
        assert!(text.contains("fan-out linkage issue: blocked"));
        assert!(text.contains("fan_in_summary_artifact_id=fan-in-1"));
        assert!(text.contains("issue_truncated=true"));
        assert!(text.contains(&format!(
            "fan_out_linkage_issue artifact_id={}",
            artifact_id
        )));
    }

    #[test]
    fn format_fan_out_linkage_issue_from_structured_payload_returns_none_when_issue_blank() {
        let artifact_id = ArtifactId::new();
        let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
            schema_version: "fan_out_linkage_issue.v1".to_string(),
            fan_in_summary_artifact_id: "fan-in-1".to_string(),
            issue: "   ".to_string(),
            issue_truncated: false,
        };

        assert!(
            format_fan_out_linkage_issue_from_structured_payload(&payload, artifact_id).is_none()
        );
    }

    #[test]
    fn format_fan_out_linkage_issue_clear_from_structured_payload_includes_artifact_handles() {
        let artifact_id = ArtifactId::new();
        let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
            schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
            fan_in_summary_artifact_id: "fan-in-1".to_string(),
        };

        let text = format_fan_out_linkage_issue_clear_from_structured_payload(&payload, artifact_id);
        assert!(text.contains("fan-out linkage issue cleared"));
        assert!(text.contains("fan_in_summary_artifact_id=fan-in-1"));
        assert!(text.contains(&format!(
            "fan_out_linkage_issue_clear artifact_id={}",
            artifact_id
        )));
    }

    #[test]
    fn format_fan_out_linkage_issue_clear_from_structured_payload_handles_blank_summary_artifact_id()
    {
        let artifact_id = ArtifactId::new();
        let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
            schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
            fan_in_summary_artifact_id: "   ".to_string(),
        };

        let text = format_fan_out_linkage_issue_clear_from_structured_payload(&payload, artifact_id);
        assert!(text.contains("fan_in_summary_artifact_id=-"));
    }

    #[test]
    fn fan_out_approval_error_contains_actionable_handles() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "process/start".to_string(),
            summary: None,
        };
        let artifact_id = omne_protocol::ArtifactId::new();
        let message = fan_out_approval_error(&issue, artifact_id);
        assert!(message.contains("fan-out task needs approval"));
        assert!(message.contains("approval_id="));
        assert!(message.contains("thread_id="));
        assert!(message.contains("turn_id="));
        assert!(message.contains("omne approval decide"));
        assert!(message.contains(&format!(
            "omne approval decide {} {} --approve",
            issue.thread_id, issue.approval_id
        )));
        assert!(message.contains(&format!(
            "omne approval decide {} {} --deny",
            issue.thread_id, issue.approval_id
        )));
        assert!(!message.contains("--thread-id"));
        assert!(message.contains(&artifact_id.to_string()));
    }

    #[test]
    fn fan_out_approval_error_includes_summary_when_present() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "subagent/proxy_approval".to_string(),
            summary: Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
        };
        let artifact_id = omne_protocol::ArtifactId::new();
        let message = fan_out_approval_error(&issue, artifact_id);
        assert!(message.contains("summary=child_thread_id=abc"));
        assert!(message.contains("path=/tmp/ws/main.rs"));
    }

    #[test]
    fn find_pending_approval_task_from_fan_in_summary_prefers_approval_id_match() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "process/start".to_string(),
            summary: None,
        };
        let payload = omne_app_server_protocol::ArtifactFanInSummaryStructuredData {
            schema_version: "fan_in_summary.v1".to_string(),
            thread_id: ThreadId::new().to_string(),
            task_count: 2,
            scheduling: omne_app_server_protocol::ArtifactFanInSummaryScheduling {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            tasks: vec![
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "t-review".to_string(),
                    title: "title".to_string(),
                    thread_id: Some("child-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    status: "NeedUserInput".to_string(),
                    reason: None,
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: Some(
                        omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                            approval_id: "not-this-one".to_string(),
                            action: "process/start".to_string(),
                            summary: None,
                            approve_cmd: None,
                            deny_cmd: None,
                        },
                    ),
                },
                omne_app_server_protocol::ArtifactFanInSummaryTask {
                    task_id: "other-task".to_string(),
                    title: "title".to_string(),
                    thread_id: Some("child-2".to_string()),
                    turn_id: Some("turn-2".to_string()),
                    status: "NeedUserInput".to_string(),
                    reason: None,
                    dependency_blocked: false,
                    dependency_blocker_task_id: None,
                    dependency_blocker_status: None,
                    result_artifact_id: None,
                    result_artifact_error: None,
                    result_artifact_error_id: None,
                    result_artifact_diagnostics: None,
                    pending_approval: Some(
                        omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                            approval_id: issue.approval_id.to_string(),
                            action: "process/start".to_string(),
                            summary: None,
                            approve_cmd: None,
                            deny_cmd: None,
                        },
                    ),
                },
            ],
        };

        let task = find_pending_approval_task_from_fan_in_summary(&payload, &issue)
            .expect("pending approval task");
        assert_eq!(task.task_id, "other-task");
    }

    #[test]
    fn fan_out_approval_error_from_structured_task_prefers_structured_handles() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "process/start".to_string(),
            summary: Some("from_issue".to_string()),
        };
        let artifact_id = ArtifactId::new();
        let task = omne_app_server_protocol::ArtifactFanInSummaryTask {
            task_id: "t-approval".to_string(),
            title: "approval task".to_string(),
            thread_id: Some("child-thread-1".to_string()),
            turn_id: Some("child-turn-1".to_string()),
            status: "NeedUserInput".to_string(),
            reason: Some("blocked".to_string()),
            dependency_blocked: false,
            dependency_blocker_task_id: None,
            dependency_blocker_status: None,
            result_artifact_id: None,
            result_artifact_error: None,
            result_artifact_error_id: None,
            result_artifact_diagnostics: None,
            pending_approval: Some(omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                approval_id: "approval-1".to_string(),
                action: "subagent/proxy_approval".to_string(),
                summary: Some("child_thread_id=abc".to_string()),
                approve_cmd: Some(
                    "omne approval decide child-thread-1 approval-1 --approve".to_string(),
                ),
                deny_cmd: Some(
                    "omne approval decide child-thread-1 approval-1 --deny".to_string(),
                ),
            }),
        };

        let message = fan_out_approval_error_from_structured_task(&issue, artifact_id, &task);
        assert!(message.contains("task_id=t-approval"));
        assert!(message.contains("thread_id=child-thread-1"));
        assert!(message.contains("turn_id=child-turn-1"));
        assert!(message.contains("approval_id=approval-1"));
        assert!(message.contains("action=subagent/proxy_approval"));
        assert!(message.contains("summary=child_thread_id=abc"));
        assert!(message
            .contains("approve_cmd=`omne approval decide child-thread-1 approval-1 --approve`"));
        assert!(message.contains("deny_cmd=`omne approval decide child-thread-1 approval-1 --deny`"));
    }

    #[test]
    fn fan_out_approval_error_from_structured_task_generates_missing_commands() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "process/start".to_string(),
            summary: None,
        };
        let artifact_id = ArtifactId::new();
        let task = omne_app_server_protocol::ArtifactFanInSummaryTask {
            task_id: "t-review".to_string(),
            title: "approval task".to_string(),
            thread_id: Some("child-thread-2".to_string()),
            turn_id: Some("child-turn-2".to_string()),
            status: "NeedUserInput".to_string(),
            reason: None,
            dependency_blocked: false,
            dependency_blocker_task_id: None,
            dependency_blocker_status: None,
            result_artifact_id: None,
            result_artifact_error: None,
            result_artifact_error_id: None,
            result_artifact_diagnostics: None,
            pending_approval: Some(omne_app_server_protocol::ArtifactFanInSummaryPendingApproval {
                approval_id: "approval-2".to_string(),
                action: "process/start".to_string(),
                summary: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
        };

        let message = fan_out_approval_error_from_structured_task(&issue, artifact_id, &task);
        assert!(message.contains("approve_cmd=`omne approval decide child-thread-2 approval-2 --approve`"));
        assert!(message.contains("deny_cmd=`omne approval decide child-thread-2 approval-2 --deny`"));
    }

    #[test]
    fn pending_approval_task_result_captures_structured_summary() {
        let thread_id = ThreadId::new();
        let turn_id = TurnId::new();
        let approval_id = ApprovalId::new();
        let result = pending_approval_task_result(
            "t-review".to_string(),
            "review task".to_string(),
            thread_id,
            turn_id,
            "subagent/proxy_approval".to_string(),
            approval_id,
            Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
        );

        assert_eq!(result.status, TurnStatus::Interrupted);
        assert_eq!(result.thread_id, Some(thread_id));
        assert_eq!(result.turn_id, Some(turn_id));
        assert!(result.result_artifact_id.is_none());
        assert!(result
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("blocked on approval")));
        let pending = result.pending_approval.expect("pending approval");
        assert_eq!(pending.approval_id, approval_id);
        assert_eq!(pending.action, "subagent/proxy_approval");
        assert!(pending
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("child_thread_id=abc")));
        let expected_approve = format!("omne approval decide {} {} --approve", thread_id, approval_id);
        let expected_deny = format!("omne approval decide {} {} --deny", thread_id, approval_id);
        assert_eq!(
            pending.approve_cmd.as_deref(),
            Some(expected_approve.as_str())
        );
        assert_eq!(
            pending.deny_cmd.as_deref(),
            Some(expected_deny.as_str())
        );
    }

    #[test]
    fn render_fan_out_approval_blocked_markdown_contains_approve_command() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "file_write".to_string(),
            summary: None,
        };
        let text = render_fan_out_approval_blocked_markdown(3, &[], &issue, test_scheduling());
        assert!(text.contains("Status: blocked (need approval)"));
        assert!(text.contains("Progress: 0/3"));
        assert!(text.contains(&issue.approval_id.to_string()));
        assert!(text.contains(&issue.thread_id.to_string()));
        assert!(text.contains("omne approval decide"));
        assert!(text.contains(&format!(
            "omne approval decide {} {} --deny",
            issue.thread_id, issue.approval_id
        )));
        assert!(text.contains("## Scheduling"));
        assert!(text.contains("env_max_concurrent_subagents: `4`"));
        assert!(text.contains("effective_concurrency_limit: `3`"));
        assert!(text.contains("priority_aging_rounds: `5`"));
    }

    #[test]
    fn render_fan_out_approval_blocked_markdown_includes_failed_task_quick_read() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "file_write".to_string(),
            summary: None,
        };
        let failed_thread_id = ThreadId::new();
        let failed_artifact_id = ArtifactId::new();
        let finished = vec![WorkflowTaskResult {
            task_id: "t-failed".to_string(),
            title: "failed task".to_string(),
            thread_id: Some(failed_thread_id),
            turn_id: Some(TurnId::new()),
            result_artifact_id: Some(failed_artifact_id),
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Failed,
            reason: Some("boom".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        }];

        let text =
            render_fan_out_approval_blocked_markdown(3, &finished, &issue, test_scheduling());
        assert!(text.contains("Failed task quick reads:"));
        assert!(text.contains("t-failed"));
        assert!(text.contains(&format!(
            "omne artifact read {} {}",
            failed_thread_id, failed_artifact_id
        )));
    }

    #[test]
    fn render_fan_out_result_markdown_includes_status_and_output() {
        let turn_id = TurnId::new();
        let text = render_fan_out_result_markdown(
            "t-review",
            "Review API",
            turn_id,
            TurnStatus::Completed,
            Some("all checks passed"),
            Some("result body"),
        );
        assert!(text.contains("# Fan-out Result"));
        assert!(text.contains("task_id: `t-review`"));
        assert!(text.contains(&turn_id.to_string()));
        assert!(text.contains("status: `Completed`"));
        assert!(text.contains("all checks passed"));
        assert!(text.contains("result body"));
    }

    #[test]
    fn render_fan_out_progress_markdown_includes_scheduling_section() {
        let finished = vec![WorkflowTaskResult {
            task_id: "t-done".to_string(),
            title: "done task".to_string(),
            thread_id: Some(ThreadId::new()),
            turn_id: Some(TurnId::new()),
            result_artifact_id: Some(ArtifactId::new()),
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Completed,
            reason: None,
            dependency_blocked: false,
            assistant_text: Some("ok".to_string()),
            pending_approval: None,
        }];
        let active = vec!["t-active"];
        let text = render_fan_out_progress_markdown(
            2,
            &finished,
            &active,
            std::time::Duration::from_secs(12),
            test_scheduling(),
        );
        assert!(text.contains("# Fan-out Progress"));
        assert!(text.contains("Progress: 1/2"));
        assert!(text.contains("## Scheduling"));
        assert!(text.contains("env_max_concurrent_subagents: `4`"));
        assert!(text.contains("effective_concurrency_limit: `3`"));
        assert!(text.contains("priority_aging_rounds: `5`"));
    }

    #[test]
    fn render_fan_in_summary_markdown_includes_scheduling_section() {
        let thread_id = ThreadId::new();
        let results = vec![WorkflowTaskResult {
            task_id: "t-summary".to_string(),
            title: "summary task".to_string(),
            thread_id: Some(thread_id),
            turn_id: Some(TurnId::new()),
            result_artifact_id: Some(ArtifactId::new()),
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Completed,
            reason: None,
            dependency_blocked: false,
            assistant_text: Some("all good".to_string()),
            pending_approval: None,
        }];
        let text = render_fan_in_summary_markdown(thread_id, &results, test_scheduling(), None);
        assert!(text.contains("# Fan-in Summary"));
        assert!(text.contains("Tasks: 1"));
        assert!(text.contains("## Scheduling"));
        assert!(text.contains("env_max_concurrent_subagents: `4`"));
        assert!(text.contains("effective_concurrency_limit: `3`"));
        assert!(text.contains("priority_aging_rounds: `5`"));
    }

    #[test]
    fn render_fan_in_summary_markdown_structured_data_includes_dependency_blocker_fields() {
        let thread_id = ThreadId::new();
        let results = vec![WorkflowTaskResult {
            task_id: "t-blocked".to_string(),
            title: "blocked task".to_string(),
            thread_id: None,
            turn_id: None,
            result_artifact_id: None,
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Cancelled,
            reason: Some("blocked by dependency: t-upstream status=Failed".to_string()),
            dependency_blocked: true,
            assistant_text: None,
            pending_approval: None,
        }];
        let text = render_fan_in_summary_markdown(thread_id, &results, test_scheduling(), None);
        assert!(text.contains("- dependency_blocked: true"));
        assert!(text.contains("- dependency_blocker_task_id: t-upstream"));
        assert!(text.contains("- dependency_blocker_status: Failed"));
        assert!(text.contains("\"dependency_blocked\": true"));
        assert!(text.contains("\"dependency_blocker_task_id\": \"t-upstream\""));
        assert!(text.contains("\"dependency_blocker_status\": \"Failed\""));
    }

    #[test]
    fn render_fan_in_summary_markdown_includes_pending_approval_summary() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let turn_id = TurnId::new();
        let approval_id = ApprovalId::new();
        let summary = "child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs";
        let results = vec![WorkflowTaskResult {
            task_id: "t-blocked".to_string(),
            title: "blocked task".to_string(),
            thread_id: Some(child_thread_id),
            turn_id: Some(turn_id),
            result_artifact_id: None,
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Interrupted,
            reason: Some("blocked on approval".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: Some(WorkflowPendingApproval {
                approval_id,
                action: "subagent/proxy_approval".to_string(),
                summary: Some(summary.to_string()),
                approve_cmd: Some(format!(
                    "omne approval decide {} {} --approve",
                    child_thread_id, approval_id
                )),
                deny_cmd: Some(format!(
                    "omne approval decide {} {} --deny",
                    child_thread_id, approval_id
                )),
            }),
        }];
        let text = render_fan_in_summary_markdown(
            parent_thread_id,
            &results,
            test_scheduling(),
            None,
        );
        assert!(text.contains("pending_approval: action=subagent/proxy_approval"));
        assert!(text.contains(&format!("approval_id={approval_id}")));
        assert!(text.contains(summary));
        assert!(text.contains(&format!(
            "approve_cmd: `omne approval decide {} {} --approve`",
            child_thread_id, approval_id
        )));
        assert!(text.contains(&format!(
            "deny_cmd: `omne approval decide {} {} --deny`",
            child_thread_id, approval_id
        )));
        assert!(text.contains("## Structured Data"));
        assert!(text.contains("\"schema_version\": \"fan_in_summary.v1\""));
        assert!(text.contains(&format!(
            "\"thread_id\": \"{}\"",
            parent_thread_id
        )));
        assert!(text.contains("\"task_count\": 1"));
        assert!(text.contains("\"scheduling\""));
        assert!(text.contains("\"env_max_concurrent_subagents\": 4"));
        assert!(text.contains("\"effective_concurrency_limit\": 3"));
        assert!(text.contains("\"priority_aging_rounds\": 5"));
        assert!(text.contains("\"pending_approval\""));
        assert!(text.contains(&format!(
            "\"approve_cmd\": \"omne approval decide {} {} --approve\"",
            child_thread_id, approval_id
        )));
        assert!(text.contains(&format!(
            "\"deny_cmd\": \"omne approval decide {} {} --deny\"",
            child_thread_id, approval_id
        )));
    }

    #[test]
    fn render_fan_in_summary_markdown_structured_data_falls_back_to_generated_approval_commands() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let approval_id = ApprovalId::new();
        let results = vec![WorkflowTaskResult {
            task_id: "t-blocked".to_string(),
            title: "blocked task".to_string(),
            thread_id: Some(child_thread_id),
            turn_id: Some(TurnId::new()),
            result_artifact_id: None,
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Interrupted,
            reason: Some("blocked on approval".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: Some(WorkflowPendingApproval {
                approval_id,
                action: "subagent/proxy_approval".to_string(),
                summary: None,
                approve_cmd: None,
                deny_cmd: None,
            }),
        }];
        let text = render_fan_in_summary_markdown(
            parent_thread_id,
            &results,
            test_scheduling(),
            None,
        );
        assert!(text.contains("## Structured Data"));
        assert!(text.contains("\"schema_version\": \"fan_in_summary.v1\""));
        assert!(text.contains(&format!(
            "\"thread_id\": \"{}\"",
            parent_thread_id
        )));
        assert!(text.contains(&format!(
            "\"approve_cmd\": \"omne approval decide {} {} --approve\"",
            child_thread_id, approval_id
        )));
        assert!(text.contains(&format!(
            "\"deny_cmd\": \"omne approval decide {} {} --deny\"",
            child_thread_id, approval_id
        )));
    }

    #[test]
    fn fan_out_result_read_command_uses_artifact_read_cli() {
        let thread_id = ThreadId::new();
        let artifact_id = ArtifactId::new();
        let command = fan_out_result_read_command(thread_id, artifact_id);
        assert_eq!(
            command,
            format!("omne artifact read {} {}", thread_id, artifact_id)
        );
    }

    #[test]
    fn render_fan_out_result_error_markdown_includes_context_fields() {
        let child_thread_id = ThreadId::new();
        let turn_id = TurnId::new();
        let text = render_fan_out_result_error_markdown(
            "t-review",
            "Review API",
            child_thread_id,
            turn_id,
            TurnStatus::Failed,
            Some("subagent failed"),
            "artifact/write rpc failed: timeout",
        );
        assert!(text.contains("# Fan-out Result Artifact Error"));
        assert!(text.contains("task_id: `t-review`"));
        assert!(text.contains("child_thread_id"));
        assert!(text.contains(&child_thread_id.to_string()));
        assert!(text.contains(&turn_id.to_string()));
        assert!(text.contains("status: `Failed`"));
        assert!(text.contains("subagent failed"));
        assert!(text.contains("artifact/write rpc failed"));
    }

    #[test]
    fn render_fan_out_approval_blocked_markdown_includes_artifact_error_column() {
        let issue = FanOutApprovalIssue {
            task_id: "t-review".to_string(),
            thread_id: ThreadId::new(),
            turn_id: TurnId::new(),
            approval_id: ApprovalId::new(),
            action: "file_write".to_string(),
            summary: Some("child_thread_id=abc child_approval_id=def | path=/tmp/ws/main.rs".to_string()),
        };
        let finished = vec![WorkflowTaskResult {
            task_id: "t-failed".to_string(),
            title: "failed task".to_string(),
            thread_id: Some(ThreadId::new()),
            turn_id: Some(TurnId::new()),
            result_artifact_id: None,
            result_artifact_error: Some("artifact/write rejected: denied".to_string()),
            result_artifact_error_id: Some(ArtifactId::new()),
            status: TurnStatus::Failed,
            reason: Some("boom".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        }];

        let text =
            render_fan_out_approval_blocked_markdown(3, &finished, &issue, test_scheduling());
        assert!(text.contains("artifact_error="));
        assert!(text.contains("artifact/write rejected: denied"));
        assert!(text.contains("summary"));
        assert!(text.contains("child_thread_id=abc"));
    }

    #[test]
    fn collect_failed_task_error_reads_returns_parent_thread_commands() {
        let parent_thread_id = ThreadId::new();
        let error_artifact_id_a = ArtifactId::new();
        let error_artifact_id_b = ArtifactId::new();
        let mut results = vec![
            WorkflowTaskResult {
                task_id: "t-b".to_string(),
                title: "failed task b".to_string(),
                thread_id: Some(ThreadId::new()),
                turn_id: Some(TurnId::new()),
                result_artifact_id: None,
                result_artifact_error: Some("failed to write".to_string()),
                result_artifact_error_id: Some(error_artifact_id_b),
                status: TurnStatus::Failed,
                reason: Some("boom".to_string()),
                dependency_blocked: false,
                assistant_text: None,
                pending_approval: None,
            },
            WorkflowTaskResult {
                task_id: "t-a".to_string(),
                title: "failed task a".to_string(),
                thread_id: Some(ThreadId::new()),
                turn_id: Some(TurnId::new()),
                result_artifact_id: None,
                result_artifact_error: Some("failed to write".to_string()),
                result_artifact_error_id: Some(error_artifact_id_a),
                status: TurnStatus::Failed,
                reason: Some("boom".to_string()),
                dependency_blocked: false,
                assistant_text: None,
                pending_approval: None,
            },
        ];
        results.push(results[1].clone());

        let reads = collect_failed_task_error_reads(parent_thread_id, &results);
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].0, "t-a");
        assert_eq!(
            reads[0].1,
            format!("omne artifact read {} {}", parent_thread_id, error_artifact_id_a)
        );
        assert_eq!(reads[1].0, "t-b");
        assert_eq!(
            reads[1].1,
            format!("omne artifact read {} {}", parent_thread_id, error_artifact_id_b)
        );
    }

    #[test]
    fn collect_failed_task_reads_returns_sorted_unique_commands() {
        let thread_id_a = ThreadId::new();
        let thread_id_b = ThreadId::new();
        let artifact_id_a = ArtifactId::new();
        let artifact_id_b = ArtifactId::new();

        let mut results = vec![
            WorkflowTaskResult {
                task_id: "t-b".to_string(),
                title: "failed task b".to_string(),
                thread_id: Some(thread_id_b),
                turn_id: Some(TurnId::new()),
                result_artifact_id: Some(artifact_id_b),
                result_artifact_error: None,
                result_artifact_error_id: None,
                status: TurnStatus::Failed,
                reason: Some("boom".to_string()),
                dependency_blocked: false,
                assistant_text: None,
                pending_approval: None,
            },
            WorkflowTaskResult {
                task_id: "t-a".to_string(),
                title: "failed task a".to_string(),
                thread_id: Some(thread_id_a),
                turn_id: Some(TurnId::new()),
                result_artifact_id: Some(artifact_id_a),
                result_artifact_error: None,
                result_artifact_error_id: None,
                status: TurnStatus::Failed,
                reason: Some("boom".to_string()),
                dependency_blocked: false,
                assistant_text: None,
                pending_approval: None,
            },
        ];
        results.push(results[1].clone());

        let reads = collect_failed_task_reads(&results);
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].0, "t-a");
        assert_eq!(
            reads[0].1,
            format!("omne artifact read {} {}", thread_id_a, artifact_id_a)
        );
        assert_eq!(reads[1].0, "t-b");
        assert_eq!(
            reads[1].1,
            format!("omne artifact read {} {}", thread_id_b, artifact_id_b)
        );
    }

    #[test]
    fn append_fan_out_linkage_issue_markdown_behaves_for_present_and_empty_values() {
        let mut with_issue = String::new();
        append_fan_out_linkage_issue_markdown(
            &mut with_issue,
            Some("fan-out linkage issue: task_id=t1 status=Failed"),
        );
        assert!(with_issue.contains("## Fan-out Linkage Issue"));
        assert!(with_issue.contains("task_id=t1"));

        let mut without_issue = String::new();
        append_fan_out_linkage_issue_markdown(&mut without_issue, Some("   "));
        assert!(without_issue.is_empty());
    }

    #[test]
    fn fan_out_linkage_issue_artifact_write_params_use_expected_type() {
        let parent_thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_artifact_write_params(
            parent_thread_id,
            Some(parent_turn_id),
            fan_in_artifact_id,
            "fan-out linkage issue: task_id=t1 status=Failed",
        )
        .expect("params");
        let params = artifact_write_params_json(params);

        assert_eq!(
            params["artifact_type"].as_str(),
            Some("fan_out_linkage_issue")
        );
        assert_eq!(params["summary"].as_str(), Some("fan-out linkage issue"));
        let parent_thread_id_s = parent_thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(parent_thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        assert!(
            params["text"]
                .as_str()
                .is_some_and(|text| text.contains(&fan_in_artifact_id.to_string()))
        );
        assert!(
            params["text"].as_str().is_some_and(|text| text.contains("## Structured Data"))
        );
        assert!(params["text"]
            .as_str()
            .is_some_and(|text| text.contains("\"schema_version\": \"fan_out_linkage_issue.v1\"")));
        assert!(
            fan_out_linkage_issue_artifact_write_params(
                parent_thread_id,
                None,
                fan_in_artifact_id,
                "  ",
            )
            .is_none()
        );
    }

    #[test]
    fn fan_out_linkage_issue_artifact_write_params_allow_null_turn_id() {
        let parent_thread_id = ThreadId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_artifact_write_params(
            parent_thread_id,
            None,
            fan_in_artifact_id,
            "fan-out linkage issue",
        )
        .expect("params");
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_out_linkage_issue_clear_artifact_write_params_use_clear_type() {
        let parent_thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_clear_artifact_write_params(
            parent_thread_id,
            Some(parent_turn_id),
            fan_in_artifact_id,
        );
        let params = artifact_write_params_json(params);

        assert_eq!(
            params["artifact_type"].as_str(),
            Some("fan_out_linkage_issue_clear")
        );
        assert_eq!(
            params["summary"].as_str(),
            Some("fan-out linkage issue cleared")
        );
        let parent_thread_id_s = parent_thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(parent_thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        assert!(
            params["text"]
                .as_str()
                .is_some_and(|text| text.contains(&fan_in_artifact_id.to_string()))
        );
        assert!(params["text"]
            .as_str()
            .is_some_and(|text| text.contains("## Structured Data")));
        assert!(params["text"].as_str().is_some_and(|text| {
            text.contains("\"schema_version\": \"fan_out_linkage_issue_clear.v1\"")
        }));
    }

    #[test]
    fn fan_out_linkage_issue_clear_artifact_write_params_allow_null_turn_id() {
        let parent_thread_id = ThreadId::new();
        let fan_in_artifact_id = ArtifactId::new();
        let params = fan_out_linkage_issue_clear_artifact_write_params(
            parent_thread_id,
            None,
            fan_in_artifact_id,
        );
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_out_result_error_artifact_write_params_include_parent_turn_id_when_present() {
        let parent_thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let params = fan_out_result_error_artifact_write_params(
            parent_thread_id,
            Some(parent_turn_id),
            "fan-out result artifact write failed: t1".to_string(),
            "error body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert_eq!(
            params["artifact_type"].as_str(),
            Some("fan_out_result_error")
        );
        let parent_thread_id_s = parent_thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(parent_thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        assert_eq!(
            params["summary"].as_str(),
            Some("fan-out result artifact write failed: t1")
        );
        assert_eq!(params["text"].as_str(), Some("error body"));
    }

    #[test]
    fn fan_out_result_error_artifact_write_params_allow_null_turn_id() {
        let params = fan_out_result_error_artifact_write_params(
            ThreadId::new(),
            None,
            "fan-out result artifact write failed: t1".to_string(),
            "error body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_in_summary_artifact_write_params_include_parent_turn_id_when_present() {
        let thread_id = ThreadId::new();
        let parent_turn_id = TurnId::new();
        let artifact_id = ArtifactId::new();
        let params = fan_in_summary_artifact_write_params(
            thread_id,
            Some(parent_turn_id),
            artifact_id,
            "summary body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert_eq!(params["artifact_type"].as_str(), Some("fan_in_summary"));
        assert_eq!(params["summary"].as_str(), Some("fan-in summary"));
        let thread_id_s = thread_id.to_string();
        assert_eq!(params["thread_id"].as_str(), Some(thread_id_s.as_str()));
        let parent_turn_id_s = parent_turn_id.to_string();
        assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        let artifact_id_s = artifact_id.to_string();
        assert_eq!(params["artifact_id"].as_str(), Some(artifact_id_s.as_str()));
        assert_eq!(params["text"].as_str(), Some("summary body"));
    }

    #[test]
    fn fan_in_summary_artifact_write_params_use_null_turn_id_when_absent() {
        let thread_id = ThreadId::new();
        let artifact_id = ArtifactId::new();
        let params = fan_in_summary_artifact_write_params(
            thread_id,
            None,
            artifact_id,
            "summary body".to_string(),
        );
        let params = artifact_write_params_json(params);

        assert!(params["turn_id"].is_null());
    }

    #[test]
    fn fan_in_related_artifact_write_params_keep_parent_turn_id_consistent_across_types() {
        let thread_id = ThreadId::new();
        let thread_id_s = thread_id.to_string();
        let artifact_id = ArtifactId::new();
        let artifact_id_s = artifact_id.to_string();
        let parent_turn_id = TurnId::new();
        let parent_turn_id_s = parent_turn_id.to_string();

        let summary_params = artifact_write_params_json(fan_in_summary_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
            "summary body".to_string(),
        ));
        let progress_params = artifact_write_params_json(fan_in_summary_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
            "progress body".to_string(),
        ));
        let linkage_issue_params = fan_out_linkage_issue_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
            "fan-out linkage issue",
        )
        .expect("linkage issue params");
        let linkage_issue_params = artifact_write_params_json(linkage_issue_params);
        let linkage_clear_params = artifact_write_params_json(
            fan_out_linkage_issue_clear_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            artifact_id,
        ));
        let result_error_params = artifact_write_params_json(
            fan_out_result_error_artifact_write_params(
            thread_id,
            Some(
                parent_turn_id_s
                    .parse::<TurnId>()
                    .expect("parent turn id should parse"),
            ),
            "fan-out result artifact write failed: t1".to_string(),
            "error body".to_string(),
        ));

        for params in [
            &summary_params,
            &progress_params,
            &linkage_issue_params,
            &linkage_clear_params,
            &result_error_params,
        ] {
            assert_eq!(params["thread_id"].as_str(), Some(thread_id_s.as_str()));
            assert_eq!(params["turn_id"].as_str(), Some(parent_turn_id_s.as_str()));
        }
        assert_eq!(summary_params["artifact_id"].as_str(), Some(artifact_id_s.as_str()));
        assert_eq!(progress_params["artifact_id"].as_str(), Some(artifact_id_s.as_str()));
        assert!(linkage_issue_params["artifact_id"].is_null());
        assert!(linkage_clear_params["artifact_id"].is_null());
    }
}
