fn is_known_agent_tool(tool_name: &str) -> bool {
    is_known_agent_tool_name(tool_name)
}

const FAN_OUT_PRIORITY_AGING_ROUNDS_ENV: &str = "OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS";
const DEFAULT_FAN_OUT_PRIORITY_AGING_ROUNDS: usize = 3;
const MIN_FAN_OUT_PRIORITY_AGING_ROUNDS: usize = 1;
const MAX_FAN_OUT_PRIORITY_AGING_ROUNDS: usize = 10_000;

fn parse_fan_out_priority_aging_rounds_value(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| {
            value.clamp(
                MIN_FAN_OUT_PRIORITY_AGING_ROUNDS,
                MAX_FAN_OUT_PRIORITY_AGING_ROUNDS,
            )
        })
        .unwrap_or(DEFAULT_FAN_OUT_PRIORITY_AGING_ROUNDS)
}

fn fan_out_priority_aging_rounds_from_env() -> usize {
    parse_fan_out_priority_aging_rounds_value(
        std::env::var(FAN_OUT_PRIORITY_AGING_ROUNDS_ENV)
            .ok()
            .as_deref(),
    )
}

fn is_plan_read_only_tool(tool_name: &str) -> bool {
    is_plan_read_only_agent_tool(tool_name)
}

fn plan_tool_action(tool_name: &str) -> Option<&'static str> {
    let action = agent_tool_action(tool_name)?;
    is_plan_read_only_tool(tool_name).then_some(action)
}

fn plan_architect_base_decision(
    mode: &omne_core::modes::ModeDef,
    tool_action: &str,
) -> Option<omne_core::modes::Decision> {
    let decision = match tool_action {
        "file/read" | "file/glob" | "file/grep" | "thread/state" | "thread/usage"
        | "thread/events" => mode.permissions.read,
        "repo/search" | "repo/index" | "repo/symbols" => {
            mode.permissions.read.combine(mode.permissions.artifact)
        }
        "mcp/list_servers" | "mcp/list_tools" | "mcp/list_resources" => mode.permissions.read,
        "process/inspect" | "process/tail" | "process/follow" => mode.permissions.process.inspect,
        "artifact/list" | "artifact/read" => mode.permissions.artifact,
        "thread/diff" => mode.permissions.command.combine(mode.permissions.artifact),
        _ => return None,
    };
    Some(decision)
}

fn plan_architect_effective_decision(
    mode: &omne_core::modes::ModeDef,
    tool_action: &str,
) -> Option<omne_core::modes::Decision> {
    let base_decision = plan_architect_base_decision(mode, tool_action)?;
    Some(crate::resolve_mode_decision_audit(mode, tool_action, base_decision).decision)
}

async fn plan_architect_file_read_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: FileReadArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = match omne_core::modes::relative_path_under_root(
        &target_root,
        std::path::Path::new(&parsed.path),
    ) {
        Ok(rel) if mode.permissions.edit.is_denied(&rel) => omne_core::modes::Decision::Deny,
        Ok(_) => mode.permissions.read,
        Err(_) => omne_core::modes::Decision::Deny,
    };
    Some(crate::resolve_mode_decision_audit(mode, "file/read", base_decision).decision)
}

fn is_glob_meta_char(ch: char) -> bool {
    matches!(ch, '*' | '?' | '[' | ']' | '{' | '}')
}

fn glob_static_prefix_for_mode_path(pattern: &str) -> (Option<&str>, bool) {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return (None, false);
    }

    let first_meta = pattern
        .char_indices()
        .find_map(|(idx, ch)| is_glob_meta_char(ch).then_some(idx));

    let prefix = match first_meta {
        None => pattern,
        Some(0) => return (None, true),
        Some(idx) => pattern[..idx].trim_end_matches('/'),
    };

    if prefix.is_empty() {
        (None, first_meta.is_some())
    } else {
        (Some(prefix), first_meta.is_some())
    }
}

async fn resolve_plan_target_root(
    thread_root: &std::path::Path,
    root: crate::FileRoot,
) -> Option<std::path::PathBuf> {
    match root {
        crate::FileRoot::Workspace => Some(thread_root.to_path_buf()),
        crate::FileRoot::Reference => omne_core::resolve_dir(
            thread_root,
            std::path::Path::new(".omne_data/reference/repo"),
        )
        .await
        .ok(),
    }
}

fn read_decision_with_optional_explicit_path(
    mode: &omne_core::modes::ModeDef,
    target_root: &std::path::Path,
    maybe_path: Option<&str>,
) -> omne_core::modes::Decision {
    let Some(path) = maybe_path else {
        return mode.permissions.read;
    };
    let (prefix, has_meta) = glob_static_prefix_for_mode_path(path);
    let Some(path) = prefix else {
        return mode.permissions.read;
    };

    match omne_core::modes::relative_path_under_root(target_root, std::path::Path::new(path)) {
        Ok(rel) if mode.permissions.edit.is_denied(&rel) => {
            return omne_core::modes::Decision::Deny;
        }
        Err(_) => return omne_core::modes::Decision::Deny,
        Ok(_) => {}
    }

    // For glob patterns, probe a synthetic child path so rules like `blocked/**` apply to
    // prefixes such as `blocked/**/*.rs`.
    if has_meta {
        let synthetic = format!("{}/__omne_glob_probe__", path.trim_end_matches('/'));
        match omne_core::modes::relative_path_under_root(
            target_root,
            std::path::Path::new(&synthetic),
        ) {
            Ok(rel) if mode.permissions.edit.is_denied(&rel) => {
                return omne_core::modes::Decision::Deny;
            }
            Err(_) => return omne_core::modes::Decision::Deny,
            Ok(_) => {}
        }
    }

    mode.permissions.read
}

async fn plan_architect_file_glob_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: FileGlobArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = read_decision_with_optional_explicit_path(
        mode,
        &target_root,
        Some(parsed.pattern.as_str()),
    );
    Some(crate::resolve_mode_decision_audit(mode, "file/glob", base_decision).decision)
}

async fn plan_architect_file_grep_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: FileGrepArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = read_decision_with_optional_explicit_path(
        mode,
        &target_root,
        parsed.include_glob.as_deref(),
    );
    Some(crate::resolve_mode_decision_audit(mode, "file/grep", base_decision).decision)
}

fn plan_architect_repo_base_decision_after_path_gate(
    mode: &omne_core::modes::ModeDef,
    target_root: &std::path::Path,
    include_glob: Option<&str>,
) -> omne_core::modes::Decision {
    let path_decision = read_decision_with_optional_explicit_path(mode, target_root, include_glob);
    if path_decision == omne_core::modes::Decision::Deny {
        omne_core::modes::Decision::Deny
    } else {
        mode.permissions.read.combine(mode.permissions.artifact)
    }
}

fn plan_architect_thread_scope_decision(
    mode: &omne_core::modes::ModeDef,
    tool_action: &str,
    current_thread_id: &omne_protocol::ThreadId,
    target_thread_id_raw: &str,
) -> Option<omne_core::modes::Decision> {
    let target_thread_id: omne_protocol::ThreadId = target_thread_id_raw.parse().ok()?;
    if target_thread_id != *current_thread_id {
        return Some(omne_core::modes::Decision::Deny);
    }
    plan_architect_effective_decision(mode, tool_action)
}

fn plan_architect_thread_state_decision(
    mode: &omne_core::modes::ModeDef,
    current_thread_id: &omne_protocol::ThreadId,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: ThreadStateArgs = serde_json::from_value(args.clone()).ok()?;
    plan_architect_thread_scope_decision(mode, "thread/state", current_thread_id, &parsed.thread_id)
}

fn plan_architect_thread_usage_decision(
    mode: &omne_core::modes::ModeDef,
    current_thread_id: &omne_protocol::ThreadId,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: ThreadUsageArgs = serde_json::from_value(args.clone()).ok()?;
    plan_architect_thread_scope_decision(mode, "thread/usage", current_thread_id, &parsed.thread_id)
}

fn plan_architect_thread_events_decision(
    mode: &omne_core::modes::ModeDef,
    current_thread_id: &omne_protocol::ThreadId,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: ThreadEventsArgs = serde_json::from_value(args.clone()).ok()?;
    plan_architect_thread_scope_decision(
        mode,
        "thread/events",
        current_thread_id,
        &parsed.thread_id,
    )
}

async fn plan_architect_process_scope_decision(
    server: &super::Server,
    mode: &omne_core::modes::ModeDef,
    tool_action: &str,
    current_thread_id: &omne_protocol::ThreadId,
    process_id_raw: &str,
) -> Option<omne_core::modes::Decision> {
    let process_id: omne_protocol::ProcessId = process_id_raw.parse().ok()?;
    let listed = super::handle_process_list(server, super::ProcessListParams { thread_id: None })
        .await
        .ok()?;
    let info = listed
        .into_iter()
        .find(|item| item.process_id == process_id);
    let Some(info) = info else {
        return Some(omne_core::modes::Decision::Deny);
    };
    if info.thread_id != *current_thread_id {
        return Some(omne_core::modes::Decision::Deny);
    }
    plan_architect_effective_decision(mode, tool_action)
}

async fn plan_architect_process_inspect_decision(
    server: &super::Server,
    mode: &omne_core::modes::ModeDef,
    current_thread_id: &omne_protocol::ThreadId,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: ProcessInspectArgs = serde_json::from_value(args.clone()).ok()?;
    plan_architect_process_scope_decision(
        server,
        mode,
        "process/inspect",
        current_thread_id,
        &parsed.process_id,
    )
    .await
}

async fn plan_architect_process_tail_decision(
    server: &super::Server,
    mode: &omne_core::modes::ModeDef,
    current_thread_id: &omne_protocol::ThreadId,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: ProcessTailArgs = serde_json::from_value(args.clone()).ok()?;
    plan_architect_process_scope_decision(
        server,
        mode,
        "process/tail",
        current_thread_id,
        &parsed.process_id,
    )
    .await
}

async fn plan_architect_process_follow_decision(
    server: &super::Server,
    mode: &omne_core::modes::ModeDef,
    current_thread_id: &omne_protocol::ThreadId,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: ProcessFollowArgs = serde_json::from_value(args.clone()).ok()?;
    plan_architect_process_scope_decision(
        server,
        mode,
        "process/follow",
        current_thread_id,
        &parsed.process_id,
    )
    .await
}

async fn plan_architect_repo_search_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: RepoSearchArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = plan_architect_repo_base_decision_after_path_gate(
        mode,
        &target_root,
        parsed.include_glob.as_deref(),
    );
    Some(crate::resolve_mode_decision_audit(mode, "repo/search", base_decision).decision)
}

async fn plan_architect_repo_index_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: RepoIndexArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = plan_architect_repo_base_decision_after_path_gate(
        mode,
        &target_root,
        parsed.include_glob.as_deref(),
    );
    Some(crate::resolve_mode_decision_audit(mode, "repo/index", base_decision).decision)
}

async fn plan_architect_repo_symbols_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: RepoSymbolsArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = plan_architect_repo_base_decision_after_path_gate(
        mode,
        &target_root,
        parsed.include_glob.as_deref(),
    );
    Some(crate::resolve_mode_decision_audit(mode, "repo/symbols", base_decision).decision)
}

async fn plan_architect_effective_decision_for_call(
    server: &super::Server,
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    thread_id: &omne_protocol::ThreadId,
    tool_action: &str,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    match tool_action {
        "file/read" => plan_architect_file_read_decision(mode, thread_root, args).await,
        "file/glob" => plan_architect_file_glob_decision(mode, thread_root, args).await,
        "file/grep" => plan_architect_file_grep_decision(mode, thread_root, args).await,
        "repo/search" => plan_architect_repo_search_decision(mode, thread_root, args).await,
        "repo/index" => plan_architect_repo_index_decision(mode, thread_root, args).await,
        "repo/symbols" => plan_architect_repo_symbols_decision(mode, thread_root, args).await,
        "thread/state" => plan_architect_thread_state_decision(mode, thread_id, args),
        "thread/usage" => plan_architect_thread_usage_decision(mode, thread_id, args),
        "thread/events" => plan_architect_thread_events_decision(mode, thread_id, args),
        "process/inspect" => {
            plan_architect_process_inspect_decision(server, mode, thread_id, args).await
        }
        "process/tail" => plan_architect_process_tail_decision(server, mode, thread_id, args).await,
        "process/follow" => {
            plan_architect_process_follow_decision(server, mode, thread_id, args).await
        }
        _ => plan_architect_effective_decision(mode, tool_action),
    }
}

async fn enforce_plan_directive_architect_mode_gate(
    server: &super::Server,
    thread_id: omne_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: &Value,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<Option<Value>> {
    let Some(tool_action) = plan_tool_action(tool_name) else {
        return Ok(None);
    };

    let (thread_rt, thread_root) = crate::load_thread_root(server, thread_id).await?;
    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let mode = catalog.mode("architect").ok_or_else(|| {
        let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
        anyhow::anyhow!(
            "tool blocked by /plan architect mode gate: architect mode missing (available: {available})"
        )
    })?;
    let effective_decision = plan_architect_effective_decision_for_call(
        server,
        mode,
        &thread_root,
        &thread_id,
        tool_action,
        args,
    )
    .await;

    if matches!(effective_decision, Some(omne_core::modes::Decision::Deny)) {
        anyhow::bail!(
            "tool blocked by /plan architect mode gate: tool={tool_name} action={tool_action}"
        );
    }

    if matches!(effective_decision, Some(omne_core::modes::Decision::Prompt)) {
        let approval_policy = {
            let handle = thread_rt.handle.lock().await;
            handle.state().approval_policy
        };
        let approval_params = serde_json::json!({
            "tool": tool_name,
            "action": tool_action,
            "tool_args": args,
            "approval": { "source": "plan_architect_mode" },
        });
        match crate::gate_approval(
            server,
            &thread_rt,
            thread_id,
            turn_id,
            approval_policy,
            crate::ApprovalRequest {
                approval_id,
                action: tool_action,
                params: &approval_params,
            },
        )
        .await?
        {
            crate::ApprovalGate::Approved => {}
            crate::ApprovalGate::Denied { remembered } => {
                return Ok(Some(serde_json::json!({
                    "denied": true,
                    "remembered": remembered,
                })));
            }
            crate::ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(Some(serde_json::json!({
                    "needs_approval": true,
                    "approval_id": approval_id,
                })));
            }
        }
    }

    Ok(None)
}

async fn turn_has_plan_directive(
    server: &super::Server,
    thread_id: omne_protocol::ThreadId,
    turn_id: TurnId,
) -> anyhow::Result<bool> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    for event in events.iter().rev() {
        let omne_protocol::ThreadEventKind::TurnStarted {
            turn_id: event_turn_id,
            directives,
            ..
        } = &event.kind
        else {
            continue;
        };
        if *event_turn_id != turn_id {
            continue;
        }
        return Ok(directives
            .iter()
            .flatten()
            .any(|directive| matches!(directive, omne_protocol::TurnDirective::Plan)));
    }

    Ok(false)
}

async fn enforce_plan_directive_tool_gate(
    server: &super::Server,
    thread_id: omne_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
) -> anyhow::Result<bool> {
    let Some(turn_id) = turn_id else {
        return Ok(false);
    };
    if !is_known_agent_tool(tool_name) {
        return Ok(false);
    }
    if !turn_has_plan_directive(server, thread_id, turn_id).await? {
        return Ok(false);
    }
    if is_plan_read_only_tool(tool_name) {
        return Ok(true);
    }

    anyhow::bail!("tool blocked by /plan directive: {tool_name}")
}

#[derive(Clone, Copy)]
enum SubagentSpawnLimitSource {
    Unlimited,
    Env,
    Mode,
    Combined,
}

impl SubagentSpawnLimitSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unlimited => "unlimited",
            Self::Env => "env",
            Self::Mode => "mode",
            Self::Combined => "combined",
        }
    }
}

struct SubagentSpawnLimit {
    effective: usize,
    source: SubagentSpawnLimitSource,
}

fn combine_subagent_spawn_limits(
    env_limit: usize,
    mode_limit: Option<usize>,
) -> SubagentSpawnLimit {
    let mode_limit = mode_limit.unwrap_or(0);
    let effective = match (env_limit, mode_limit) {
        (0, 0) => 0,
        (0, mode) => mode,
        (env, 0) => env,
        (env, mode) => env.min(mode),
    };
    let source = match (env_limit > 0, mode_limit > 0) {
        (false, false) => SubagentSpawnLimitSource::Unlimited,
        (true, false) => SubagentSpawnLimitSource::Env,
        (false, true) => SubagentSpawnLimitSource::Mode,
        (true, true) => SubagentSpawnLimitSource::Combined,
    };
    SubagentSpawnLimit { effective, source }
}

fn usage_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

fn configured_total_token_budget_limit() -> Option<u64> {
    std::env::var("OMNE_AGENT_MAX_TOTAL_TOKENS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn token_budget_snapshot(
    total_tokens_used: u64,
    token_budget_limit: Option<u64>,
) -> (Option<u64>, Option<f64>, Option<bool>) {
    let Some(limit) = token_budget_limit else {
        return (None, None, None);
    };
    let remaining = Some(limit.saturating_sub(total_tokens_used));
    let utilization = usage_ratio(total_tokens_used, limit);
    let exceeded = Some(total_tokens_used > limit);
    (remaining, utilization, exceeded)
}

struct AgentSpawnDefaults {
    spawn_mode: AgentSpawnMode,
    mode: String,
    workspace_mode: AgentSpawnWorkspaceMode,
    priority: AgentSpawnTaskPriority,
    model: Option<String>,
    openai_base_url: Option<String>,
    expected_artifact_type: String,
}

struct PreparedAgentSpawn {
    plans: Vec<SubagentSpawnTaskPlan>,
    approval_params: Value,
    priority_aging_rounds: usize,
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn validate_agent_spawn_dependencies(plans: &[SubagentSpawnTaskPlan]) -> anyhow::Result<()> {
    let mut by_id = std::collections::HashMap::<String, usize>::new();
    for (idx, task) in plans.iter().enumerate() {
        by_id.insert(task.id.clone(), idx);
    }
    for task in plans {
        for dep in &task.depends_on {
            if !by_id.contains_key(dep) {
                anyhow::bail!("unknown depends_on: {dep} (task_id={})", task.id);
            }
        }
    }

    let mut indegree = std::collections::HashMap::<String, usize>::new();
    let mut edges = std::collections::HashMap::<String, Vec<String>>::new();
    for task in plans {
        indegree.insert(task.id.clone(), task.depends_on.len());
        for dep in &task.depends_on {
            edges.entry(dep.clone()).or_default().push(task.id.clone());
        }
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
    if visited != plans.len() {
        anyhow::bail!("task dependencies contain a cycle");
    }
    Ok(())
}

fn prepare_agent_spawn(args: AgentSpawnArgs) -> anyhow::Result<PreparedAgentSpawn> {
    if args.tasks.is_empty() {
        anyhow::bail!("tasks must not be empty");
    }

    let defaults = AgentSpawnDefaults {
        spawn_mode: args.spawn_mode.unwrap_or(AgentSpawnMode::New),
        mode: normalize_optional_string(args.mode).unwrap_or_else(|| "reviewer".to_string()),
        workspace_mode: args
            .workspace_mode
            .unwrap_or(AgentSpawnWorkspaceMode::ReadOnly),
        priority: args.priority.unwrap_or(AgentSpawnTaskPriority::Normal),
        model: normalize_optional_string(args.model),
        openai_base_url: normalize_optional_string(args.openai_base_url),
        expected_artifact_type: normalize_optional_string(args.expected_artifact_type)
            .unwrap_or_else(|| "fan_out_result".to_string()),
    };

    let mut seen_ids = std::collections::BTreeSet::<String>::new();
    let mut plans = Vec::<SubagentSpawnTaskPlan>::new();
    for task in args.tasks {
        let id = task.id.trim().to_string();
        if id.is_empty() {
            anyhow::bail!("tasks[].id must not be empty");
        }
        if !seen_ids.insert(id.clone()) {
            anyhow::bail!("duplicate task id: {id}");
        }

        let input = task.input.trim().to_string();
        if input.is_empty() {
            anyhow::bail!("task input must not be empty (task_id={id})");
        }

        let title = task.title.unwrap_or_default().trim().to_string();
        let mut depends_on = Vec::<String>::new();
        let mut seen_deps = std::collections::BTreeSet::<String>::new();
        for dep in task.depends_on {
            let dep = dep.trim().to_string();
            if dep.is_empty() {
                continue;
            }
            if dep == id {
                anyhow::bail!("task depends_on itself: {id}");
            }
            if seen_deps.insert(dep.clone()) {
                depends_on.push(dep);
            }
        }

        plans.push(SubagentSpawnTaskPlan {
            id,
            title,
            input,
            depends_on,
            priority: task.priority.unwrap_or(defaults.priority),
            spawn_mode: task.spawn_mode.unwrap_or(defaults.spawn_mode),
            mode: normalize_optional_string(task.mode).unwrap_or_else(|| defaults.mode.clone()),
            workspace_mode: task.workspace_mode.unwrap_or(defaults.workspace_mode),
            model: normalize_optional_string(task.model).or_else(|| defaults.model.clone()),
            openai_base_url: normalize_optional_string(task.openai_base_url)
                .or_else(|| defaults.openai_base_url.clone()),
            expected_artifact_type: normalize_optional_string(task.expected_artifact_type)
                .unwrap_or_else(|| defaults.expected_artifact_type.clone()),
        });
    }

    validate_agent_spawn_dependencies(&plans)?;

    let task_previews = plans
        .iter()
        .map(|task| {
            let input_preview = omne_core::redact_text(&truncate_chars(&task.input, 400));
            serde_json::json!({
                "id": task.id.clone(),
                "spawn_mode": spawn_mode_label(task.spawn_mode),
                "mode": task.mode.clone(),
                "workspace_mode": workspace_mode_label(task.workspace_mode),
                "priority": priority_label(task.priority),
                "depends_on": task.depends_on.clone(),
                "input_chars": task.input.chars().count(),
                "input_preview": input_preview,
            })
        })
        .collect::<Vec<_>>();
    let priority_aging_rounds = fan_out_priority_aging_rounds_from_env();

    let approval_params = serde_json::json!({
        "task_count": plans.len(),
        "tasks": task_previews,
        "default_spawn_mode": spawn_mode_label(defaults.spawn_mode),
        "default_mode": defaults.mode,
        "default_workspace_mode": workspace_mode_label(defaults.workspace_mode),
        "default_priority": priority_label(defaults.priority),
        "priority_aging_rounds": priority_aging_rounds,
        "default_expected_artifact_type": defaults.expected_artifact_type,
        "model": defaults.model,
        "openai_base_url": defaults.openai_base_url,
    });

    Ok(PreparedAgentSpawn {
        plans,
        approval_params,
        priority_aging_rounds,
    })
}

async fn start_agent_spawn_schedule(
    server: &super::Server,
    thread_id: ThreadId,
    plans: Vec<SubagentSpawnTaskPlan>,
    active_threads: Vec<ThreadId>,
    max_concurrent_subagents: usize,
    env_max_concurrent_subagents: usize,
    priority_aging_rounds: usize,
    thread_cwd: &str,
    thread_root: &std::path::Path,
    approval_policy: omne_protocol::ApprovalPolicy,
) -> anyhow::Result<Vec<Value>> {
    let mut tasks = Vec::<SubagentSpawnTask>::with_capacity(plans.len());
    for plan in plans {
        let isolated_cwd = if matches!(plan.workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite) {
            Some(prepare_isolated_workspace(server, thread_id, &plan.id, thread_root).await?)
        } else {
            None
        };
        let spawned_cwd = isolated_cwd.as_ref().map(|path| path.display().to_string());
        let spawned = match plan.spawn_mode {
            AgentSpawnMode::Fork => {
                let forked = super::handle_thread_fork(
                    server,
                    super::ThreadForkParams {
                        thread_id,
                        cwd: spawned_cwd.clone(),
                    },
                )
                .await?;
                SpawnedThread {
                    thread_id: forked.thread_id,
                    log_path: forked.log_path,
                    last_seq: forked.last_seq,
                }
            }
            AgentSpawnMode::New => {
                create_new_thread(server, spawned_cwd.as_deref().unwrap_or(thread_cwd)).await?
            }
        };

        let approval_override = if matches!(plan.spawn_mode, AgentSpawnMode::New) {
            Some(approval_policy)
        } else {
            None
        };
        let sandbox_policy = match plan.workspace_mode {
            AgentSpawnWorkspaceMode::ReadOnly => omne_protocol::SandboxPolicy::ReadOnly,
            AgentSpawnWorkspaceMode::IsolatedWrite => omne_protocol::SandboxPolicy::WorkspaceWrite,
        };
        super::handle_thread_configure(
            server,
            super::ThreadConfigureParams {
                thread_id: spawned.thread_id,
                approval_policy: approval_override,
                sandbox_policy: Some(sandbox_policy),
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: Some(plan.mode.clone()),
                model: plan.model.clone(),
                thinking: None,
                show_thinking: None,
                openai_base_url: plan.openai_base_url.clone(),
                allowed_tools: None,
                execpolicy_rules: None,
            },
        )
        .await?;

        tasks.push(SubagentSpawnTask {
            id: plan.id,
            title: plan.title,
            input: plan.input,
            depends_on: plan.depends_on,
            priority: plan.priority,
            spawn_mode: plan.spawn_mode,
            mode: plan.mode,
            workspace_mode: plan.workspace_mode,
            model: plan.model,
            openai_base_url: plan.openai_base_url,
            expected_artifact_type: plan.expected_artifact_type,
            workspace_cwd: spawned_cwd,
            thread_id: spawned.thread_id,
            log_path: spawned.log_path,
            last_seq: spawned.last_seq,
            turn_id: None,
            status: SubagentTaskStatus::Pending,
            error: None,
        });
    }

    let external_active = active_threads
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let mut schedule = SubagentSpawnSchedule::new(
        thread_id,
        tasks,
        external_active,
        max_concurrent_subagents,
        priority_aging_rounds,
    );
    schedule.set_env_max_concurrent_subagents(env_max_concurrent_subagents);
    schedule.start_ready_tasks(server).await;
    schedule.catch_up_running_events(server).await;
    let snapshot = schedule.snapshot();
    spawn_subagent_scheduler(server.clone(), schedule);
    Ok(snapshot)
}

async fn run_tool_call_once(
    server: &super::Server,
    thread_id: omne_protocol::ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    args: Value,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<Value> {
    let has_plan_directive =
        enforce_plan_directive_tool_gate(server, thread_id, turn_id, tool_name).await?;
    if has_plan_directive {
        if let Some(output) = enforce_plan_directive_architect_mode_gate(
            server,
            thread_id,
            turn_id,
            tool_name,
            &args,
            approval_id,
        )
        .await?
        {
            return Ok(output);
        }
    }

    match tool_name {
        "file_read" => {
            let args: FileReadArgs = serde_json::from_value(args)?;
            super::handle_file_read(
                server,
                super::FileReadParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    path: args.path,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_glob" => {
            let args: FileGlobArgs = serde_json::from_value(args)?;
            super::handle_file_glob(
                server,
                super::FileGlobParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    pattern: args.pattern,
                    max_results: args.max_results,
                },
            )
            .await
        }
        "file_grep" => {
            let args: FileGrepArgs = serde_json::from_value(args)?;
            super::handle_file_grep(
                server,
                super::FileGrepParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    query: args.query,
                    is_regex: args.is_regex,
                    include_glob: args.include_glob,
                    max_matches: args.max_matches,
                    max_bytes_per_file: None,
                    max_files: None,
                },
            )
            .await
        }
        "repo_search" => {
            let args: RepoSearchArgs = serde_json::from_value(args)?;
            super::handle_repo_search(
                server,
                super::RepoSearchParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    query: args.query,
                    is_regex: args.is_regex,
                    include_glob: args.include_glob,
                    max_matches: args.max_matches,
                    max_bytes_per_file: args.max_bytes_per_file,
                    max_files: args.max_files,
                },
            )
            .await
        }
        "repo_index" => {
            let args: RepoIndexArgs = serde_json::from_value(args)?;
            super::handle_repo_index(
                server,
                super::RepoIndexParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    include_glob: args.include_glob,
                    max_files: args.max_files,
                },
            )
            .await
        }
        "repo_symbols" => {
            let args: RepoSymbolsArgs = serde_json::from_value(args)?;
            super::handle_repo_symbols(
                server,
                super::RepoSymbolsParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    include_glob: args.include_glob,
                    max_files: args.max_files,
                    max_bytes_per_file: args.max_bytes_per_file,
                    max_symbols: args.max_symbols,
                },
            )
            .await
        }
        "mcp_list_servers" => {
            super::handle_mcp_list_servers(
                server,
                super::McpListServersParams {
                    thread_id,
                    turn_id,
                    approval_id,
                },
            )
            .await
        }
        "mcp_list_tools" => {
            let args: McpListToolsArgs = serde_json::from_value(args)?;
            super::handle_mcp_list_tools(
                server,
                super::McpListToolsParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                },
            )
            .await
        }
        "mcp_list_resources" => {
            let args: McpListResourcesArgs = serde_json::from_value(args)?;
            super::handle_mcp_list_resources(
                server,
                super::McpListResourcesParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                },
            )
            .await
        }
        "mcp_call" => {
            let args: McpCallArgs = serde_json::from_value(args)?;
            super::handle_mcp_call(
                server,
                super::McpCallParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                    tool: args.tool,
                    arguments: args.arguments,
                },
            )
            .await
        }
        "file_write" => {
            let args: FileWriteArgs = serde_json::from_value(args)?;
            super::handle_file_write(
                server,
                super::FileWriteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    text: args.text,
                    create_parent_dirs: args.create_parent_dirs,
                },
            )
            .await
        }
        "file_patch" => {
            let args: FilePatchArgs = serde_json::from_value(args)?;
            super::handle_file_patch(
                server,
                super::FilePatchParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    patch: args.patch,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_edit" => {
            let args: FileEditArgs = serde_json::from_value(args)?;
            let edits = args
                .edits
                .into_iter()
                .map(|op| super::FileEditOp {
                    old: op.old,
                    new: op.new,
                    expected_replacements: op.expected_replacements,
                })
                .collect();
            super::handle_file_edit(
                server,
                super::FileEditParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    edits,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "file_delete" => {
            let args: FileDeleteArgs = serde_json::from_value(args)?;
            super::handle_file_delete(
                server,
                super::FileDeleteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    recursive: args.recursive,
                },
            )
            .await
        }
        "fs_mkdir" => {
            let args: FsMkdirArgs = serde_json::from_value(args)?;
            super::handle_fs_mkdir(
                server,
                super::FsMkdirParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    path: args.path,
                    recursive: args.recursive,
                },
            )
            .await
        }
        "process_start" => {
            let args: ProcessStartArgs = serde_json::from_value(args)?;
            super::handle_process_start(
                server,
                super::ProcessStartParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    argv: args.argv,
                    cwd: args.cwd,
                    timeout_ms: args.timeout_ms,
                },
            )
            .await
        }
        "process_inspect" => {
            let args: ProcessInspectArgs = serde_json::from_value(args)?;
            super::handle_process_inspect(
                server,
                super::ProcessInspectParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    max_lines: args.max_lines,
                },
            )
            .await
        }
        "process_tail" => {
            let args: ProcessTailArgs = serde_json::from_value(args)?;
            super::handle_process_tail(
                server,
                super::ProcessTailParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    stream: args.stream,
                    max_lines: args.max_lines,
                },
            )
            .await
        }
        "process_follow" => {
            let args: ProcessFollowArgs = serde_json::from_value(args)?;
            super::handle_process_follow(
                server,
                super::ProcessFollowParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    stream: args.stream,
                    since_offset: args.since_offset,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "process_kill" => {
            let args: ProcessKillArgs = serde_json::from_value(args)?;
            super::handle_process_kill(
                server,
                super::ProcessKillParams {
                    process_id: args.process_id.parse()?,
                    turn_id,
                    approval_id,
                    reason: args.reason,
                },
            )
            .await
        }
        "artifact_write" => {
            let args: ArtifactWriteArgs = serde_json::from_value(args)?;
            super::handle_artifact_write(
                server,
                super::ArtifactWriteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: None,
                    artifact_type: args.artifact_type,
                    summary: args.summary,
                    text: args.text,
                },
            )
            .await
        }
        "artifact_list" => {
            super::handle_artifact_list(
                server,
                super::ArtifactListParams {
                    thread_id,
                    turn_id,
                    approval_id,
                },
            )
            .await
        }
        "artifact_read" => {
            let args: ArtifactReadArgs = serde_json::from_value(args)?;
            super::handle_artifact_read(
                server,
                super::ArtifactReadParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: args.artifact_id.parse()?,
                    version: args.version,
                    max_bytes: args.max_bytes,
                },
            )
            .await
        }
        "artifact_delete" => {
            let args: ArtifactDeleteArgs = serde_json::from_value(args)?;
            super::handle_artifact_delete(
                server,
                super::ArtifactDeleteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: args.artifact_id.parse()?,
                },
            )
            .await
        }
        "thread_diff" => {
            let args: ThreadDiffArgs = serde_json::from_value(args)?;
            let response = super::handle_thread_diff(
                server,
                super::ThreadDiffParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    max_bytes: args.max_bytes,
                    wait_seconds: args.wait_seconds,
                },
            )
            .await?;
            serde_json::to_value(response).context("serialize thread/diff response")
        }
        "thread_state" => {
            let args: ThreadStateArgs = serde_json::from_value(args)?;
            handle_thread_state_tool(server, args).await
        }
        "thread_usage" => {
            let args: ThreadUsageArgs = serde_json::from_value(args)?;
            handle_thread_usage_tool(server, args).await
        }
        "thread_events" => {
            let args: ThreadEventsArgs = serde_json::from_value(args)?;
            handle_thread_events_tool(server, args).await
        }
        "thread_hook_run" => {
            let args: ThreadHookRunArgs = serde_json::from_value(args)?;
            let response = super::handle_thread_hook_run(
                server,
                super::ThreadHookRunParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    hook: args.hook,
                },
            )
            .await?;
            serde_json::to_value(response).context("serialize thread/hook_run response")
        }
        "agent_spawn" => {
            let args: AgentSpawnArgs = serde_json::from_value(args)?;
            handle_agent_spawn_tool(server, thread_id, turn_id, approval_id, args).await
        }
        _ => anyhow::bail!("unknown tool: {tool_name}"),
    }
}

async fn handle_agent_spawn_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: AgentSpawnArgs,
) -> anyhow::Result<Value> {
    let PreparedAgentSpawn {
        plans,
        approval_params,
        priority_aging_rounds,
    } = prepare_agent_spawn(args)?;

    let tool_id = omne_protocol::ToolId::new();
    let (thread_rt, thread_root) = super::load_thread_root(server, thread_id).await?;
    let (approval_policy, mode_name, thread_cwd) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone(), state.cwd.clone())
    };
    let thread_cwd = thread_cwd.ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {thread_id}"))?;

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let Some(mode) = catalog.mode(&mode_name) else {
        let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
        let decision = omne_core::modes::Decision::Deny;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: "subagent/spawn".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("unknown mode".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision": decision,
                    "available": available,
                    "load_error": catalog.load_error.clone(),
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision": decision,
            "available": available,
            "load_error": catalog.load_error,
        }));
    };

    let mode_decision = crate::resolve_mode_decision_audit(
        mode,
        "subagent/spawn",
        mode.permissions.subagent.spawn.decision,
    );
    let effective_decision = mode_decision.decision;
    let tool_override_hit = mode_decision.tool_override_hit;
    let decision_source = mode_decision.decision_source;

    if effective_decision == omne_core::modes::Decision::Deny {
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: "subagent/spawn".to_string(),
                params: Some(approval_params),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some("mode denies subagent/spawn".to_string()),
                result: Some(serde_json::json!({
                    "mode": mode_name,
                    "decision_source": decision_source,
                    "tool_override_hit": tool_override_hit,
                    "decision": effective_decision,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "mode": mode_name,
            "decision_source": decision_source,
            "tool_override_hit": tool_override_hit,
            "decision": effective_decision,
        }));
    }

    let env_max_concurrent_subagents = parse_env_usize("OMNE_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
    let mode_max_concurrent_subagents = mode.permissions.subagent.spawn.max_threads;
    let limit = combine_subagent_spawn_limits(
        env_max_concurrent_subagents,
        mode_max_concurrent_subagents,
    );
    let max_concurrent_subagents = limit.effective;
    if let Some(allowed) = mode.permissions.subagent.spawn.allowed_modes.as_ref() {
        let mut disallowed = std::collections::BTreeSet::<String>::new();
        for task in &plans {
            if !allowed.iter().any(|name| name == &task.mode) {
                disallowed.insert(task.mode.clone());
            }
        }
        if !disallowed.is_empty() {
            let requested_modes = disallowed.into_iter().collect::<Vec<_>>();
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                    tool_id,
                    turn_id,
                    tool: "subagent/spawn".to_string(),
                    params: Some(approval_params),
                })
                .await?;
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Denied,
                    error: Some("mode forbids spawning this subagent mode".to_string()),
                    result: Some(serde_json::json!({
                        "mode": mode_name,
                        "decision_source": decision_source,
                        "tool_override_hit": tool_override_hit,
                        "decision": effective_decision,
                        "requested_modes": requested_modes,
                        "allowed_modes": allowed,
                    })),
                })
                .await?;
            return Ok(serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision_source": decision_source,
                "tool_override_hit": tool_override_hit,
                "decision": effective_decision,
                "allowed_modes": allowed,
                "priority_aging_rounds": priority_aging_rounds,
                "limit_policy": "min_non_zero",
                "limit_source": limit.source.as_str(),
                "env_max_concurrent_subagents": env_max_concurrent_subagents,
                "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
                "max_concurrent_subagents": max_concurrent_subagents,
            }));
        }
    }

    let active_threads = if max_concurrent_subagents > 0 {
        active_subagent_threads(server, thread_id).await?
    } else {
        Vec::new()
    };
    if max_concurrent_subagents > 0 && active_threads.len() >= max_concurrent_subagents {
        let active = active_threads.len();
        let active_thread_ids = active_threads
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>();

        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: "subagent/spawn".to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                error: Some(format!(
                    "max_concurrent_subagents limit reached: active={active}, max={max_concurrent_subagents}"
                )),
                result: Some(serde_json::json!({
                    "limit_policy": "min_non_zero",
                    "limit_source": limit.source.as_str(),
                    "priority_aging_rounds": priority_aging_rounds,
                    "env_max_concurrent_subagents": env_max_concurrent_subagents,
                    "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
                    "max_concurrent_subagents": max_concurrent_subagents,
                    "active": active,
                    "active_threads": active_thread_ids,
                })),
            })
            .await?;
        return Ok(serde_json::json!({
            "tool_id": tool_id,
            "denied": true,
            "limit_policy": "min_non_zero",
            "limit_source": limit.source.as_str(),
            "priority_aging_rounds": priority_aging_rounds,
            "env_max_concurrent_subagents": env_max_concurrent_subagents,
            "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
            "max_concurrent_subagents": max_concurrent_subagents,
            "active": active,
        }));
    }

    if effective_decision == omne_core::modes::Decision::Prompt {
        match super::gate_approval(
            server,
            &thread_rt,
            thread_id,
            turn_id,
            approval_policy,
            super::ApprovalRequest {
                approval_id,
                action: "subagent/spawn",
                params: &approval_params,
            },
        )
        .await?
        {
            super::ApprovalGate::Approved => {}
            super::ApprovalGate::Denied { remembered } => {
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: "subagent/spawn".to_string(),
                        params: Some(approval_params),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        error: Some(super::approval_denied_error(remembered).to_string()),
                        result: Some(serde_json::json!({
                            "approval_policy": approval_policy,
                        })),
                    })
                    .await?;
                return Ok(serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "remembered": remembered,
                }));
            }
            super::ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(serde_json::json!({
                    "needs_approval": true,
                    "approval_id": approval_id,
                }));
            }
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: "subagent/spawn".to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    let outcome = start_agent_spawn_schedule(
        server,
        thread_id,
        plans,
        active_threads,
        max_concurrent_subagents,
        env_max_concurrent_subagents,
        priority_aging_rounds,
        &thread_cwd,
        &thread_root,
        approval_policy,
    )
    .await;

    match outcome {
        Ok(tasks) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    error: None,
                    result: Some(serde_json::json!({
                        "tasks": tasks,
                        "priority_aging_rounds": priority_aging_rounds,
                        "limit_policy": "min_non_zero",
                        "limit_source": limit.source.as_str(),
                        "env_max_concurrent_subagents": env_max_concurrent_subagents,
                        "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
                        "max_concurrent_subagents": max_concurrent_subagents,
                    })),
                })
                .await?;

            Ok(serde_json::json!({
                "tool_id": tool_id,
                "tasks": tasks,
                "priority_aging_rounds": priority_aging_rounds,
                "limit_policy": "min_non_zero",
                "limit_source": limit.source.as_str(),
                "env_max_concurrent_subagents": env_max_concurrent_subagents,
                "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
                "max_concurrent_subagents": max_concurrent_subagents,
            }))
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_thread_state_tool(server: &super::Server, args: ThreadStateArgs) -> anyhow::Result<Value> {
    let thread_id: omne_protocol::ThreadId = args.thread_id.parse()?;
    let rt = server.get_or_load_thread(thread_id).await?;
    let handle = rt.handle.lock().await;
    let state = handle.state();
    let archived_at = state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok());
    let paused_at = state.paused_at.and_then(|ts| ts.format(&Rfc3339).ok());
    Ok(serde_json::json!({
        "thread_id": handle.thread_id(),
        "cwd": state.cwd,
        "archived": state.archived,
        "archived_at": archived_at,
        "archived_reason": state.archived_reason,
        "paused": state.paused,
        "paused_at": paused_at,
        "paused_reason": state.paused_reason,
        "approval_policy": state.approval_policy,
        "sandbox_policy": state.sandbox_policy,
        "model": state.model,
        "openai_base_url": state.openai_base_url,
        "last_seq": handle.last_seq().0,
        "active_turn_id": state.active_turn_id,
        "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
        "last_turn_id": state.last_turn_id,
        "last_turn_status": state.last_turn_status,
        "last_turn_reason": state.last_turn_reason,
    }))
}

async fn handle_thread_usage_tool(server: &super::Server, args: ThreadUsageArgs) -> anyhow::Result<Value> {
    let thread_id: omne_protocol::ThreadId = args.thread_id.parse()?;
    let rt = server.get_or_load_thread(thread_id).await?;
    let handle = rt.handle.lock().await;
    let state = handle.state();
    let non_cache_input_tokens_used = state
        .input_tokens_used
        .saturating_sub(state.cache_input_tokens_used);
    let token_budget_limit = configured_total_token_budget_limit();
    let (token_budget_remaining, token_budget_utilization, token_budget_exceeded) =
        token_budget_snapshot(state.total_tokens_used, token_budget_limit);
    Ok(serde_json::json!({
        "thread_id": handle.thread_id(),
        "last_seq": handle.last_seq().0,
        "total_tokens_used": state.total_tokens_used,
        "input_tokens_used": state.input_tokens_used,
        "output_tokens_used": state.output_tokens_used,
        "cache_input_tokens_used": state.cache_input_tokens_used,
        "cache_creation_input_tokens_used": state.cache_creation_input_tokens_used,
        "non_cache_input_tokens_used": non_cache_input_tokens_used,
        "cache_input_ratio": usage_ratio(state.cache_input_tokens_used, state.input_tokens_used),
        "output_ratio": usage_ratio(state.output_tokens_used, state.total_tokens_used),
        "token_budget_limit": token_budget_limit,
        "token_budget_remaining": token_budget_remaining,
        "token_budget_utilization": token_budget_utilization,
        "token_budget_exceeded": token_budget_exceeded,
    }))
}

async fn handle_thread_events_tool(server: &super::Server, args: ThreadEventsArgs) -> anyhow::Result<Value> {
    let thread_id: omne_protocol::ThreadId = args.thread_id.parse()?;
    let since = EventSeq(args.since_seq);

    let mut events = server
        .thread_store
        .read_events_since(thread_id, since)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {}", thread_id))?;

    let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

    let mut has_more = false;
    if let Some(max_events) = args.max_events {
        let max_events = max_events.clamp(1, 50_000);
        if events.len() > max_events {
            events.truncate(max_events);
            has_more = true;
        }
    }

    let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);

    Ok(serde_json::json!({
        "events": events,
        "last_seq": last_seq,
        "thread_last_seq": thread_last_seq,
        "has_more": has_more,
    }))
}
