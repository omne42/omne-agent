use super::*;

pub(super) fn parse_workflow_tasks(body: &str) -> anyhow::Result<Vec<WorkflowTask>> {
    let mut raw_tasks = Vec::<WorkflowTask>::new();
    let mut seen_ids = BTreeSet::<String>::new();

    let mut current_id = None::<String>;
    let mut current_title = String::new();
    let mut current_body = String::new();

    for line in body.split_inclusive('\n') {
        if let Some((id, title)) = parse_task_header(line) {
            if let Some(id) = current_id.take() {
                raw_tasks.push(WorkflowTask {
                    id,
                    title: std::mem::take(&mut current_title),
                    body: std::mem::take(&mut current_body),
                    depends_on: Vec::new(),
                    priority: WorkflowTaskPriority::Normal,
                });
            }

            ensure_valid_var_name(&id, "task id")?;
            if !seen_ids.insert(id.clone()) {
                anyhow::bail!("duplicate task id: {id}");
            }
            current_id = Some(id);
            current_title = title;
            continue;
        }

        if current_id.is_some() {
            current_body.push_str(line);
        }
    }

    if let Some(id) = current_id.take() {
        raw_tasks.push(WorkflowTask {
            id,
            title: current_title,
            body: current_body,
            depends_on: Vec::new(),
            priority: WorkflowTaskPriority::Normal,
        });
    }

    let mut tasks = Vec::<WorkflowTask>::with_capacity(raw_tasks.len());
    for task in raw_tasks {
        let directives = parse_task_directives(&task.id, &task.body)?;
        tasks.push(WorkflowTask {
            id: task.id,
            title: task.title,
            body: directives.body,
            depends_on: directives.depends_on,
            priority: directives.priority,
        });
    }
    validate_workflow_task_dependencies(&tasks)?;

    Ok(tasks)
}

pub(super) struct WorkflowTaskDirectives {
    pub(super) depends_on: Vec<String>,
    pub(super) priority: WorkflowTaskPriority,
    pub(super) body: String,
}

pub(super) fn parse_task_directives(
    task_id: &str,
    body: &str,
) -> anyhow::Result<WorkflowTaskDirectives> {
    let mut depends_on = Vec::<String>::new();
    let mut seen_depends = BTreeSet::<String>::new();
    let mut priority = WorkflowTaskPriority::Normal;
    let mut saw_depends_on_directive = false;
    let mut saw_priority_directive = false;
    let mut out = String::new();
    let mut in_leading_directives = true;

    for line in body.split_inclusive('\n') {
        let trimmed = line.trim();
        if in_leading_directives {
            if trimmed.is_empty() {
                out.push_str(line);
                continue;
            }

            let depends_directive = trimmed
                .strip_prefix("depends_on:")
                .or_else(|| trimmed.strip_prefix("depends-on:"));
            if let Some(rest) = depends_directive {
                if saw_depends_on_directive {
                    anyhow::bail!("duplicate depends_on directive: {task_id}");
                }
                saw_depends_on_directive = true;
                let mut parsed_dependency_count = 0usize;
                for dep in rest.split(',') {
                    let dep = dep.trim();
                    if dep.is_empty() {
                        continue;
                    }
                    parsed_dependency_count += 1;
                    ensure_valid_var_name(dep, "depends_on")?;
                    if dep == task_id {
                        anyhow::bail!("task depends_on itself: {task_id}");
                    }
                    if seen_depends.insert(dep.to_string()) {
                        depends_on.push(dep.to_string());
                    }
                }
                if parsed_dependency_count == 0 {
                    anyhow::bail!(
                        "depends_on directive must include at least one task id: {task_id}"
                    );
                }
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("priority:") {
                if saw_priority_directive {
                    anyhow::bail!("duplicate priority directive: {task_id}");
                }
                priority = WorkflowTaskPriority::parse(rest).ok_or_else(|| {
                    anyhow::anyhow!(
                        "invalid priority for task {task_id}: {} (expected high|normal|low)",
                        rest.trim()
                    )
                })?;
                saw_priority_directive = true;
                continue;
            }

            in_leading_directives = false;
        }
        out.push_str(line);
    }

    Ok(WorkflowTaskDirectives {
        depends_on,
        priority,
        body: out,
    })
}

pub(super) fn validate_workflow_task_dependencies(tasks: &[WorkflowTask]) -> anyhow::Result<()> {
    let mut by_id = BTreeSet::<String>::new();
    for task in tasks {
        by_id.insert(task.id.clone());
    }

    let mut indegree = BTreeMap::<String, usize>::new();
    let mut edges = BTreeMap::<String, Vec<String>>::new();
    for task in tasks {
        for dep in &task.depends_on {
            if !by_id.contains(dep) {
                anyhow::bail!("unknown depends_on: {dep} (task_id={})", task.id);
            }
            edges.entry(dep.clone()).or_default().push(task.id.clone());
        }
        indegree.insert(task.id.clone(), task.depends_on.len());
    }

    let mut queue = std::collections::VecDeque::<String>::new();
    for (id, degree) in &indegree {
        if *degree == 0 {
            queue.push_back(id.clone());
        }
    }

    let mut visited = 0usize;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(children) = edges.get(&id) {
            for child in children {
                if let Some(degree) = indegree.get_mut(child) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
    }

    if visited != tasks.len() {
        anyhow::bail!("task dependencies contain a cycle");
    }
    Ok(())
}

pub(super) fn dependency_blocker(
    task: &WorkflowTask,
    task_statuses: &BTreeMap<String, TurnStatus>,
) -> Option<(String, TurnStatus)> {
    for dep in &task.depends_on {
        let Some(status) = task_statuses.get(dep).copied() else {
            continue;
        };
        if !matches!(status, TurnStatus::Completed) {
            return Some((dep.clone(), status));
        }
    }
    None
}

pub(super) fn collect_dependency_blocked_task_ids(
    tasks: &[WorkflowTask],
    started: &BTreeSet<String>,
    task_statuses: &BTreeMap<String, TurnStatus>,
) -> Vec<(String, String, TurnStatus)> {
    let mut blocked = Vec::<(String, String, TurnStatus)>::new();
    for task in tasks {
        if started.contains(&task.id) {
            continue;
        }
        if let Some((dep, status)) = dependency_blocker(task, task_statuses) {
            blocked.push((task.id.clone(), dep, status));
        }
    }
    blocked
}

pub(super) fn is_runnable_task(
    task: &WorkflowTask,
    task_statuses: &BTreeMap<String, TurnStatus>,
) -> bool {
    task.depends_on
        .iter()
        .all(|dep| matches!(task_statuses.get(dep), Some(TurnStatus::Completed)))
}

pub(super) fn update_ready_wait_rounds(
    tasks: &[WorkflowTask],
    started: &BTreeSet<String>,
    task_statuses: &BTreeMap<String, TurnStatus>,
    ready_wait_rounds: &mut BTreeMap<String, usize>,
) {
    let mut ready_ids = BTreeSet::<String>::new();
    for task in tasks {
        if started.contains(&task.id) {
            continue;
        }
        if is_runnable_task(task, task_statuses) {
            ready_ids.insert(task.id.clone());
        }
    }

    ready_wait_rounds.retain(|task_id, _| ready_ids.contains(task_id));
    for task_id in ready_ids {
        *ready_wait_rounds.entry(task_id).or_insert(0) += 1;
    }
}

pub(super) fn aged_priority_rank(
    task: &WorkflowTask,
    ready_wait_rounds: &BTreeMap<String, usize>,
    priority_aging_rounds: usize,
) -> usize {
    let base = task.priority.rank();
    let waited_rounds = ready_wait_rounds.get(&task.id).copied().unwrap_or(0);
    base.saturating_sub(waited_rounds / priority_aging_rounds)
}

pub(super) fn pick_next_runnable_task<'a>(
    tasks: &'a [WorkflowTask],
    started: &BTreeSet<String>,
    task_statuses: &BTreeMap<String, TurnStatus>,
) -> Option<&'a WorkflowTask> {
    tasks
        .iter()
        .filter(|task| !started.contains(&task.id))
        .filter(|task| is_runnable_task(task, task_statuses))
        .min_by_key(|task| task.priority.rank())
}

pub(super) fn pick_next_runnable_task_fair<'a>(
    tasks: &'a [WorkflowTask],
    started: &BTreeSet<String>,
    task_statuses: &BTreeMap<String, TurnStatus>,
    ready_wait_rounds: &BTreeMap<String, usize>,
    priority_aging_rounds: usize,
) -> Option<&'a WorkflowTask> {
    tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| !started.contains(&task.id))
        .filter(|(_, task)| is_runnable_task(task, task_statuses))
        .min_by_key(|(idx, task)| {
            (
                aged_priority_rank(task, ready_wait_rounds, priority_aging_rounds),
                *idx,
            )
        })
        .map(|(_, task)| task)
}

pub(super) fn display_thread_id(thread_id: Option<ThreadId>) -> String {
    thread_id
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn display_turn_id(turn_id: Option<TurnId>) -> String {
    turn_id
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn display_artifact_id(artifact_id: Option<ArtifactId>) -> String {
    artifact_id
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(super) fn display_artifact_error(error: Option<&str>) -> String {
    error
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("-")
        .to_string()
}

pub(super) fn blocked_task_result(
    task: &WorkflowTask,
    dep_id: &str,
    dep_status: TurnStatus,
) -> WorkflowTaskResult {
    WorkflowTaskResult {
        task_id: task.id.clone(),
        title: task.title.clone(),
        thread_id: None,
        turn_id: None,
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Cancelled,
        reason: Some(format!(
            "blocked by dependency: {dep_id} status={dep_status:?}"
        )),
        dependency_blocked: true,
        assistant_text: None,
        pending_approval: None,
    }
}

pub(super) fn dependency_blocker_fields(
    dependency_blocked: bool,
    reason: Option<&str>,
) -> (Option<String>, Option<String>) {
    if !dependency_blocked {
        return (None, None);
    }
    let Some(reason) = reason.map(str::trim) else {
        return (None, None);
    };
    let Some(rest) = reason.strip_prefix("blocked by dependency: ") else {
        return (None, None);
    };
    let Some((dependency_task_id, dependency_status)) = rest.split_once(" status=") else {
        return (None, None);
    };
    let dependency_task_id = dependency_task_id.trim();
    let dependency_status = dependency_status.trim();
    if dependency_task_id.is_empty() || dependency_status.is_empty() {
        return (None, None);
    }
    (
        Some(dependency_task_id.to_string()),
        Some(dependency_status.to_string()),
    )
}

pub(super) fn pending_approval_task_result(
    task_id: String,
    title: String,
    thread_id: ThreadId,
    turn_id: TurnId,
    action: String,
    approval_id: ApprovalId,
    summary: Option<String>,
) -> WorkflowTaskResult {
    let approve_cmd = approval_decide_command(thread_id, approval_id, true);
    let deny_cmd = approval_decide_command(thread_id, approval_id, false);
    let mut reason = format!("blocked on approval: action={action} approval_id={approval_id}");
    if let Some(summary) = summary.as_deref().filter(|value| !value.trim().is_empty()) {
        reason.push_str(&format!(" summary={summary}"));
    }
    WorkflowTaskResult {
        task_id,
        title,
        thread_id: Some(thread_id),
        turn_id: Some(turn_id),
        result_artifact_id: None,
        result_artifact_error: None,
        result_artifact_error_id: None,
        status: TurnStatus::Interrupted,
        reason: Some(reason),
        dependency_blocked: false,
        assistant_text: None,
        pending_approval: Some(WorkflowPendingApproval {
            approval_id,
            action,
            summary,
            approve_cmd: Some(approve_cmd),
            deny_cmd: Some(deny_cmd),
        }),
    }
}

#[derive(Debug, Clone)]
pub(super) struct FanOutApprovalIssue {
    pub(super) task_id: String,
    pub(super) thread_id: ThreadId,
    pub(super) turn_id: TurnId,
    pub(super) approval_id: ApprovalId,
    pub(super) action: String,
    pub(super) summary: Option<String>,
}

pub(super) fn fan_out_approval_command(issue: &FanOutApprovalIssue) -> String {
    approval_decide_command(issue.thread_id, issue.approval_id, true)
}

pub(super) fn fan_out_deny_command(issue: &FanOutApprovalIssue) -> String {
    approval_decide_command(issue.thread_id, issue.approval_id, false)
}

pub(super) fn approval_decide_command(
    thread_id: ThreadId,
    approval_id: ApprovalId,
    approve: bool,
) -> String {
    let decision_flag = if approve { "--approve" } else { "--deny" };
    format!("omne approval decide {thread_id} {approval_id} {decision_flag}")
}

pub(super) fn fan_out_result_read_command(thread_id: ThreadId, artifact_id: ArtifactId) -> String {
    format!("omne artifact read {thread_id} {artifact_id}")
}

pub(super) fn normalize_optional_command(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn pending_approval_commands_for_result(
    result: &WorkflowTaskResult,
    pending: &WorkflowPendingApproval,
) -> (Option<String>, Option<String>) {
    let mut approve_cmd = normalize_optional_command(pending.approve_cmd.as_deref());
    let mut deny_cmd = normalize_optional_command(pending.deny_cmd.as_deref());

    if (approve_cmd.is_none() || deny_cmd.is_none())
        && let Some(thread_id) = result.thread_id
    {
        if approve_cmd.is_none() {
            approve_cmd = Some(approval_decide_command(
                thread_id,
                pending.approval_id,
                true,
            ));
        }
        if deny_cmd.is_none() {
            deny_cmd = Some(approval_decide_command(
                thread_id,
                pending.approval_id,
                false,
            ));
        }
    }

    (approve_cmd, deny_cmd)
}

pub(super) fn render_fan_in_summary_structured_json(
    thread_id: ThreadId,
    results: &[WorkflowTaskResult],
    scheduling: FanOutSchedulingParams,
) -> String {
    let tasks = results
        .iter()
        .map(|result| {
            let pending_approval = result.pending_approval.as_ref().map(|pending| {
                let (approve_cmd, deny_cmd) = pending_approval_commands_for_result(result, pending);
                omne_workflow_spec::FanInPendingApprovalStructuredData {
                    approval_id: pending.approval_id.to_string(),
                    action: pending.action.clone(),
                    summary: pending.summary.clone(),
                    approve_cmd,
                    deny_cmd,
                }
            });
            let (dependency_blocker_task_id, dependency_blocker_status) =
                dependency_blocker_fields(result.dependency_blocked, result.reason.as_deref());

            omne_workflow_spec::FanInTaskStructuredData {
                task_id: result.task_id.clone(),
                title: result.title.clone(),
                thread_id: result.thread_id.map(|value| value.to_string()),
                turn_id: result.turn_id.map(|value| value.to_string()),
                status: format!("{:?}", result.status),
                reason: result.reason.clone(),
                dependency_blocked: result.dependency_blocked,
                dependency_blocker_task_id,
                dependency_blocker_status,
                result_artifact_id: result.result_artifact_id.map(|value| value.to_string()),
                result_artifact_error: result.result_artifact_error.clone(),
                result_artifact_error_id: result
                    .result_artifact_error_id
                    .map(|value| value.to_string()),
                result_artifact_diagnostics: None,
                pending_approval,
            }
        })
        .collect::<Vec<_>>();

    let payload = omne_workflow_spec::FanInSummaryStructuredData::new(
        thread_id.to_string(),
        omne_workflow_spec::FanInSchedulingStructuredData {
            env_max_concurrent_subagents: scheduling.env_max_concurrent_subagents,
            effective_concurrency_limit: scheduling.effective_concurrency_limit,
            priority_aging_rounds: scheduling.priority_aging_rounds,
        },
        tasks,
    );
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{\"tasks\":[]}".to_string())
}

pub(super) fn normalize_task_read_commands(
    mut values: Vec<(String, String)>,
) -> Vec<(String, String)> {
    values.sort_unstable();
    values.dedup();
    values
}

pub(super) fn collect_failed_task_reads(results: &[WorkflowTaskResult]) -> Vec<(String, String)> {
    normalize_task_read_commands(
        results
            .iter()
            .filter(|result| !matches!(result.status, TurnStatus::Completed))
            .filter_map(
                |result| match (result.thread_id, result.result_artifact_id) {
                    (Some(thread_id), Some(result_artifact_id)) => Some((
                        result.task_id.clone(),
                        fan_out_result_read_command(thread_id, result_artifact_id),
                    )),
                    _ => None,
                },
            )
            .collect::<Vec<_>>(),
    )
}

pub(super) fn collect_failed_task_error_reads(
    parent_thread_id: ThreadId,
    results: &[WorkflowTaskResult],
) -> Vec<(String, String)> {
    normalize_task_read_commands(
        results
            .iter()
            .filter_map(|result| {
                result.result_artifact_error_id.map(|artifact_id| {
                    (
                        result.task_id.clone(),
                        fan_out_result_read_command(parent_thread_id, artifact_id),
                    )
                })
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn append_fan_out_linkage_issue_markdown(
    text: &mut String,
    linkage_issue: Option<&str>,
) {
    let Some(linkage_issue) = linkage_issue
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    text.push_str("## Fan-out Linkage Issue\n\n");
    text.push_str("```text\n");
    text.push_str(&truncate_chars(linkage_issue, 8_000));
    if linkage_issue.chars().count() > 8_000 {
        text.push_str("\n<...truncated...>\n");
    }
    text.push_str("\n```\n\n");
}

pub(super) fn append_fan_out_scheduling_markdown(
    text: &mut String,
    scheduling: FanOutSchedulingParams,
) {
    text.push_str("## Scheduling\n\n");
    text.push_str(&format!(
        "- env_max_concurrent_subagents: `{}`\n",
        scheduling.env_max_concurrent_subagents
    ));
    text.push_str(&format!(
        "- effective_concurrency_limit: `{}`\n",
        scheduling.effective_concurrency_limit
    ));
    text.push_str(&format!(
        "- priority_aging_rounds: `{}`\n\n",
        scheduling.priority_aging_rounds
    ));
}

pub(super) fn fan_out_approval_error(
    issue: &FanOutApprovalIssue,
    artifact_id: omne_protocol::ArtifactId,
) -> String {
    let mut text = format!(
        "fan-out task needs approval: task_id={} thread_id={} turn_id={} approval_id={} action={}",
        issue.task_id, issue.thread_id, issue.turn_id, issue.approval_id, issue.action
    );
    if let Some(summary) = issue
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        text.push_str(&format!(" summary={summary}"));
    }
    let approve_cmd = fan_out_approval_command(issue);
    let deny_cmd = fan_out_deny_command(issue);
    text.push_str(&format!(
        " (approve_cmd=`{approve_cmd}` deny_cmd=`{deny_cmd}`; see fan_in_summary artifact_id={artifact_id})",
    ));
    text
}

pub(super) fn find_pending_approval_task_from_fan_in_summary<'a>(
    payload: &'a omne_app_server_protocol::ArtifactFanInSummaryStructuredData,
    issue: &FanOutApprovalIssue,
) -> Option<&'a omne_app_server_protocol::ArtifactFanInSummaryTask> {
    let approval_id = issue.approval_id.to_string();
    payload
        .tasks
        .iter()
        .find(|task| {
            task.pending_approval
                .as_ref()
                .is_some_and(|pending| pending.approval_id == approval_id)
        })
        .or_else(|| {
            payload
                .tasks
                .iter()
                .find(|task| task.task_id == issue.task_id && task.pending_approval.is_some())
        })
        .or_else(|| {
            payload
                .tasks
                .iter()
                .find(|task| task.pending_approval.is_some())
        })
}

pub(super) fn fan_out_approval_error_from_structured_task(
    issue: &FanOutApprovalIssue,
    artifact_id: omne_protocol::ArtifactId,
    task: &omne_app_server_protocol::ArtifactFanInSummaryTask,
) -> String {
    let Some(pending) = task.pending_approval.as_ref() else {
        return fan_out_approval_error(issue, artifact_id);
    };

    let issue_thread_id = issue.thread_id.to_string();
    let issue_turn_id = issue.turn_id.to_string();
    let issue_approval_id = issue.approval_id.to_string();

    let task_id = if task.task_id.trim().is_empty() {
        issue.task_id.as_str()
    } else {
        task.task_id.as_str()
    };
    let thread_id = task
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(issue_thread_id.as_str());
    let turn_id = task
        .turn_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(issue_turn_id.as_str());
    let approval_id_trimmed = pending.approval_id.trim();
    let approval_id = if approval_id_trimmed.is_empty() {
        issue_approval_id.as_str()
    } else {
        approval_id_trimmed
    };
    let action_trimmed = pending.action.trim();
    let action = if action_trimmed.is_empty() {
        issue.action.as_str()
    } else {
        action_trimmed
    };

    let mut text = format!(
        "fan-out task needs approval: task_id={task_id} thread_id={thread_id} turn_id={turn_id} approval_id={approval_id} action={action}",
    );
    let summary = pending
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            issue
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        });
    if let Some(summary) = summary {
        text.push_str(&format!(" summary={summary}"));
    }

    let approve_cmd = pending
        .approve_cmd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("omne approval decide {thread_id} {approval_id} --approve"));
    let deny_cmd = pending
        .deny_cmd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("omne approval decide {thread_id} {approval_id} --deny"));
    text.push_str(&format!(
        " (approve_cmd=`{approve_cmd}` deny_cmd=`{deny_cmd}`; see fan_in_summary artifact_id={artifact_id})",
    ));
    text
}

pub(super) async fn fan_out_approval_error_with_artifact_fallback(
    app: &mut App,
    parent_thread_id: ThreadId,
    issue: &FanOutApprovalIssue,
    artifact_id: omne_protocol::ArtifactId,
) -> String {
    let maybe_structured = app
        .artifact_read(parent_thread_id, artifact_id, None, None, None)
        .await
        .ok()
        .and_then(|read| read.fan_in_summary)
        .and_then(|payload| {
            find_pending_approval_task_from_fan_in_summary(&payload, issue)
                .map(|task| fan_out_approval_error_from_structured_task(issue, artifact_id, task))
        });
    maybe_structured.unwrap_or_else(|| fan_out_approval_error(issue, artifact_id))
}

pub(super) fn render_fan_out_approval_blocked_markdown(
    total_tasks: usize,
    finished: &[WorkflowTaskResult],
    issue: &FanOutApprovalIssue,
    scheduling: FanOutSchedulingParams,
) -> String {
    let mut text = String::new();
    text.push_str("# Fan-in Summary\n\n");
    text.push_str("Status: blocked (need approval)\n\n");
    text.push_str(&format!("Progress: {}/{}\n\n", finished.len(), total_tasks));
    append_fan_out_scheduling_markdown(&mut text, scheduling);
    text.push_str("| task_id | thread_id | turn_id | approval_id | action | summary |\n");
    text.push_str("| --- | --- | --- | --- | --- | --- |\n");
    text.push_str(&format!(
        "| {} | {} | {} | {} | {} | {} |\n\n",
        issue.task_id,
        issue.thread_id,
        issue.turn_id,
        issue.approval_id,
        issue.action,
        issue
            .summary
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("-")
    ));
    text.push_str("Approval quick commands:\n\n");
    text.push_str("```bash\n");
    text.push_str(&fan_out_approval_command(issue));
    text.push('\n');
    text.push_str(&fan_out_deny_command(issue));
    text.push('\n');
    text.push_str("```\n\n");
    text.push_str("Decide approval, then re-run this command.\n\n");

    text.push_str("Completed tasks:\n\n");
    if finished.is_empty() {
        text.push_str("- (none)\n");
    } else {
        for result in finished {
            text.push_str(&format!(
                "- {} status={:?} thread_id={} turn_id={} artifact_id={} artifact_error={} error_artifact_id={}\n",
                result.task_id,
                result.status,
                display_thread_id(result.thread_id),
                display_turn_id(result.turn_id),
                display_artifact_id(result.result_artifact_id),
                display_artifact_error(result.result_artifact_error.as_deref()),
                display_artifact_id(result.result_artifact_error_id)
            ));
        }
    }

    let failed_reads = collect_failed_task_reads(finished);
    if !failed_reads.is_empty() {
        text.push_str("\nFailed task quick reads:\n\n");
        for (task_id, command) in failed_reads {
            text.push_str(&format!("- {}: `{}`\n", task_id, command));
        }
    }
    text
}

pub(super) fn render_fan_out_result_markdown(
    task_id: &str,
    title: &str,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
    assistant_text: Option<&str>,
) -> String {
    let mut text = String::new();
    text.push_str("# Fan-out Result\n\n");
    text.push_str(&format!("- task_id: `{task_id}`\n"));
    if !title.trim().is_empty() {
        text.push_str(&format!("- title: {title}\n"));
    }
    text.push_str(&format!("- turn_id: `{turn_id}`\n"));
    text.push_str(&format!("- status: `{:?}`\n", status));
    if let Some(reason) = reason.filter(|v| !v.trim().is_empty()) {
        text.push_str(&format!("- reason: {}\n", reason));
    }
    if let Some(msg) = assistant_text.filter(|v| !v.trim().is_empty()) {
        text.push_str("\n## Assistant Output\n\n```text\n");
        text.push_str(&truncate_chars(msg, 8_000));
        if msg.chars().count() > 8_000 {
            text.push_str("\n<...truncated...>\n");
        }
        text.push_str("\n```\n");
    }
    text
}

pub(super) async fn try_write_fan_out_result_artifact(
    app: &mut App,
    thread_id: ThreadId,
    task_id: &str,
    title: &str,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
    assistant_text: Option<&str>,
) -> Result<ArtifactId, String> {
    let summary = format!("fan-out result: {task_id} ({status:?})");
    let text =
        render_fan_out_result_markdown(task_id, title, turn_id, status, reason, assistant_text);
    let parsed = app
        .artifact_write(omne_app_server_protocol::ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "fan_out_result".to_string(),
            summary,
            text,
        })
        .await
        .map_err(|err| format!("artifact/write failed: {err}"))?;
    Ok(parsed.artifact_id)
}

pub(super) fn render_fan_out_result_error_markdown(
    task_id: &str,
    title: &str,
    child_thread_id: ThreadId,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
    write_error: &str,
) -> String {
    let mut text = String::new();
    text.push_str("# Fan-out Result Artifact Error\n\n");
    text.push_str(&format!("- task_id: `{task_id}`\n"));
    if !title.trim().is_empty() {
        text.push_str(&format!("- title: {title}\n"));
    }
    text.push_str(&format!("- child_thread_id: `{child_thread_id}`\n"));
    text.push_str(&format!("- turn_id: `{turn_id}`\n"));
    text.push_str(&format!("- status: `{:?}`\n", status));
    if let Some(reason) = reason.filter(|v| !v.trim().is_empty()) {
        text.push_str(&format!("- reason: {}\n", reason));
    }
    text.push_str(&format!("- error: {}\n", write_error));
    text
}

pub(super) async fn write_fan_out_result_error_artifact(
    app: &mut App,
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    task_id: &str,
    title: &str,
    child_thread_id: ThreadId,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
    write_error: &str,
) -> Option<ArtifactId> {
    let summary = format!("fan-out result artifact write failed: {task_id}");
    let text = render_fan_out_result_error_markdown(
        task_id,
        title,
        child_thread_id,
        turn_id,
        status,
        reason,
        write_error,
    );
    let params =
        fan_out_result_error_artifact_write_params(parent_thread_id, parent_turn_id, summary, text);
    match app.artifact_write(params).await {
        Ok(response) => Some(response.artifact_id),
        Err(err) => {
            eprintln!(
                "[fan-out] failed to write result error artifact task_id={task_id}: {err} (original={write_error})"
            );
            None
        }
    }
}

pub(super) fn fan_out_result_error_artifact_write_params(
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    summary: String,
    text: String,
) -> omne_app_server_protocol::ArtifactWriteParams {
    omne_app_server_protocol::ArtifactWriteParams {
        thread_id: parent_thread_id,
        turn_id: parent_turn_id,
        approval_id: None,
        artifact_id: None,
        artifact_type: "fan_out_result_error".to_string(),
        summary,
        text,
    }
}

pub(super) fn fan_out_linkage_issue_artifact_text(
    fan_in_artifact_id: omne_protocol::ArtifactId,
    linkage_issue: &str,
) -> Option<String> {
    let issue = linkage_issue.trim();
    if issue.is_empty() {
        return None;
    }
    let issue_truncated = issue.chars().count() > 8_000;
    let issue_excerpt = truncate_chars(issue, 8_000);
    let structured = omne_workflow_spec::FanOutLinkageIssueStructuredData::new(
        fan_in_artifact_id.to_string(),
        issue_excerpt.clone(),
        issue_truncated,
    );
    let structured_json = serde_json::to_string_pretty(&structured).ok()?;
    let mut text = String::new();
    text.push_str("# Fan-out Linkage Issue\n\n");
    text.push_str("```text\n");
    text.push_str(&issue_excerpt);
    if issue_truncated {
        text.push_str("\n<...truncated...>\n");
    }
    text.push_str("\n```\n\n");
    text.push_str(&format!(
        "- fan_in_summary_artifact_id: `{fan_in_artifact_id}`\n"
    ));
    text.push_str("\n## Structured Data\n\n");
    text.push_str("```json\n");
    text.push_str(&structured_json);
    text.push_str("\n```\n");
    Some(text)
}

pub(super) fn fan_in_related_artifact_write_params(
    thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    artifact_id: Option<omne_protocol::ArtifactId>,
    artifact_type: &'static str,
    summary: &'static str,
    text: String,
) -> omne_app_server_protocol::ArtifactWriteParams {
    omne_app_server_protocol::ArtifactWriteParams {
        thread_id,
        turn_id: parent_turn_id,
        approval_id: None,
        artifact_id,
        artifact_type: artifact_type.to_string(),
        summary: summary.to_string(),
        text,
    }
}

pub(super) fn fan_out_linkage_issue_artifact_write_params(
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    fan_in_artifact_id: omne_protocol::ArtifactId,
    linkage_issue: &str,
) -> Option<omne_app_server_protocol::ArtifactWriteParams> {
    let text = fan_out_linkage_issue_artifact_text(fan_in_artifact_id, linkage_issue)?;
    Some(fan_in_related_artifact_write_params(
        parent_thread_id,
        parent_turn_id,
        None,
        "fan_out_linkage_issue",
        "fan-out linkage issue",
        text,
    ))
}

pub(super) fn fan_out_linkage_issue_clear_artifact_write_params(
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    fan_in_artifact_id: omne_protocol::ArtifactId,
) -> omne_app_server_protocol::ArtifactWriteParams {
    let structured = omne_workflow_spec::FanOutLinkageIssueClearStructuredData::new(
        fan_in_artifact_id.to_string(),
    );
    let structured_json =
        serde_json::to_string_pretty(&structured).unwrap_or_else(|_| "{}".to_string());
    let text = format!(
        "# Fan-out Linkage Issue Cleared\n\n- fan_in_summary_artifact_id: `{fan_in_artifact_id}`\n\n## Structured Data\n\n```json\n{structured_json}\n```\n"
    );
    fan_in_related_artifact_write_params(
        parent_thread_id,
        parent_turn_id,
        None,
        "fan_out_linkage_issue_clear",
        "fan-out linkage issue cleared",
        text,
    )
}

pub(super) fn fan_in_summary_artifact_write_params(
    thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    artifact_id: omne_protocol::ArtifactId,
    text: String,
) -> omne_app_server_protocol::ArtifactWriteParams {
    fan_in_related_artifact_write_params(
        thread_id,
        parent_turn_id,
        Some(artifact_id),
        "fan_in_summary",
        "fan-in summary",
        text,
    )
}

pub(super) async fn try_write_fan_out_linkage_issue_artifact(
    app: &mut App,
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    fan_in_artifact_id: omne_protocol::ArtifactId,
    linkage_issue: &str,
) -> Option<ArtifactId> {
    let Some(params) = fan_out_linkage_issue_artifact_write_params(
        parent_thread_id,
        parent_turn_id,
        fan_in_artifact_id,
        linkage_issue,
    ) else {
        return None;
    };
    match app.artifact_write(params).await {
        Ok(response) => Some(response.artifact_id),
        Err(err) => {
            eprintln!("[fan-out] failed to write linkage issue artifact: {err}");
            None
        }
    }
}

pub(super) async fn try_clear_fan_out_linkage_issue_marker(
    app: &mut App,
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    fan_in_artifact_id: omne_protocol::ArtifactId,
) -> Option<ArtifactId> {
    let params = fan_out_linkage_issue_clear_artifact_write_params(
        parent_thread_id,
        parent_turn_id,
        fan_in_artifact_id,
    );
    match app.artifact_write(params).await {
        Ok(response) => Some(response.artifact_id),
        Err(err) => {
            eprintln!("[fan-out] failed to write linkage issue clear artifact: {err}");
            None
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct FanOutResultArtifactWriteOutcome {
    pub(super) result_artifact_id: Option<ArtifactId>,
    pub(super) result_artifact_error: Option<String>,
    pub(super) result_artifact_error_id: Option<ArtifactId>,
}

pub(super) async fn write_fan_out_result_artifacts(
    app: &mut App,
    parent_thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    child_thread_id: ThreadId,
    task_id: &str,
    title: &str,
    turn_id: TurnId,
    status: TurnStatus,
    reason: Option<&str>,
    assistant_text: Option<&str>,
) -> FanOutResultArtifactWriteOutcome {
    match try_write_fan_out_result_artifact(
        app,
        child_thread_id,
        task_id,
        title,
        turn_id,
        status,
        reason,
        assistant_text,
    )
    .await
    {
        Ok(result_artifact_id) => FanOutResultArtifactWriteOutcome {
            result_artifact_id: Some(result_artifact_id),
            result_artifact_error: None,
            result_artifact_error_id: None,
        },
        Err(write_error) => {
            let error_artifact_id = write_fan_out_result_error_artifact(
                app,
                parent_thread_id,
                parent_turn_id,
                task_id,
                title,
                child_thread_id,
                turn_id,
                status,
                reason,
                &write_error,
            )
            .await;
            let result_artifact_error = if let Some(error_artifact_id) = error_artifact_id {
                format!("{write_error} (error_artifact_id={error_artifact_id})")
            } else {
                write_error
            };
            FanOutResultArtifactWriteOutcome {
                result_artifact_id: None,
                result_artifact_error: Some(result_artifact_error),
                result_artifact_error_id: error_artifact_id,
            }
        }
    }
}

pub(super) async fn write_fan_out_approval_blocked_artifact(
    app: &mut App,
    thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    artifact_id: omne_protocol::ArtifactId,
    total_tasks: usize,
    finished: &[WorkflowTaskResult],
    issue: &FanOutApprovalIssue,
    scheduling: FanOutSchedulingParams,
) -> anyhow::Result<()> {
    let text = render_fan_out_approval_blocked_markdown(total_tasks, finished, issue, scheduling);
    let params = fan_in_summary_artifact_write_params(thread_id, parent_turn_id, artifact_id, text);
    let _ = app.artifact_write(params).await?;
    Ok(())
}

pub(super) fn parse_task_header(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]).trim_start();
    let rest = trimmed.strip_prefix("## Task:")?.trim();
    if rest.is_empty() {
        return None;
    }

    let mut split_at = None::<usize>;
    for (idx, ch) in rest.char_indices() {
        if ch.is_whitespace() {
            split_at = Some(idx);
            break;
        }
    }

    let (id, title) = match split_at {
        Some(idx) => (&rest[..idx], rest[idx..].trim()),
        None => (rest, ""),
    };

    Some((id.to_string(), title.to_string()))
}

pub(super) async fn resolve_thread_cwd(
    app: &mut App,
    thread_id: ThreadId,
) -> anyhow::Result<String> {
    let state = app.thread_state(thread_id).await?;
    let cwd = state
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("thread cwd missing: {thread_id}"))?;
    Ok(cwd.to_string())
}

pub(super) async fn write_fan_out_progress_artifact<T: std::fmt::Debug>(
    app: &mut App,
    thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    artifact_id: omne_protocol::ArtifactId,
    total_tasks: usize,
    finished: &[WorkflowTaskResult],
    active: &[T],
    started_at: Instant,
    scheduling: FanOutSchedulingParams,
) -> anyhow::Result<()> {
    let text = render_fan_out_progress_markdown(
        total_tasks,
        finished,
        active,
        started_at.elapsed(),
        scheduling,
    );
    let params = fan_in_summary_artifact_write_params(thread_id, parent_turn_id, artifact_id, text);

    let _ = app.artifact_write(params).await?;
    Ok(())
}

pub(super) fn render_fan_out_progress_markdown<T: std::fmt::Debug>(
    total_tasks: usize,
    finished: &[WorkflowTaskResult],
    active: &[T],
    elapsed: Duration,
    scheduling: FanOutSchedulingParams,
) -> String {
    let done = finished.len();
    let eta_seconds = if done == 0 {
        None
    } else {
        let elapsed_secs = elapsed.as_secs_f64();
        let total_secs_estimate = elapsed_secs * (total_tasks as f64) / (done as f64);
        let eta_secs = (total_secs_estimate - elapsed_secs).max(0.0);
        Some(eta_secs.round() as u64)
    };

    let mut text = String::new();
    text.push_str("# Fan-out Progress\n\n");
    text.push_str(&format!("Progress: {done}/{total_tasks}\n\n"));
    text.push_str(&format!("Elapsed: {}s\n\n", elapsed.as_secs()));
    if let Some(eta_seconds) = eta_seconds {
        text.push_str(&format!("ETA (rough): {}s\n\n", eta_seconds));
    }
    append_fan_out_scheduling_markdown(&mut text, scheduling);

    text.push_str("Active tasks:\n\n");
    text.push_str("```text\n");
    if active.is_empty() {
        text.push_str("(none)\n");
    } else {
        for entry in active {
            text.push_str(&format!("{entry:?}\n"));
        }
    }
    text.push_str("```\n\n");

    text.push_str("Completed tasks:\n\n");
    if finished.is_empty() {
        text.push_str("- (none)\n");
    } else {
        for result in finished {
            text.push_str(&format!(
                "- {} status={:?} thread_id={} turn_id={} artifact_id={} artifact_error={} error_artifact_id={}\n",
                result.task_id,
                result.status,
                display_thread_id(result.thread_id),
                display_turn_id(result.turn_id),
                display_artifact_id(result.result_artifact_id),
                display_artifact_error(result.result_artifact_error.as_deref()),
                display_artifact_id(result.result_artifact_error_id)
            ));
        }
    }

    text
}

pub(super) async fn write_fan_in_summary_artifact(
    app: &mut App,
    thread_id: ThreadId,
    parent_turn_id: Option<TurnId>,
    artifact_id: omne_protocol::ArtifactId,
    results: &[WorkflowTaskResult],
    scheduling: FanOutSchedulingParams,
    linkage_issue: Option<&str>,
) -> anyhow::Result<String> {
    let text = render_fan_in_summary_markdown(thread_id, results, scheduling, linkage_issue);
    let params = fan_in_summary_artifact_write_params(thread_id, parent_turn_id, artifact_id, text);

    let parsed = app.artifact_write(params).await?;
    Ok(parsed.content_path)
}

pub(super) fn render_fan_in_summary_markdown(
    thread_id: ThreadId,
    results: &[WorkflowTaskResult],
    scheduling: FanOutSchedulingParams,
    linkage_issue: Option<&str>,
) -> String {
    let mut text = String::new();
    text.push_str("# Fan-in Summary\n\n");
    text.push_str(&format!("Tasks: {}\n\n", results.len()));
    append_fan_out_scheduling_markdown(&mut text, scheduling);

    text.push_str(
        "| task_id | thread_id | turn_id | artifact_id | artifact_error | error_artifact_id | status |\n",
    );
    text.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for result in results {
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:?} |\n",
            result.task_id,
            display_thread_id(result.thread_id),
            display_turn_id(result.turn_id),
            display_artifact_id(result.result_artifact_id),
            display_artifact_error(result.result_artifact_error.as_deref()),
            display_artifact_id(result.result_artifact_error_id),
            result.status
        ));
    }
    text.push('\n');

    for result in results {
        let (dependency_blocker_task_id, dependency_blocker_status) =
            dependency_blocker_fields(result.dependency_blocked, result.reason.as_deref());
        text.push_str(&format!("## {} {}\n\n", result.task_id, result.title));
        text.push_str(&format!("- status: `{:?}`\n", result.status));
        text.push_str(&format!(
            "- result_artifact_id: {}\n",
            display_artifact_id(result.result_artifact_id)
        ));
        if let Some(error) = result
            .result_artifact_error
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            text.push_str(&format!("- result_artifact_error: {}\n", error));
        }
        if let Some(error_artifact_id) = result.result_artifact_error_id {
            text.push_str(&format!(
                "- result_artifact_error_id: {}\n",
                error_artifact_id
            ));
            text.push_str(&format!(
                "- result_error_read_cmd: `{}`\n",
                fan_out_result_read_command(thread_id, error_artifact_id)
            ));
        }
        if let (Some(thread_id), Some(result_artifact_id)) =
            (result.thread_id, result.result_artifact_id)
        {
            text.push_str(&format!(
                "- result_read_cmd: `{}`\n",
                fan_out_result_read_command(thread_id, result_artifact_id)
            ));
        }
        if result.dependency_blocked {
            text.push_str("- dependency_blocked: true\n");
            if let Some(task_id) = dependency_blocker_task_id.as_deref() {
                text.push_str(&format!("- dependency_blocker_task_id: {}\n", task_id));
            }
            if let Some(status) = dependency_blocker_status.as_deref() {
                text.push_str(&format!("- dependency_blocker_status: {}\n", status));
            }
        }
        if let Some(reason) = result.reason.as_deref().filter(|v| !v.trim().is_empty()) {
            text.push_str(&format!("- reason: {}\n", reason));
        }
        if let Some(pending) = result.pending_approval.as_ref() {
            if let Some(summary) = pending
                .summary
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                text.push_str(&format!(
                    "- pending_approval: action={} approval_id={} summary={}\n",
                    pending.action, pending.approval_id, summary
                ));
            } else {
                text.push_str(&format!(
                    "- pending_approval: action={} approval_id={}\n",
                    pending.action, pending.approval_id
                ));
            }
            let (approve_cmd, deny_cmd) = pending_approval_commands_for_result(result, pending);
            if let Some(approve_cmd) = approve_cmd {
                text.push_str(&format!("- approve_cmd: `{approve_cmd}`\n"));
            }
            if let Some(deny_cmd) = deny_cmd {
                text.push_str(&format!("- deny_cmd: `{deny_cmd}`\n"));
            }
        }
        if let Some(msg) = result
            .assistant_text
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            text.push('\n');
            text.push_str("```text\n");
            text.push_str(&truncate_chars(msg, 8_000));
            if msg.chars().count() > 8_000 {
                text.push_str("\n<...truncated...>\n");
            }
            text.push_str("\n```\n\n");
        }
    }

    text.push_str("## Structured Data\n\n```json\n");
    text.push_str(&render_fan_in_summary_structured_json(
        thread_id, results, scheduling,
    ));
    text.push_str("\n```\n\n");

    append_fan_out_linkage_issue_markdown(&mut text, linkage_issue);

    let failed_reads = collect_failed_task_reads(results);
    if !failed_reads.is_empty() {
        text.push_str("## Failed Task Quick Reads\n\n");
        for (task_id, command) in &failed_reads {
            text.push_str(&format!("- {}: `{}`\n", task_id, command));
        }
        text.push('\n');
    }

    let error_reads = collect_failed_task_error_reads(thread_id, results);
    if !error_reads.is_empty() {
        text.push_str("## Result Artifact Error Quick Reads\n\n");
        for (task_id, command) in &error_reads {
            text.push_str(&format!("- {}: `{}`\n", task_id, command));
        }
        text.push('\n');
    }

    text
}
