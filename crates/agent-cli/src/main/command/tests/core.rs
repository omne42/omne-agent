use super::*;

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
                    structured_error: None,
                    error_code: Some("mode_denied".to_string()),
                    config_path: Some("/tmp/.omne_data/hooks/setup".to_string()),
                    detail: omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                        omne_app_server_protocol::ProcessDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            thread_id,
                            remembered: None,
                            structured_error: None,
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
                    structured_error: None,
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
