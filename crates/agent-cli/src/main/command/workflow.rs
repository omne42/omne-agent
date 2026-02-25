use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandListItem {
    pub(super) name: String,
    pub(super) version: u32,
    pub(super) mode: String,
    pub(super) file: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandListError {
    pub(super) file: String,
    pub(super) error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandResultSummary {
    pub(super) ok: bool,
    pub(super) commands_dir: String,
    pub(super) item_count: usize,
    pub(super) error_count: usize,
}

impl CommandResultSummary {
    pub(super) fn new(commands_dir: String, item_count: usize, error_count: usize) -> Self {
        Self {
            ok: error_count == 0,
            commands_dir,
            item_count,
            error_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandListResult {
    #[serde(flatten)]
    pub(super) summary: CommandResultSummary,
    pub(super) command_count: usize,
    pub(super) commands: Vec<CommandListItem>,
    pub(super) errors: Vec<CommandListError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) modes_load_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandValidateItem {
    pub(super) name: String,
    pub(super) version: u32,
    pub(super) mode: String,
    pub(super) file: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandValidateError {
    pub(super) file: String,
    pub(super) error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CommandValidateResult {
    #[serde(flatten)]
    pub(super) summary: CommandResultSummary,
    pub(super) strict: bool,
    pub(super) target: String,
    pub(super) validated_count: usize,
    pub(super) validated: Vec<CommandValidateItem>,
    pub(super) errors: Vec<CommandValidateError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) modes_load_error: Option<String>,
}

#[derive(Debug)]
pub(super) enum CommandSpecError {
    UnknownMode { mode: String, available: String },
    UnknownAllowedTool { tool: String, known: String },
    AllowedToolDeniedByMode { mode: String, tool: String },
    AllowedToolDecisionMappingMissing { tool: String },
}

impl CommandSpecError {
    pub(super) fn error_code(&self) -> &'static str {
        match self {
            Self::UnknownMode { .. } => "mode_unknown",
            Self::UnknownAllowedTool { .. } => "allowed_tools_unknown_tool",
            Self::AllowedToolDeniedByMode { .. } => "allowed_tools_mode_denied",
            Self::AllowedToolDecisionMappingMissing { .. } => "allowed_tools_mapping_missing",
        }
    }
}

impl std::fmt::Display for CommandSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMode { mode, available } => {
                write!(f, "unknown mode: {mode} (available: {available})")
            }
            Self::UnknownAllowedTool { tool, known } => {
                write!(f, "unknown tool in allowed_tools: {tool} (known tools: {known})")
            }
            Self::AllowedToolDeniedByMode { mode, tool } => {
                write!(f, "allowed_tools tool is denied by mode: mode={mode} tool={tool}")
            }
            Self::AllowedToolDecisionMappingMissing { tool } => {
                write!(f, "tool decision mapping is missing for allowed_tools entry: {tool}")
            }
        }
    }
}

impl std::error::Error for CommandSpecError {}

pub(super) fn ensure_valid_var_name(name: &str, label: &str) -> anyhow::Result<()> {
    omne_workflow_spec::ensure_valid_var_name(name, label)
}

pub(super) fn split_frontmatter(contents: &str) -> anyhow::Result<(&str, &str)> {
    omne_workflow_spec::split_frontmatter(contents)
}

pub(super) fn normalize_string(value: String, label: &str) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    Ok(value)
}

pub(super) fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub(super) fn normalize_unique_list(values: Vec<String>) -> Vec<String> {
    omne_workflow_spec::normalize_unique_list(values)
}

pub(super) fn validate_workflow_mode(
    mode: &str,
    mode_catalog: &omne_core::modes::ModeCatalog,
) -> anyhow::Result<()> {
    if mode_catalog.mode(mode).is_some() {
        return Ok(());
    }
    let available = mode_catalog.mode_names().collect::<Vec<_>>().join(", ");
    Err(CommandSpecError::UnknownMode {
        mode: mode.to_string(),
        available,
    }
    .into())
}

pub(super) fn validate_allowed_tools_for_mode(
    mode_name: &str,
    tools: &[String],
    mode_catalog: &omne_core::modes::ModeCatalog,
) -> anyhow::Result<()> {
    let mode = mode_catalog.mode(mode_name).ok_or_else(|| {
        let available = mode_catalog.mode_names().collect::<Vec<_>>().join(", ");
        anyhow::anyhow!(CommandSpecError::UnknownMode {
            mode: mode_name.to_string(),
            available,
        })
    })?;

    for tool in tools {
        if !omne_core::allowed_tools::is_known_allowed_tool(tool.as_str()) {
            let known = omne_core::allowed_tools::known_allowed_tools().join(", ");
            return Err(CommandSpecError::UnknownAllowedTool {
                tool: tool.clone(),
                known,
            }
            .into());
        }

        let decision = omne_core::allowed_tools::effective_mode_decision_for_tool(mode, tool)
            .ok_or_else(|| {
                anyhow::anyhow!(CommandSpecError::AllowedToolDecisionMappingMissing {
                    tool: tool.clone(),
                })
            })?;
        if decision == omne_core::modes::Decision::Deny {
            return Err(CommandSpecError::AllowedToolDeniedByMode {
                mode: mode_name.to_string(),
                tool: tool.clone(),
            }
            .into());
        }
    }

    Ok(())
}

pub(super) fn sanitize_frontmatter(
    mut fm: WorkflowFileFrontmatterV1,
    default_name: String,
    mode_catalog: &omne_core::modes::ModeCatalog,
) -> anyhow::Result<WorkflowFileFrontmatterV1> {
    if fm.version != 1 {
        anyhow::bail!("unsupported command version: {} (expected 1)", fm.version);
    }
    fm.name = normalize_optional_string(fm.name).or(Some(default_name));
    fm.mode = normalize_string(fm.mode, "mode")?;
    validate_workflow_mode(&fm.mode, mode_catalog)?;

    if let Some(tools) = fm.allowed_tools.take() {
        let tools = normalize_unique_list(tools);
        validate_allowed_tools_for_mode(&fm.mode, &tools, mode_catalog)?;
        fm.allowed_tools = Some(tools);
    }

    let mut seen_inputs = BTreeSet::<String>::new();
    for input in &mut fm.inputs {
        input.name = normalize_string(std::mem::take(&mut input.name), "inputs[].name")?;
        ensure_valid_var_name(&input.name, "inputs[].name")?;
        if !seen_inputs.insert(input.name.clone()) {
            anyhow::bail!("duplicate input name: {}", input.name);
        }
    }

    for step in &mut fm.context {
        if step.argv.is_empty() {
            anyhow::bail!("context argv must not be empty");
        }
        step.summary = normalize_string(std::mem::take(&mut step.summary), "context.summary")?;
        step.argv = step
            .argv
            .drain(..)
            .map(|v| normalize_string(v, "context.argv"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        if let Some(codes) = step.ok_exit_codes.as_mut() {
            if codes.is_empty() {
                anyhow::bail!("context.ok_exit_codes must not be empty");
            }
            codes.sort_unstable();
            codes.dedup();
        }
    }

    Ok(fm)
}

pub(super) async fn load_workflow_file(cli: &Cli, name: &str) -> anyhow::Result<WorkflowFile> {
    omne_workflow_spec::validate_workflow_name(name)?;
    let omne_root = resolve_pm_root(cli)?;
    let mode_catalog = omne_core::modes::ModeCatalog::load(&omne_root).await;
    let dir = omne_workflow_spec::workflow_spec_dir(&omne_root);
    if !tokio::fs::try_exists(&dir).await? {
        anyhow::bail!(
            "commands dir is missing: {} (run `omne init`?)",
            dir.display()
        );
    }

    let path = dir.join(format!("{name}.md"));
    let path_canon = tokio::fs::canonicalize(&path)
        .await
        .with_context(|| format!("canonicalize {}", path.display()))?;
    let dir_canon = tokio::fs::canonicalize(&dir)
        .await
        .with_context(|| format!("canonicalize {}", dir.display()))?;
    if !path_canon.starts_with(&dir_canon) {
        anyhow::bail!(
            "refusing to load command outside spec dir: {}",
            path.display()
        );
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let (yaml, body) = split_frontmatter(&raw)?;

    let default_name = name.to_string();
    let fm = serde_yaml::from_str::<WorkflowFileFrontmatterV1>(yaml)
        .context("parse command frontmatter yaml")?;
    let fm = sanitize_frontmatter(fm, default_name, &mode_catalog)?;

    Ok(WorkflowFile {
        frontmatter: fm,
        body: body.to_string(),
        modes_load_error: mode_catalog.load_error.clone(),
    })
}

pub(super) fn collect_vars(vars: &[CommandVar]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::<String, String>::new();
    for var in vars {
        if out.contains_key(&var.key) {
            anyhow::bail!("duplicate --var: {}", var.key);
        }
        out.insert(var.key.clone(), var.value.clone());
    }
    Ok(out)
}

pub(super) fn render_template(
    template: &str,
    declared: &BTreeSet<String>,
    vars: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    omne_workflow_spec::render_template(template, declared, vars)
}

pub(super) fn fan_out_require_completed() -> bool {
    parse_env_bool("OMNE_COMMAND_FAN_OUT_REQUIRE_COMPLETED", true)
}

pub(super) fn fan_out_priority_aging_rounds() -> usize {
    parse_env_usize("OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS", 3, 1, 10_000)
}

pub(super) fn fan_out_scheduling_params(total_tasks: usize) -> FanOutSchedulingParams {
    let env_max_concurrent_subagents = parse_env_usize("OMNE_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
    let effective_concurrency_limit = if env_max_concurrent_subagents == 0 {
        total_tasks.max(1)
    } else {
        env_max_concurrent_subagents
    };
    let priority_aging_rounds = fan_out_priority_aging_rounds();
    FanOutSchedulingParams {
        env_max_concurrent_subagents,
        effective_concurrency_limit,
        priority_aging_rounds,
    }
}

pub(super) fn duplicate_command_name_errors(validated: &[CommandValidateItem]) -> Vec<CommandValidateError> {
    let mut by_name = std::collections::BTreeMap::<String, Vec<String>>::new();
    for item in validated {
        by_name
            .entry(item.name.clone())
            .or_default()
            .push(item.file.clone());
    }

    let mut errors = Vec::<CommandValidateError>::new();
    for (name, files) in by_name {
        if files.len() < 2 {
            continue;
        }
        for file in &files {
            let others = files
                .iter()
                .filter(|other| *other != file)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(CommandValidateError {
                file: file.clone(),
                error: format!("duplicate command name `{name}` also found in: {others}"),
                error_code: Some("duplicate_command_name".to_string()),
            });
        }
    }
    errors
}

pub(super) fn command_error_code(err: &anyhow::Error) -> Option<&'static str> {
    for cause in err.chain() {
        if let Some(spec) = cause.downcast_ref::<CommandSpecError>() {
            return Some(spec.error_code());
        }
        if cause.downcast_ref::<serde_yaml::Error>().is_some() {
            return Some("frontmatter_yaml_parse_failed");
        }
    }
    let text = err.to_string();
    if text.contains("parse frontmatter") {
        return Some("frontmatter_parse_failed");
    }
    if text.contains("read ") {
        return Some("read_failed");
    }
    None
}

pub(super) fn command_run_error_code(err: &anyhow::Error) -> Option<&'static str> {
    if let Some(code) = command_error_code(err) {
        return Some(code);
    }

    let text = err.to_string();
    if text.contains("--fan-out-early-return requires --fan-out") {
        return Some("fan_out_early_return_requires_fan_out");
    }
    if text.contains("duplicate --var:") {
        return Some("command_var_duplicate");
    }
    if text.contains("--var references undeclared input:") {
        return Some("command_var_undeclared_input");
    }
    if text.contains("missing required --var:") {
        return Some("command_var_missing_required");
    }
    if text.contains("context step abandoned:") {
        return Some("context_step_abandoned");
    }
    if text.contains("context step ended with unexpected status:") {
        return Some("context_step_unexpected_status");
    }
    if text.contains("context step missing exit_code:") {
        return Some("context_step_missing_exit_code");
    }
    if text.contains("context step failed:") {
        return Some("context_step_failed");
    }
    if text.contains("fan-out linkage issue") {
        return Some("fan_out_linkage_issue");
    }
    if text.contains("fan-out task is not completed") {
        return Some("fan_out_task_not_completed");
    }
    if text.contains("thread/start auto hook denied:") {
        return Some("thread_start_auto_hook_denied");
    }
    if text.contains("thread/start auto hook error:") {
        return Some("thread_start_auto_hook_error");
    }
    None
}

pub(super) fn ensure_auto_hook_ready(
    action: &str,
    hook_context: &str,
    auto_hook: &omne_app_server_protocol::ThreadAutoHookResponse,
) -> anyhow::Result<()> {
    match auto_hook {
        omne_app_server_protocol::ThreadAutoHookResponse::Ok(_) => Ok(()),
        omne_app_server_protocol::ThreadAutoHookResponse::NeedsApproval(response) => {
            eprintln!(
                "[{action}] {hook_context} needs approval: hook={} thread_id={} approval_id={} approve_cmd=`omne approval decide {} {} --approve` deny_cmd=`omne approval decide {} {} --deny`",
                response.hook,
                response.thread_id,
                response.approval_id,
                response.thread_id,
                response.approval_id,
                response.thread_id,
                response.approval_id,
            );
            Ok(())
        }
        omne_app_server_protocol::ThreadAutoHookResponse::Denied(response) => {
            let detail =
                serde_json::to_string(response).unwrap_or_else(|_| format!("{response:?}"));
            if let Some(error_code) = response.error_code.as_deref() {
                anyhow::bail!(
                    "[rpc error_code] {error_code}; {action} {hook_context} denied: {detail}"
                );
            }
            anyhow::bail!("{action} {hook_context} denied: {detail}");
        }
        omne_app_server_protocol::ThreadAutoHookResponse::Error(response) => {
            anyhow::bail!(
                "{action} {hook_context} error: hook={} error={}",
                response.hook,
                response.error
            );
        }
    }
}

pub(super) fn ensure_thread_start_auto_hook_ready(
    action: &str,
    started: &omne_app_server_protocol::ThreadStartResponse,
) -> anyhow::Result<()> {
    ensure_auto_hook_ready(action, "thread/start auto hook", &started.auto_hook)
}

pub(super) fn first_non_completed_result(results: &[WorkflowTaskResult]) -> Option<&WorkflowTaskResult> {
    results
        .iter()
        .find(|result| !matches!(result.status, TurnStatus::Completed))
}

pub(super) fn format_non_completed_fan_out_issue(
    prefix: &str,
    result: &WorkflowTaskResult,
    parent_thread_id: ThreadId,
    artifact_id: omne_protocol::ArtifactId,
) -> String {
    let artifact_error = result
        .result_artifact_error
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("-");
    let artifact_error_read_cmd = result
        .result_artifact_error_id
        .map(|error_artifact_id| fan_out_result_read_command(parent_thread_id, error_artifact_id))
        .unwrap_or_else(|| "-".to_string());
    let pending_approval = result
        .pending_approval
        .as_ref()
        .map(|pending| {
            let mut text = format!(
                " pending_approval_action={} pending_approval_id={}",
                pending.action, pending.approval_id
            );
            if let Some(summary) = pending
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                text.push_str(&format!(" pending_approval_summary={summary}"));
            }
            let approve_cmd = pending
                .approve_cmd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            let deny_cmd = pending
                .deny_cmd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);

            if let Some(approve_cmd) = approve_cmd {
                text.push_str(&format!(" approve_cmd={approve_cmd}"));
            }
            if let Some(deny_cmd) = deny_cmd {
                text.push_str(&format!(" deny_cmd={deny_cmd}"));
            }

            if (pending.approve_cmd.is_none() || pending.deny_cmd.is_none())
                && let Some(thread_id) = result.thread_id
            {
                if pending.approve_cmd.is_none() {
                    let approve_cmd = approval_decide_command(thread_id, pending.approval_id, true);
                    text.push_str(&format!(" approve_cmd={approve_cmd}"));
                }
                if pending.deny_cmd.is_none() {
                    let deny_cmd = approval_decide_command(thread_id, pending.approval_id, false);
                    text.push_str(&format!(" deny_cmd={deny_cmd}"));
                }
            }
            text
        })
        .unwrap_or_default();
    format!(
        "{prefix}: task_id={} status={:?} thread_id={} turn_id={} artifact_error={} artifact_error_read_cmd={}{} (see fan_in_summary artifact_id={artifact_id})",
        result.task_id,
        result.status,
        display_thread_id(result.thread_id),
        display_turn_id(result.turn_id),
        artifact_error,
        artifact_error_read_cmd,
        pending_approval,
    )
}

pub(super) fn first_non_completed_task_from_fan_in_summary(
    payload: &omne_app_server_protocol::ArtifactFanInSummaryStructuredData,
) -> Option<&omne_app_server_protocol::ArtifactFanInSummaryTask> {
    payload
        .tasks
        .iter()
        .find(|task| !task.status.eq_ignore_ascii_case("completed"))
}

pub(super) fn display_optional_structured_id(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
}

pub(super) fn format_non_completed_fan_out_issue_from_structured_task(
    prefix: &str,
    parent_thread_id: &str,
    task: &omne_app_server_protocol::ArtifactFanInSummaryTask,
    artifact_id: omne_protocol::ArtifactId,
) -> String {
    let artifact_error = task
        .result_artifact_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-");
    let artifact_error_read_cmd = task
        .result_artifact_error_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|error_artifact_id| format!("omne artifact read {parent_thread_id} {error_artifact_id}"))
        .unwrap_or_else(|| "-".to_string());
    let pending_approval = task
        .pending_approval
        .as_ref()
        .map(|pending| {
            let mut text = format!(
                " pending_approval_action={} pending_approval_id={}",
                pending.action, pending.approval_id
            );
            if let Some(summary) = pending
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                text.push_str(&format!(" pending_approval_summary={summary}"));
            }
            if let Some(approve_cmd) = pending
                .approve_cmd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                text.push_str(&format!(" approve_cmd={approve_cmd}"));
            }
            if let Some(deny_cmd) = pending
                .deny_cmd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                text.push_str(&format!(" deny_cmd={deny_cmd}"));
            }
            text
        })
        .unwrap_or_default();
    let dependency_blocker = if task.dependency_blocked
        || task
            .dependency_blocker_task_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || task
            .dependency_blocker_status
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        let mut text = format!(" dependency_blocked={}", task.dependency_blocked);
        if let Some(task_id) = task
            .dependency_blocker_task_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            text.push_str(&format!(" dependency_blocker_task_id={task_id}"));
        }
        if let Some(status) = task
            .dependency_blocker_status
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            text.push_str(&format!(" dependency_blocker_status={status}"));
        }
        text
    } else {
        String::new()
    };

    format!(
        "{prefix}: task_id={} status={} thread_id={} turn_id={} artifact_error={} artifact_error_read_cmd={}{}{} (see fan_in_summary artifact_id={artifact_id})",
        task.task_id,
        task.status,
        display_optional_structured_id(task.thread_id.as_deref()),
        display_optional_structured_id(task.turn_id.as_deref()),
        artifact_error,
        artifact_error_read_cmd,
        pending_approval,
        dependency_blocker,
    )
}

pub(super) async fn format_non_completed_fan_out_issue_from_summary_artifact(
    app: &mut App,
    parent_thread_id: ThreadId,
    artifact_id: omne_protocol::ArtifactId,
    prefix: &str,
) -> Option<String> {
    let read = app
        .artifact_read(parent_thread_id, artifact_id, None, None, None)
        .await
        .ok()?;
    let payload = read.fan_in_summary?;
    let task = first_non_completed_task_from_fan_in_summary(&payload)?;
    Some(format_non_completed_fan_out_issue_from_structured_task(
        prefix,
        payload.thread_id.as_str(),
        task,
        artifact_id,
    ))
}

pub(super) fn format_fan_out_linkage_issue_from_structured_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData,
    artifact_id: omne_protocol::ArtifactId,
) -> Option<String> {
    format_fan_out_linkage_issue_detail_from_payload(payload, artifact_id)
}

pub(super) async fn format_fan_out_linkage_issue_from_artifact(
    app: &mut App,
    parent_thread_id: ThreadId,
    artifact_id: omne_protocol::ArtifactId,
) -> Option<String> {
    let read = app
        .artifact_read(parent_thread_id, artifact_id, None, None, None)
        .await
        .ok()?;
    let payload = read.fan_out_linkage_issue?;
    format_fan_out_linkage_issue_from_structured_payload(&payload, artifact_id)
}

pub(super) fn format_fan_out_linkage_issue_clear_from_structured_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData,
    artifact_id: omne_protocol::ArtifactId,
) -> String {
    format_fan_out_linkage_issue_clear_detail_from_payload(payload, artifact_id)
}

pub(super) fn default_fan_out_linkage_issue_clear_text(artifact_id: omne_protocol::ArtifactId) -> String {
    format!("fan-out linkage issue cleared (see fan_out_linkage_issue_clear artifact_id={artifact_id})")
}

pub(super) async fn format_fan_out_linkage_issue_clear_from_artifact(
    app: &mut App,
    parent_thread_id: ThreadId,
    artifact_id: omne_protocol::ArtifactId,
) -> Option<String> {
    let read = app
        .artifact_read(parent_thread_id, artifact_id, None, None, None)
        .await
        .ok()?;
    let payload = read.fan_out_linkage_issue_clear?;
    Some(format_fan_out_linkage_issue_clear_from_structured_payload(
        &payload,
        artifact_id,
    ))
}

pub(super) async fn validate_fan_out_results_with_artifact_fallback(
    app: &mut App,
    results: &[WorkflowTaskResult],
    parent_thread_id: ThreadId,
    artifact_id: omne_protocol::ArtifactId,
    require_completed: bool,
    prefix: &str,
) -> anyhow::Result<()> {
    if let Err(err) = validate_fan_out_results(results, parent_thread_id, artifact_id, require_completed) {
        if let Some(issue) = format_non_completed_fan_out_issue_from_summary_artifact(
            app,
            parent_thread_id,
            artifact_id,
            prefix,
        )
        .await
        {
            anyhow::bail!("{issue}");
        }
        return Err(err);
    }
    Ok(())
}

pub(super) fn validate_fan_out_results(
    results: &[WorkflowTaskResult],
    parent_thread_id: ThreadId,
    artifact_id: omne_protocol::ArtifactId,
    require_completed: bool,
) -> anyhow::Result<()> {
    if !require_completed {
        return Ok(());
    }
    if let Some(result) = first_non_completed_result(results) {
        anyhow::bail!(
            "{}",
            format_non_completed_fan_out_issue(
                "fan-out task is not completed",
                result,
                parent_thread_id,
                artifact_id,
            )
        );
    }
    Ok(())
}

pub(super) async fn wait_for_process_exit(
    app: &mut App,
    process_id: ProcessId,
    summary: &str,
    ok_exit_codes: &[i32],
) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(250);
    loop {
        let resp = app.process_inspect(process_id, Some(0), None).await?;
        let process = resp.process;
        if process.status == omne_app_server_protocol::ProcessStatus::Running {
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        if process.status == omne_app_server_protocol::ProcessStatus::Abandoned {
            anyhow::bail!("context step abandoned: summary={summary} process_id={process_id}");
        }
        if process.status != omne_app_server_protocol::ProcessStatus::Exited {
            let status = match process.status {
                omne_app_server_protocol::ProcessStatus::Running => "running",
                omne_app_server_protocol::ProcessStatus::Exited => "exited",
                omne_app_server_protocol::ProcessStatus::Abandoned => "abandoned",
            };
            anyhow::bail!(
                "context step ended with unexpected status: summary={summary} process_id={process_id} status={status}"
            );
        }
        let exit_code = process.exit_code.ok_or_else(|| {
            anyhow::anyhow!("context step missing exit_code: summary={summary} process_id={process_id}")
        })?;
        if !ok_exit_codes.contains(&exit_code) {
            anyhow::bail!(
                "context step failed: summary={summary} process_id={process_id} exit_code={exit_code} ok_exit_codes={ok_exit_codes:?}"
            );
        }
        return Ok(());
    }
}

pub(super) async fn run_command_list(cli: &Cli, json: bool) -> anyhow::Result<()> {
    let omne_root = resolve_pm_root(cli)?;
    let mode_catalog = omne_core::modes::ModeCatalog::load(&omne_root).await;
    let dir = omne_workflow_spec::workflow_spec_dir(&omne_root);
    if !tokio::fs::try_exists(&dir).await? {
        anyhow::bail!(
            "commands dir is missing: {} (run `omne init`?)",
            dir.display()
        );
    }

    let result = collect_command_list_result_with_mode_catalog(&dir, &mode_catalog).await?;
    if json {
        print_json_or_pretty(true, &serde_json::to_value(&result)?)?;
    } else {
        if let Some(load_error) = result.modes_load_error.as_deref() {
            eprintln!("[command/list modes load warning] {load_error}");
        }
        if result.commands.is_empty() {
            println!("(no commands)");
        } else {
            for entry in &result.commands {
                println!(
                    "{} mode={} version={}",
                    entry.name, entry.mode, entry.version
                );
            }
        }
        if !result.errors.is_empty() {
            eprintln!("[command/list parse errors: {}]", result.errors.len());
            for item in result.errors.iter().take(3) {
                eprintln!("- {}: {}", item.file, item.error);
            }
            if result.errors.len() > 3 {
                eprintln!("- ... and {} more", result.errors.len() - 3);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) async fn collect_command_list_result(dir: &std::path::Path) -> anyhow::Result<CommandListResult> {
    let mode_catalog = omne_core::modes::ModeCatalog::builtin();
    collect_command_list_result_with_mode_catalog(dir, &mode_catalog).await
}

pub(super) async fn collect_command_list_result_with_mode_catalog(
    dir: &std::path::Path,
    mode_catalog: &omne_core::modes::ModeCatalog,
) -> anyhow::Result<CommandListResult> {
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .with_context(|| format!("read dir {}", dir.display()))?;
    let mut commands = Vec::<CommandListItem>::new();
    let mut errors = Vec::<CommandListError>::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        let parsed = async {
            let raw = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("read {}", path.display()))?;
            let (yaml, _) = split_frontmatter(&raw)
                .with_context(|| format!("parse frontmatter {}", path.display()))?;
            let fm = serde_yaml::from_str::<WorkflowFileFrontmatterV1>(yaml)
                .with_context(|| format!("parse frontmatter yaml {}", path.display()))?;
            sanitize_frontmatter(fm, stem.to_string(), mode_catalog)
                .with_context(|| format!("sanitize {}", path.display()))
        }
        .await;

        match parsed {
            Ok(fm) => commands.push(CommandListItem {
                name: fm.name.unwrap_or_else(|| stem.to_string()),
                version: fm.version,
                mode: fm.mode,
                file: path.display().to_string(),
            }),
            Err(err) => errors.push(CommandListError {
                file: path.display().to_string(),
                error: format!("{err:#}"),
                error_code: command_error_code(&err).map(ToString::to_string),
            }),
        }
    }

    commands.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.file.cmp(&b.file)));
    errors.sort_by(|a, b| a.file.cmp(&b.file));
    let command_count = commands.len();
    let summary = CommandResultSummary::new(dir.display().to_string(), command_count, errors.len());
    Ok(CommandListResult {
        summary,
        command_count,
        commands,
        errors,
        modes_load_error: mode_catalog.load_error.clone(),
    })
}

pub(super) async fn run_command_show(cli: &Cli, name: &str, json: bool) -> anyhow::Result<()> {
    let wf = load_workflow_file(cli, name).await?;
    if json {
        let mut v = serde_json::Map::new();
        v.insert("frontmatter".to_string(), serde_json::to_value(wf.frontmatter)?);
        v.insert("body".to_string(), serde_json::Value::String(wf.body));
        if let Some(load_error) = wf.modes_load_error {
            v.insert(
                "modes_load_error".to_string(),
                serde_json::Value::String(load_error),
            );
        }
        print_json_or_pretty(true, &serde_json::Value::Object(v))?;
        return Ok(());
    }

    if let Some(load_error) = wf.modes_load_error.as_deref() {
        eprintln!("[command/show modes load warning] {load_error}");
    }

    println!("---");
    print!("{}", serde_yaml::to_string(&wf.frontmatter)?);
    println!("---");
    print!("{}", wf.body);
    if !wf.body.ends_with('\n') {
        println!();
    }
    Ok(())
}

pub(super) async fn run_command_validate(
    cli: &Cli,
    name: Option<String>,
    strict: bool,
    json: bool,
) -> anyhow::Result<()> {
    let result = collect_command_validate_result(cli, name, strict).await?;

    if json {
        print_json_or_pretty(true, &serde_json::to_value(&result)?)?;
    } else {
        if let Some(load_error) = result.modes_load_error.as_deref() {
            eprintln!("[command/validate modes load warning] {load_error}");
        }
        if result.validated.is_empty() {
            println!("(no commands validated)");
        } else {
            for item in &result.validated {
                println!(
                    "ok: {} mode={} version={} {}",
                    item.name, item.mode, item.version, item.file
                );
            }
        }
        if !result.errors.is_empty() {
            eprintln!("[command/validate errors: {}]", result.errors.len());
            for item in &result.errors {
                eprintln!("- {}: {}", item.file, item.error);
            }
        }
    }

    if result.summary.ok {
        return Ok(());
    }

    if result.target != "all" {
        if result.errors.len() == 1 {
            anyhow::bail!(
                "command `{}` validation failed: {}",
                result.target,
                result.errors[0].error
            );
        }
        anyhow::bail!(
            "command `{}` validation failed: {} issue(s)",
            result.target,
            result.errors.len()
        );
    }

    anyhow::bail!(
        "command validation failed: {} file(s) with errors",
        result.errors.len()
    )
}

pub(super) async fn collect_command_validate_result(
    cli: &Cli,
    name: Option<String>,
    strict: bool,
) -> anyhow::Result<CommandValidateResult> {
    let omne_root = resolve_pm_root(cli)?;
    let mode_catalog = omne_core::modes::ModeCatalog::load(&omne_root).await;
    let dir = omne_workflow_spec::workflow_spec_dir(&omne_root);
    if !tokio::fs::try_exists(&dir).await? {
        anyhow::bail!(
            "commands dir is missing: {} (run `omne init`?)",
            dir.display()
        );
    }

    let requested_name = name.clone();
    let mut targets = Vec::<(String, PathBuf)>::new();
    if let Some(name) = name {
        omne_workflow_spec::validate_workflow_name(&name)?;
        targets.push((name.clone(), dir.join(format!("{name}.md"))));
    } else {
        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .with_context(|| format!("read dir {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            targets.push((stem.to_string(), path));
        }
        targets.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    }

    let mut validated = Vec::<CommandValidateItem>::new();
    let mut errors = Vec::<CommandValidateError>::new();
    for (stem, path) in targets {
        let raw = match tokio::fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(err) => {
                errors.push(CommandValidateError {
                    file: path.display().to_string(),
                    error: format!("read {}: {err}", path.display()),
                    error_code: Some("read_failed".to_string()),
                });
                continue;
            }
        };

        let parsed = (|| -> anyhow::Result<WorkflowFileFrontmatterV1> {
            let (yaml, _) =
                split_frontmatter(&raw).with_context(|| format!("parse frontmatter {}", path.display()))?;
            let fm = serde_yaml::from_str::<WorkflowFileFrontmatterV1>(yaml)
                .with_context(|| format!("parse frontmatter yaml {}", path.display()))?;
            sanitize_frontmatter(fm, stem.clone(), &mode_catalog)
                .with_context(|| format!("sanitize {}", path.display()))
        })();

        match parsed {
            Ok(fm) => {
                let name = fm.name.unwrap_or(stem);
                validated.push(CommandValidateItem {
                    name,
                    version: fm.version,
                    mode: fm.mode,
                    file: path.display().to_string(),
                });
            }
            Err(err) => errors.push(CommandValidateError {
                file: path.display().to_string(),
                error: format!("{err:#}"),
                error_code: command_error_code(&err).map(ToString::to_string),
            }),
        }
    }

    validated.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.file.cmp(&b.file)));
    if strict {
        errors.extend(duplicate_command_name_errors(&validated));
    }
    errors.sort_by(|a, b| a.file.cmp(&b.file));

    let validated_count = validated.len();
    let summary = CommandResultSummary::new(dir.display().to_string(), validated_count, errors.len());
    let result = CommandValidateResult {
        summary,
        strict,
        target: requested_name.unwrap_or_else(|| "all".to_string()),
        validated_count,
        validated,
        errors,
        modes_load_error: mode_catalog.load_error.clone(),
    };
    Ok(result)
}

pub(super) async fn run_command_run(
    cli: &Cli,
    app: &mut App,
    command: &CommandCommand,
) -> anyhow::Result<()> {
    let CommandCommand::Run(args) = command else {
        anyhow::bail!("command execution is only supported via `omne command run`");
    };
    if args.fan_out_early_return && !args.fan_out {
        anyhow::bail!("--fan-out-early-return requires --fan-out");
    }

    let wf = load_workflow_file(cli, &args.name).await?;
    if let Some(load_error) = wf.modes_load_error.as_deref() {
        eprintln!("[command/run modes load warning] {load_error}");
    }

    let mut declared = BTreeSet::<String>::new();
    let mut required = BTreeSet::<String>::new();
    for input in &wf.frontmatter.inputs {
        declared.insert(input.name.clone());
        if input.required {
            required.insert(input.name.clone());
        }
    }

    let vars = collect_vars(&args.vars)?;
    for k in vars.keys() {
        if !declared.contains(k) {
            anyhow::bail!("--var references undeclared input: {k}");
        }
    }
    for k in required {
        if !vars.contains_key(&k) {
            anyhow::bail!("missing required --var: {k}");
        }
    }

    let rendered_body = render_template(&wf.body, &declared, &vars)?;
    let fan_out_require_completed = fan_out_require_completed();

    let thread_id = if let Some(thread_id) = args.thread_id {
        let resumed = app.thread_resume(thread_id).await?;
        resumed.thread_id
    } else {
        let cwd = args.cwd.as_ref().map(|p| p.display().to_string());
        let started = app.thread_start(cwd).await?;
        ensure_thread_start_auto_hook_ready("command/run", &started)?;
        started.thread_id
    };
    app.thread_configure_rpc(omne_app_server_protocol::ThreadConfigureParams {
        thread_id,
        approval_policy: None,
        sandbox_policy: None,
        sandbox_writable_roots: None,
        sandbox_network_access: None,
        mode: Some(wf.frontmatter.mode.clone()),
        model: None,
        thinking: None,
        openai_base_url: None,
        allowed_tools: wf.frontmatter.allowed_tools.clone().map(Some),
        execpolicy_rules: None,
    })
    .await?;

    let mut process_ids = Vec::<ProcessId>::new();
    for step in &wf.frontmatter.context {
        let summary = render_template(&step.summary, &declared, &vars)?;
        let argv = step
            .argv
            .iter()
            .map(|arg| render_template(arg, &declared, &vars))
            .collect::<anyhow::Result<Vec<_>>>()?;
        eprintln!("[command context] {summary}");

        let parsed = app
            .process_start(omne_app_server_protocol::ProcessStartParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                argv,
                cwd: None,
                timeout_ms: None,
            })
            .await?;
        let process_id = parsed.process_id;
        let ok_exit_codes = step.ok_exit_codes.as_deref().unwrap_or(&[0]);
        wait_for_process_exit(app, process_id, &summary, ok_exit_codes).await?;
        process_ids.push(process_id);
    }

    let mut fan_in_artifact_id: Option<omne_protocol::ArtifactId> = None;
    let mut fan_out_results = Vec::<WorkflowTaskResult>::new();
    let mut fan_out_scheduler: Option<FanOutScheduler> = None;
    if args.fan_out {
        let tasks = parse_workflow_tasks(&rendered_body)?;
        if !tasks.is_empty() {
            let scheduling = fan_out_scheduling_params(tasks.len());
            let artifact_id = omne_protocol::ArtifactId::new();
            if args.fan_out_early_return {
                let mut scheduler = FanOutScheduler::start(
                    app,
                    thread_id,
                    tasks,
                    artifact_id,
                    wf.frontmatter.subagent_fork,
                )
                .await?;
                scheduler.tick(app, thread_id, None).await?;
                fan_out_scheduler = Some(scheduler);
            } else {
                let scheduler = FanOutScheduler::start(
                    app,
                    thread_id,
                    tasks,
                    artifact_id,
                    wf.frontmatter.subagent_fork,
                )
                .await?;
                fan_out_results = scheduler.run_to_completion(app, thread_id, None).await?;
                let _artifact_path =
                    write_fan_in_summary_artifact(
                        app,
                        thread_id,
                        None,
                        artifact_id,
                        &fan_out_results,
                        scheduling,
                        None,
                    )
                    .await?;
                validate_fan_out_results_with_artifact_fallback(
                    app,
                    &fan_out_results,
                    thread_id,
                    artifact_id,
                    fan_out_require_completed,
                    "fan-out task is not completed",
                )
                .await?;
                if let Some(clear_artifact_id) =
                    try_clear_fan_out_linkage_issue_marker(app, thread_id, None, artifact_id).await
                {
                    let clear_text = format_fan_out_linkage_issue_clear_from_artifact(
                        app,
                        thread_id,
                        clear_artifact_id,
                    )
                    .await
                    .unwrap_or_else(|| default_fan_out_linkage_issue_clear_text(clear_artifact_id));
                    eprintln!("[fan-out] {clear_text}");
                }
            }
            fan_in_artifact_id = Some(artifact_id);
        }
    }

    let mut input = String::new();
    if !process_ids.is_empty() {
        let ids = process_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        input.push_str(&format!(
            "Context steps executed. process_id(s)={ids}. Use `omne process inspect/tail/follow` for details.\n\n"
        ));
    }
    if let Some(artifact_id) = fan_in_artifact_id {
        if args.fan_out_early_return {
            input.push_str(&format!(
                "Fan-out tasks started (early return). fan_in_summary artifact_id={artifact_id} (updates while the main turn runs).\n"
            ));
            if let Some(scheduler) = fan_out_scheduler.as_ref() {
                for task in &scheduler.tasks {
                    input.push_str(&format!("- task_id={} title={}\n", task.id, task.title));
                }
            }
        } else {
            input.push_str(&format!(
                "Fan-out tasks completed. fan_in_summary artifact_id={artifact_id}.\n"
            ));
            for result in &fan_out_results {
                input.push_str(&format!(
                    "- task_id={} thread_id={} turn_id={} status={:?}\n",
                    result.task_id,
                    display_thread_id(result.thread_id),
                    display_turn_id(result.turn_id),
                    result.status
                ));
            }
        }
        input.push('\n');
        input.push_str(&format!(
            "Use `omne artifact read {thread_id} {artifact_id}` for the full fan-in summary.\n\n"
        ));
    }
    input.push_str(&rendered_body);

    let ask_args = AskArgs {
        thread_id: Some(thread_id),
        cwd: None,
        approval_policy: None,
        sandbox_policy: None,
        mode: None,
        model: None,
        openai_base_url: None,
        input,
    };

    if let Some(scheduler) = fan_out_scheduler {
        let artifact_id = fan_in_artifact_id.unwrap_or_default();
        let scheduler = Arc::new(tokio::sync::Mutex::new(Some(scheduler)));
        let fan_out_abort_reason = Arc::new(tokio::sync::Mutex::new(None::<String>));

        let scheduler_for_tick = scheduler.clone();
        let fan_out_abort_reason_for_tick = fan_out_abort_reason.clone();
        let parent_turn_id = run_ask_with_tick(app, ask_args, move |app, parent_thread_id, turn_id| {
            let scheduler_for_tick = scheduler_for_tick.clone();
            let fan_out_abort_reason_for_tick = fan_out_abort_reason_for_tick.clone();
            Box::pin(async move {
                let mut guard = scheduler_for_tick.lock().await;
                let Some(mut scheduler) = guard.take() else {
                    return Ok(());
                };
                drop(guard);

                let mut linkage_issue = None::<String>;
                if let Err(err) = scheduler.tick(app, parent_thread_id, Some(turn_id)).await {
                    linkage_issue = Some(format!("fan-out linkage issue: {err}"));
                } else if fan_out_require_completed {
                    let has_non_completed = scheduler.first_non_completed_result().is_some();
                    if has_non_completed {
                        linkage_issue = format_non_completed_fan_out_issue_from_summary_artifact(
                            app,
                            parent_thread_id,
                            artifact_id,
                            "fan-out linkage issue",
                        )
                        .await;

                        if linkage_issue.is_none()
                            && let Some(result) = scheduler.first_non_completed_result()
                        {
                            linkage_issue = Some(format_non_completed_fan_out_issue(
                                "fan-out linkage issue",
                                result,
                                parent_thread_id,
                                artifact_id,
                            ));
                        }
                    }
                }

                if let Some(issue) = linkage_issue {
                    let should_interrupt = fan_out_abort_reason_for_tick.lock().await.is_none();
                    if should_interrupt {
                        let mut issue_with_handles = issue;
                        if let Some(linkage_artifact_id) = try_write_fan_out_linkage_issue_artifact(
                            app,
                            parent_thread_id,
                            Some(turn_id),
                            artifact_id,
                            &issue_with_handles,
                        )
                        .await
                        {
                            if let Some(structured_issue) = format_fan_out_linkage_issue_from_artifact(
                                app,
                                parent_thread_id,
                                linkage_artifact_id,
                            )
                            .await
                            {
                                issue_with_handles = structured_issue;
                            }
                            issue_with_handles.push_str(&format!(
                                " linkage_issue_read_cmd={}",
                                fan_out_result_read_command(parent_thread_id, linkage_artifact_id)
                            ));
                        }
                        {
                            let mut reason = fan_out_abort_reason_for_tick.lock().await;
                            if reason.is_none() {
                                *reason = Some(issue_with_handles.clone());
                            }
                        }
                        eprintln!("[fan-out] {issue_with_handles}");
                        let _ = app
                            .turn_interrupt(parent_thread_id, turn_id, Some(issue_with_handles))
                            .await;
                    }
                }

                if scheduler.is_done() && !scheduler.final_summary_written {
                    let results = scheduler.results_ordered();
                    let summary_linkage_issue = fan_out_abort_reason_for_tick.lock().await.clone();
                    let outcome =
                        write_fan_in_summary_artifact(
                            app,
                            parent_thread_id,
                            Some(turn_id),
                            artifact_id,
                            &results,
                            scheduler.scheduling,
                            summary_linkage_issue.as_deref(),
                        )
                        .await;
                    if let Err(err) = outcome {
                        eprintln!("[fan-out] final summary artifact write failed: {err}");
                    } else {
                        scheduler.final_summary_written = true;
                    }
                }

                let mut guard = scheduler_for_tick.lock().await;
                *guard = Some(scheduler);
                Ok(())
            })
        })
        .await?;

        let mut scheduler = scheduler
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow::anyhow!("fan-out scheduler missing"))?;
        while !scheduler.is_done() {
            scheduler
                .tick(app, thread_id, Some(parent_turn_id))
                .await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let results = scheduler.results_ordered();
        if !scheduler.final_summary_written {
            let summary_linkage_issue = fan_out_abort_reason.lock().await.clone();
            let _artifact_path =
                write_fan_in_summary_artifact(
                    app,
                    thread_id,
                    Some(parent_turn_id),
                    artifact_id,
                    &results,
                    scheduler.scheduling,
                    summary_linkage_issue.as_deref(),
                )
                .await?;
        }
        validate_fan_out_results_with_artifact_fallback(
            app,
            &results,
            thread_id,
            artifact_id,
            fan_out_require_completed,
            "fan-out task is not completed",
        )
        .await?;
        let abort_reason = fan_out_abort_reason.lock().await.clone();
        if abort_reason.is_none() {
            if let Some(clear_artifact_id) = try_clear_fan_out_linkage_issue_marker(
                app,
                thread_id,
                Some(parent_turn_id),
                artifact_id,
            )
            .await
            {
                let clear_text = format_fan_out_linkage_issue_clear_from_artifact(
                    app,
                    thread_id,
                    clear_artifact_id,
                )
                .await
                .unwrap_or_else(|| default_fan_out_linkage_issue_clear_text(clear_artifact_id));
                eprintln!("[fan-out] {clear_text}");
            }
        }
        if let Some(reason) = abort_reason {
            anyhow::bail!("{reason}");
        }
    } else {
        run_ask(app, ask_args).await?;
    }

    Ok(())
}
