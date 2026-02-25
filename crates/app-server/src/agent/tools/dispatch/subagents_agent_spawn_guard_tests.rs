#[cfg(test)]
mod agent_spawn_guard_tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::Path;
    use tokio::time::{Duration, Instant};

    #[tokio::test]
    async fn isolated_workspace_copy_skips_runtime_dirs() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let omne_root = tmp.path().join(".omne_data");
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join("src")).await?;
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/threads")).await?;
        tokio::fs::write(repo_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::write(repo_dir.join("src/lib.rs"), "pub fn demo() {}\n").await?;
        tokio::fs::write(repo_dir.join(".omne_data/threads/skip.txt"), "skip\n").await?;

        let server = crate::build_test_server_shared(omne_root.clone());
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = handle.thread_id();
        drop(handle);

        let isolated_root =
            prepare_isolated_workspace(&server, parent_thread_id, "t1", &repo_dir).await?;
        let isolated_root = Path::new(&isolated_root);
        assert!(isolated_root.join("hello.txt").exists());
        assert!(isolated_root.join("src/lib.rs").exists());
        assert!(!isolated_root.join(".omne_data/threads/skip.txt").exists());
        assert!(
            isolated_root.starts_with(
                omne_root
                    .join(".omne_data")
                    .join("tmp")
                    .join("subagents")
                    .join(parent_thread_id.to_string())
            )
        );
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_denies_disallowed_child_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "coder",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("mode_permission"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(false));
        assert!(result["allowed_modes"].as_array().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_denied_by_tool_override_reports_decision_source() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with explicit spawn deny override"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
    tool_overrides:
      - tool: "subagent/spawn"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["decision_source"].as_str(), Some("tool_override"));
        assert_eq!(result["tool_override_hit"].as_bool(), Some(true));
        assert_eq!(result["decision"].as_str(), Some("deny"));
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_enforces_default_max_concurrent_subagents() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(omne_protocol::ThreadEventKind::TurnStarted {
                    turn_id: omne_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    directives: None,
                    priority: omne_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = omne_protocol::ToolId::new();
            parent
                .append(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["priority_aging_rounds"].as_u64(), Some(3));
        assert_eq!(result["limit_source"].as_str(), Some("env"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_enforces_mode_max_concurrent_subagents() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with strict subagent limit"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 1
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
        let child_id = child.thread_id();
        child
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: omne_protocol::TurnId::new(),
                input: "child".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Foreground,
            })
            .await?;
        drop(child);

        let tool_id = omne_protocol::ToolId::new();
        parent
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id: None,
                tool: "subagent/spawn".to_string(),
                params: None,
            })
            .await?;
        parent
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "thread_id": child_id,
                })),
            })
            .await?;
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["limit_source"].as_str(), Some("combined"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(
            result["mode_max_concurrent_subagents"]
                .as_u64()
                .unwrap_or(0),
            1
        );
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 1);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 1);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_mode_max_threads_zero_falls_back_to_env_limit() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with unlimited mode max"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 0
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(omne_protocol::ThreadEventKind::TurnStarted {
                    turn_id: omne_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    directives: None,
                    priority: omne_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = omne_protocol::ToolId::new();
            parent
                .append(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["limit_source"].as_str(), Some("env"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(result["env_max_concurrent_subagents"].as_u64(), Some(4));
        assert_eq!(result["mode_max_concurrent_subagents"].as_u64(), Some(0));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_uses_stricter_env_limit_when_mode_limit_is_higher() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with loose mode max"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 10
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(omne_protocol::ThreadEventKind::TurnStarted {
                    turn_id: omne_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    directives: None,
                    priority: omne_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = omne_protocol::ToolId::new();
            parent
                .append(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "thread_id": child_id,
                    })),
                })
                .await?;
        }
        drop(parent);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "tasks": [{
                    "id": "t1",
                    "input": "x",
                    "mode": "reviewer",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["limit_source"].as_str(), Some("combined"));
        assert_eq!(result["limit_policy"].as_str(), Some("min_non_zero"));
        assert_eq!(result["env_max_concurrent_subagents"].as_u64(), Some(4));
        assert_eq!(result["mode_max_concurrent_subagents"].as_u64(), Some(10));
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_priority_defaults_and_overrides_are_reflected_in_preview()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(repo_dir.join(".omne_data/spec")).await?;
        tokio::fs::write(
            repo_dir.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  coder:
    description: "coder with explicit spawn deny override"
    permissions:
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
    tool_overrides:
      - tool: "subagent/spawn"
        decision: deny
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let _result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "agent_spawn",
            serde_json::json!({
                "priority": "low",
                "tasks": [
                    {
                        "id": "t-default",
                        "input": "x",
                    },
                    {
                        "id": "t-high",
                        "input": "y",
                        "priority": "high",
                    }
                ],
            }),
            None,
        )
        .await?;

        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        let params = events
            .into_iter()
            .find_map(|event| match event.kind {
                omne_protocol::ThreadEventKind::ToolStarted { tool, params, .. }
                    if tool == "subagent/spawn" =>
                {
                    params
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing subagent/spawn ToolStarted params"))?;

        assert_eq!(params["default_priority"].as_str(), Some("low"));
        assert_eq!(params["priority_aging_rounds"].as_u64(), Some(3));
        let tasks = params["tasks"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing task previews"))?;
        let mut by_id = std::collections::HashMap::<String, String>::new();
        for task in tasks {
            let id = task.get("id").and_then(Value::as_str).unwrap_or_default();
            let priority = task
                .get("priority")
                .and_then(Value::as_str)
                .unwrap_or_default();
            by_id.insert(id.to_string(), priority.to_string());
        }

        assert_eq!(by_id.get("t-default").map(String::as_str), Some("low"));
        assert_eq!(by_id.get("t-high").map(String::as_str), Some("high"));
        Ok(())
    }

    #[test]
    fn subagent_schedule_blocks_dependents_when_dependency_fails() {
        let parent_thread = ThreadId::new();
        let t1_thread = ThreadId::new();
        let t1_turn = TurnId::new();
        let t2_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t1".to_string(),
                title: "first".to_string(),
                input: "run first".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: t1_thread,
                log_path: "t1.log".to_string(),
                last_seq: 0,
                turn_id: Some(t1_turn),
                status: SubagentTaskStatus::Running,
                error: None,
            },
            SubagentSpawnTask {
                id: "t2".to_string(),
                title: "second".to_string(),
                input: "run second".to_string(),
                depends_on: vec!["t1".to_string()],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: t2_thread,
                log_path: "t2.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        schedule.handle_turn_completed(
            t1_thread,
            t1_turn,
            omne_protocol::TurnStatus::Failed,
            Some("boom".to_string()),
        );

        let t1 = &schedule.tasks[0];
        let t2 = &schedule.tasks[1];
        assert!(matches!(t1.status, SubagentTaskStatus::Failed));
        assert!(matches!(t2.status, SubagentTaskStatus::Failed));
        assert!(
            t2.error
                .as_deref()
                .unwrap_or("")
                .contains("blocked by dependency")
        );
        assert_eq!(
            schedule.task_statuses.get("t2").copied(),
            Some(omne_protocol::TurnStatus::Cancelled)
        );

        let snapshot = schedule.snapshot();
        assert_eq!(snapshot[0]["dependency_blocked"].as_bool(), Some(false));
        assert!(snapshot[0]["dependency_blocker_task_id"].is_null());
        assert!(snapshot[0]["dependency_blocker_status"].is_null());
        assert_eq!(snapshot[1]["dependency_blocked"].as_bool(), Some(true));
        assert_eq!(
            snapshot[1]["dependency_blocker_task_id"].as_str(),
            Some("t1")
        );
        assert_eq!(
            snapshot[1]["dependency_blocker_status"].as_str(),
            Some("Failed")
        );
    }

    #[test]
    fn subagent_schedule_snapshot_marks_non_dependency_failures_as_not_dependency_blocked() {
        let parent_thread = ThreadId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "first".to_string(),
            input: "run first".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: ThreadId::new(),
            log_path: "t1.log".to_string(),
            last_seq: 0,
            turn_id: None,
            status: SubagentTaskStatus::Failed,
            error: Some("turn finished with status=Failed".to_string()),
        }];
        let schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        let snapshot = schedule.snapshot();
        assert_eq!(snapshot[0]["dependency_blocked"].as_bool(), Some(false));
        assert!(snapshot[0]["dependency_blocker_task_id"].is_null());
        assert!(snapshot[0]["dependency_blocker_status"].is_null());
    }

    #[test]
    fn subagent_schedule_snapshot_includes_pending_approval_handles() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        let proxy_approval_id = ApprovalId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        schedule
            .approval_proxy_by_child
            .insert(child_key, proxy_approval_id);
        schedule
            .approval_proxy_targets
            .insert(proxy_approval_id, child_key);
        schedule.set_pending_approval(child_key, child_turn_id, "process/start", proxy_approval_id);

        let snapshot = schedule.snapshot();
        let pending = snapshot[0]["pending_approval"]
            .as_object()
            .expect("missing pending_approval snapshot");
        let approve_cmd = format!(
            "omne approval decide {} {} --approve",
            parent_thread_id, proxy_approval_id
        );
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let child_approval_id_text = child_approval_id.to_string();
        let proxy_approval_id_text = proxy_approval_id.to_string();

        assert_eq!(
            pending.get("action").and_then(Value::as_str),
            Some("subagent/proxy_approval")
        );
        assert_eq!(
            pending.get("approval_id").and_then(Value::as_str),
            Some(proxy_approval_id_text.as_str())
        );
        assert_eq!(
            pending.get("approve_cmd").and_then(Value::as_str),
            Some(approve_cmd.as_str())
        );
        assert_eq!(
            pending.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            pending.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert_eq!(
            pending.get("child_approval_id").and_then(Value::as_str),
            Some(child_approval_id_text.as_str())
        );
        assert_eq!(
            pending.get("child_action").and_then(Value::as_str),
            Some("process/start")
        );
        let summary = pending.get("summary").and_then(Value::as_str).unwrap_or("");
        assert!(summary.contains("child_thread_id="));
        assert!(summary.contains("child_turn_id="));
        assert!(summary.contains("child_approval_id="));
        assert!(summary.contains("child_action=process/start"));
    }

    #[test]
    fn subagent_schedule_snapshot_clears_pending_approval_after_proxy_resolution() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        let proxy_approval_id = ApprovalId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        schedule
            .approval_proxy_by_child
            .insert(child_key, proxy_approval_id);
        schedule
            .approval_proxy_targets
            .insert(proxy_approval_id, child_key);
        schedule.set_pending_approval(child_key, child_turn_id, "process/start", proxy_approval_id);

        schedule.clear_proxy_mapping(proxy_approval_id);
        assert!(schedule.pending_approvals_by_child.is_empty());
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());
        let snapshot = schedule.snapshot();
        assert!(snapshot[0]["pending_approval"].is_null());
    }

    #[test]
    fn subagent_schedule_prefers_high_priority_task_when_multiple_ready() {
        let parent_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-normal".to_string(),
                title: "normal".to_string(),
                input: "run normal".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "normal.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        let idx = schedule.pick_next_ready_task_index();
        assert_eq!(idx.map(|v| schedule.tasks[v].id.as_str()), Some("t-high"));
    }

    #[test]
    fn subagent_schedule_available_slots_saturates_with_running_and_external_threads() {
        let parent_thread = ThreadId::new();
        let running_thread = ThreadId::new();
        let running_turn = TurnId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t-running".to_string(),
            title: "running".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: running_thread,
            log_path: "running.log".to_string(),
            last_seq: 0,
            turn_id: Some(running_turn),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let external = HashSet::from([external_thread]);
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, external, 1, 3);

        // max=1 with one running task + one external reservation should saturate to zero slots.
        assert_eq!(schedule.available_slots(), 0);

        // Completing external work only releases one reservation.
        schedule.handle_turn_completed(
            external_thread,
            TurnId::new(),
            omne_protocol::TurnStatus::Completed,
            None,
        );
        assert_eq!(schedule.available_slots(), 0);

        schedule.handle_turn_completed(
            running_thread,
            running_turn,
            omne_protocol::TurnStatus::Completed,
            None,
        );
        assert_eq!(schedule.available_slots(), 1);
    }

    #[test]
    fn subagent_schedule_external_completion_releases_slot_without_mutating_pending_tasks() {
        let parent_thread = ThreadId::new();
        let pending_thread = ThreadId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![SubagentSpawnTask {
            id: "t-pending".to_string(),
            title: "pending".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: pending_thread,
            log_path: "pending.log".to_string(),
            last_seq: 0,
            turn_id: None,
            status: SubagentTaskStatus::Pending,
            error: None,
        }];
        let external = HashSet::from([external_thread]);
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, external, 1, 3);

        assert_eq!(schedule.available_slots(), 0);
        schedule.handle_turn_completed(
            external_thread,
            TurnId::new(),
            omne_protocol::TurnStatus::Failed,
            Some("external-failed".to_string()),
        );

        assert_eq!(schedule.available_slots(), 1);
        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Pending
        ));
        assert!(schedule.task_statuses.is_empty());
    }

    #[test]
    fn subagent_schedule_priority_aging_can_promote_low_priority_task() {
        let parent_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(parent_thread, tasks, HashSet::new(), 4, 3);
        for _ in 0..6 {
            schedule.update_ready_wait_rounds();
        }
        let idx = schedule.pick_next_ready_task_index();
        assert_eq!(idx.map(|v| schedule.tasks[v].id.as_str()), Some("t-low"));
    }

    #[tokio::test]
    async fn subagent_schedule_no_slots_still_accumulates_ready_wait_rounds() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_thread = ThreadId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(
            parent_thread,
            tasks,
            HashSet::from([external_thread]),
            1,
            3,
        );

        assert_eq!(schedule.available_slots(), 0);
        schedule.start_ready_tasks(&server).await;
        assert_eq!(schedule.ready_wait_rounds.get("t-low").copied(), Some(1));
        assert_eq!(schedule.ready_wait_rounds.get("t-high").copied(), Some(1));
        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Pending
        ));
        assert!(matches!(
            schedule.tasks[1].status,
            SubagentTaskStatus::Pending
        ));

        schedule.start_ready_tasks(&server).await;
        assert_eq!(schedule.ready_wait_rounds.get("t-low").copied(), Some(2));
        assert_eq!(schedule.ready_wait_rounds.get("t-high").copied(), Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_contention_rounds_can_shift_pick_order() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_thread = ThreadId::new();
        let external_thread = ThreadId::new();
        let tasks = vec![
            SubagentSpawnTask {
                id: "t-low".to_string(),
                title: "low".to_string(),
                input: "run low".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Low,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "low.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
            SubagentSpawnTask {
                id: "t-high".to_string(),
                title: "high".to_string(),
                input: "run high".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::High,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: ThreadId::new(),
                log_path: "high.log".to_string(),
                last_seq: 0,
                turn_id: None,
                status: SubagentTaskStatus::Pending,
                error: None,
            },
        ];
        let mut schedule = SubagentSpawnSchedule::new(
            parent_thread,
            tasks,
            HashSet::from([external_thread]),
            1,
            3,
        );

        assert_eq!(schedule.available_slots(), 0);
        for _ in 0..6 {
            schedule.start_ready_tasks(&server).await;
        }
        assert_eq!(
            schedule
                .pick_next_ready_task_index()
                .map(|idx| schedule.tasks[idx].id.as_str()),
            Some("t-low")
        );

        schedule.handle_turn_completed(
            external_thread,
            TurnId::new(),
            omne_protocol::TurnStatus::Completed,
            None,
        );
        assert_eq!(schedule.available_slots(), 1);
        Ok(())
    }

    #[test]
    fn fan_out_priority_aging_rounds_parser_defaults_for_missing_or_invalid_values() {
        assert_eq!(parse_fan_out_priority_aging_rounds_value(None), 3);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("")), 3);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("abc")), 3);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("-1")), 3);
    }

    #[test]
    fn fan_out_priority_aging_rounds_parser_clamps_to_expected_bounds() {
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("0")), 1);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("1")), 1);
        assert_eq!(parse_fan_out_priority_aging_rounds_value(Some("7")), 7);
        assert_eq!(
            parse_fan_out_priority_aging_rounds_value(Some("10000")),
            10_000
        );
        assert_eq!(
            parse_fan_out_priority_aging_rounds_value(Some("10001")),
            10_000
        );
    }

    #[test]
    fn combine_subagent_spawn_limits_covers_all_sources() {
        let unlimited = combine_subagent_spawn_limits(0, None);
        assert_eq!(unlimited.effective, 0);
        assert_eq!(unlimited.source.as_str(), "unlimited");

        let env_only = combine_subagent_spawn_limits(4, None);
        assert_eq!(env_only.effective, 4);
        assert_eq!(env_only.source.as_str(), "env");

        let mode_only = combine_subagent_spawn_limits(0, Some(2));
        assert_eq!(mode_only.effective, 2);
        assert_eq!(mode_only.source.as_str(), "mode");

        let combined = combine_subagent_spawn_limits(4, Some(2));
        assert_eq!(combined.effective, 2);
        assert_eq!(combined.source.as_str(), "combined");
    }

    async fn wait_for_artifact_write_completion_result(
        server: &super::super::Server,
        thread_id: ThreadId,
    ) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let mut artifact_write_tool_ids = std::collections::HashSet::new();
            for event in events {
                match event.kind {
                    omne_protocol::ThreadEventKind::ToolStarted { tool_id, tool, .. } => {
                        if tool == "artifact/write" {
                            artifact_write_tool_ids.insert(tool_id);
                        }
                    }
                    omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status,
                        result,
                        ..
                    } => {
                        if !artifact_write_tool_ids.contains(&tool_id) {
                            continue;
                        }
                        if status == omne_protocol::ToolStatus::Completed {
                            return result.ok_or_else(|| {
                                anyhow::anyhow!("artifact/write ToolCompleted missing result")
                            });
                        }
                        anyhow::bail!(
                            "artifact/write finished with non-completed status={status:?}"
                        );
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for artifact/write ToolCompleted");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_hook_input_by_point(
        server: &super::super::Server,
        thread_id: ThreadId,
        hook_point: &str,
    ) -> anyhow::Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let mut matching_hook_tool_ids =
                std::collections::HashSet::<omne_protocol::ToolId>::new();
            for event in events {
                match event.kind {
                    omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        tool,
                        params,
                        ..
                    } if tool == "hook/run" => {
                        if params
                            .as_ref()
                            .and_then(|params| params.get("hook_point"))
                            .and_then(Value::as_str)
                            == Some(hook_point)
                        {
                            matching_hook_tool_ids.insert(tool_id);
                        }
                    }
                    omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Completed,
                        result: Some(result),
                        ..
                    } if matching_hook_tool_ids.contains(&tool_id) => {
                        let input_path = result
                            .get("input_path")
                            .and_then(Value::as_str)
                            .ok_or_else(|| anyhow::anyhow!("missing input_path in hook result"))?;
                        let input_bytes = tokio::fs::read(input_path).await?;
                        return serde_json::from_slice::<Value>(&input_bytes)
                            .context("parse hook input json");
                    }
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for hook input for point={hook_point}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn assert_no_artifact_write_started_for_duration(
        server: &super::super::Server,
        thread_id: ThreadId,
        duration: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + duration;
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            if events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::ToolStarted { ref tool, .. } if tool == "artifact/write"
                )
            }) {
                anyhow::bail!("unexpected artifact/write ToolStarted event");
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_linkage_marker_set(
        server: &super::super::Server,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerSet {
                        marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                        artifact_id: Some(got_artifact_id),
                        ..
                    } if got_artifact_id == artifact_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerSet(FanOutLinkageIssue) with artifact_id={artifact_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_linkage_marker_cleared(
        server: &super::super::Server,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                        marker: omne_protocol::AttentionMarkerKind::FanOutLinkageIssue,
                        turn_id: Some(got_turn_id),
                        ..
                    } if got_turn_id == turn_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerCleared(FanOutLinkageIssue) turn_id={turn_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_auto_apply_marker_set(
        server: &super::super::Server,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerSet {
                        marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                        artifact_id: Some(got_artifact_id),
                        ..
                    } if got_artifact_id == artifact_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerSet(FanOutAutoApplyError) with artifact_id={artifact_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_fan_out_auto_apply_marker_cleared(
        server: &super::super::Server,
        thread_id: ThreadId,
        turn_id: TurnId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.into_iter().any(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::AttentionMarkerCleared {
                        marker: omne_protocol::AttentionMarkerKind::FanOutAutoApplyError,
                        turn_id: Some(got_turn_id),
                        ..
                    } if got_turn_id == turn_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for AttentionMarkerCleared(FanOutAutoApplyError) turn_id={turn_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn run_git(repo_dir: &std::path::Path, args: &[&str]) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
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

    async fn wait_for_artifact_id_by_type(
        server: &super::super::Server,
        thread_id: ThreadId,
        artifact_type: &str,
    ) -> anyhow::Result<omne_protocol::ArtifactId> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

            let mut seen = std::collections::HashSet::<omne_protocol::ArtifactId>::new();
            for event in events {
                let omne_protocol::ThreadEventKind::ToolCompleted {
                    status: omne_protocol::ToolStatus::Completed,
                    result: Some(result),
                    ..
                } = event.kind
                else {
                    continue;
                };
                let Some(raw_id) = result.get("artifact_id").and_then(Value::as_str) else {
                    continue;
                };
                let Ok(artifact_id) = raw_id.parse::<omne_protocol::ArtifactId>() else {
                    continue;
                };
                if !seen.insert(artifact_id) {
                    continue;
                }
                let read = crate::handle_artifact_read(
                    server,
                    crate::ArtifactReadParams {
                        thread_id,
                        turn_id: None,
                        approval_id: None,
                        artifact_id,
                        version: None,
                        max_bytes: None,
                    },
                )
                .await;
                let Ok(read) = read else {
                    continue;
                };
                let Ok(read) =
                    serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read)
                else {
                    continue;
                };
                if read.metadata.artifact_type == artifact_type {
                    return Ok(read.metadata.artifact_id);
                }
            }

            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for artifact_type={artifact_type}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_for_fan_in_summary_with_task_result_artifact_id(
        server: &super::super::Server,
        thread_id: ThreadId,
        task_id: &str,
    ) -> anyhow::Result<omne_app_server_protocol::ArtifactReadResponse> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

            let mut seen = std::collections::HashSet::<omne_protocol::ArtifactId>::new();
            for event in events {
                let omne_protocol::ThreadEventKind::ToolCompleted {
                    status: omne_protocol::ToolStatus::Completed,
                    result: Some(result),
                    ..
                } = event.kind
                else {
                    continue;
                };
                let Some(raw_id) = result.get("artifact_id").and_then(Value::as_str) else {
                    continue;
                };
                let Ok(artifact_id) = raw_id.parse::<omne_protocol::ArtifactId>() else {
                    continue;
                };
                if !seen.insert(artifact_id) {
                    continue;
                }
                let read = crate::handle_artifact_read(
                    server,
                    crate::ArtifactReadParams {
                        thread_id,
                        turn_id: None,
                        approval_id: None,
                        artifact_id,
                        version: None,
                        max_bytes: None,
                    },
                )
                .await;
                let Ok(read) = read else {
                    continue;
                };
                let Ok(read) =
                    serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read)
                else {
                    continue;
                };
                if read.metadata.artifact_type != "fan_in_summary" {
                    continue;
                }
                let has_result_artifact_id = read
                    .fan_in_summary
                    .as_ref()
                    .map(|payload| {
                        payload.tasks.iter().any(|task| {
                            task.task_id == task_id && task.result_artifact_id.is_some()
                        })
                    })
                    .unwrap_or(false);
                if has_result_artifact_id {
                    return Ok(read);
                }
            }

            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for fan_in_summary with task result_artifact_id task_id={task_id}"
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn parse_fan_in_summary_structured_data_from_text(
        text: &str,
    ) -> anyhow::Result<omne_workflow_spec::FanInSummaryStructuredData> {
        let marker = "## Structured Data";
        let marker_idx = text
            .find(marker)
            .ok_or_else(|| anyhow::anyhow!("missing Structured Data section"))?;
        let after_marker = &text[(marker_idx + marker.len())..];
        let fence_start_rel = after_marker
            .find("```json")
            .ok_or_else(|| anyhow::anyhow!("missing Structured Data json fence start"))?;
        let json_start = marker_idx + marker.len() + fence_start_rel + "```json".len();
        let remainder = &text[json_start..];
        let fence_end_rel = remainder
            .find("```")
            .ok_or_else(|| anyhow::anyhow!("missing Structured Data json fence end"))?;
        let json_block = remainder[..fence_end_rel].trim();
        serde_json::from_str::<omne_workflow_spec::FanInSummaryStructuredData>(json_block)
            .context("parse fan_in_summary structured data json")
    }

    #[tokio::test]
    async fn fan_out_result_writer_writes_artifact_with_expected_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t1".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("boom".to_string()),
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        let artifact_id_raw = completion
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("artifact/write completion missing artifact_id"))?;
        let artifact_id = artifact_id_raw
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse artifact_id {artifact_id_raw}: {err}"))?;

        let read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read).context("parse artifact/read response")?;

        assert_eq!(read.metadata.artifact_type, "fan_out_result");
        assert_eq!(read.metadata.summary, "fan-out result");
        assert!(read.text.contains("\"task_id\": \"t1\""));
        assert!(read.text.contains("\"workspace_mode\": \"read_only\""));
        assert!(read.text.contains("\"workspace_cwd\": null"));
        assert!(read.text.contains("\"isolated_write_patch\": null"));
        assert!(read.text.contains("\"isolated_write_handoff\": null"));
        assert!(read.text.contains("\"status\": \"failed\""));
        assert!(read.text.contains("\"reason\": \"boom\""));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_includes_isolated_write_handoff_payload() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let workspace_cwd = "/tmp/isolated/subagent/repo".to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-isolated".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd.clone()),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        let artifact_id_raw = completion
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("artifact/write completion missing artifact_id"))?;
        let artifact_id = artifact_id_raw
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse artifact_id {artifact_id_raw}: {err}"))?;

        let read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read).context("parse artifact/read response")?;

        assert!(read.text.contains("\"workspace_mode\": \"isolated_write\""));
        assert!(
            read.text
                .contains(&format!("\"workspace_cwd\": \"{workspace_cwd}\""))
        );
        assert!(read.text.contains("\"isolated_write_patch\""));
        assert!(read.text.contains("\"isolated_write_handoff\""));
        assert!(read.text.contains("\"status_argv\""));
        assert!(read.text.contains("\"diff_argv\""));
        assert!(read.text.contains("\"apply_patch_hint\""));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_writes_patch_artifact_for_isolated_workspace()
    -> anyhow::Result<()> {
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

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let workspace_cwd = repo_dir.display().to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-isolated-patch".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let patch_artifact_id = wait_for_artifact_id_by_type(&server, thread_id, "patch").await?;
        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;

        let read_patch = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: patch_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_patch: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_patch).context("parse patch artifact/read response")?;
        assert_eq!(read_patch.metadata.artifact_type, "patch");
        assert!(
            read_patch
                .text
                .contains("diff --git a/hello.txt b/hello.txt")
        );
        assert!(read_patch.text.contains("+world"));

        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        assert_eq!(read_result.metadata.artifact_type, "fan_out_result");
        assert!(read_result.text.contains("\"isolated_write_patch\""));
        assert!(read_result.text.contains("\"artifact_type\": \"patch\""));
        assert!(
            read_result
                .text
                .contains("\"read_cmd\": \"omne artifact read")
        );
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_auto_applies_patch_to_parent_workspace_when_enabled()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;

        run_git(&parent_repo_dir, &["init"]).await?;
        run_git(
            &parent_repo_dir,
            &["config", "user.email", "test@example.com"],
        )
        .await?;
        run_git(&parent_repo_dir, &["config", "user.name", "Test User"]).await?;
        let parent_file_path = parent_repo_dir.join("hello.txt");
        tokio::fs::write(&parent_file_path, "hello\n").await?;
        run_git(&parent_repo_dir, &["add", "hello.txt"]).await?;
        run_git(&parent_repo_dir, &["commit", "-m", "init"]).await?;

        let isolated_repo_dir = tmp.path().join("isolated-repo");
        let parent_repo_dir_text = parent_repo_dir.display().to_string();
        let isolated_repo_dir_text = isolated_repo_dir.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_dir_text.as_str(),
                isolated_repo_dir_text.as_str(),
            ],
        )
        .await?;
        let isolated_file_path = isolated_repo_dir.join("hello.txt");
        tokio::fs::write(&isolated_file_path, "hello\nworld\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server
            .thread_store
            .create_thread(isolated_repo_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer_with_target_workspace(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-auto-apply".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(isolated_repo_dir.display().to_string()),
            Some(parent_repo_dir.display().to_string()),
            true,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        wait_for_fan_out_auto_apply_marker_cleared(&server, thread_id, turn_id).await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        assert!(read_result.text.contains("\"isolated_write_auto_apply\""));
        assert!(read_result.text.contains("\"applied\": true"));

        let parent_file = tokio::fs::read_to_string(&parent_file_path).await?;
        assert!(parent_file.contains("world"));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_auto_applies_untracked_file_to_parent_workspace_when_enabled()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;

        run_git(&parent_repo_dir, &["init"]).await?;
        run_git(
            &parent_repo_dir,
            &["config", "user.email", "test@example.com"],
        )
        .await?;
        run_git(&parent_repo_dir, &["config", "user.name", "Test User"]).await?;
        let parent_file_path = parent_repo_dir.join("hello.txt");
        tokio::fs::write(&parent_file_path, "hello\n").await?;
        run_git(&parent_repo_dir, &["add", "hello.txt"]).await?;
        run_git(&parent_repo_dir, &["commit", "-m", "init"]).await?;

        let isolated_repo_dir = tmp.path().join("isolated-repo");
        let parent_repo_dir_text = parent_repo_dir.display().to_string();
        let isolated_repo_dir_text = isolated_repo_dir.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_dir_text.as_str(),
                isolated_repo_dir_text.as_str(),
            ],
        )
        .await?;
        let new_file_path = isolated_repo_dir.join("new_file.txt");
        tokio::fs::write(&new_file_path, "brand new\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server
            .thread_store
            .create_thread(isolated_repo_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer_with_target_workspace(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-auto-apply-untracked".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(isolated_repo_dir.display().to_string()),
            Some(parent_repo_dir.display().to_string()),
            true,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        wait_for_fan_out_auto_apply_marker_cleared(&server, thread_id, turn_id).await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let auto_apply = payload.isolated_write_auto_apply.ok_or_else(|| {
            anyhow::anyhow!("missing isolated_write_auto_apply structured payload")
        })?;
        assert!(auto_apply.applied);

        let parent_new_file =
            tokio::fs::read_to_string(parent_repo_dir.join("new_file.txt")).await?;
        assert_eq!(parent_new_file, "brand new\n");
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_auto_apply_records_check_patch_failure_stage()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let parent_repo_dir = tmp.path().join("parent-repo");
        tokio::fs::create_dir_all(&parent_repo_dir).await?;

        run_git(&parent_repo_dir, &["init"]).await?;
        run_git(
            &parent_repo_dir,
            &["config", "user.email", "test@example.com"],
        )
        .await?;
        run_git(&parent_repo_dir, &["config", "user.name", "Test User"]).await?;
        let parent_file_path = parent_repo_dir.join("hello.txt");
        tokio::fs::write(&parent_file_path, "hello\nbase\n").await?;
        run_git(&parent_repo_dir, &["add", "hello.txt"]).await?;
        run_git(&parent_repo_dir, &["commit", "-m", "init"]).await?;

        let isolated_repo_dir = tmp.path().join("isolated-repo");
        let parent_repo_dir_text = parent_repo_dir.display().to_string();
        let isolated_repo_dir_text = isolated_repo_dir.display().to_string();
        run_git(
            tmp.path(),
            &[
                "clone",
                parent_repo_dir_text.as_str(),
                isolated_repo_dir_text.as_str(),
            ],
        )
        .await?;

        let isolated_file_path = isolated_repo_dir.join("hello.txt");
        tokio::fs::write(&isolated_file_path, "hello\nchild-change\n").await?;
        tokio::fs::write(&parent_file_path, "hello\nparent-change\n").await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server
            .thread_store
            .create_thread(isolated_repo_dir.clone())
            .await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer_with_target_workspace(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-auto-apply-conflict".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(isolated_repo_dir.display().to_string()),
            Some(parent_repo_dir.display().to_string()),
            true,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        wait_for_fan_out_auto_apply_marker_set(&server, thread_id, result_artifact_id).await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let auto_apply = payload.isolated_write_auto_apply.ok_or_else(|| {
            anyhow::anyhow!("missing isolated_write_auto_apply structured payload")
        })?;
        assert!(auto_apply.enabled);
        assert!(auto_apply.attempted);
        assert!(!auto_apply.applied);
        assert_eq!(
            auto_apply.failure_stage,
            Some(
                omne_app_server_protocol::ArtifactFanOutResultIsolatedWriteAutoApplyFailureStage::CheckPatch,
            )
        );
        assert!(
            auto_apply
                .recovery_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("resolve apply-check conflicts"))
        );
        assert!(
            auto_apply
                .error
                .as_deref()
                .is_some_and(|err| err.contains("git apply --check failed"))
        );
        assert!(
            auto_apply
                .patch_artifact_id
                .as_deref()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(
            auto_apply
                .patch_read_cmd
                .as_deref()
                .is_some_and(|cmd| cmd.contains("omne artifact read"))
        );
        assert!(!auto_apply.recovery_commands.is_empty());
        assert_eq!(auto_apply.recovery_commands[0].label, "read_patch_artifact");
        assert!(
            auto_apply
                .recovery_commands
                .iter()
                .any(|cmd| cmd.label == "apply_with_patch_stdin")
        );

        let parent_file = tokio::fs::read_to_string(&parent_file_path).await?;
        assert_eq!(parent_file, "hello\nparent-change\n");
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_skips_patch_artifact_when_workspace_is_clean()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        run_git(&repo_dir, &["init"]).await?;
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(&repo_dir, &["config", "user.name", "Test User"]).await?;
        tokio::fs::write(repo_dir.join("hello.txt"), "hello\n").await?;
        run_git(&repo_dir, &["add", "hello.txt"]).await?;
        run_git(&repo_dir, &["commit", "-m", "init"]).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let workspace_cwd = repo_dir.display().to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-clean".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        assert!(payload.isolated_write_patch.is_none());

        let list = crate::handle_artifact_list(
            &server,
            crate::ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id: None,
            },
        )
        .await?;
        let list: omne_app_server_protocol::ArtifactListResponse =
            serde_json::from_value(list).context("parse artifact/list response")?;
        assert!(
            list.artifacts
                .iter()
                .all(|meta| meta.artifact_type.as_str() != "patch")
        );
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_records_patch_error_for_invalid_workspace() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        let invalid_workspace = tmp.path().join("missing-workspace");
        let workspace_cwd = invalid_workspace.display().to_string();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-missing".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(workspace_cwd.clone()),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let patch = payload
            .isolated_write_patch
            .ok_or_else(|| anyhow::anyhow!("missing isolated_write_patch payload"))?;
        assert_eq!(patch.workspace_cwd.as_deref(), Some(workspace_cwd.as_str()));
        assert!(
            patch
                .error
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(patch.artifact_id.is_none());
        assert!(patch.read_cmd.is_none());
        let handoff_patch_error = payload
            .isolated_write_handoff
            .as_ref()
            .and_then(|handoff| handoff.patch.as_ref())
            .and_then(|patch| patch.error.as_deref());
        assert!(handoff_patch_error.is_some_and(|value| !value.is_empty()));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_marks_patch_as_truncated_for_large_diff() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        run_git(&repo_dir, &["init"]).await?;
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]).await?;
        run_git(&repo_dir, &["config", "user.name", "Test User"]).await?;
        let file_path = repo_dir.join("large.txt");
        // Keep this deterministic without env overrides by ensuring diff > 64MiB clamp.
        let original = format!("{}\n", "a".repeat(256)).repeat(170_000);
        let modified = format!("{}\n", "b".repeat(256)).repeat(170_000);
        tokio::fs::write(&file_path, original).await?;
        run_git(&repo_dir, &["add", "large.txt"]).await?;
        run_git(&repo_dir, &["commit", "-m", "init"]).await?;
        tokio::fs::write(&file_path, modified).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-large-diff".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::IsolatedWrite,
            Some(repo_dir.display().to_string()),
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let patch_artifact_id = wait_for_artifact_id_by_type(&server, thread_id, "patch").await?;
        let result_artifact_id =
            wait_for_artifact_id_by_type(&server, thread_id, "fan_out_result").await?;

        let read_patch = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: patch_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_patch: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_patch).context("parse patch artifact/read response")?;
        let full_patch = tokio::fs::read_to_string(&read_patch.metadata.content_path)
            .await
            .with_context(|| format!("read {}", read_patch.metadata.content_path))?;
        assert!(full_patch.contains("# <...truncated...>"));

        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result: omne_app_server_protocol::ArtifactReadResponse =
            serde_json::from_value(read_result)
                .context("parse fan_out_result artifact/read response")?;
        let payload = read_result
            .fan_out_result
            .ok_or_else(|| anyhow::anyhow!("missing fan_out_result structured payload"))?;
        let patch = payload
            .isolated_write_patch
            .ok_or_else(|| anyhow::anyhow!("missing isolated_write_patch payload"))?;
        let patch_artifact_id_text = patch_artifact_id.to_string();
        assert_eq!(patch.artifact_type.as_deref(), Some("patch"));
        assert_eq!(
            patch.artifact_id.as_deref(),
            Some(patch_artifact_id_text.as_str())
        );
        assert_eq!(patch.truncated, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_ignores_non_matching_completion_events() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-ignore".to_string(),
            "fan_out_result".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        // Mode/approval setup for the eventual matching write path.
        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let other_thread_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id: ThreadId::new(),
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": other_thread_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);
        assert_no_artifact_write_started_for_duration(
            &server,
            thread_id,
            Duration::from_millis(150),
        )
        .await?;

        let other_turn_event = omne_protocol::ThreadEvent {
            seq: EventSeq(2),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: TurnId::new(),
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": other_turn_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);
        assert_no_artifact_write_started_for_duration(
            &server,
            thread_id,
            Duration::from_millis(150),
        )
        .await?;

        let matching_event = omne_protocol::ThreadEvent {
            seq: EventSeq(3),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": matching_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        assert_eq!(completion["created"].as_bool(), Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_skips_artifact_write_when_type_is_invalid() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-invalid".to_string(),
            String::new(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        assert_no_artifact_write_started_for_duration(
            &server,
            thread_id,
            Duration::from_millis(300),
        )
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_sets_linkage_attention_marker_for_linkage_issue_type()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-linkage".to_string(),
            "fan_out_linkage_issue".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("fan-out linkage issue".to_string()),
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let completion = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        let artifact_id_raw = completion
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("artifact/write completion missing artifact_id"))?;
        let artifact_id = artifact_id_raw
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse artifact_id {artifact_id_raw}: {err}"))?;

        wait_for_fan_out_linkage_marker_set(&server, thread_id, artifact_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn fan_out_result_writer_clears_linkage_attention_marker_for_clear_type()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let handle = server.thread_store.create_thread(repo_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let turn_id = TurnId::new();
        let notify_rx = server.notify_tx.subscribe();
        spawn_fan_out_result_writer(
            server.clone(),
            notify_rx,
            thread_id,
            turn_id,
            "t-linkage-clear".to_string(),
            "fan_out_linkage_issue_clear".to_string(),
            AgentSpawnWorkspaceMode::ReadOnly,
            None,
        );

        let thread_rt = server.get_or_load_thread(thread_id).await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ThreadConfigUpdated {
                approval_policy: omne_protocol::ApprovalPolicy::AutoApprove,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some("coder".to_string()),
                model: None,
                thinking: None,
                openai_base_url: None,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;

        let completion_event = omne_protocol::ThreadEvent {
            seq: EventSeq(1),
            timestamp: time::OffsetDateTime::now_utc(),
            thread_id,
            kind: omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            },
        };
        let line = serde_json::json!({
            "method": "turn/completed",
            "params": completion_event,
        })
        .to_string();
        let _ = server.notify_tx.send(line);

        let _ = wait_for_artifact_write_completion_result(&server, thread_id).await?;
        wait_for_fan_out_linkage_marker_cleared(&server, thread_id, turn_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_bridges_child_approval_to_parent_thread() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let schedule = SubagentSpawnSchedule::new(
            parent_thread_id,
            vec![SubagentSpawnTask {
                id: "t1".to_string(),
                title: "child task".to_string(),
                input: "run".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: child_thread_id,
                log_path: "child.log".to_string(),
                last_seq: 0,
                turn_id: Some(child_turn_id),
                status: SubagentTaskStatus::Running,
                error: None,
            }],
            HashSet::new(),
            4,
            3,
        );
        spawn_subagent_scheduler(server.clone(), schedule);

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;

        let decision = crate::handle_approval_decide(
            &server,
            crate::ApprovalDecideParams {
                thread_id: parent_thread_id,
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved in parent".to_string()),
            },
        )
        .await?;
        let decision: omne_app_server_protocol::ApprovalDecideResponse =
            serde_json::from_value(decision).context("parse approval/decide test response")?;
        assert!(decision.forwarded);
        assert_eq!(decision.child_thread_id, Some(child_thread_id));
        assert_eq!(decision.child_approval_id, Some(child_approval_id));

        wait_for_approval_decided(&server, child_thread_id, child_approval_id).await?;
        wait_for_approval_decided(&server, parent_thread_id, proxy_approval_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catches_up_completed_turn_before_scheduler_start()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.set_env_max_concurrent_subagents(9);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Completed
        ));
        assert_eq!(
            schedule.task_statuses.get("t1").copied(),
            Some(omne_protocol::TurnStatus::Completed)
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_start_ready_tasks_triggers_subagent_start_hook() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  subagent_start:
    - argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: None,
            status: SubagentTaskStatus::Pending,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                schedule.start_ready_tasks(&server).await;
            })
            .await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Running
        ));
        let child_turn_id = schedule.tasks[0]
            .turn_id
            .ok_or_else(|| anyhow::anyhow!("expected child turn_id after schedule start"))?;
        let parent_thread_id_text = parent_thread_id.to_string();
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();

        let hook_input =
            wait_for_hook_input_by_point(&server, parent_thread_id, "subagent_start").await?;
        assert_eq!(
            hook_input.get("thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        let subagent = hook_input
            .get("subagent")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing subagent context"))?;
        assert_eq!(subagent.get("task_id").and_then(Value::as_str), Some("t1"));
        assert_eq!(
            subagent.get("parent_thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert!(subagent.get("status").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_running_events_triggers_subagent_stop_hook()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        let spec_dir = repo_dir.join(".omne_data").join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        tokio::fs::write(
            spec_dir.join("hooks.yaml"),
            r#"
version: 1
hooks:
  subagent_stop:
    - when_turn_status: ["failed"]
      argv: ["sh", "-c", "exit 0"]
      emit_additional_context: false
"#,
        )
        .await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("Bearer super-secret-token-abcdefghijklmnopqrstuvwxyz".to_string()),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.set_env_max_concurrent_subagents(9);
        schedule.catch_up_running_events(&server).await;
        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Failed
        ));

        let parent_thread_id_text = parent_thread_id.to_string();
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let hook_input =
            wait_for_hook_input_by_point(&server, parent_thread_id, "subagent_stop").await?;
        assert_eq!(
            hook_input.get("thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            hook_input.get("turn_status").and_then(Value::as_str),
            Some("failed")
        );
        let subagent = hook_input
            .get("subagent")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow::anyhow!("missing subagent context"))?;
        assert_eq!(subagent.get("task_id").and_then(Value::as_str), Some("t1"));
        assert_eq!(
            subagent.get("parent_thread_id").and_then(Value::as_str),
            Some(parent_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_thread_id").and_then(Value::as_str),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(
            subagent.get("child_turn_id").and_then(Value::as_str),
            Some(child_turn_id_text.as_str())
        );
        assert_eq!(
            subagent.get("status").and_then(Value::as_str),
            Some("failed")
        );
        let reason = subagent
            .get("reason")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing subagent stop reason"))?;
        assert!(reason.contains("Bearer <REDACTED>"));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_running_events_writes_fan_in_summary_artifact()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("child failed".to_string()),
            })
            .await?;
        let patch_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: patch_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "patch",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: patch_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": omne_protocol::ArtifactId::new().to_string(),
                })),
            })
            .await?;
        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        let fan_out_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.set_env_max_concurrent_subagents(9);
        schedule.catch_up_running_events(&server).await;

        let artifact_id =
            wait_for_artifact_id_by_type(&server, parent_thread_id, "fan_in_summary").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read_result)
                .context("parse fan_in_summary artifact/read response")?;
        assert_eq!(read_result.metadata.artifact_type, "fan_in_summary");

        let payload = read_result
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary structured payload"))?;
        assert_eq!(
            payload.schema_version,
            omne_workflow_spec::FAN_IN_SUMMARY_SCHEMA_V1
        );
        assert_eq!(payload.thread_id, parent_thread_id.to_string());
        assert_eq!(payload.task_count, 1);
        assert_eq!(payload.scheduling.env_max_concurrent_subagents, 9);
        assert_eq!(payload.scheduling.effective_concurrency_limit, 4);
        assert_eq!(payload.scheduling.priority_aging_rounds, 3);

        let task = payload
            .tasks
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary task"))?;
        let child_thread_id_text = child_thread_id.to_string();
        let child_turn_id_text = child_turn_id.to_string();
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(task.task_id, "t1");
        assert_eq!(task.status, "Failed");
        assert_eq!(task.reason.as_deref(), Some("child failed"));
        assert_eq!(
            task.thread_id.as_deref(),
            Some(child_thread_id_text.as_str())
        );
        assert_eq!(task.turn_id.as_deref(), Some(child_turn_id_text.as_str()));
        assert!(!task.dependency_blocked);
        assert_eq!(
            task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(task.result_artifact_error.is_none());
        assert!(task.result_artifact_error_id.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_running_events_records_result_error_artifact_id()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("child failed".to_string()),
            })
            .await?;
        let fan_out_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("artifact/write denied: approval required".to_string()),
                result: None,
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        let artifact_id =
            wait_for_artifact_id_by_type(&server, parent_thread_id, "fan_in_summary").await?;
        let read_result = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let read_result =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(read_result)
                .context("parse fan_in_summary artifact/read response")?;
        let payload = read_result
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary structured payload"))?;
        let task = payload
            .tasks
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary task"))?;
        assert!(task.result_artifact_id.is_none());
        let result_error = task
            .result_artifact_error
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_error"))?;
        assert!(result_error.contains("artifact/write denied: approval required"));
        let result_error_id = task
            .result_artifact_error_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_error_id"))?;
        let result_error_artifact_id = result_error_id
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse result_error_artifact_id: {err}"))?;

        let error_read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: result_error_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let error_read =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(error_read)
                .context("parse fan_out_result_error artifact/read response")?;
        assert_eq!(error_read.metadata.artifact_type, "fan_out_result_error");
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_result_artifact_scan_state_handles_incremental_tool_events()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        let fan_out_tool_id = omne_protocol::ToolId::new();
        let tool_started_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let first = schedule.fan_in_summary_structured_data(&server).await;
        let first_task = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in first fan_in_summary"))?;
        assert!(first_task.result_artifact_id.is_none());
        let first_scan_state = schedule
            .result_artifact_scan_state_by_task
            .get("t1")
            .ok_or_else(|| anyhow::anyhow!("missing scan state after first summary"))?;
        assert_eq!(first_scan_state.matching_tool_ids.len(), 1);
        assert!(first_scan_state.matching_tool_ids.contains(&fan_out_tool_id));
        assert!(first_scan_state.last_scanned_seq >= tool_started_event.seq.0);
        let first_scanned_seq = first_scan_state.last_scanned_seq;

        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        let tool_completed_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let second = schedule.fan_in_summary_structured_data(&server).await;
        let second_task = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in second fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            second_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        let second_scan_state = schedule
            .result_artifact_scan_state_by_task
            .get("t1")
            .ok_or_else(|| anyhow::anyhow!("missing scan state after second summary"))?;
        assert!(second_scan_state.matching_tool_ids.is_empty());
        assert_eq!(
            second_scan_state.summary.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(second_scan_state.last_scanned_seq >= tool_completed_event.seq.0);
        assert!(second_scan_state.last_scanned_seq >= first_scanned_seq);
        let second_scanned_seq = second_scan_state.last_scanned_seq;

        let third = schedule.fan_in_summary_structured_data(&server).await;
        let third_task = third
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in third fan_in_summary"))?;
        assert_eq!(
            third_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        let third_scan_state = schedule
            .result_artifact_scan_state_by_task
            .get("t1")
            .ok_or_else(|| anyhow::anyhow!("missing scan state after third summary"))?;
        assert_eq!(third_scan_state.last_scanned_seq, second_scanned_seq);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_result_artifact_success_clears_error_artifact_tracking()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let first_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: first_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: first_tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("artifact/write denied: approval required".to_string()),
                result: None,
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let first = schedule.fan_in_summary_structured_data(&server).await;
        let first_task = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in first fan_in_summary"))?;
        assert!(first_task.result_artifact_id.is_none());
        let first_error_id = first_task
            .result_artifact_error_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_error_id after failed write"))?;
        assert!(
            schedule
                .result_artifact_error_ids_by_task
                .contains_key("t1")
        );

        let second_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: second_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: second_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let second = schedule.fan_in_summary_structured_data(&server).await;
        let second_task = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in second fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            second_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(second_task.result_artifact_error.is_none());
        assert!(second_task.result_artifact_error_id.is_none());
        assert!(!schedule
            .result_artifact_error_ids_by_task
            .contains_key("t1"));

        let first_error_artifact_id = first_error_id
            .parse::<omne_protocol::ArtifactId>()
            .map_err(|err| anyhow::anyhow!("parse first result_artifact_error_id: {err}"))?;
        let first_error_read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: first_error_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let first_error_read =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(
                first_error_read,
            )
            .context("parse first fan_out_result_error artifact/read response")?;
        assert_eq!(first_error_read.metadata.artifact_type, "fan_out_result_error");
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_result_artifact_scan_state_isolated_per_task_under_interleaved_events()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child1_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let child1_thread_id = child1_handle.thread_id();
        let child1_turn_id = TurnId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child1_turn_id,
                input: "child1 task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let mut child2_handle = server.thread_store.create_thread(repo_dir).await?;
        let child2_thread_id = child2_handle.thread_id();
        let child2_turn_id = TurnId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child2_turn_id,
                input: "child2 task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let tasks = vec![
            SubagentSpawnTask {
                id: "t1".to_string(),
                title: "child1 task".to_string(),
                input: "run child1".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: child1_thread_id,
                log_path: "child1.log".to_string(),
                last_seq: 0,
                turn_id: Some(child1_turn_id),
                status: SubagentTaskStatus::Completed,
                error: None,
            },
            SubagentSpawnTask {
                id: "t2".to_string(),
                title: "child2 task".to_string(),
                input: "run child2".to_string(),
                depends_on: vec![],
                priority: AgentSpawnTaskPriority::Normal,
                spawn_mode: AgentSpawnMode::Fork,
                mode: "reviewer".to_string(),
                workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
                model: None,
                openai_base_url: None,
                expected_artifact_type: "fan_out_result".to_string(),
                workspace_cwd: None,
                thread_id: child2_thread_id,
                log_path: "child2.log".to_string(),
                last_seq: 0,
                turn_id: Some(child2_turn_id),
                status: SubagentTaskStatus::Completed,
                error: None,
            },
        ];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        let child1_wave1_tool_id = omne_protocol::ToolId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child1_wave1_tool_id,
                turn_id: Some(child1_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let child2_wave1_tool_id = omne_protocol::ToolId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child2_wave1_tool_id,
                turn_id: Some(child2_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;

        let child1_wave1_artifact_id = omne_protocol::ArtifactId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child1_wave1_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child1_wave1_artifact_id.to_string(),
                })),
            })
            .await?;
        let child2_wave1_artifact_id = omne_protocol::ArtifactId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child2_wave1_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child2_wave1_artifact_id.to_string(),
                })),
            })
            .await?;

        let first = schedule.fan_in_summary_structured_data(&server).await;
        let first_t1 = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in first fan_in_summary"))?;
        let first_t2 = first
            .tasks
            .iter()
            .find(|task| task.task_id == "t2")
            .ok_or_else(|| anyhow::anyhow!("missing t2 in first fan_in_summary"))?;
        let child1_wave1_artifact_id_text = child1_wave1_artifact_id.to_string();
        let child2_wave1_artifact_id_text = child2_wave1_artifact_id.to_string();
        assert_eq!(
            first_t1.result_artifact_id.as_deref(),
            Some(child1_wave1_artifact_id_text.as_str())
        );
        assert_eq!(
            first_t2.result_artifact_id.as_deref(),
            Some(child2_wave1_artifact_id_text.as_str())
        );
        let first_t1_diag = first_t1
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t1 diagnostics after first wave"))?;
        let first_t2_diag = first_t2
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t2 diagnostics after first wave"))?;
        assert_eq!(first_t1_diag.matched_completion_count, 1);
        assert_eq!(first_t2_diag.matched_completion_count, 1);
        assert_eq!(first_t1_diag.pending_matching_tool_ids, 0);
        assert_eq!(first_t2_diag.pending_matching_tool_ids, 0);
        let first_t1_scan_last_seq = first_t1_diag.scan_last_seq;
        let first_t2_scan_last_seq = first_t2_diag.scan_last_seq;

        let child2_wave2_tool_id = omne_protocol::ToolId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child2_wave2_tool_id,
                turn_id: Some(child2_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let child1_wave2_tool_id = omne_protocol::ToolId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: child1_wave2_tool_id,
                turn_id: Some(child1_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;

        let child2_wave2_artifact_id = omne_protocol::ArtifactId::new();
        child2_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child2_wave2_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child2_wave2_artifact_id.to_string(),
                })),
            })
            .await?;
        let child1_wave2_artifact_id = omne_protocol::ArtifactId::new();
        child1_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: child1_wave2_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": child1_wave2_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child1_handle);
        drop(child2_handle);

        let second = schedule.fan_in_summary_structured_data(&server).await;
        let second_t1 = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in second fan_in_summary"))?;
        let second_t2 = second
            .tasks
            .iter()
            .find(|task| task.task_id == "t2")
            .ok_or_else(|| anyhow::anyhow!("missing t2 in second fan_in_summary"))?;
        let child1_wave2_artifact_id_text = child1_wave2_artifact_id.to_string();
        let child2_wave2_artifact_id_text = child2_wave2_artifact_id.to_string();
        assert_eq!(
            second_t1.result_artifact_id.as_deref(),
            Some(child1_wave2_artifact_id_text.as_str())
        );
        assert_eq!(
            second_t2.result_artifact_id.as_deref(),
            Some(child2_wave2_artifact_id_text.as_str())
        );
        let second_t1_diag = second_t1
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t1 diagnostics after second wave"))?;
        let second_t2_diag = second_t2
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing t2 diagnostics after second wave"))?;
        assert_eq!(second_t1_diag.matched_completion_count, 2);
        assert_eq!(second_t2_diag.matched_completion_count, 2);
        assert_eq!(second_t1_diag.pending_matching_tool_ids, 0);
        assert_eq!(second_t2_diag.pending_matching_tool_ids, 0);
        assert!(second_t1_diag.scan_last_seq >= first_t1_scan_last_seq);
        assert!(second_t2_diag.scan_last_seq >= first_t2_scan_last_seq);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_fan_in_artifact_read_clears_result_error_fields_after_success()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;

        let first_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: first_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: first_tool_id,
                status: omne_protocol::ToolStatus::Failed,
                error: Some("artifact/write denied: approval required".to_string()),
                result: None,
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.write_fan_in_summary_artifact_best_effort(&server).await;
        let first_fan_in_artifact_id =
            wait_for_artifact_id_by_type(&server, parent_thread_id, "fan_in_summary").await?;
        let first_read = crate::handle_artifact_read(
            &server,
            crate::ArtifactReadParams {
                thread_id: parent_thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: first_fan_in_artifact_id,
                version: None,
                max_bytes: None,
            },
        )
        .await?;
        let first_read =
            serde_json::from_value::<omne_app_server_protocol::ArtifactReadResponse>(first_read)
                .context("parse first fan_in_summary artifact/read response")?;
        let first_payload = first_read
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing first fan_in_summary payload"))?;
        let first_task = first_payload
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in first fan_in_summary"))?;
        assert!(first_task.result_artifact_id.is_none());
        assert!(first_task.result_artifact_error.is_some());
        assert!(first_task.result_artifact_error_id.is_some());
        let first_structured =
            parse_fan_in_summary_structured_data_from_text(first_read.text.as_str())?;
        let first_structured_task = first_structured
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in first structured fan_in_summary"))?;
        let first_diagnostics = first_structured_task
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_diagnostics in first summary"))?;
        assert_eq!(first_diagnostics.matched_completion_count, 1);
        assert_eq!(first_diagnostics.pending_matching_tool_ids, 0);
        assert!(first_diagnostics.scan_last_seq > 0);

        let second_tool_id = omne_protocol::ToolId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: second_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: second_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        schedule.write_fan_in_summary_artifact_best_effort(&server).await;
        let second_read =
            wait_for_fan_in_summary_with_task_result_artifact_id(&server, parent_thread_id, "t1")
                .await?;
        let second_payload = second_read
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing second fan_in_summary payload"))?;
        let second_task = second_payload
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in second fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            second_task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        assert!(second_task.result_artifact_error.is_none());
        assert!(second_task.result_artifact_error_id.is_none());
        let second_structured =
            parse_fan_in_summary_structured_data_from_text(second_read.text.as_str())?;
        let second_structured_task = second_structured
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing t1 in second structured fan_in_summary"))?;
        let second_diagnostics = second_structured_task
            .result_artifact_diagnostics
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing result_artifact_diagnostics in second summary"))?;
        assert_eq!(second_diagnostics.matched_completion_count, 2);
        assert_eq!(second_diagnostics.pending_matching_tool_ids, 0);
        assert!(second_diagnostics.scan_last_seq >= first_diagnostics.scan_last_seq);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_scheduler_settles_late_result_artifact_notifications_before_exit()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Completed,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        let mut notify_rx = server.notify_tx.subscribe();

        let fan_out_result_artifact_id = omne_protocol::ArtifactId::new();
        let fan_out_tool_id = omne_protocol::ToolId::new();
        let tool_started_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id: fan_out_tool_id,
                turn_id: Some(child_turn_id),
                tool: "artifact/write".to_string(),
                params: Some(serde_json::json!({
                    "artifact_type": "fan_out_result",
                })),
            })
            .await?;
        let tool_completed_event = child_handle
            .append(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id: fan_out_tool_id,
                status: omne_protocol::ToolStatus::Completed,
                error: None,
                result: Some(serde_json::json!({
                    "artifact_id": fan_out_result_artifact_id.to_string(),
                })),
            })
            .await?;
        drop(child_handle);

        let tool_started_line = serde_json::json!({
            "method": "item/started",
            "params": tool_started_event,
        })
        .to_string();
        let _ = server.notify_tx.send(tool_started_line);
        let tool_completed_line = serde_json::json!({
            "method": "item/completed",
            "params": tool_completed_event,
        })
        .to_string();
        let _ = server.notify_tx.send(tool_completed_line);
        schedule
            .settle_late_result_artifacts_before_exit(&server, &mut notify_rx)
            .await;

        let read =
            wait_for_fan_in_summary_with_task_result_artifact_id(&server, parent_thread_id, "t1")
                .await?;
        let payload = read
            .fan_in_summary
            .ok_or_else(|| anyhow::anyhow!("missing fan_in_summary payload"))?;
        let task = payload
            .tasks
            .iter()
            .find(|task| task.task_id == "t1")
            .ok_or_else(|| anyhow::anyhow!("missing task t1 in fan_in_summary"))?;
        let fan_out_result_artifact_id_text = fan_out_result_artifact_id.to_string();
        assert_eq!(
            task.result_artifact_id.as_deref(),
            Some(fan_out_result_artifact_id_text.as_str())
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_ignores_late_approval_after_completion_in_batch()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: ApprovalId::new(),
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Completed
        ));
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        assert!(!parent_events.iter().any(|event| {
            matches!(
                &event.kind,
                omne_protocol::ThreadEventKind::ApprovalRequested { action, .. }
                    if action == "subagent/proxy_approval"
            )
        }));
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_advances_last_seq_for_running_task() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Running
        ));
        assert!(schedule.tasks[0].last_seq >= 2);
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_is_idempotent_for_approval_forwarding() -> anyhow::Result<()>
    {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        let first_last_seq = schedule.tasks[0].last_seq;
        let first_proxy_count = count_proxy_approval_requests(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;
        assert_eq!(first_proxy_count, 1);

        schedule.catch_up_running_events(&server).await;
        let second_last_seq = schedule.tasks[0].last_seq;
        let second_proxy_count = count_proxy_approval_requests(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;
        assert_eq!(second_proxy_count, 1);
        assert_eq!(second_last_seq, first_last_seq);
        assert_eq!(schedule.approval_proxy_by_child.len(), 1);
        assert_eq!(schedule.approval_proxy_targets.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_auto_denies_stale_parent_proxy_approval_on_turn_completion()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Failed,
                reason: Some("child failed".to_string()),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        assert!(matches!(
            schedule.tasks[0].status,
            SubagentTaskStatus::Failed
        ));
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;
        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let decided = parent_events
            .iter()
            .filter_map(|event| {
                let omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    reason,
                    ..
                } = &event.kind
                else {
                    return None;
                };
                if *approval_id != proxy_approval_id {
                    return None;
                }
                Some((*decision, reason.clone()))
            })
            .collect::<Vec<_>>();
        assert_eq!(decided.len(), 1);
        assert_eq!(decided[0].0, omne_protocol::ApprovalDecision::Denied);
        assert!(
            decided[0]
                .1
                .as_deref()
                .unwrap_or_default()
                .starts_with(crate::SUBAGENT_PROXY_AUTO_DENIED_REASON_PREFIX)
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_does_not_override_existing_parent_proxy_decision()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;

        child_handle
            .append(omne_protocol::ThreadEventKind::TurnCompleted {
                turn_id: child_turn_id,
                status: omne_protocol::TurnStatus::Completed,
                reason: None,
            })
            .await?;
        drop(child_handle);

        // Simulate parent decision already persisted before scheduler replays completion.
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("approved from parent".to_string()),
            })
            .await?;

        schedule.catch_up_running_events(&server).await;

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let decided = parent_events
            .iter()
            .filter_map(|event| {
                let omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    ..
                } = &event.kind
                else {
                    return None;
                };
                if *approval_id != proxy_approval_id {
                    return None;
                }
                Some(*decision)
            })
            .collect::<Vec<_>>();
        assert_eq!(decided, vec![omne_protocol::ApprovalDecision::Approved]);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_auto_deny_stale_proxy_is_idempotent_for_duplicate_completion_handling()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);
        schedule.catch_up_running_events(&server).await;

        let proxy_approval_id = wait_for_proxy_request(
            &server,
            parent_thread_id,
            child_thread_id,
            child_approval_id,
        )
        .await?;

        let stale_proxy_approval_ids = schedule.handle_turn_completed(
            child_thread_id,
            child_turn_id,
            omne_protocol::TurnStatus::Failed,
            Some("boom".to_string()),
        );
        schedule
            .auto_deny_stale_parent_proxy_approvals(
                &server,
                child_thread_id,
                child_turn_id,
                omne_protocol::TurnStatus::Failed,
                stale_proxy_approval_ids,
            )
            .await;

        // Duplicate completion handling should be a no-op for proxy auto-deny.
        let stale_proxy_approval_ids = schedule.handle_turn_completed(
            child_thread_id,
            child_turn_id,
            omne_protocol::TurnStatus::Failed,
            Some("boom".to_string()),
        );
        schedule
            .auto_deny_stale_parent_proxy_approvals(
                &server,
                child_thread_id,
                child_turn_id,
                omne_protocol::TurnStatus::Failed,
                stale_proxy_approval_ids,
            )
            .await;

        let parent_events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let denied_count = parent_events
            .into_iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id,
                        decision: omne_protocol::ApprovalDecision::Denied,
                        ..
                    } if approval_id == proxy_approval_id
                )
            })
            .count();
        assert_eq!(denied_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_reuses_existing_pending_parent_proxy_request()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let existing_proxy_approval_id = ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: existing_proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "task_title": "child task",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": { "argv": ["echo", "hi"] },
                    }
                }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        let child_key = SubagentApprovalKey {
            thread_id: child_thread_id,
            approval_id: child_approval_id,
        };
        assert_eq!(
            schedule.approval_proxy_by_child.get(&child_key),
            Some(&existing_proxy_approval_id)
        );
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );
        let snapshot = schedule.snapshot();
        let pending = snapshot[0]["pending_approval"]
            .as_object()
            .expect("missing pending_approval after catch-up");
        let existing_proxy_approval_id_text = existing_proxy_approval_id.to_string();
        assert_eq!(
            pending.get("approval_id").and_then(Value::as_str),
            Some(existing_proxy_approval_id_text.as_str())
        );
        assert_eq!(
            pending.get("action").and_then(Value::as_str),
            Some("subagent/proxy_approval")
        );
        assert_eq!(
            pending.get("child_action").and_then(Value::as_str),
            Some("process/start")
        );

        schedule.catch_up_running_events(&server).await;
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_reconciles_child_with_existing_decided_parent_proxy()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        drop(child_handle);

        let existing_proxy_approval_id = ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: existing_proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "task_title": "child task",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": { "argv": ["echo", "hi"] },
                    }
                }),
            })
            .await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: existing_proxy_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: true,
                reason: Some("approved from parent".to_string()),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: 0,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        wait_for_approval_decided(&server, child_thread_id, child_approval_id).await?;
        assert!(schedule.approval_proxy_by_child.is_empty());
        assert!(schedule.approval_proxy_targets.is_empty());
        assert!(schedule.pending_approvals_by_child.is_empty());
        let snapshot = schedule.snapshot();
        assert!(snapshot[0]["pending_approval"].is_null());
        assert_eq!(
            count_proxy_approval_requests(
                &server,
                parent_thread_id,
                child_thread_id,
                child_approval_id
            )
            .await?,
            1
        );

        schedule.catch_up_running_events(&server).await;

        let child_events = server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {child_thread_id}"))?;
        let child_decisions = child_events
            .iter()
            .filter_map(|event| {
                let omne_protocol::ThreadEventKind::ApprovalDecided {
                    approval_id,
                    decision,
                    remember,
                    reason,
                } = &event.kind
                else {
                    return None;
                };
                if *approval_id != child_approval_id {
                    return None;
                }
                Some((*decision, *remember, reason.clone()))
            })
            .collect::<Vec<_>>();
        assert_eq!(child_decisions.len(), 1);
        assert_eq!(
            child_decisions[0].0,
            omne_protocol::ApprovalDecision::Approved
        );
        assert!(child_decisions[0].1);
        assert!(
            child_decisions[0]
                .2
                .as_deref()
                .unwrap_or_default()
                .starts_with(crate::SUBAGENT_PROXY_FORWARDED_REASON_PREFIX)
        );

        Ok(())
    }

    #[tokio::test]
    async fn subagent_schedule_catch_up_forwards_child_decision_when_request_event_was_already_seen()
    -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = crate::build_test_server_shared(tmp.path().join(".omne_data"));
        let parent_handle = server.thread_store.create_thread(repo_dir.clone()).await?;
        let parent_thread_id = parent_handle.thread_id();
        drop(parent_handle);

        let mut child_handle = server.thread_store.create_thread(repo_dir).await?;
        let child_thread_id = child_handle.thread_id();
        let child_turn_id = TurnId::new();
        let child_approval_id = ApprovalId::new();
        child_handle
            .append(omne_protocol::ThreadEventKind::TurnStarted {
                turn_id: child_turn_id,
                input: "child task".to_string(),
                context_refs: None,
                attachments: None,
                directives: None,
                priority: omne_protocol::TurnPriority::Background,
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: child_approval_id,
                turn_id: Some(child_turn_id),
                action: "process/start".to_string(),
                params: serde_json::json!({ "argv": ["echo", "hi"] }),
            })
            .await?;
        child_handle
            .append(omne_protocol::ThreadEventKind::ApprovalDecided {
                approval_id: child_approval_id,
                decision: omne_protocol::ApprovalDecision::Approved,
                remember: false,
                reason: Some("child approved".to_string()),
            })
            .await?;
        drop(child_handle);

        let child_events = server
            .thread_store
            .read_events_since(child_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {child_thread_id}"))?;
        let request_seq = child_events
            .into_iter()
            .find_map(|event| match event.kind {
                omne_protocol::ThreadEventKind::ApprovalRequested { approval_id, .. }
                    if approval_id == child_approval_id =>
                {
                    Some(event.seq.0)
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("missing child approval requested event"))?;

        let existing_proxy_approval_id = ApprovalId::new();
        let parent_rt = server.get_or_load_thread(parent_thread_id).await?;
        parent_rt
            .append_event(omne_protocol::ThreadEventKind::ApprovalRequested {
                approval_id: existing_proxy_approval_id,
                turn_id: None,
                action: "subagent/proxy_approval".to_string(),
                params: serde_json::json!({
                    "subagent_proxy": {
                        "kind": "approval",
                        "task_id": "t1",
                        "task_title": "child task",
                        "child_thread_id": child_thread_id,
                        "child_turn_id": child_turn_id,
                        "child_approval_id": child_approval_id,
                    },
                    "child_request": {
                        "action": "process/start",
                        "params": { "argv": ["echo", "hi"] },
                    }
                }),
            })
            .await?;

        let tasks = vec![SubagentSpawnTask {
            id: "t1".to_string(),
            title: "child task".to_string(),
            input: "run".to_string(),
            depends_on: vec![],
            priority: AgentSpawnTaskPriority::Normal,
            spawn_mode: AgentSpawnMode::Fork,
            mode: "reviewer".to_string(),
            workspace_mode: AgentSpawnWorkspaceMode::ReadOnly,
            model: None,
            openai_base_url: None,
            expected_artifact_type: "fan_out_result".to_string(),
            workspace_cwd: None,
            thread_id: child_thread_id,
            log_path: "child.log".to_string(),
            last_seq: request_seq,
            turn_id: Some(child_turn_id),
            status: SubagentTaskStatus::Running,
            error: None,
        }];
        let mut schedule =
            SubagentSpawnSchedule::new(parent_thread_id, tasks, HashSet::new(), 4, 3);

        schedule.catch_up_running_events(&server).await;
        assert_eq!(
            count_approval_decisions(&server, parent_thread_id, existing_proxy_approval_id).await?,
            1
        );

        schedule.catch_up_running_events(&server).await;
        assert_eq!(
            count_approval_decisions(&server, parent_thread_id, existing_proxy_approval_id).await?,
            1
        );
        Ok(())
    }

    async fn count_proxy_approval_requests(
        server: &super::super::Server,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        child_approval_id: ApprovalId,
    ) -> anyhow::Result<usize> {
        let events = server
            .thread_store
            .read_events_since(parent_thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
        let child_thread = child_thread_id.to_string();
        let child_approval = child_approval_id.to_string();
        let count = events
            .into_iter()
            .filter(|event| {
                let omne_protocol::ThreadEventKind::ApprovalRequested { action, params, .. } =
                    &event.kind
                else {
                    return false;
                };
                if action != "subagent/proxy_approval" {
                    return false;
                }
                let Some(proxy) = params.get("subagent_proxy") else {
                    return false;
                };
                let proxy_thread = proxy
                    .get("child_thread_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let proxy_approval = proxy
                    .get("child_approval_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                proxy_thread == child_thread && proxy_approval == child_approval
            })
            .count();
        Ok(count)
    }

    async fn count_approval_decisions(
        server: &super::super::Server,
        thread_id: ThreadId,
        approval_id: ApprovalId,
    ) -> anyhow::Result<usize> {
        let events = server
            .thread_store
            .read_events_since(thread_id, EventSeq::ZERO)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
        Ok(events
            .into_iter()
            .filter(|event| {
                matches!(
                    event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id: got, ..
                    } if got == approval_id
                )
            })
            .count())
    }

    async fn wait_for_proxy_request(
        server: &super::super::Server,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
        child_approval_id: ApprovalId,
    ) -> anyhow::Result<ApprovalId> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let child_thread = child_thread_id.to_string();
        let child_approval = child_approval_id.to_string();
        loop {
            let events = server
                .thread_store
                .read_events_since(parent_thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {parent_thread_id}"))?;
            for event in events {
                let omne_protocol::ThreadEventKind::ApprovalRequested {
                    approval_id,
                    action,
                    params,
                    ..
                } = event.kind
                else {
                    continue;
                };
                if action != "subagent/proxy_approval" {
                    continue;
                }
                let Some(proxy) = params.get("subagent_proxy") else {
                    continue;
                };
                let proxy_thread = proxy
                    .get("child_thread_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let proxy_approval = proxy
                    .get("child_approval_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if proxy_thread == child_thread && proxy_approval == child_approval {
                    return Ok(approval_id);
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for parent proxy approval request");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_approval_decided(
        server: &super::super::Server,
        thread_id: ThreadId,
        approval_id: ApprovalId,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = server
                .thread_store
                .read_events_since(thread_id, EventSeq::ZERO)
                .await?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let found = events.iter().any(|event| {
                matches!(
                    &event.kind,
                    omne_protocol::ThreadEventKind::ApprovalDecided {
                        approval_id: got, ..
                    } if *got == approval_id
                )
            });
            if found {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let mut seen = Vec::new();
                for event in &events {
                    match &event.kind {
                        omne_protocol::ThreadEventKind::ApprovalRequested {
                            approval_id,
                            action,
                            ..
                        } => {
                            seen.push(format!("requested id={approval_id} action={action}"));
                        }
                        omne_protocol::ThreadEventKind::ApprovalDecided {
                            approval_id,
                            decision,
                            ..
                        } => {
                            seen.push(format!("decided id={approval_id} decision={decision:?}"));
                        }
                        _ => {}
                    }
                }
                anyhow::bail!(
                    "timed out waiting for approval decided: thread_id={thread_id} approval_id={approval_id} seen={}",
                    seen.join("; ")
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
