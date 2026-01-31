struct SubagentSpawnTaskPlan {
    id: String,
    title: String,
    input: String,
    depends_on: Vec<String>,
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    openai_provider: Option<String>,
    model: Option<String>,
    thinking: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
}

#[derive(Debug, Deserialize)]
struct SpawnedThread {
    thread_id: ThreadId,
    log_path: String,
    last_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubagentTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug)]
struct SubagentSpawnTask {
    id: String,
    title: String,
    input: String,
    depends_on: Vec<String>,
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    openai_provider: Option<String>,
    model: Option<String>,
    thinking: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
    thread_id: ThreadId,
    log_path: String,
    last_seq: u64,
    turn_id: Option<TurnId>,
    status: SubagentTaskStatus,
    error: Option<String>,
}

struct SubagentSpawnSchedule {
    tasks: Vec<SubagentSpawnTask>,
    by_id: std::collections::HashMap<String, usize>,
    completed: std::collections::HashSet<String>,
    running_by_thread: std::collections::HashMap<ThreadId, (String, TurnId)>,
    external_active: std::collections::HashSet<ThreadId>,
    max_concurrent: usize,
}

impl SubagentSpawnSchedule {
    fn new(
        tasks: Vec<SubagentSpawnTask>,
        external_active: std::collections::HashSet<ThreadId>,
        max_concurrent: usize,
    ) -> Self {
        let mut by_id = std::collections::HashMap::<String, usize>::new();
        let mut completed = std::collections::HashSet::<String>::new();
        let mut running_by_thread = std::collections::HashMap::<ThreadId, (String, TurnId)>::new();

        for (idx, task) in tasks.iter().enumerate() {
            by_id.insert(task.id.clone(), idx);
            match task.status {
                SubagentTaskStatus::Completed | SubagentTaskStatus::Failed => {
                    completed.insert(task.id.clone());
                }
                SubagentTaskStatus::Running => {
                    if let Some(turn_id) = task.turn_id {
                        running_by_thread.insert(task.thread_id, (task.id.clone(), turn_id));
                    }
                }
                SubagentTaskStatus::Pending => {}
            }
        }

        Self {
            tasks,
            by_id,
            completed,
            running_by_thread,
            external_active,
            max_concurrent,
        }
    }

    fn is_done(&self) -> bool {
        self.completed.len() >= self.tasks.len()
    }

    fn available_slots(&self) -> usize {
        if self.max_concurrent == 0 {
            usize::MAX
        } else {
            self.max_concurrent
                .saturating_sub(self.running_by_thread.len() + self.external_active.len())
        }
    }

    async fn start_ready_tasks(&mut self, server: &super::Server) {
        let mut available = self.available_slots();
        if available == 0 {
            return;
        }

        for idx in 0..self.tasks.len() {
            if available == 0 {
                break;
            }
            let task = &mut self.tasks[idx];
            if task.status != SubagentTaskStatus::Pending {
                continue;
            }
            if !task
                .depends_on
                .iter()
                .all(|id| self.completed.contains(id))
            {
                continue;
            }

            let task_id = task.id.clone();
            match start_subagent_turn(server, task).await {
                Ok(turn_id) => {
                    task.turn_id = Some(turn_id);
                    task.status = SubagentTaskStatus::Running;
                    self.running_by_thread
                        .insert(task.thread_id, (task_id, turn_id));
                    available = available.saturating_sub(1);
                }
                Err(err) => {
                    task.status = SubagentTaskStatus::Failed;
                    task.error = Some(err.to_string());
                    self.completed.insert(task_id);
                }
            }
        }
    }

    fn handle_turn_completed(&mut self, thread_id: ThreadId, turn_id: TurnId) {
        if self.external_active.remove(&thread_id) {
            return;
        }
        let Some((task_id, expected_turn_id)) = self.running_by_thread.get(&thread_id).cloned()
        else {
            return;
        };
        if expected_turn_id != turn_id {
            return;
        }
        self.running_by_thread.remove(&thread_id);
        if let Some(idx) = self.by_id.get(&task_id).copied() {
            self.tasks[idx].status = SubagentTaskStatus::Completed;
            self.completed.insert(task_id);
        }
    }

    fn snapshot(&self) -> Vec<Value> {
        self.tasks
            .iter()
            .map(|task| {
                serde_json::json!({
                    "id": task.id.clone(),
                    "title": task.title.clone(),
                    "spawn_mode": spawn_mode_label(task.spawn_mode),
                    "mode": task.mode.clone(),
                    "workspace_mode": workspace_mode_label(task.workspace_mode),
                    "thread_id": task.thread_id,
                    "turn_id": task.turn_id,
                    "log_path": task.log_path.clone(),
                    "last_seq": task.last_seq,
                    "depends_on": task.depends_on.clone(),
                    "expected_artifact_type": task.expected_artifact_type.clone(),
                    "openai_provider": task.openai_provider.clone(),
                    "model": task.model.clone(),
                    "thinking": task.thinking.clone(),
                    "openai_base_url": task.openai_base_url.clone(),
                    "status": task_status_label(task.status),
                    "error": task.error.clone(),
                })
            })
            .collect::<Vec<_>>()
    }
}

fn spawn_subagent_scheduler(server: super::Server, mut schedule: SubagentSpawnSchedule) {
    tokio::spawn(async move {
        let mut notify_rx = server.notify_tx.subscribe();
        loop {
            schedule.start_ready_tasks(&server).await;
            if schedule.is_done() {
                return;
            }

            match notify_rx.recv().await {
                Ok(line) => {
                    let Ok(val) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if val.get("method").and_then(Value::as_str) != Some("turn/completed") {
                        continue;
                    }
                    let Some(params) = val.get("params") else {
                        continue;
                    };
                    let Ok(event) = serde_json::from_value::<pm_protocol::ThreadEvent>(params.clone())
                    else {
                        continue;
                    };
                    let pm_protocol::ThreadEventKind::TurnCompleted { turn_id, .. } = event.kind else {
                        continue;
                    };
                    schedule.handle_turn_completed(event.thread_id, turn_id);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

fn spawn_mode_label(mode: AgentSpawnMode) -> &'static str {
    match mode {
        AgentSpawnMode::Fork => "fork",
        AgentSpawnMode::New => "new",
    }
}

fn workspace_mode_label(mode: AgentSpawnWorkspaceMode) -> &'static str {
    match mode {
        AgentSpawnWorkspaceMode::ReadOnly => "read_only",
        AgentSpawnWorkspaceMode::IsolatedWrite => "isolated_write",
    }
}

fn task_status_label(status: SubagentTaskStatus) -> &'static str {
    match status {
        SubagentTaskStatus::Pending => "pending",
        SubagentTaskStatus::Running => "running",
        SubagentTaskStatus::Completed => "completed",
        SubagentTaskStatus::Failed => "failed",
    }
}

async fn start_subagent_turn(
    server: &super::Server,
    task: &SubagentSpawnTask,
) -> anyhow::Result<TurnId> {
    let rt = server.get_or_load_thread(task.thread_id).await?;
    let server_arc = Arc::new(server.clone());
    let turn_id = rt
        .start_turn(
            server_arc,
            task.input.clone(),
            None,
            None,
            pm_protocol::TurnPriority::Background,
        )
        .await?;

    let notify_rx = server.notify_tx.subscribe();
    spawn_fan_out_result_writer(
        server.clone(),
        notify_rx,
        task.thread_id,
        turn_id,
        task.id.clone(),
        task.expected_artifact_type.clone(),
    );

    Ok(turn_id)
}

async fn create_new_thread(
    server: &super::Server,
    cwd: &str,
) -> anyhow::Result<SpawnedThread> {
    let handle = server
        .thread_store
        .create_thread(PathBuf::from(cwd))
        .await?;
    let thread_id = handle.thread_id();
    let log_path = handle.log_path().display().to_string();
    let last_seq = handle.last_seq().0;

    let rt = Arc::new(crate::ThreadRuntime::new(
        handle,
        server.notify_tx.clone(),
        server.notify_hub.clone(),
    ));
    server.threads.lock().await.insert(thread_id, rt);

    Ok(SpawnedThread {
        thread_id,
        log_path,
        last_seq,
    })
}

fn spawn_fan_out_result_writer(
    server: super::Server,
    mut notify_rx: tokio::sync::broadcast::Receiver<String>,
    thread_id: pm_protocol::ThreadId,
    turn_id: TurnId,
    task_id: String,
    expected_artifact_type: String,
) {
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(line) => {
                    let Ok(val) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if val.get("method").and_then(Value::as_str) != Some("turn/completed") {
                        continue;
                    }
                    let Some(params) = val.get("params") else {
                        continue;
                    };
                    let Ok(event) = serde_json::from_value::<pm_protocol::ThreadEvent>(params.clone())
                    else {
                        continue;
                    };
                    if event.thread_id != thread_id {
                        continue;
                    }
                    let pm_protocol::ThreadEventKind::TurnCompleted {
                        turn_id: completed_turn_id,
                        status,
                        reason,
                    } = event.kind
                    else {
                        continue;
                    };
                    if completed_turn_id != turn_id {
                        continue;
                    }

                    let payload = serde_json::json!({
                        "task_id": task_id,
                        "thread_id": thread_id,
                        "turn_id": turn_id,
                        "status": status,
                        "reason": reason,
                    });
                    let text = match serde_json::to_string_pretty(&payload) {
                        Ok(json) => format!("```json\n{json}\n```\n"),
                        Err(_) => payload.to_string(),
                    };

                    let _ = super::handle_artifact_write(
                        &server,
                        super::ArtifactWriteParams {
                            thread_id,
                            turn_id: Some(turn_id),
                            approval_id: None,
                            artifact_id: None,
                            artifact_type: expected_artifact_type,
                            summary: "fan-out result".to_string(),
                            text,
                        },
                    )
                    .await;
                    return;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

#[cfg(test)]
mod agent_spawn_guard_tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn build_test_server(pm_root: PathBuf) -> super::super::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        super::super::Server {
            cwd: pm_root.clone(),
            notify_tx,
            notify_hub: crate::default_notify_hub(),
            thread_store: super::super::ThreadStore::new(super::super::PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(super::super::McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
            db_vfs: None,
        }
    }

    #[tokio::test]
    async fn agent_spawn_denies_isolated_write_workspace_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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
                    "workspace_mode": "isolated_write",
                }],
            }),
            None,
        )
        .await?;

        assert!(result["denied"].as_bool().unwrap_or(false));
        assert_eq!(result["workspace_mode"].as_str().unwrap_or(""), "isolated_write");
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_denies_disallowed_child_mode() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
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
        assert!(result["allowed_modes"].as_array().is_some());
        Ok(())
    }

    #[tokio::test]
    async fn agent_spawn_enforces_default_max_concurrent_subagents() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo_dir = tmp.path().join("repo");
        tokio::fs::create_dir_all(&repo_dir).await?;

        let server = build_test_server(tmp.path().join(".codepm_data"));
        let mut parent = server.thread_store.create_thread(repo_dir.clone()).await?;
        let thread_id = parent.thread_id();

        for _ in 0..4 {
            let mut child = server.thread_store.create_thread(repo_dir.clone()).await?;
            let child_id = child.thread_id();
            child
                .append(pm_protocol::ThreadEventKind::TurnStarted {
                    turn_id: pm_protocol::TurnId::new(),
                    input: "child".to_string(),
                    context_refs: None,
                    attachments: None,
                    priority: pm_protocol::TurnPriority::Foreground,
                })
                .await?;
            drop(child);

            let tool_id = pm_protocol::ToolId::new();
            parent
                .append(pm_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id: None,
                    tool: "subagent/spawn".to_string(),
                    params: None,
                })
                .await?;
            parent
                .append(pm_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: pm_protocol::ToolStatus::Completed,
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
        assert_eq!(result["max_concurrent_subagents"].as_u64().unwrap_or(0), 4);
        assert_eq!(result["active"].as_u64().unwrap_or(0), 4);
        Ok(())
    }
}

#[cfg(test)]
mod reference_repo_file_tools_tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    fn build_test_server(pm_root: PathBuf) -> super::super::Server {
        let (notify_tx, _notify_rx) = broadcast::channel::<String>(16);
        super::super::Server {
            cwd: pm_root.clone(),
            notify_tx,
            notify_hub: crate::default_notify_hub(),
            thread_store: super::super::ThreadStore::new(super::super::PmPaths::new(pm_root)),
            threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            mcp: Arc::new(tokio::sync::Mutex::new(super::super::McpManager::default())),
            disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            exec_policy: pm_execpolicy::Policy::empty(),
            db_vfs: None,
        }
    }

    #[tokio::test]
    async fn file_glob_excludes_codepm_reference_dir_for_workspace_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".codepm_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".codepm_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = build_test_server(tmp.path().join("pm_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let result = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "pattern": "**/*.txt" }),
            None,
        )
        .await?;

        let paths = result["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("hello.txt")));
        assert!(
            !paths
                .iter()
                .any(|p| p.as_str().unwrap_or("").contains(".codepm_data/reference/"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn file_glob_and_read_can_use_reference_root() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;

        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;
        tokio::fs::create_dir_all(project_dir.join(".codepm_data/reference/repo")).await?;
        tokio::fs::write(
            project_dir.join(".codepm_data/reference/repo/ref.txt"),
            "ref\n",
        )
        .await?;

        let server = build_test_server(tmp.path().join("pm_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let glob = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_glob",
            serde_json::json!({ "root": "reference", "pattern": "**/*.txt" }),
            None,
        )
        .await?;
        let paths = glob["paths"].as_array().cloned().unwrap_or_default();
        assert!(paths.iter().any(|p| p.as_str() == Some("ref.txt")));
        assert!(!paths.iter().any(|p| p.as_str() == Some("hello.txt")));

        let read = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await?;
        assert_eq!(read["text"].as_str(), Some("ref\n"));
        assert_eq!(read["root"].as_str(), Some("reference"));
        Ok(())
    }

    #[tokio::test]
    async fn reference_root_fails_closed_when_not_configured() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let project_dir = tmp.path().join("project");
        tokio::fs::create_dir_all(&project_dir).await?;
        tokio::fs::write(project_dir.join("hello.txt"), "hello\n").await?;

        let server = build_test_server(tmp.path().join("pm_root"));
        let handle = server.thread_store.create_thread(project_dir).await?;
        let thread_id = handle.thread_id();
        drop(handle);

        let err = run_tool_call_once(
            &server,
            thread_id,
            None,
            "file_read",
            serde_json::json!({ "root": "reference", "path": "ref.txt" }),
            None,
        )
        .await
        .expect_err("expected root=reference to fail when not configured");
        assert!(
            err.to_string().contains("reference repo root")
                || err.to_string().contains(".codepm_data/reference/repo")
        );
        Ok(())
    }
}
