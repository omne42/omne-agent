use super::*;

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
    let body = "## Task: t1 First\nintro\n\n## Task: t2 Second\ndepends_on: t1\nrun second\n";
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
fn parse_workflow_tasks_rejects_empty_depends_on_directive() {
    let body = "## Task: t1 First\ndepends_on:\nrun first\n";
    let err = parse_workflow_tasks(body).unwrap_err();
    assert!(
        err.to_string()
            .contains("depends_on directive must include at least one task id")
    );
}

#[test]
fn parse_workflow_tasks_rejects_empty_depends_on_directive_with_separators() {
    let body = "## Task: t1 First\ndepends-on: , ,\nrun first\n";
    let err = parse_workflow_tasks(body).unwrap_err();
    assert!(
        err.to_string()
            .contains("depends_on directive must include at least one task id")
    );
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
    let (task_id, status) =
        dependency_blocker_fields(true, Some("blocked by dependency: task_a status=Failed"));
    assert_eq!(task_id.as_deref(), Some("task_a"));
    assert_eq!(status.as_deref(), Some("Failed"));

    let (task_id, status) = dependency_blocker_fields(true, Some("turn finished with status=Failed"));
    assert!(task_id.is_none());
    assert!(status.is_none());

    let (task_id, status) =
        dependency_blocker_fields(false, Some("blocked by dependency: task_a status=Failed"));
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
