fn is_known_agent_tool(tool_name: &str) -> bool {
    is_known_agent_tool_name(tool_name)
}

async fn resolve_dynamic_tool_for_thread(
    server: &super::Server,
    thread_id: ThreadId,
    tool_name: &str,
) -> anyhow::Result<Option<DynamicToolSpec>> {
    let (_, thread_root) = super::load_thread_root(server, thread_id).await?;
    Ok(find_dynamic_tool_spec(Some(&thread_root), tool_name))
}

const FAN_OUT_PRIORITY_AGING_ROUNDS_ENV: &str = "OMNE_FAN_OUT_PRIORITY_AGING_ROUNDS";
const DEFAULT_FAN_OUT_PRIORITY_AGING_ROUNDS: usize = 3;
const MIN_FAN_OUT_PRIORITY_AGING_ROUNDS: usize = 1;
const MAX_FAN_OUT_PRIORITY_AGING_ROUNDS: usize = 10_000;
const DEFAULT_SUBAGENT_WAIT_TIMEOUT_MS: u64 = 30_000;
const MIN_SUBAGENT_WAIT_TIMEOUT_MS: u64 = 10_000;
const MAX_SUBAGENT_WAIT_TIMEOUT_MS: u64 = 300_000;

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
        "file/read"
        | "file/glob"
        | "file/grep"
        | "thread/state"
        | "thread/usage"
        | "thread/events"
        | "thread/request_user_input"
        | "subagent/wait" => mode.permissions.read,
        "web/search" | "web/fetch" | "web/view_image" => mode.permissions.browser,
        "repo/search"
        | "repo/index"
        | "repo/symbols"
        | "repo/goto_definition"
        | "repo/find_references" => mode.permissions.read.combine(mode.permissions.artifact),
        "mcp/list_servers" => mode.permissions.command,
        "mcp/list_tools" | "mcp/list_resources" => mode.permissions.read,
        "process/inspect" | "process/tail" | "process/follow" => mode.permissions.process.inspect,
        "artifact/list" | "artifact/read" => mode.permissions.artifact,
        "thread/diff" => mode.permissions.command.combine(mode.permissions.artifact),
        _ => return None,
    };
    Some(decision)
}

#[cfg(test)]
mod plan_tool_policy_tests {
    use super::*;

    #[test]
    fn mcp_list_servers_is_not_treated_as_plan_read_only() {
        assert!(!is_plan_read_only_tool("mcp_list_servers"));
        assert_eq!(plan_tool_action("mcp_list_servers"), None);
    }

    #[test]
    fn architect_uses_command_permission_for_mcp_list_servers() {
        let mode = omne_core::modes::ModeCatalog::builtin()
            .mode("architect")
            .cloned()
            .expect("builtin architect mode");

        assert_eq!(
            plan_architect_base_decision(&mode, "mcp/list_servers"),
            Some(omne_core::modes::Decision::Prompt)
        );
    }
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

async fn plan_architect_repo_goto_definition_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: RepoGotoDefinitionArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = plan_architect_repo_base_decision_after_path_gate(
        mode,
        &target_root,
        parsed.include_glob.as_deref(),
    );
    Some(crate::resolve_mode_decision_audit(mode, "repo/goto_definition", base_decision).decision)
}

async fn plan_architect_repo_find_references_decision(
    mode: &omne_core::modes::ModeDef,
    thread_root: &std::path::Path,
    args: &Value,
) -> Option<omne_core::modes::Decision> {
    let parsed: RepoFindReferencesArgs = serde_json::from_value(args.clone()).ok()?;
    let root = parsed.root.unwrap_or(crate::FileRoot::Workspace);
    let target_root = resolve_plan_target_root(thread_root, root).await?;

    let base_decision = plan_architect_repo_base_decision_after_path_gate(
        mode,
        &target_root,
        parsed.include_glob.as_deref(),
    );
    Some(crate::resolve_mode_decision_audit(mode, "repo/find_references", base_decision).decision)
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
        "repo/goto_definition" => {
            plan_architect_repo_goto_definition_decision(mode, thread_root, args).await
        }
        "repo/find_references" => {
            plan_architect_repo_find_references_decision(mode, thread_root, args).await
        }
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
    if !turn_has_plan_directive(server, thread_id, turn_id).await? {
        return Ok(false);
    }

    if is_known_agent_tool(tool_name) {
        if is_plan_read_only_tool(tool_name) {
            return Ok(true);
        }
        anyhow::bail!("tool blocked by /plan directive: {tool_name}")
    }

    if let Some(spec) = resolve_dynamic_tool_for_thread(server, thread_id, tool_name).await? {
        if is_plan_read_only_tool(&spec.mapped_tool) {
            return Ok(true);
        }
        anyhow::bail!("tool blocked by /plan directive: {tool_name}")
    }

    Ok(false)
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

#[derive(Debug)]
struct NormalizedUpdatePlanStep {
    step: String,
    status: &'static str,
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_update_plan_status(status: &str) -> Option<&'static str> {
    match status.trim().to_ascii_lowercase().as_str() {
        "pending" => Some("pending"),
        "in_progress" | "in-progress" => Some("in_progress"),
        "completed" => Some("completed"),
        _ => None,
    }
}

fn normalize_update_plan_steps(
    args: UpdatePlanArgs,
) -> anyhow::Result<(Option<String>, Vec<NormalizedUpdatePlanStep>)> {
    let explanation = normalize_optional_string(args.explanation);
    if args.plan.is_empty() {
        anyhow::bail!("plan must not be empty");
    }

    let mut normalized = Vec::<NormalizedUpdatePlanStep>::with_capacity(args.plan.len());
    let mut in_progress_count = 0usize;
    for (idx, step) in args.plan.into_iter().enumerate() {
        let step_text = step.step.trim();
        if step_text.is_empty() {
            anyhow::bail!("plan[{idx}].step must not be empty");
        }
        let Some(status) = normalize_update_plan_status(&step.status) else {
            anyhow::bail!("plan[{idx}].status must be one of: pending, in_progress, completed");
        };
        if status == "in_progress" {
            in_progress_count += 1;
        }
        normalized.push(NormalizedUpdatePlanStep {
            step: step_text.to_string(),
            status,
        });
    }

    if in_progress_count > 1 {
        anyhow::bail!("plan can contain at most one in_progress step");
    }

    Ok((explanation, normalized))
}

fn summarize_update_plan_artifact(
    explanation: Option<&str>,
    plan: &[NormalizedUpdatePlanStep],
) -> String {
    let summary_source = explanation
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| plan.first().map(|step| step.step.as_str()))
        .unwrap_or("plan");
    let summary = summary_source.chars().take(120).collect::<String>();
    if summary.is_empty() {
        "plan".to_string()
    } else {
        summary
    }
}

fn render_update_plan_artifact_text(
    explanation: Option<&str>,
    plan: &[NormalizedUpdatePlanStep],
) -> String {
    let mut text = String::from("# Plan\n\n");

    if let Some(explanation) = explanation {
        let explanation = explanation.trim();
        if !explanation.is_empty() {
            text.push_str("## Explanation\n\n");
            text.push_str(explanation);
            text.push_str("\n\n");
        }
    }

    text.push_str("## Steps\n\n");
    for (idx, step) in plan.iter().enumerate() {
        text.push_str(&format!("{}. [{}] {}\n", idx + 1, step.status, step.step));
    }
    text
}

#[derive(Debug, Clone)]
struct NormalizedRequestUserInputOption {
    label: String,
    description: String,
}

#[derive(Debug, Clone)]
struct NormalizedRequestUserInputQuestion {
    header: String,
    id: String,
    question: String,
    options: Vec<NormalizedRequestUserInputOption>,
}

const REQUEST_USER_INPUT_MAX_QUESTIONS: usize = 3;
const REQUEST_USER_INPUT_MIN_OPTIONS: usize = 2;
const REQUEST_USER_INPUT_MAX_OPTIONS: usize = 3;
const REQUEST_USER_INPUT_HEADER_MAX_CHARS: usize = 12;
const REQUEST_USER_INPUT_SINGLE_KEY: &str = "__single__";

fn is_valid_request_user_input_question_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() || id.ends_with('_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn normalize_request_user_input_args(
    args: RequestUserInputArgs,
) -> anyhow::Result<Vec<NormalizedRequestUserInputQuestion>> {
    if args.questions.is_empty() {
        anyhow::bail!("questions must not be empty");
    }
    if args.questions.len() > REQUEST_USER_INPUT_MAX_QUESTIONS {
        anyhow::bail!("questions supports at most {REQUEST_USER_INPUT_MAX_QUESTIONS} items");
    }

    let mut seen_ids = std::collections::BTreeSet::<String>::new();
    let mut normalized =
        Vec::<NormalizedRequestUserInputQuestion>::with_capacity(args.questions.len());
    for (question_idx, question) in args.questions.into_iter().enumerate() {
        let header = question.header.trim().to_string();
        if header.is_empty() {
            anyhow::bail!("questions[{question_idx}].header must not be empty");
        }
        if header.chars().count() > REQUEST_USER_INPUT_HEADER_MAX_CHARS {
            anyhow::bail!(
                "questions[{question_idx}].header must be <= {REQUEST_USER_INPUT_HEADER_MAX_CHARS} chars"
            );
        }

        let id = question.id.trim().to_string();
        if !is_valid_request_user_input_question_id(&id) {
            anyhow::bail!(
                "questions[{question_idx}].id must be snake_case and start with a letter"
            );
        }
        if !seen_ids.insert(id.clone()) {
            anyhow::bail!("duplicate questions[].id: {id}");
        }

        let prompt = question.question.trim().to_string();
        if prompt.is_empty() {
            anyhow::bail!("questions[{question_idx}].question must not be empty");
        }

        if question.options.len() < REQUEST_USER_INPUT_MIN_OPTIONS
            || question.options.len() > REQUEST_USER_INPUT_MAX_OPTIONS
        {
            anyhow::bail!(
                "questions[{question_idx}].options must contain {}-{} items",
                REQUEST_USER_INPUT_MIN_OPTIONS,
                REQUEST_USER_INPUT_MAX_OPTIONS
            );
        }

        let mut option_labels_seen = std::collections::BTreeSet::<String>::new();
        let mut options =
            Vec::<NormalizedRequestUserInputOption>::with_capacity(question.options.len());
        for (option_idx, option) in question.options.into_iter().enumerate() {
            let label = option.label.trim().to_string();
            if label.is_empty() {
                anyhow::bail!(
                    "questions[{question_idx}].options[{option_idx}].label must not be empty"
                );
            }
            let label_key = label.to_ascii_lowercase();
            if !option_labels_seen.insert(label_key) {
                anyhow::bail!("questions[{question_idx}].options contains duplicate labels");
            }

            let description = option.description.trim().to_string();
            if description.is_empty() {
                anyhow::bail!(
                    "questions[{question_idx}].options[{option_idx}].description must not be empty"
                );
            }

            options.push(NormalizedRequestUserInputOption { label, description });
        }

        normalized.push(NormalizedRequestUserInputQuestion {
            header,
            id,
            question: prompt,
            options,
        });
    }

    Ok(normalized)
}

fn request_user_input_questions_to_json(
    questions: &[NormalizedRequestUserInputQuestion],
) -> Vec<Value> {
    questions
        .iter()
        .map(|question| {
            let options = question
                .options
                .iter()
                .map(|option| {
                    serde_json::json!({
                        "label": option.label,
                        "description": option.description,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "header": question.header,
                "id": question.id,
                "question": question.question,
                "options": options,
            })
        })
        .collect()
}

fn request_user_input_approval_params(questions: &[NormalizedRequestUserInputQuestion]) -> Value {
    serde_json::json!({
        "questions": request_user_input_questions_to_json(questions),
        "response_format": {
            "type": "json_object",
            "description": "Provide answers in approval reason as JSON object: {\"question_id\": \"option_label_or_index\"}",
        },
        "approval": {
            "requirement": "prompt_strict",
            "source": "request_user_input",
        },
    })
}

async fn load_approval_decision_reason(
    server: &super::Server,
    thread_id: ThreadId,
    approval_id: ApprovalId,
) -> anyhow::Result<Option<String>> {
    let events = server
        .thread_store
        .read_events_since(thread_id, EventSeq::ZERO)
        .await?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

    for event in events.into_iter().rev() {
        let omne_protocol::ThreadEventKind::ApprovalDecided {
            approval_id: decided_id,
            reason,
            ..
        } = event.kind
        else {
            continue;
        };
        if decided_id == approval_id {
            return Ok(reason);
        }
    }

    Ok(None)
}

fn parse_request_user_input_answer_map(
    reason: Option<&str>,
) -> std::collections::BTreeMap<String, Value> {
    let mut answers = std::collections::BTreeMap::<String, Value>::new();
    let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) else {
        return answers;
    };

    if let Ok(parsed) = serde_json::from_str::<Value>(reason) {
        match parsed {
            Value::Object(mut object) => {
                if let Some(Value::Object(answer_object)) = object.remove("answers") {
                    for (key, value) in answer_object {
                        answers.insert(key, value);
                    }
                    return answers;
                }

                for (key, value) in object {
                    answers.insert(key, value);
                }
                return answers;
            }
            Value::String(value) => {
                answers.insert(
                    REQUEST_USER_INPUT_SINGLE_KEY.to_string(),
                    Value::String(value),
                );
                return answers;
            }
            Value::Number(value) => {
                answers.insert(
                    REQUEST_USER_INPUT_SINGLE_KEY.to_string(),
                    Value::Number(value),
                );
                return answers;
            }
            _ => {}
        }
    }

    answers.insert(
        REQUEST_USER_INPUT_SINGLE_KEY.to_string(),
        Value::String(reason.to_string()),
    );
    answers
}

fn normalize_request_user_input_option_index(raw_index: u64, option_count: usize) -> Option<usize> {
    if option_count == 0 {
        return None;
    }
    let raw_index = raw_index as usize;
    if (1..=option_count).contains(&raw_index) {
        Some(raw_index - 1)
    } else if raw_index < option_count {
        Some(raw_index)
    } else {
        None
    }
}

fn match_request_user_input_option_index(
    answer: &Value,
    options: &[NormalizedRequestUserInputOption],
) -> Option<usize> {
    match answer {
        Value::Number(number) => number
            .as_u64()
            .and_then(|raw| normalize_request_user_input_option_index(raw, options.len())),
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty() {
                return None;
            }

            if let Ok(raw) = value.parse::<u64>() {
                if let Some(index) = normalize_request_user_input_option_index(raw, options.len()) {
                    return Some(index);
                }
            }

            let value = value.to_ascii_lowercase();
            options
                .iter()
                .position(|option| option.label.to_ascii_lowercase() == value)
        }
        _ => None,
    }
}

fn resolve_request_user_input_answers(
    reason: Option<&str>,
    questions: &[NormalizedRequestUserInputQuestion],
) -> (Vec<Value>, usize) {
    let answer_map = parse_request_user_input_answer_map(reason);
    let single_answer = answer_map.get(REQUEST_USER_INPUT_SINGLE_KEY);

    let mut answered_count = 0usize;
    let mut answers = Vec::<Value>::with_capacity(questions.len());
    for question in questions {
        let answer_value = answer_map.get(&question.id).or({
            if questions.len() == 1 {
                single_answer
            } else {
                None
            }
        });

        if answer_value.is_some() {
            answered_count += 1;
        }

        let selected_index = answer_value
            .and_then(|value| match_request_user_input_option_index(value, &question.options));
        let selected_option = selected_index.and_then(|index| question.options.get(index));

        answers.push(serde_json::json!({
            "id": question.id,
            "question": question.question,
            "answer_raw": answer_value.cloned(),
            "selected_option_index": selected_index.map(|index| index + 1),
            "selected_option_label": selected_option.map(|option| option.label.clone()),
            "selected_option_description": selected_option.map(|option| option.description.clone()),
        }));
    }

    (answers, answered_count)
}

async fn handle_request_user_input_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: RequestUserInputArgs,
) -> anyhow::Result<Value> {
    let questions = normalize_request_user_input_args(args)?;
    let (thread_rt, _thread_root) = super::load_thread_root(server, thread_id).await?;
    let approval_policy = {
        let handle = thread_rt.handle.lock().await;
        handle.state().approval_policy
    };
    let approval_params = request_user_input_approval_params(&questions);

    match super::gate_approval(
        server,
        &thread_rt,
        thread_id,
        turn_id,
        approval_policy,
        super::ApprovalRequest {
            approval_id,
            action: "thread/request_user_input",
            params: &approval_params,
        },
    )
    .await?
    {
        super::ApprovalGate::Approved => {}
        super::ApprovalGate::Denied { remembered } => {
            return Ok(serde_json::json!({
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

    let approval_id = approval_id
        .ok_or_else(|| anyhow::anyhow!("request_user_input approved without approval_id"))?;
    let reason = load_approval_decision_reason(server, thread_id, approval_id).await?;
    let (answers, answered_count) =
        resolve_request_user_input_answers(reason.as_deref(), &questions);
    Ok(serde_json::json!({
        "approval_id": approval_id,
        "questions": request_user_input_questions_to_json(&questions),
        "answers": answers,
        "answered_count": answered_count,
        "reason": reason,
    }))
}

const WEB_SEARCH_DEFAULT_MAX_RESULTS: usize = 5;
const WEB_SEARCH_MAX_RESULTS: usize = 10;
const WEB_FETCH_DEFAULT_MAX_BYTES: u64 = 120_000;
const WEB_FETCH_MAX_BYTES_LIMIT: u64 = 1_000_000;
const VIEW_IMAGE_DEFAULT_MAX_BYTES: u64 = 2_000_000;
const VIEW_IMAGE_MAX_BYTES_LIMIT: u64 = 8_000_000;
const WEB_HTTP_TIMEOUT_SECONDS: u64 = 20;
const WEB_TEXT_MAX_CHARS: usize = 40_000;
const WEB_HTTP_MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, Copy)]
struct ImageProbe {
    format: &'static str,
    mime_type: &'static str,
    width: Option<u32>,
    height: Option<u32>,
}

fn clamp_max_bytes(value: Option<u64>, default: u64, limit: u64) -> u64 {
    value.unwrap_or(default).clamp(1, limit)
}

fn clamp_max_results(value: Option<usize>) -> usize {
    value
        .unwrap_or(WEB_SEARCH_DEFAULT_MAX_RESULTS)
        .clamp(1, WEB_SEARCH_MAX_RESULTS)
}

fn validate_http_url(url: &str) -> anyhow::Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url.trim()).context("parse url")?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => anyhow::bail!("unsupported url scheme: {other}"),
    }
    if parsed.host().is_none() {
        anyhow::bail!("url must include a host");
    }
    Ok(parsed)
}

fn is_blocked_web_host_name(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("localhost.")
        || host.to_ascii_lowercase().ends_with(".localhost")
}

fn is_blocked_web_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && matches!(b, 18 | 19))
                || a >= 240
        }
        std::net::IpAddr::V6(ip) => {
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

async fn validate_web_outbound_url(url: &reqwest::Url) -> anyhow::Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("url must include a host"))?;
    if is_blocked_web_host_name(host) {
        anyhow::bail!("blocked local host: {host}");
    }

    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_blocked_web_ip(ip) {
            anyhow::bail!("blocked local address: {ip}");
        }
        return Ok(());
    }

    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("url must include a known port"))?;
    let mut resolved_any = false;
    for addr in tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolve host {host}:{port}"))?
    {
        resolved_any = true;
        if is_blocked_web_ip(addr.ip()) {
            anyhow::bail!("blocked local address in dns result for {host}");
        }
    }
    if !resolved_any {
        anyhow::bail!("host did not resolve: {host}");
    }
    Ok(())
}

fn resolve_web_redirect_url(
    current_url: &reqwest::Url,
    location: &str,
) -> anyhow::Result<reqwest::Url> {
    let next_url = current_url
        .join(location)
        .with_context(|| format!("resolve redirect location from {}", current_url))?;
    match next_url.scheme() {
        "http" | "https" => Ok(next_url),
        other => anyhow::bail!("unsupported redirect url scheme: {other}"),
    }
}

fn build_web_http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(WEB_HTTP_TIMEOUT_SECONDS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("omne-agent/0.1 (web-tools)")
        .build()
        .context("build web http client")
}

fn collapse_whitespace(value: &str) -> String {
    let mut out = String::new();
    for token in value.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
    }
    out
}

fn decode_html_entities_basic(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn html_to_text(value: &str) -> String {
    static SCRIPT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static STYLE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static TAG_RE: OnceLock<regex::Regex> = OnceLock::new();
    static COMMENT_RE: OnceLock<regex::Regex> = OnceLock::new();

    let no_script = SCRIPT_RE
        .get_or_init(|| regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("script regex"))
        .replace_all(value, " ");
    let no_style = STYLE_RE
        .get_or_init(|| regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("style regex"))
        .replace_all(&no_script, " ");
    let no_comment = COMMENT_RE
        .get_or_init(|| regex::Regex::new(r"(?is)<!--.*?-->").expect("comment regex"))
        .replace_all(&no_style, " ");
    let no_tags = TAG_RE
        .get_or_init(|| regex::Regex::new(r"(?is)<[^>]+>").expect("tag regex"))
        .replace_all(&no_comment, " ");
    collapse_whitespace(&decode_html_entities_basic(&no_tags))
}

fn extract_html_title(value: &str) -> Option<String> {
    static TITLE_RE: OnceLock<regex::Regex> = OnceLock::new();
    let captures = TITLE_RE
        .get_or_init(|| regex::Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("title regex"))
        .captures(value)?;
    let title = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
    let title = html_to_text(title);
    (!title.is_empty()).then_some(title)
}

fn truncate_text_chars(value: &str, max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            return (out, true);
        }
        out.push(ch);
    }
    (out, false)
}

fn normalize_duckduckgo_result_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return raw.to_string();
    }
    if raw.starts_with("//") {
        return format!("https:{raw}");
    }

    let joined = reqwest::Url::parse("https://duckduckgo.com")
        .ok()
        .and_then(|base| base.join(raw).ok());
    let Some(joined) = joined else {
        return raw.to_string();
    };

    if joined.path().starts_with("/l/")
        && let Some((_, value)) = joined.query_pairs().find(|(key, _)| key == "uddg")
    {
        return value.into_owned();
    }
    joined.to_string()
}

fn parse_duckduckgo_html_results(html: &str, max_results: usize) -> Vec<Value> {
    static RESULT_RE: OnceLock<regex::Regex> = OnceLock::new();
    let mut out = Vec::<Value>::new();
    let mut seen_urls = std::collections::BTreeSet::<String>::new();

    let result_re = RESULT_RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?is)<a[^>]*class=\"[^\"]*result__a[^\"]*\"[^>]*href=\"([^\"]+)\"[^>]*>(.*?)</a>"#,
        )
        .expect("result regex")
    });

    for captures in result_re.captures_iter(html) {
        if out.len() >= max_results {
            break;
        }

        let raw_url = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
        let url = normalize_duckduckgo_result_url(raw_url);
        if url.is_empty() || !seen_urls.insert(url.clone()) {
            continue;
        }

        let title_html = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
        let title = html_to_text(title_html);
        if title.is_empty() {
            continue;
        }

        out.push(serde_json::json!({
            "title": title,
            "url": url,
        }));
    }

    out
}

async fn fetch_http_bytes_limited(
    client: &reqwest::Client,
    url: reqwest::Url,
    max_bytes: u64,
) -> anyhow::Result<(
    reqwest::StatusCode,
    reqwest::Url,
    Option<String>,
    Vec<u8>,
    bool,
)> {
    let mut current_url = url;
    for _ in 0..=WEB_HTTP_MAX_REDIRECTS {
        validate_web_outbound_url(&current_url).await?;
        let response = client
            .get(current_url.clone())
            .send()
            .await
            .context("send web request")?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| anyhow::anyhow!("redirect response missing Location header"))?
                .to_str()
                .context("redirect Location header is not valid utf-8")?;
            current_url = resolve_web_redirect_url(&current_url, location)?;
            continue;
        }

        let final_url = response.url().clone();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);

        let mut truncated = false;
        let mut bytes = Vec::<u8>::new();
        let max_bytes = usize::try_from(max_bytes).unwrap_or(usize::MAX);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("read web response body chunk")?;
            if bytes.len() >= max_bytes {
                truncated = true;
                break;
            }

            let remaining = max_bytes.saturating_sub(bytes.len());
            if chunk.len() > remaining {
                bytes.extend_from_slice(&chunk[..remaining]);
                truncated = true;
                break;
            }

            bytes.extend_from_slice(&chunk);
        }

        return Ok((status, final_url, content_type, bytes, truncated));
    }
    anyhow::bail!("too many redirects")
}

fn is_textual_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type.map(|value| value.to_ascii_lowercase()) else {
        return true;
    };
    content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
        || content_type.contains("yaml")
        || content_type.contains("csv")
}

async fn enforce_browser_mode_and_approval(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    action: &'static str,
    approval_params: &Value,
) -> anyhow::Result<Option<Value>> {
    let (thread_rt, thread_root) = super::load_thread_root(server, thread_id).await?;
    let (approval_policy, mode_name) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (state.approval_policy, state.mode.clone())
    };

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let Some(mode) = catalog.mode(&mode_name) else {
        let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
        return Ok(Some(ensure_machine_error_fields(
            serde_json::json!({
                "denied": true,
                "mode": mode_name,
                "decision": "deny",
                "available_modes": available,
                "error_code": "mode_unknown",
            }),
            "mode_unknown",
        )));
    };

    let mode_decision = crate::resolve_mode_decision_audit(mode, action, mode.permissions.browser);
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        return Ok(Some(ensure_machine_error_fields(
            serde_json::json!({
                "denied": true,
                "mode": mode_name,
                "decision": mode_decision.decision,
                "decision_source": mode_decision.decision_source,
                "tool_override_hit": mode_decision.tool_override_hit,
                "error_code": "mode_denied",
            }),
            "mode_denied",
        )));
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match super::gate_approval(
            server,
            &thread_rt,
            thread_id,
            turn_id,
            approval_policy,
            super::ApprovalRequest {
                approval_id,
                action,
                params: approval_params,
            },
        )
        .await?
        {
            super::ApprovalGate::Approved => {}
            super::ApprovalGate::Denied { remembered } => {
                return Ok(Some(ensure_machine_error_fields(
                    serde_json::json!({
                        "denied": true,
                        "remembered": remembered,
                        "error_code": "approval_denied",
                    }),
                    "approval_denied",
                )));
            }
            super::ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(Some(serde_json::json!({
                    "needs_approval": true,
                    "approval_id": approval_id,
                })));
            }
        }
    }

    Ok(None)
}

async fn handle_web_search_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: WebSearchArgs,
) -> anyhow::Result<Value> {
    let query = args.query.trim().to_string();
    if query.is_empty() {
        anyhow::bail!("query must not be empty");
    }
    let max_results = clamp_max_results(args.max_results);

    let approval_params = serde_json::json!({
        "query": query,
        "max_results": max_results,
    });
    if let Some(result) = enforce_browser_mode_and_approval(
        server,
        thread_id,
        turn_id,
        approval_id,
        "web/search",
        &approval_params,
    )
    .await?
    {
        return Ok(result);
    }

    let client = build_web_http_client()?;
    let search_url =
        reqwest::Url::parse_with_params("https://duckduckgo.com/html/", &[("q", query.as_str())])
            .context("build search url")?;
    let response = client
        .get(search_url.clone())
        .send()
        .await
        .context("execute web search request")?;

    let status = response.status();
    let final_url = response.url().to_string();
    let html = response
        .text()
        .await
        .context("read web search response body")?;
    let results = parse_duckduckgo_html_results(&html, max_results);

    Ok(serde_json::json!({
        "engine": "duckduckgo_html",
        "query": query,
        "status": status.as_u16(),
        "final_url": final_url,
        "results": results,
        "result_count": results.len(),
    }))
}

async fn handle_webfetch_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: WebFetchArgs,
) -> anyhow::Result<Value> {
    let url = validate_http_url(&args.url)?;
    let max_bytes = clamp_max_bytes(
        args.max_bytes,
        WEB_FETCH_DEFAULT_MAX_BYTES,
        WEB_FETCH_MAX_BYTES_LIMIT,
    );
    let approval_params = serde_json::json!({
        "url": url.as_str(),
        "max_bytes": max_bytes,
    });
    if let Some(result) = enforce_browser_mode_and_approval(
        server,
        thread_id,
        turn_id,
        approval_id,
        "web/fetch",
        &approval_params,
    )
    .await?
    {
        return Ok(result);
    }

    let client = build_web_http_client()?;
    let (status, final_url, content_type, bytes, fetch_truncated) =
        fetch_http_bytes_limited(&client, url, max_bytes).await?;

    let is_textual = is_textual_content_type(content_type.as_deref());
    let mut text = String::new();
    let mut text_truncated = false;
    let mut title: Option<String> = None;
    let mut binary_preview_base64: Option<String> = None;

    if is_textual {
        let body = String::from_utf8_lossy(&bytes);
        let body = body.as_ref();
        let looks_html = content_type
            .as_deref()
            .map(|value| value.to_ascii_lowercase().contains("html"))
            .unwrap_or(false)
            || body.contains("<html")
            || body.contains("<HTML");
        let normalized = if looks_html {
            title = extract_html_title(body);
            html_to_text(body)
        } else {
            collapse_whitespace(body)
        };
        let (truncated_text, was_truncated) = truncate_text_chars(&normalized, WEB_TEXT_MAX_CHARS);
        text = truncated_text;
        text_truncated = was_truncated;
    } else {
        let preview = &bytes[..bytes.len().min(512)];
        binary_preview_base64 = Some(base64::engine::general_purpose::STANDARD.encode(preview));
    }

    Ok(serde_json::json!({
        "url": args.url,
        "final_url": final_url.to_string(),
        "status": status.as_u16(),
        "content_type": content_type,
        "bytes_read": bytes.len(),
        "fetch_truncated": fetch_truncated,
        "text_truncated": text_truncated,
        "is_textual": is_textual,
        "title": title,
        "text": text,
        "binary_preview_base64": binary_preview_base64,
    }))
}

fn probe_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut idx = 2usize;
    while idx + 3 < bytes.len() {
        if bytes[idx] != 0xFF {
            idx += 1;
            continue;
        }
        let marker = bytes[idx + 1];
        idx += 2;

        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if idx + 2 > bytes.len() {
            break;
        }

        let segment_len = u16::from_be_bytes([bytes[idx], bytes[idx + 1]]) as usize;
        if segment_len < 2 || idx + segment_len > bytes.len() {
            break;
        }

        let is_sof = matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        );
        if is_sof && segment_len >= 7 {
            let height = u16::from_be_bytes([bytes[idx + 3], bytes[idx + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[idx + 5], bytes[idx + 6]]) as u32;
            return Some((width, height));
        }

        idx += segment_len;
    }
    None
}

fn guess_image_mime_from_name(name: Option<&str>) -> Option<(&'static str, &'static str)> {
    let name = name?.to_ascii_lowercase();
    if name.ends_with(".png") {
        Some(("png", "image/png"))
    } else if name.ends_with(".jpg") || name.ends_with(".jpeg") {
        Some(("jpeg", "image/jpeg"))
    } else if name.ends_with(".gif") {
        Some(("gif", "image/gif"))
    } else if name.ends_with(".webp") {
        Some(("webp", "image/webp"))
    } else if name.ends_with(".bmp") {
        Some(("bmp", "image/bmp"))
    } else if name.ends_with(".svg") {
        Some(("svg", "image/svg+xml"))
    } else {
        None
    }
}

fn probe_image(bytes: &[u8], name_hint: Option<&str>) -> ImageProbe {
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return ImageProbe {
            format: "png",
            mime_type: "image/png",
            width: Some(width),
            height: Some(height),
        };
    }

    if bytes.len() >= 10 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return ImageProbe {
            format: "gif",
            mime_type: "image/gif",
            width: Some(width),
            height: Some(height),
        };
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP" as &[u8])
    {
        return ImageProbe {
            format: "webp",
            mime_type: "image/webp",
            width: None,
            height: None,
        };
    }

    if bytes.len() >= 26 && bytes.starts_with(b"BM") {
        let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        return ImageProbe {
            format: "bmp",
            mime_type: "image/bmp",
            width: (width > 0).then_some(width as u32),
            height: (height != 0).then_some(height.unsigned_abs()),
        };
    }

    if let Some((width, height)) = probe_jpeg_dimensions(bytes) {
        return ImageProbe {
            format: "jpeg",
            mime_type: "image/jpeg",
            width: Some(width),
            height: Some(height),
        };
    }

    if let Ok(text) = std::str::from_utf8(bytes)
        && text.to_ascii_lowercase().contains("<svg")
    {
        return ImageProbe {
            format: "svg",
            mime_type: "image/svg+xml",
            width: None,
            height: None,
        };
    }

    if let Some((format, mime_type)) = guess_image_mime_from_name(name_hint) {
        return ImageProbe {
            format,
            mime_type,
            width: None,
            height: None,
        };
    }

    ImageProbe {
        format: "unknown",
        mime_type: "application/octet-stream",
        width: None,
        height: None,
    }
}

async fn handle_view_image_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: ViewImageArgs,
) -> anyhow::Result<Value> {
    let path = normalize_optional_string(args.path);
    let url = normalize_optional_string(args.url);
    if path.is_some() {
        anyhow::bail!("view_image only supports url inputs");
    }
    let Some(url) = url else {
        anyhow::bail!("url is required");
    };

    let max_bytes = clamp_max_bytes(
        args.max_bytes,
        VIEW_IMAGE_DEFAULT_MAX_BYTES,
        VIEW_IMAGE_MAX_BYTES_LIMIT,
    );
    let approval_params = serde_json::json!({
        "url": url,
        "max_bytes": max_bytes,
    });
    if let Some(result) = enforce_browser_mode_and_approval(
        server,
        thread_id,
        turn_id,
        approval_id,
        "web/view_image",
        &approval_params,
    )
    .await?
    {
        return Ok(result);
    }

    let parsed_url = validate_http_url(&url)?;
    let client = build_web_http_client()?;
    let (_status, final_url, _content_type, bytes, truncated) =
        fetch_http_bytes_limited(&client, parsed_url, max_bytes).await?;
    let name_hint = final_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(|s| s.to_string());

    let probe = probe_image(&bytes, name_hint.as_deref());

    Ok(serde_json::json!({
        "source": "url",
        "path": Value::Null,
        "url": final_url.to_string(),
        "format": probe.format,
        "mime_type": probe.mime_type,
        "width": probe.width,
        "height": probe.height,
        "bytes_read": bytes.len(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
mod update_plan_tests {
    use super::*;

    #[test]
    fn normalize_update_plan_steps_rejects_multiple_in_progress() {
        let err = normalize_update_plan_steps(UpdatePlanArgs {
            explanation: None,
            plan: vec![
                UpdatePlanStepArgs {
                    step: "first".to_string(),
                    status: "in_progress".to_string(),
                },
                UpdatePlanStepArgs {
                    step: "second".to_string(),
                    status: "in_progress".to_string(),
                },
            ],
        })
        .expect_err("expected validation error");

        assert!(err.to_string().contains("at most one in_progress"));
    }

    #[test]
    fn normalize_update_plan_steps_normalizes_status_and_summary_source() {
        let (explanation, steps) = normalize_update_plan_steps(UpdatePlanArgs {
            explanation: Some("  investigate root cause  ".to_string()),
            plan: vec![
                UpdatePlanStepArgs {
                    step: " collect traces ".to_string(),
                    status: "IN-PROGRESS".to_string(),
                },
                UpdatePlanStepArgs {
                    step: "ship fix".to_string(),
                    status: "completed".to_string(),
                },
            ],
        })
        .expect("expected normalized plan");

        assert_eq!(explanation.as_deref(), Some("investigate root cause"));
        assert_eq!(steps[0].step, "collect traces");
        assert_eq!(steps[0].status, "in_progress");
        assert_eq!(steps[1].status, "completed");
        assert_eq!(
            summarize_update_plan_artifact(explanation.as_deref(), &steps),
            "investigate root cause"
        );
    }

    #[test]
    fn render_update_plan_artifact_text_contains_expected_sections() {
        let text = render_update_plan_artifact_text(
            Some("explain"),
            &[
                NormalizedUpdatePlanStep {
                    step: "do work".to_string(),
                    status: "pending",
                },
                NormalizedUpdatePlanStep {
                    step: "verify".to_string(),
                    status: "completed",
                },
            ],
        );

        assert!(text.starts_with("# Plan\n\n"));
        assert!(text.contains("## Explanation\n\nexplain"));
        assert!(text.contains("## Steps\n\n1. [pending] do work\n2. [completed] verify"));
    }
}

#[cfg(test)]
mod request_user_input_tests {
    use super::*;

    #[test]
    fn normalize_request_user_input_args_rejects_invalid_id() {
        let err = normalize_request_user_input_args(RequestUserInputArgs {
            questions: vec![RequestUserInputQuestionArgs {
                header: "choice".to_string(),
                id: "NotSnakeCase".to_string(),
                question: "Pick one".to_string(),
                options: vec![
                    RequestUserInputOptionArgs {
                        label: "A".to_string(),
                        description: "Option A".to_string(),
                    },
                    RequestUserInputOptionArgs {
                        label: "B".to_string(),
                        description: "Option B".to_string(),
                    },
                ],
            }],
        })
        .expect_err("expected invalid id");

        assert!(err.to_string().contains("must be snake_case"));
    }

    #[test]
    fn resolve_request_user_input_answers_supports_json_mapping() {
        let questions = normalize_request_user_input_args(RequestUserInputArgs {
            questions: vec![
                RequestUserInputQuestionArgs {
                    header: "model".to_string(),
                    id: "model_choice".to_string(),
                    question: "Use which model?".to_string(),
                    options: vec![
                        RequestUserInputOptionArgs {
                            label: "gpt-5".to_string(),
                            description: "High quality".to_string(),
                        },
                        RequestUserInputOptionArgs {
                            label: "gpt-5-mini".to_string(),
                            description: "Lower cost".to_string(),
                        },
                    ],
                },
                RequestUserInputQuestionArgs {
                    header: "depth".to_string(),
                    id: "analysis_depth".to_string(),
                    question: "How deep?".to_string(),
                    options: vec![
                        RequestUserInputOptionArgs {
                            label: "quick".to_string(),
                            description: "Fast pass".to_string(),
                        },
                        RequestUserInputOptionArgs {
                            label: "deep".to_string(),
                            description: "Thorough".to_string(),
                        },
                        RequestUserInputOptionArgs {
                            label: "full".to_string(),
                            description: "Exhaustive".to_string(),
                        },
                    ],
                },
            ],
        })
        .expect("expected normalized questions");

        let reason = r#"{"model_choice":"gpt-5-mini","analysis_depth":2}"#;
        let (answers, answered_count) =
            resolve_request_user_input_answers(Some(reason), &questions);

        assert_eq!(answered_count, 2);
        assert_eq!(
            answers[0]["selected_option_label"].as_str(),
            Some("gpt-5-mini")
        );
        assert_eq!(answers[1]["selected_option_index"].as_u64(), Some(2));
        assert_eq!(answers[1]["selected_option_label"].as_str(), Some("deep"));
    }

    #[test]
    fn resolve_request_user_input_answers_supports_single_plain_text_answer() {
        let questions = normalize_request_user_input_args(RequestUserInputArgs {
            questions: vec![RequestUserInputQuestionArgs {
                header: "plan".to_string(),
                id: "next_step".to_string(),
                question: "Next step?".to_string(),
                options: vec![
                    RequestUserInputOptionArgs {
                        label: "fix".to_string(),
                        description: "Fix now".to_string(),
                    },
                    RequestUserInputOptionArgs {
                        label: "investigate".to_string(),
                        description: "Investigate first".to_string(),
                    },
                ],
            }],
        })
        .expect("expected normalized question");

        let (answers, answered_count) =
            resolve_request_user_input_answers(Some("investigate"), &questions);
        assert_eq!(answered_count, 1);
        assert_eq!(
            answers[0]["selected_option_label"].as_str(),
            Some("investigate")
        );
    }
}

#[cfg(test)]
mod web_tools_tests {
    use super::*;

    #[test]
    fn parse_duckduckgo_html_results_extracts_redirect_url() {
        let html = r#"
        <div>
          <a class="result__a" href="/l/?uddg=https%3A%2F%2Fexample.com%2Fdoc">Example &amp; Title</a>
          <a class="result__a" href="https://example.org/guide">Guide</a>
        </div>
        "#;
        let results = parse_duckduckgo_html_results(html, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["url"].as_str(), Some("https://example.com/doc"));
        assert_eq!(results[0]["title"].as_str(), Some("Example & Title"));
        assert_eq!(
            results[1]["url"].as_str(),
            Some("https://example.org/guide")
        );
    }

    #[test]
    fn probe_image_detects_png_dimensions() {
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        bytes[16..20].copy_from_slice(&42u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&24u32.to_be_bytes());
        let probe = probe_image(&bytes, Some("demo.png"));
        assert_eq!(probe.format, "png");
        assert_eq!(probe.mime_type, "image/png");
        assert_eq!(probe.width, Some(42));
        assert_eq!(probe.height, Some(24));
    }

    #[test]
    fn html_to_text_removes_tags_and_decodes_entities() {
        let html = "<html><body><h1>Hi&nbsp;there</h1><p>A &amp; B</p></body></html>";
        assert_eq!(html_to_text(html), "Hi there A & B");
    }
}

#[cfg(test)]
mod facade_tool_tests {
    use super::*;

    #[test]
    fn facade_collect_mapped_args_accepts_top_level_legacy_shape() {
        let request: FacadeToolArgs = serde_json::from_value(serde_json::json!({
            "op": "read",
            "path": "README.md",
            "root": "workspace",
        }))
        .expect("parse facade request");

        let args = facade_collect_mapped_args(&request).expect("collect args");
        assert_eq!(args.get("path").and_then(Value::as_str), Some("README.md"));
        assert_eq!(args.get("root").and_then(Value::as_str), Some("workspace"));
    }

    #[test]
    fn facade_collect_and_normalize_workspace_glob_uses_reserved_topic_field() {
        let request: FacadeToolArgs = serde_json::from_value(serde_json::json!({
            "op": "glob",
            "topic": "safe-fs-tools/cli/**/*"
        }))
        .expect("parse facade request");

        let mapped = facade_collect_mapped_args(&request).expect("collect args");
        let normalized = facade_normalize_mapped_args(FacadeToolKind::Workspace, "glob", mapped);
        assert_eq!(
            normalized.get("pattern").and_then(Value::as_str),
            Some("safe-fs-tools/cli/**/*")
        );
    }

    #[test]
    fn facade_collect_mapped_args_rejects_nested_args_wrapper() {
        let request: FacadeToolArgs = serde_json::from_value(serde_json::json!({
            "op": "write",
            "path": "README.md",
            "args": { "text": "hello", "path": "tmp/a.txt" }
        }))
        .expect("parse facade request");

        let err = facade_collect_mapped_args(&request).expect_err("should reject args wrapper");
        assert!(
            err.to_string()
                .contains("args wrapper is not allowed for facade calls")
        );
    }

    #[test]
    fn facade_normalize_workspace_edit_translates_old_text_shape() {
        let normalized = facade_normalize_mapped_args(
            FacadeToolKind::Workspace,
            "edit",
            serde_json::json!({
                "path": "README.md",
                "old_text": "OmneAgent",
                "new_text": "Omne Agent",
            }),
        );
        let edits = normalized
            .get("edits")
            .and_then(Value::as_array)
            .expect("edits");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].get("old").and_then(Value::as_str),
            Some("OmneAgent")
        );
        assert_eq!(
            edits[0].get("new").and_then(Value::as_str),
            Some("Omne Agent")
        );
    }

    #[test]
    fn facade_normalize_workspace_glob_maps_topic_alias_to_pattern() {
        let normalized = facade_normalize_mapped_args(
            FacadeToolKind::Workspace,
            "glob",
            serde_json::json!({
                "topic": "safe-fs-tools/cli/**/*",
            }),
        );
        assert_eq!(
            normalized.get("pattern").and_then(Value::as_str),
            Some("safe-fs-tools/cli/**/*")
        );
    }

    #[test]
    fn facade_normalize_workspace_glob_keeps_pattern_and_drops_aliases() {
        let normalized = facade_normalize_mapped_args(
            FacadeToolKind::Workspace,
            "glob",
            serde_json::json!({
                "pattern": "**/*.rs",
                "topic": "ignored",
                "path": "ignored",
                "query": "ignored",
            }),
        );
        assert_eq!(
            normalized.get("pattern").and_then(Value::as_str),
            Some("**/*.rs")
        );
        assert!(normalized.get("topic").is_none());
        assert!(normalized.get("path").is_none());
        assert!(normalized.get("query").is_none());
    }

    #[test]
    fn facade_normalize_process_start_supports_command_alias() {
        let normalized = facade_normalize_mapped_args(
            FacadeToolKind::Process,
            "start",
            serde_json::json!({
                "command": ["echo", "hello"],
            }),
        );
        let argv = normalized
            .get("argv")
            .and_then(Value::as_array)
            .expect("argv");
        assert_eq!(argv.len(), 2);
        assert_eq!(argv[0].as_str(), Some("echo"));
        assert_eq!(argv[1].as_str(), Some("hello"));
    }

    #[test]
    fn facade_normalize_thread_send_input_supports_subagent_alias() {
        let normalized = facade_normalize_mapped_args(
            FacadeToolKind::Thread,
            "send_input",
            serde_json::json!({
                "subagent_id": "sa_001",
                "input": "continue",
            }),
        );
        assert_eq!(normalized.get("id").and_then(Value::as_str), Some("sa_001"));
        assert_eq!(
            normalized.get("message").and_then(Value::as_str),
            Some("continue")
        );
    }

    #[test]
    fn facade_help_returns_quickstart_and_advanced() {
        let value = facade_help_value(FacadeToolKind::Workspace, None).expect("facade help");
        let quickstart = value
            .get("quickstart")
            .and_then(Value::as_array)
            .expect("quickstart array");
        let advanced = value
            .get("advanced")
            .and_then(Value::as_array)
            .expect("advanced array");
        assert!(!quickstart.is_empty(), "quickstart should not be empty");
        assert!(!advanced.is_empty(), "advanced should not be empty");
    }

    #[test]
    fn facade_help_rejects_unknown_topic() {
        let value = facade_help_value(FacadeToolKind::Process, Some("unknown".to_string()))
            .expect("facade help");
        assert_eq!(
            value
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some(FACADE_ERROR_UNSUPPORTED_OP)
        );
    }

    #[test]
    fn facade_route_maps_known_operations() {
        assert_eq!(
            facade_route(FacadeToolKind::Workspace, "read"),
            Some(("file_read", "file/read"))
        );
        assert_eq!(
            facade_route(FacadeToolKind::Integration, "web_fetch"),
            Some(("webfetch", "web/fetch"))
        );
        assert_eq!(
            facade_route(FacadeToolKind::Thread, "send_input"),
            Some(("subagent_send_input", "subagent/send_input"))
        );
        assert_eq!(
            facade_route(FacadeToolKind::Thread, "close_agent"),
            Some(("subagent_close", "subagent/close"))
        );
        assert_eq!(facade_route(FacadeToolKind::Thread, "unknown"), None);
    }

    #[test]
    fn facade_error_contains_stable_code() {
        let value = facade_error_value(
            "workspace",
            Some("read".to_string()),
            Some("file/read".to_string()),
            FACADE_ERROR_INVALID_PARAMS,
            "bad args",
        );
        assert_eq!(
            value
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some(FACADE_ERROR_INVALID_PARAMS)
        );
    }
}

#[cfg(test)]
mod mcp_call_args_tests {
    use super::*;

    #[test]
    fn mcp_call_accepts_flattened_top_level_fields() {
        let parsed = parse_mcp_call_args_with_flatten_compat(serde_json::json!({
            "server": "default",
            "tool": "echo",
            "text": "hello",
        }))
        .expect("flattened args should be accepted");

        assert_eq!(parsed.server, "default");
        assert_eq!(parsed.tool, "echo");
        assert_eq!(
            parsed.arguments.get("text").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn mcp_call_unwraps_input_wrapper_object() {
        let parsed = parse_mcp_call_args_with_flatten_compat(serde_json::json!({
            "server": "default",
            "tool": "echo",
            "input": { "text": "hello" },
        }))
        .expect("wrapped input args should be accepted");

        assert_eq!(
            parsed.arguments.get("text").and_then(Value::as_str),
            Some("hello")
        );
        assert!(parsed.arguments.get("input").is_none());
    }

    #[test]
    fn mcp_call_merges_explicit_arguments_and_top_level_supplements() {
        let parsed = parse_mcp_call_args_with_flatten_compat(serde_json::json!({
            "server": "default",
            "tool": "echo",
            "arguments": { "text": "hello" },
            "lang": "zh-CN",
        }))
        .expect("mixed arguments should be accepted");

        assert_eq!(
            parsed.arguments.get("text").and_then(Value::as_str),
            Some("hello")
        );
        assert_eq!(
            parsed.arguments.get("lang").and_then(Value::as_str),
            Some("zh-CN")
        );
    }
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
        let isolated_cwd = if matches!(plan.workspace_mode, AgentSpawnWorkspaceMode::IsolatedWrite)
        {
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
            AgentSpawnWorkspaceMode::ReadOnly => policy_meta::WriteScope::ReadOnly,
            AgentSpawnWorkspaceMode::IsolatedWrite => policy_meta::WriteScope::WorkspaceWrite,
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
                role: Some(plan.mode.clone()),
                model: plan.model.clone(),
                clear_model: false,
                thinking: None,
                clear_thinking: false,
                show_thinking: None,
                clear_show_thinking: false,
                openai_base_url: plan.openai_base_url.clone(),
                clear_openai_base_url: false,
                allowed_tools: None,
                execpolicy_rules: None,
                clear_execpolicy_rules: false,
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

#[derive(Clone, Copy)]
enum FacadeToolKind {
    Workspace,
    Process,
    Thread,
    Artifact,
    Integration,
}

impl FacadeToolKind {
    fn from_tool_name(tool_name: &str) -> Option<Self> {
        match tool_name {
            "workspace" => Some(Self::Workspace),
            "process" => Some(Self::Process),
            "thread" => Some(Self::Thread),
            "artifact" => Some(Self::Artifact),
            "integration" => Some(Self::Integration),
            _ => None,
        }
    }

    fn as_tool_name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Process => "process",
            Self::Thread => "thread",
            Self::Artifact => "artifact",
            Self::Integration => "integration",
        }
    }
}

fn facade_normalized_op(request: &FacadeToolArgs) -> Option<String> {
    if request.help.unwrap_or(false) {
        return Some("help".to_string());
    }
    let op = request.op.as_deref()?.trim().to_ascii_lowercase();
    (!op.is_empty()).then_some(op)
}

fn facade_collect_mapped_args(request: &FacadeToolArgs) -> anyhow::Result<Value> {
    let mut merged = request.extra.clone();
    if merged.contains_key("args") {
        anyhow::bail!(
            "args wrapper is not allowed for facade calls; use flat root-level parameters"
        );
    }
    if let Some(topic) = request
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        merged
            .entry("topic".to_string())
            .or_insert_with(|| Value::String(topic.to_string()));
    }
    Ok(Value::Object(merged))
}

fn facade_normalize_mapped_args(kind: FacadeToolKind, op: &str, mapped_args: Value) -> Value {
    let Value::Object(mut object) = mapped_args else {
        return mapped_args;
    };

    match kind {
        FacadeToolKind::Workspace => match op {
            "write" => {
                if !object.contains_key("text") {
                    if let Some(content) = object.remove("content") {
                        object.insert("text".to_string(), content);
                    }
                } else {
                    object.remove("content");
                }
            }
            "edit" => {
                if !object.contains_key("edits") {
                    let old_text = object.get("old_text").cloned();
                    let new_text = object.get("new_text").cloned();
                    if let (Some(old), Some(new)) = (old_text, new_text) {
                        object.remove("old_text");
                        object.remove("new_text");
                        let mut edit = serde_json::Map::new();
                        edit.insert("old".to_string(), old);
                        edit.insert("new".to_string(), new);
                        if let Some(expected) = object.remove("expected_replacements") {
                            edit.insert("expected_replacements".to_string(), expected);
                        }
                        object.insert("edits".to_string(), Value::Array(vec![Value::Object(edit)]));
                    }
                }
            }
            "glob" => {
                if !object.contains_key("pattern") {
                    if let Some(pattern) = object
                        .remove("topic")
                        .or_else(|| object.remove("path"))
                        .or_else(|| object.remove("query"))
                    {
                        object.insert("pattern".to_string(), pattern);
                    }
                } else {
                    object.remove("topic");
                    object.remove("path");
                    object.remove("query");
                }
            }
            _ => {}
        },
        FacadeToolKind::Process => {
            if op == "start" && !object.contains_key("argv") {
                if let Some(command) = object.remove("command") {
                    match command {
                        Value::Array(_) => {
                            object.insert("argv".to_string(), command);
                        }
                        Value::String(raw) => {
                            let argv = raw
                                .split_whitespace()
                                .filter(|token| !token.is_empty())
                                .map(|token| Value::String(token.to_string()))
                                .collect::<Vec<_>>();
                            if !argv.is_empty() {
                                object.insert("argv".to_string(), Value::Array(argv));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        FacadeToolKind::Thread => match op {
            "request_input" => {
                if !object.contains_key("questions") {
                    let prompt = object
                        .get("input")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned);
                    if let Some(prompt) = prompt {
                        object.insert(
                            "questions".to_string(),
                            serde_json::json!([{
                                "header": "Input",
                                "id": "q1",
                                "question": prompt,
                                "options": [
                                    {"label": "Yes", "description": "Continue"},
                                    {"label": "No", "description": "Stop"}
                                ]
                            }]),
                        );
                    }
                }
            }
            "spawn_agent" | "agent_spawn" => {
                if !object.contains_key("tasks") {
                    let goal = object
                        .get("goal")
                        .or_else(|| object.get("input"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned);
                    if let Some(goal) = goal {
                        object.insert(
                            "tasks".to_string(),
                            serde_json::json!([{ "id": "task_1", "input": goal }]),
                        );
                    }
                }
            }
            "send_input" => {
                if !object.contains_key("id") {
                    if let Some(id) = object.get("subagent_id").cloned() {
                        object.insert("id".to_string(), id);
                    }
                }
                if !object.contains_key("message") {
                    if let Some(message) = object.get("input").cloned() {
                        object.insert("message".to_string(), message);
                    }
                }
            }
            "wait" => {
                if !object.contains_key("ids") {
                    if let Some(id) = object.get("subagent_id").cloned() {
                        object.insert("ids".to_string(), Value::Array(vec![id]));
                    }
                }
            }
            "close" | "close_agent" => {
                if !object.contains_key("id") {
                    if let Some(id) = object.get("subagent_id").cloned() {
                        object.insert("id".to_string(), id);
                    }
                }
            }
            _ => {}
        },
        FacadeToolKind::Artifact => match op {
            "write" => {
                if !object.contains_key("text") {
                    if let Some(content) = object.remove("content") {
                        object.insert("text".to_string(), content);
                    }
                } else {
                    object.remove("content");
                }
            }
            "update_plan" => {
                if let Some(plan) = object.remove("plan") {
                    let normalized = match plan {
                        Value::Object(step) => Value::Array(vec![Value::Object(step)]),
                        other => other,
                    };
                    object.insert("plan".to_string(), normalized);
                } else {
                    let step = object.get("step").cloned();
                    let status = object.get("status").cloned();
                    if let (Some(step), Some(status)) = (step, status) {
                        object.insert(
                            "plan".to_string(),
                            Value::Array(vec![serde_json::json!({
                                "step": step,
                                "status": status,
                            })]),
                        );
                    }
                }
            }
            _ => {}
        },
        FacadeToolKind::Integration => match op {
            "web_search" => {
                if !object.contains_key("query") {
                    if let Some(query) = object.remove("q") {
                        object.insert("query".to_string(), query);
                    }
                }
            }
            "web_fetch" | "webfetch" => {
                if !object.contains_key("url") {
                    if let Some(url) = object.remove("link") {
                        object.insert("url".to_string(), url);
                    }
                }
            }
            "view_image" => {}
            _ => {}
        },
    }

    Value::Object(object)
}

fn facade_error_value(
    facade_tool: &'static str,
    op: Option<String>,
    mapped_action: Option<String>,
    code: &'static str,
    message: impl Into<String>,
) -> Value {
    serde_json::to_value(FacadeErrorResponse {
        facade_tool,
        op: op.clone(),
        mapped_action: mapped_action.clone(),
        error: FacadeErrorBody {
            code,
            message: message.into(),
        },
    })
    .unwrap_or_else(|_| {
        serde_json::json!({
            "facade_tool": facade_tool,
            "op": op,
            "mapped_action": mapped_action,
            "error": {
                "code": code,
                "message": "failed to serialize facade error",
            }
        })
    })
}

fn facade_help_spec(
    kind: FacadeToolKind,
) -> (Vec<FacadeQuickstartExample>, Vec<FacadeAdvancedTopic>) {
    match kind {
        FacadeToolKind::Workspace => (
            vec![
                FacadeQuickstartExample {
                    op: "read",
                    example: serde_json::json!({ "op": "read", "path": "README.md" }),
                },
                FacadeQuickstartExample {
                    op: "grep",
                    example: serde_json::json!({
                        "op": "grep",
                        "query": "TODO",
                        "include_glob": "**/*.rs"
                    }),
                },
            ],
            vec![
                FacadeAdvancedTopic {
                    topic: "read",
                    summary: "Read UTF-8 text from workspace/reference root.",
                    args_schema_hint: serde_json::json!({
                        "path": "string",
                        "root?": "workspace|reference",
                        "max_bytes?": "integer >= 1"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_INVALID_PARAMS,
                        "example": "path must not escape root",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "write",
                    summary: "Write/patch/edit/delete/mkdir file-system operations.",
                    args_schema_hint: serde_json::json!({
                        "write": { "path": "string", "text": "string" },
                        "patch": { "path": "string", "patch": "string" },
                        "edit": { "path": "string", "edits": "array" },
                        "delete": { "path": "string", "recursive?": "bool" },
                        "mkdir": { "path": "string", "recursive?": "bool" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "mode/approval/allowed_tools denies write action",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "repo_search",
                    summary: "Repo-wide search/index/symbol extraction operations.",
                    args_schema_hint: serde_json::json!({
                        "repo_search": { "query": "string", "is_regex?": "bool", "max_matches?": "integer" },
                        "repo_index": { "include_glob?": "string", "max_files?": "integer" },
                        "repo_symbols": { "include_glob?": "string", "max_symbols?": "integer" },
                        "repo_goto_definition": { "symbol": "string", "path?": "string", "include_glob?": "string", "max_results?": "integer" },
                        "repo_find_references": { "symbol": "string", "path?": "string", "include_glob?": "string", "max_matches?": "integer" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "artifact permission denied for repo operations",
                    })],
                },
            ],
        ),
        FacadeToolKind::Process => (
            vec![
                FacadeQuickstartExample {
                    op: "start",
                    example: serde_json::json!({
                        "op": "start",
                        "argv": ["bash", "-lc", "echo hello"]
                    }),
                },
                FacadeQuickstartExample {
                    op: "inspect",
                    example: serde_json::json!({ "op": "inspect", "process_id": "proc_xxx" }),
                },
            ],
            vec![
                FacadeAdvancedTopic {
                    topic: "start",
                    summary: "Start a non-interactive process.",
                    args_schema_hint: serde_json::json!({
                        "argv": "string[]",
                        "cwd?": "string",
                        "timeout_ms?": "integer"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "sandbox/execpolicy/approval denied process/start",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "inspect",
                    summary: "Inspect/tail/follow/kill an existing process.",
                    args_schema_hint: serde_json::json!({
                        "inspect": { "process_id": "string", "max_lines?": "integer" },
                        "tail": { "process_id": "string", "stream": "stdout|stderr", "max_lines?": "integer" },
                        "follow": { "process_id": "string", "stream": "stdout|stderr", "since_offset?": "integer", "max_bytes?": "integer" },
                        "kill": { "process_id": "string", "reason?": "string" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_INVALID_PARAMS,
                        "example": "process_id is required for inspect/tail/follow/kill",
                    })],
                },
            ],
        ),
        FacadeToolKind::Thread => (
            vec![
                FacadeQuickstartExample {
                    op: "diff",
                    example: serde_json::json!({ "op": "diff" }),
                },
                FacadeQuickstartExample {
                    op: "request_input",
                    example: serde_json::json!({
                        "op": "request_input",
                        "questions": [{
                            "header": "Confirm",
                            "id": "confirm",
                            "question": "Proceed?",
                            "options": [
                                { "label": "Yes", "description": "Continue" },
                                { "label": "No", "description": "Stop" }
                            ]
                        }]
                    }),
                },
                FacadeQuickstartExample {
                    op: "send_input",
                    example: serde_json::json!({
                        "op": "send_input",
                        "id": "thread_xxx",
                        "message": "Continue with the next validation step.",
                        "interrupt": false
                    }),
                },
            ],
            vec![
                FacadeAdvancedTopic {
                    topic: "diff",
                    summary: "Read thread diff/state/usage/events.",
                    args_schema_hint: serde_json::json!({
                        "diff": { "max_bytes?": "integer", "wait_seconds?": "integer" },
                        "state": { "thread_id": "string" },
                        "usage": { "thread_id": "string" },
                        "events": { "thread_id": "string", "since_seq?": "integer", "max_events?": "integer" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_INVALID_PARAMS,
                        "example": "thread_id is required for state/usage/events",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "hook_run",
                    summary: "Run configured workspace hook.",
                    args_schema_hint: serde_json::json!({
                        "hook": "session_start|pre_tool_use|post_tool_use|notification|stop"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "mode/approval denied thread/hook_run",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "spawn_agent",
                    summary: "Spawn subagent tasks with dependencies.",
                    args_schema_hint: serde_json::json!({
                        "tasks": "array(required)",
                        "mode?": "string",
                        "workspace_mode?": "read_only|isolated_write",
                        "priority?": "high|normal|low"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "subagent/spawn denied by mode/approval",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "send_input",
                    summary: "Send input to an existing subagent thread and optionally interrupt current turn.",
                    args_schema_hint: serde_json::json!({
                        "id": "string (thread id)",
                        "message": "string (required, non-empty)",
                        "interrupt?": "bool (default false)"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "subagent/send_input denied by allowed_tools/mode/approval",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "wait",
                    summary: "Wait for one or more subagent threads to reach a final status.",
                    args_schema_hint: serde_json::json!({
                        "ids": "string[] (required, non-empty)",
                        "timeout_ms?": "integer, clamped to [10000, 300000]"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_INVALID_PARAMS,
                        "example": "ids must be non-empty",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "close",
                    summary: "Close a subagent by archiving its thread (force=true).",
                    args_schema_hint: serde_json::json!({
                        "id": "string (thread id)",
                        "reason?": "string"
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "subagent/close denied by mode/approval",
                    })],
                },
            ],
        ),
        FacadeToolKind::Artifact => (
            vec![
                FacadeQuickstartExample {
                    op: "write",
                    example: serde_json::json!({
                        "op": "write",
                        "artifact_type": "note",
                        "summary": "short summary",
                        "text": "# Notes"
                    }),
                },
                FacadeQuickstartExample {
                    op: "list",
                    example: serde_json::json!({ "op": "list" }),
                },
            ],
            vec![
                FacadeAdvancedTopic {
                    topic: "write",
                    summary: "Write a thread artifact or update plan artifact.",
                    args_schema_hint: serde_json::json!({
                        "write": { "artifact_type": "string", "summary": "string", "text": "string" },
                        "update_plan": { "explanation?": "string", "plan": "array(required)" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_INVALID_PARAMS,
                        "example": "update_plan requires at most one in_progress step",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "read",
                    summary: "List/read/delete artifacts.",
                    args_schema_hint: serde_json::json!({
                        "list": {},
                        "read": { "artifact_id": "string", "version?": "integer", "max_bytes?": "integer" },
                        "delete": { "artifact_id": "string" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "artifact/delete denied by mode/allowed_tools",
                    })],
                },
            ],
        ),
        FacadeToolKind::Integration => (
            vec![
                FacadeQuickstartExample {
                    op: "web_search",
                    example: serde_json::json!({
                        "op": "web_search",
                        "query": "Rust async tutorial",
                        "max_results": 5
                    }),
                },
                FacadeQuickstartExample {
                    op: "mcp_list_servers",
                    example: serde_json::json!({ "op": "mcp_list_servers" }),
                },
            ],
            vec![
                FacadeAdvancedTopic {
                    topic: "web_search",
                    summary: "Web search/fetch/image tools.",
                    args_schema_hint: serde_json::json!({
                        "web_search": { "query": "string", "max_results?": "integer" },
                        "web_fetch": { "url": "string", "max_bytes?": "integer" },
                        "view_image": { "url": "string", "max_bytes?": "integer" }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "browser mode/approval denied web action",
                    })],
                },
                FacadeAdvancedTopic {
                    topic: "mcp_list_servers",
                    summary: "MCP list/call operations.",
                    args_schema_hint: serde_json::json!({
                        "mcp_list_servers": {},
                        "mcp_list_tools": { "server": "string" },
                        "mcp_list_resources": { "server": "string" },
                        "mcp_call": {
                            "server": "string",
                            "tool": "string",
                            "arguments?": "object(optional)",
                            "<extra root fields>": "allowed; runtime auto-packs them into arguments"
                        }
                    }),
                    error_examples: vec![serde_json::json!({
                        "code": FACADE_ERROR_POLICY_DENIED,
                        "example": "mcp disabled or denied by policy",
                    })],
                },
            ],
        ),
    }
}

fn facade_help_value(kind: FacadeToolKind, topic: Option<String>) -> anyhow::Result<Value> {
    let (quickstart, advanced) = facade_help_spec(kind);
    let topic = topic.and_then(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        (!normalized.is_empty()).then_some(normalized)
    });

    let quickstart = if let Some(topic) = topic.as_deref() {
        quickstart
            .into_iter()
            .filter(|example| example.op == topic)
            .collect::<Vec<_>>()
    } else {
        quickstart
    };
    let advanced = if let Some(topic) = topic.as_deref() {
        advanced
            .into_iter()
            .filter(|entry| entry.topic == topic)
            .collect::<Vec<_>>()
    } else {
        advanced
    };

    if topic.is_some() && advanced.is_empty() {
        return Ok(facade_error_value(
            kind.as_tool_name(),
            Some("help".to_string()),
            None,
            FACADE_ERROR_UNSUPPORTED_OP,
            format!(
                "unsupported help topic for {}: {}",
                kind.as_tool_name(),
                topic.unwrap_or_default()
            ),
        ));
    }

    serde_json::to_value(FacadeHelpResponse {
        facade_tool: kind.as_tool_name(),
        op: "help",
        topic,
        quickstart,
        advanced,
    })
    .context("serialize facade help response")
}

fn facade_route(kind: FacadeToolKind, op: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        FacadeToolKind::Workspace => match op {
            "read" => Some(("file_read", "file/read")),
            "glob" => Some(("file_glob", "file/glob")),
            "grep" => Some(("file_grep", "file/grep")),
            "repo_search" | "search" => Some(("repo_search", "repo/search")),
            "repo_index" | "index" => Some(("repo_index", "repo/index")),
            "repo_symbols" | "symbols" => Some(("repo_symbols", "repo/symbols")),
            "repo_goto_definition" | "goto_definition" => {
                Some(("repo_goto_definition", "repo/goto_definition"))
            }
            "repo_find_references" | "find_references" => {
                Some(("repo_find_references", "repo/find_references"))
            }
            "write" => Some(("file_write", "file/write")),
            "patch" => Some(("file_patch", "file/patch")),
            "edit" => Some(("file_edit", "file/edit")),
            "delete" => Some(("file_delete", "file/delete")),
            "mkdir" => Some(("fs_mkdir", "fs/mkdir")),
            _ => None,
        },
        FacadeToolKind::Process => match op {
            "start" => Some(("process_start", "process/start")),
            "inspect" => Some(("process_inspect", "process/inspect")),
            "tail" => Some(("process_tail", "process/tail")),
            "follow" => Some(("process_follow", "process/follow")),
            "kill" => Some(("process_kill", "process/kill")),
            _ => None,
        },
        FacadeToolKind::Thread => match op {
            "diff" => Some(("thread_diff", "thread/diff")),
            "state" => Some(("thread_state", "thread/state")),
            "usage" => Some(("thread_usage", "thread/usage")),
            "events" => Some(("thread_events", "thread/events")),
            "hook_run" => Some(("thread_hook_run", "thread/hook_run")),
            "request_input" | "request_user_input" => {
                Some(("request_user_input", "thread/request_user_input"))
            }
            "spawn_agent" | "agent_spawn" => Some(("agent_spawn", "subagent/spawn")),
            "send_input" => Some(("subagent_send_input", "subagent/send_input")),
            "wait" => Some(("subagent_wait", "subagent/wait")),
            "close" | "close_agent" => Some(("subagent_close", "subagent/close")),
            _ => None,
        },
        FacadeToolKind::Artifact => match op {
            "write" => Some(("artifact_write", "artifact/write")),
            "update_plan" => Some(("update_plan", "artifact/write")),
            "list" => Some(("artifact_list", "artifact/list")),
            "read" => Some(("artifact_read", "artifact/read")),
            "delete" => Some(("artifact_delete", "artifact/delete")),
            _ => None,
        },
        FacadeToolKind::Integration => match op {
            "mcp_list_servers" => Some(("mcp_list_servers", "mcp/list_servers")),
            "mcp_list_tools" => Some(("mcp_list_tools", "mcp/list_tools")),
            "mcp_list_resources" => Some(("mcp_list_resources", "mcp/list_resources")),
            "mcp_call" => Some(("mcp_call", "mcp/call")),
            "web_search" => Some(("web_search", "web/search")),
            "web_fetch" | "webfetch" => Some(("webfetch", "web/fetch")),
            "view_image" => Some(("view_image", "web/view_image")),
            _ => None,
        },
    }
}

fn facade_annotate_result(
    result: Value,
    facade_tool: &'static str,
    op: &str,
    mapped_action: &str,
) -> Value {
    let mut object = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("output".to_string(), other);
            map
        }
    };
    object.insert(
        "facade_tool".to_string(),
        Value::String(facade_tool.to_string()),
    );
    object.insert("op".to_string(), Value::String(op.to_string()));
    object.insert(
        "mapped_action".to_string(),
        Value::String(mapped_action.to_string()),
    );
    let value = Value::Object(object);
    if value
        .get("denied")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return ensure_machine_error_fields(value, FACADE_ERROR_POLICY_DENIED);
    }
    value
}

const DYNAMIC_ERROR_INVALID_PARAMS: &str = "dynamic_invalid_params";

fn ensure_machine_error_fields(value: Value, error_code: &'static str) -> Value {
    let mut object = match value {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("output".to_string(), other);
            map
        }
    };
    if !object.contains_key("error_code") {
        object.insert(
            "error_code".to_string(),
            Value::String(error_code.to_string()),
        );
    }
    if !object.contains_key("structured_error")
        && let Ok(structured_error) = crate::catalog_structured_error(error_code)
        && let Ok(serialized) = serde_json::to_value(structured_error)
    {
        object.insert("structured_error".to_string(), serialized);
    }
    Value::Object(object)
}

fn dynamic_error_value(
    dynamic_tool: &str,
    mapped_tool: &str,
    mapped_action: &str,
    error_code: &'static str,
    message: impl Into<String>,
) -> Value {
    ensure_machine_error_fields(
        serde_json::json!({
            "dynamic_tool": dynamic_tool,
            "mapped_tool": mapped_tool,
            "mapped_action": mapped_action,
            "error_code": error_code,
            "message": message.into(),
        }),
        error_code,
    )
}

fn dynamic_annotate_result(result: Value, spec: &DynamicToolSpec) -> Value {
    let mut object = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("output".to_string(), other);
            map
        }
    };
    object.insert("dynamic_tool".to_string(), Value::String(spec.name.clone()));
    object.insert(
        "mapped_tool".to_string(),
        Value::String(spec.mapped_tool.clone()),
    );
    object.insert(
        "mapped_action".to_string(),
        Value::String(spec.mapped_action.clone()),
    );
    Value::Object(object)
}

fn merge_dynamic_args(user_args: Value, fixed_args: &Value) -> anyhow::Result<Value> {
    let mut merged = match user_args {
        Value::Object(map) => map,
        _ => anyhow::bail!("dynamic tool args must be a JSON object"),
    };
    let fixed = fixed_args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("dynamic tool fixed_args must be a JSON object"))?;
    for (key, value) in fixed {
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}

async fn append_dynamic_audit_events(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    spec: &DynamicToolSpec,
    mapped_args: &Value,
    mapped_call: std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Value>> + '_>>,
) -> anyhow::Result<Value> {
    let (thread_rt, _) = super::load_thread_root(server, thread_id).await?;
    let tool_id = omne_protocol::ToolId::new();
    let dynamic_action = format!("dynamic/{}", spec.name);

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: dynamic_action,
            params: Some(serde_json::json!({
                "dynamic_tool": spec.name,
                "mapped_tool": spec.mapped_tool,
                "mapped_action": spec.mapped_action,
                "args": mapped_args,
            })),
        })
        .await?;

    match mapped_call.await {
        Ok(result) => {
            let annotated = dynamic_annotate_result(result, spec);
            let status = if annotated
                .get("denied")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                omne_protocol::ToolStatus::Denied
            } else {
                omne_protocol::ToolStatus::Completed
            };
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status,
                    structured_error: crate::structured_error_from_result_value(&annotated),
                    error: None,
                    result: Some(annotated.clone()),
                })
                .await?;
            Ok(annotated)
        }
        Err(err) => {
            let result = ensure_machine_error_fields(
                serde_json::json!({
                    "dynamic_tool": spec.name,
                    "mapped_tool": spec.mapped_tool,
                    "mapped_action": spec.mapped_action,
                    "error_code": "dynamic_dispatch_failed",
                }),
                "dynamic_dispatch_failed",
            );
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: crate::structured_error_from_result_value(&result),
                    error: Some(err.to_string()),
                    result: Some(result),
                })
                .await?;
            Err(err)
        }
    }
}

async fn append_facade_audit_events(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    facade_tool: &'static str,
    op: &str,
    mapped_action: &str,
    mapped_args: &Value,
    mapped_call: std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Value>> + '_>>,
) -> anyhow::Result<Value> {
    let (thread_rt, _) = super::load_thread_root(server, thread_id).await?;
    let facade_tool_id = omne_protocol::ToolId::new();
    let facade_tool_action = format!("facade/{facade_tool}");
    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id: facade_tool_id,
            turn_id,
            tool: facade_tool_action.clone(),
            params: Some(serde_json::json!({
                "facade_tool": facade_tool,
                "op": op,
                "mapped_action": mapped_action,
                "args": mapped_args,
            })),
        })
        .await?;

    match mapped_call.await {
        Ok(result) => {
            let annotated = facade_annotate_result(result, facade_tool, op, mapped_action);
            let status = if annotated
                .get("denied")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                omne_protocol::ToolStatus::Denied
            } else {
                omne_protocol::ToolStatus::Completed
            };
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id: facade_tool_id,
                    status,
                    structured_error: crate::structured_error_from_result_value(&annotated),
                    error: None,
                    result: Some(annotated.clone()),
                })
                .await?;
            Ok(annotated)
        }
        Err(err) => {
            let result = ensure_machine_error_fields(
                serde_json::json!({
                    "facade_tool": facade_tool,
                    "op": op,
                    "mapped_action": mapped_action,
                    "error_code": FACADE_ERROR_POLICY_DENIED,
                }),
                FACADE_ERROR_POLICY_DENIED,
            );
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id: facade_tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: crate::structured_error_from_result_value(&result),
                    error: Some(err.to_string()),
                    result: Some(result),
                })
                .await?;
            Err(err)
        }
    }
}

async fn dispatch_facade_tool_call(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    facade_kind: FacadeToolKind,
    raw_args: Value,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<Value> {
    let facade_tool = facade_kind.as_tool_name();
    let request: FacadeToolArgs = match serde_json::from_value(raw_args) {
        Ok(value) => value,
        Err(err) => {
            return Ok(facade_error_value(
                facade_tool,
                None,
                None,
                FACADE_ERROR_INVALID_PARAMS,
                format!("invalid facade request: {err}"),
            ));
        }
    };

    let Some(op) = facade_normalized_op(&request) else {
        return Ok(facade_error_value(
            facade_tool,
            None,
            None,
            FACADE_ERROR_INVALID_PARAMS,
            "missing required field: op",
        ));
    };

    if op == "help" {
        return facade_help_value(facade_kind, request.topic);
    }

    let Some((mapped_tool, mapped_action)) = facade_route(facade_kind, &op) else {
        return Ok(facade_error_value(
            facade_tool,
            Some(op),
            None,
            FACADE_ERROR_UNSUPPORTED_OP,
            format!("unsupported op for {facade_tool}"),
        ));
    };

    let mapped_args = match facade_collect_mapped_args(&request) {
        Ok(args) => args,
        Err(err) => {
            return Ok(facade_error_value(
                facade_tool,
                Some(op),
                Some(mapped_action.to_string()),
                FACADE_ERROR_INVALID_PARAMS,
                err.to_string(),
            ));
        }
    };
    if !mapped_args.is_object() {
        return Ok(facade_error_value(
            facade_tool,
            Some(op),
            Some(mapped_action.to_string()),
            FACADE_ERROR_INVALID_PARAMS,
            "facade parameters must be a JSON object with root-level fields",
        ));
    }
    let mapped_args = facade_normalize_mapped_args(facade_kind, &op, mapped_args);
    let mapped_args_for_call = mapped_args.clone();

    append_facade_audit_events(
        server,
        thread_id,
        turn_id,
        facade_tool,
        &op,
        mapped_action,
        &mapped_args,
        Box::pin(run_tool_call_once(
            server,
            thread_id,
            turn_id,
            mapped_tool,
            mapped_args_for_call,
            approval_id,
        )),
    )
    .await
}

async fn dispatch_dynamic_tool_call(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    tool_name: &str,
    raw_args: Value,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<Option<Value>> {
    let (_, thread_root) = super::load_thread_root(server, thread_id).await?;
    let Some(spec) = find_dynamic_tool_spec(Some(&thread_root), tool_name) else {
        return Ok(None);
    };

    let mapped_args = match merge_dynamic_args(raw_args, &spec.fixed_args) {
        Ok(args) => args,
        Err(err) => {
            return Ok(Some(dynamic_error_value(
                &spec.name,
                &spec.mapped_tool,
                &spec.mapped_action,
                DYNAMIC_ERROR_INVALID_PARAMS,
                err.to_string(),
            )));
        }
    };

    let mapped_args_for_call = mapped_args.clone();
    let mapped_tool = spec.mapped_tool.clone();
    let mapped_action = spec.mapped_action.clone();
    let spec_for_audit = spec.clone();
    let result = append_dynamic_audit_events(
        server,
        thread_id,
        turn_id,
        &spec_for_audit,
        &mapped_args,
        Box::pin(async move {
            run_tool_call_once(
                server,
                thread_id,
                turn_id,
                mapped_tool.as_str(),
                mapped_args_for_call,
                approval_id,
            )
            .await
            .map(|value| {
                if value.get("error_code").and_then(Value::as_str).is_none()
                    && value
                        .get("denied")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                {
                    ensure_machine_error_fields(value, "dynamic_policy_denied")
                } else {
                    value
                }
            })
        }),
    )
    .await?;

    let mut out = match result {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("output".to_string(), other);
            map
        }
    };
    out.insert("mapped_action".to_string(), Value::String(mapped_action));
    Ok(Some(Value::Object(out)))
}

fn parse_mcp_call_args_with_flatten_compat(raw_args: Value) -> anyhow::Result<McpCallArgs> {
    let strict_err = match serde_json::from_value::<McpCallArgs>(raw_args.clone()) {
        Ok(parsed) => {
            let has_extra_fields = raw_args
                .as_object()
                .map(|object| {
                    object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "server" | "tool" | "arguments"))
                })
                .unwrap_or(false);
            if !has_extra_fields {
                return Ok(parsed);
            }
            // Continue with compatibility merge so extra top-level fields are folded into arguments.
            anyhow::Error::msg("compatibility merge required")
        }
        Err(err) => err.into(),
    };
    let strict_err_text = strict_err.to_string();
    let strict_err = || anyhow::anyhow!("invalid mcp_call args: {strict_err_text}");

    let Value::Object(mut object) = raw_args else {
        return Err(strict_err());
    };

    let server = match object.remove("server") {
        Some(Value::String(value)) if !value.trim().is_empty() => value,
        _ => return Err(strict_err()),
    };
    let tool = match object.remove("tool") {
        Some(Value::String(value)) if !value.trim().is_empty() => value,
        _ => return Err(strict_err()),
    };

    let mut arguments = match object.remove("arguments") {
        Some(Value::Object(map)) => map,
        Some(Value::String(raw)) => {
            // Be tolerant when the model serializes arguments as a JSON string.
            let parsed = serde_json::from_str::<Value>(&raw).ok();
            match parsed {
                Some(Value::Object(map)) => map,
                _ => return Err(strict_err()),
            }
        }
        Some(_) => return Err(strict_err()),
        None => serde_json::Map::new(),
    };

    // Common wrapper keys produced by models.
    for wrapper in ["input", "params", "payload", "args"] {
        match object.remove(wrapper) {
            Some(Value::Object(map)) => {
                for (key, value) in map {
                    arguments.entry(key).or_insert(value);
                }
            }
            Some(value) => {
                arguments.entry(wrapper.to_string()).or_insert(value);
            }
            None => {}
        }
    }

    // Compatibility fallback: fold remaining top-level fields into MCP arguments.
    for (key, value) in object {
        arguments.entry(key).or_insert(value);
    }

    Ok(McpCallArgs {
        server,
        tool,
        arguments,
    })
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
        "workspace" | "process" | "thread" | "artifact" | "integration" => {
            let facade_kind = FacadeToolKind::from_tool_name(tool_name)
                .ok_or_else(|| anyhow::anyhow!("unknown facade tool: {tool_name}"))?;
            dispatch_facade_tool_call(server, thread_id, turn_id, facade_kind, args, approval_id)
                .await
        }
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
        "repo_goto_definition" => {
            let args: RepoGotoDefinitionArgs = serde_json::from_value(args)?;
            super::handle_repo_goto_definition(
                server,
                super::RepoGotoDefinitionParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    symbol: args.symbol,
                    path: args.path,
                    include_glob: args.include_glob,
                    max_results: args.max_results,
                    max_files: args.max_files,
                    max_bytes_per_file: args.max_bytes_per_file,
                    max_symbols: args.max_symbols,
                },
            )
            .await
        }
        "repo_find_references" => {
            let args: RepoFindReferencesArgs = serde_json::from_value(args)?;
            super::handle_repo_find_references(
                server,
                super::RepoFindReferencesParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    root: args.root,
                    symbol: args.symbol,
                    path: args.path,
                    include_glob: args.include_glob,
                    max_matches: args.max_matches,
                    max_bytes_per_file: args.max_bytes_per_file,
                    max_files: args.max_files,
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
            let args = parse_mcp_call_args_with_flatten_compat(args)?;
            super::handle_mcp_call(
                server,
                super::McpCallParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    server: args.server,
                    tool: args.tool,
                    arguments: Some(serde_json::Value::Object(args.arguments)),
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
        "update_plan" => {
            let args: UpdatePlanArgs = serde_json::from_value(args)?;
            let (explanation, plan) = normalize_update_plan_steps(args)?;
            let summary = summarize_update_plan_artifact(explanation.as_deref(), &plan);
            let text = render_update_plan_artifact_text(explanation.as_deref(), &plan);
            super::handle_artifact_write(
                server,
                super::ArtifactWriteParams {
                    thread_id,
                    turn_id,
                    approval_id,
                    artifact_id: None,
                    artifact_type: "plan".to_string(),
                    summary,
                    text,
                },
            )
            .await
        }
        "request_user_input" => {
            let args: RequestUserInputArgs = serde_json::from_value(args)?;
            handle_request_user_input_tool(server, thread_id, turn_id, approval_id, args).await
        }
        "web_search" => {
            let args: WebSearchArgs = serde_json::from_value(args)?;
            handle_web_search_tool(server, thread_id, turn_id, approval_id, args).await
        }
        "webfetch" => {
            let args: WebFetchArgs = serde_json::from_value(args)?;
            handle_webfetch_tool(server, thread_id, turn_id, approval_id, args).await
        }
        "view_image" => {
            let args: ViewImageArgs = serde_json::from_value(args)?;
            handle_view_image_tool(server, thread_id, turn_id, approval_id, args).await
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
        "subagent_send_input" => {
            let args: SubagentSendInputArgs = serde_json::from_value(args)?;
            handle_subagent_send_input_tool(server, thread_id, turn_id, approval_id, args).await
        }
        "subagent_wait" => {
            let args: SubagentWaitArgs = serde_json::from_value(args)?;
            handle_subagent_wait_tool(server, thread_id, turn_id, approval_id, args).await
        }
        "subagent_close" => {
            let args: SubagentCloseArgs = serde_json::from_value(args)?;
            handle_subagent_close_tool(server, thread_id, turn_id, approval_id, args).await
        }
        _ => {
            if let Some(output) =
                dispatch_dynamic_tool_call(server, thread_id, turn_id, tool_name, args, approval_id)
                    .await?
            {
                Ok(output)
            } else {
                anyhow::bail!("unknown tool: {tool_name}")
            }
        }
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
    let thread_cwd =
        thread_cwd.ok_or_else(|| anyhow::anyhow!("thread cwd is missing: {thread_id}"))?;

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let Some(mode) = catalog.mode(&mode_name) else {
        let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
        let decision = omne_core::modes::Decision::Deny;
        let denied = ensure_machine_error_fields(
            serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": decision,
                "available": available,
                "load_error": catalog.load_error,
            }),
            "mode_unknown",
        );
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
                structured_error: crate::structured_error_from_result_value(&denied),
                error: Some("unknown mode".to_string()),
                result: Some(denied.clone()),
            })
            .await?;
        return Ok(denied);
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
        let denied = ensure_machine_error_fields(
            serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision_source": decision_source,
                "tool_override_hit": tool_override_hit,
                "decision": effective_decision,
            }),
            "mode_denied",
        );
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
                structured_error: crate::structured_error_from_result_value(&denied),
                error: Some("mode denies subagent/spawn".to_string()),
                result: Some(denied.clone()),
            })
            .await?;
        return Ok(denied);
    }

    let env_max_concurrent_subagents = parse_env_usize("OMNE_MAX_CONCURRENT_SUBAGENTS", 4, 0, 64);
    let mode_max_concurrent_subagents = mode.permissions.subagent.spawn.max_threads;
    let limit =
        combine_subagent_spawn_limits(env_max_concurrent_subagents, mode_max_concurrent_subagents);
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
            let denied = ensure_machine_error_fields(
                serde_json::json!({
                    "tool_id": tool_id,
                    "denied": true,
                    "mode": mode_name,
                    "decision_source": decision_source,
                    "tool_override_hit": tool_override_hit,
                    "decision": effective_decision,
                    "requested_modes": requested_modes,
                    "allowed_modes": allowed,
                    "priority_aging_rounds": priority_aging_rounds,
                    "limit_policy": "min_non_zero",
                    "limit_source": limit.source.as_str(),
                    "env_max_concurrent_subagents": env_max_concurrent_subagents,
                    "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
                    "max_concurrent_subagents": max_concurrent_subagents,
                }),
                "subagent_spawn_allowed_modes_denied",
            );
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
                    structured_error: crate::structured_error_from_result_value(&denied),
                    error: Some("mode forbids spawning this subagent mode".to_string()),
                    result: Some(denied.clone()),
                })
                .await?;
            return Ok(denied);
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
        let denied = ensure_machine_error_fields(
            serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "limit_policy": "min_non_zero",
                "limit_source": limit.source.as_str(),
                "priority_aging_rounds": priority_aging_rounds,
                "env_max_concurrent_subagents": env_max_concurrent_subagents,
                "mode_max_concurrent_subagents": mode_max_concurrent_subagents,
                "max_concurrent_subagents": max_concurrent_subagents,
                "active": active,
                "active_threads": active_thread_ids,
            }),
            "subagent_spawn_limit_reached",
        );

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
                structured_error: crate::structured_error_from_result_value(&denied),
                error: Some(format!(
                    "max_concurrent_subagents limit reached: active={active}, max={max_concurrent_subagents}"
                )),
                result: Some(denied.clone()),
            })
            .await?;
        return Ok(denied);
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
                let denied = ensure_machine_error_fields(
                    serde_json::json!({
                        "tool_id": tool_id,
                        "denied": true,
                        "remembered": remembered,
                        "approval_policy": approval_policy,
                    }),
                    "approval_denied",
                );
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
                        structured_error: crate::structured_error_from_result_value(&denied),
                        error: Some(super::approval_denied_error(remembered).to_string()),
                        result: Some(denied.clone()),
                    })
                    .await?;
                return Ok(denied);
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
                    structured_error: None,
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
                    structured_error: None,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

#[derive(Clone, Copy)]
enum SubagentLifecycleAction {
    SendInput,
    Wait,
    Close,
}

impl SubagentLifecycleAction {
    fn action_name(self) -> &'static str {
        match self {
            Self::SendInput => "subagent/send_input",
            Self::Wait => "subagent/wait",
            Self::Close => "subagent/close",
        }
    }

    fn base_mode_decision(self, mode: &omne_core::modes::ModeDef) -> omne_core::modes::Decision {
        match self {
            Self::SendInput => mode.permissions.subagent.spawn.decision,
            Self::Wait => mode.permissions.read,
            Self::Close => mode
                .permissions
                .subagent
                .spawn
                .decision
                .combine(mode.permissions.process.kill),
        }
    }
}

enum SubagentLifecycleGateOutcome {
    Allowed {
        tool_id: omne_protocol::ToolId,
        thread_rt: std::sync::Arc<crate::ThreadRuntime>,
    },
    Return(Value),
}

async fn gate_subagent_lifecycle_action(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    action: SubagentLifecycleAction,
    approval_params: &Value,
) -> anyhow::Result<SubagentLifecycleGateOutcome> {
    let action_name = action.action_name();
    let tool_id = omne_protocol::ToolId::new();
    let (thread_rt, thread_root) = super::load_thread_root(server, thread_id).await?;
    let (approval_policy, mode_name, allowed_tools) = {
        let handle = thread_rt.handle.lock().await;
        let state = handle.state();
        (
            state.approval_policy,
            state.mode.clone(),
            state.allowed_tools.clone(),
        )
    };

    if let Some(denied) = crate::enforce_thread_allowed_tools(
        &thread_rt,
        tool_id,
        turn_id,
        action_name,
        Some(approval_params.clone()),
        &allowed_tools,
    )
    .await?
    {
        return Ok(SubagentLifecycleGateOutcome::Return(
            ensure_machine_error_fields(denied, "allowed_tools_denied"),
        ));
    }

    let catalog = omne_core::modes::ModeCatalog::load(&thread_root).await;
    let Some(mode) = catalog.mode(&mode_name) else {
        let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
        let denied = ensure_machine_error_fields(
            serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": "deny",
                "available": available,
                "load_error": catalog.load_error,
                "error_code": "mode_unknown",
            }),
            "mode_unknown",
        );
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: action_name.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                structured_error: crate::structured_error_from_result_value(&denied),
                error: Some("unknown mode".to_string()),
                result: Some(denied.clone()),
            })
            .await?;
        return Ok(SubagentLifecycleGateOutcome::Return(denied));
    };

    let mode_decision =
        crate::resolve_mode_decision_audit(mode, action_name, action.base_mode_decision(mode));
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let denied = ensure_machine_error_fields(
            serde_json::json!({
                "tool_id": tool_id,
                "denied": true,
                "mode": mode_name,
                "decision": mode_decision.decision,
                "decision_source": mode_decision.decision_source,
                "tool_override_hit": mode_decision.tool_override_hit,
                "error_code": "mode_denied",
            }),
            "mode_denied",
        );
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                tool_id,
                turn_id,
                tool: action_name.to_string(),
                params: Some(approval_params.clone()),
            })
            .await?;
        thread_rt
            .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                tool_id,
                status: omne_protocol::ToolStatus::Denied,
                structured_error: crate::structured_error_from_result_value(&denied),
                error: Some(format!("mode denies {action_name}")),
                result: Some(denied.clone()),
            })
            .await?;
        return Ok(SubagentLifecycleGateOutcome::Return(denied));
    }

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match super::gate_approval(
            server,
            &thread_rt,
            thread_id,
            turn_id,
            approval_policy,
            super::ApprovalRequest {
                approval_id,
                action: action_name,
                params: approval_params,
            },
        )
        .await?
        {
            super::ApprovalGate::Approved => {}
            super::ApprovalGate::Denied { remembered } => {
                let denied = ensure_machine_error_fields(
                    serde_json::json!({
                        "tool_id": tool_id,
                        "denied": true,
                        "remembered": remembered,
                        "approval_policy": approval_policy,
                        "error_code": "approval_denied",
                    }),
                    "approval_denied",
                );
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolStarted {
                        tool_id,
                        turn_id,
                        tool: action_name.to_string(),
                        params: Some(approval_params.clone()),
                    })
                    .await?;
                thread_rt
                    .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                        tool_id,
                        status: omne_protocol::ToolStatus::Denied,
                        structured_error: crate::structured_error_from_result_value(&denied),
                        error: Some(super::approval_denied_error(remembered).to_string()),
                        result: Some(denied.clone()),
                    })
                    .await?;
                return Ok(SubagentLifecycleGateOutcome::Return(denied));
            }
            super::ApprovalGate::NeedsApproval { approval_id } => {
                return Ok(SubagentLifecycleGateOutcome::Return(serde_json::json!({
                    "needs_approval": true,
                    "approval_id": approval_id,
                })));
            }
        }
    }

    thread_rt
        .append_event(omne_protocol::ThreadEventKind::ToolStarted {
            tool_id,
            turn_id,
            tool: action_name.to_string(),
            params: Some(approval_params.clone()),
        })
        .await?;

    Ok(SubagentLifecycleGateOutcome::Allowed { tool_id, thread_rt })
}

async fn handle_subagent_send_input_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: SubagentSendInputArgs,
) -> anyhow::Result<Value> {
    let prompt = args.message.trim().to_string();
    if prompt.is_empty() {
        anyhow::bail!("message must not be empty");
    }

    let approval_params = serde_json::json!({
        "id": args.id.clone(),
        "message": prompt.clone(),
        "interrupt": args.interrupt,
    });
    let gate = gate_subagent_lifecycle_action(
        server,
        thread_id,
        turn_id,
        approval_id,
        SubagentLifecycleAction::SendInput,
        &approval_params,
    )
    .await?;
    let (tool_id, thread_rt) = match gate {
        SubagentLifecycleGateOutcome::Allowed { tool_id, thread_rt } => (tool_id, thread_rt),
        SubagentLifecycleGateOutcome::Return(value) => return Ok(value),
    };

    let outcome: anyhow::Result<Value> = async {
        let child_thread_id: ThreadId = args.id.parse()?;
        let child_rt = server.get_or_load_thread(child_thread_id).await?;

        let mut interrupted_turn_id: Option<TurnId> = None;
        if args.interrupt {
            let active_turn_id = {
                let handle = child_rt.handle.lock().await;
                handle.state().active_turn_id
            };
            if let Some(active_turn_id) = active_turn_id {
                let reason = Some(format!(
                    "subagent/send_input interrupted by parent thread {thread_id}"
                ));
                child_rt
                    .interrupt_turn(active_turn_id, reason.clone())
                    .await?;
                crate::interrupt_processes_for_turn(
                    server,
                    child_thread_id,
                    active_turn_id,
                    reason.clone(),
                )
                .await;
                let server_arc = std::sync::Arc::new(server.clone());
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    crate::kill_processes_for_turn(
                        &server_arc,
                        child_thread_id,
                        active_turn_id,
                        reason,
                    )
                    .await;
                });
                interrupted_turn_id = Some(active_turn_id);
            }
        }

        let child_turn_id = child_rt
            .start_turn(
                std::sync::Arc::new(server.clone()),
                prompt.clone(),
                None,
                None,
                None,
                omne_protocol::TurnPriority::Background,
            )
            .await?;

        Ok(serde_json::json!({
            "tool_id": tool_id,
            "thread_id": child_thread_id,
            "turn_id": child_turn_id,
            "interrupted_turn_id": interrupted_turn_id,
        }))
    }
    .await;

    match outcome {
        Ok(result) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(result.clone()),
                })
                .await?;
            Ok(result)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: None,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_subagent_wait_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: SubagentWaitArgs,
) -> anyhow::Result<Value> {
    if args.ids.is_empty() {
        anyhow::bail!("ids must be non-empty");
    }
    let timeout_ms = args
        .timeout_ms
        .unwrap_or(DEFAULT_SUBAGENT_WAIT_TIMEOUT_MS)
        .clamp(MIN_SUBAGENT_WAIT_TIMEOUT_MS, MAX_SUBAGENT_WAIT_TIMEOUT_MS);
    let approval_params = serde_json::json!({
        "ids": args.ids.clone(),
        "timeout_ms": timeout_ms,
    });
    let gate = gate_subagent_lifecycle_action(
        server,
        thread_id,
        turn_id,
        approval_id,
        SubagentLifecycleAction::Wait,
        &approval_params,
    )
    .await?;
    let (tool_id, thread_rt) = match gate {
        SubagentLifecycleGateOutcome::Allowed { tool_id, thread_rt } => (tool_id, thread_rt),
        SubagentLifecycleGateOutcome::Return(value) => return Ok(value),
    };

    let outcome: anyhow::Result<Value> = async {
        let child_thread_ids = args
            .ids
            .iter()
            .map(|id| id.parse::<ThreadId>())
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        let mut statuses = serde_json::Map::<String, Value>::new();
        let mut timed_out = false;

        loop {
            statuses.clear();
            let mut any_final = false;

            for child_thread_id in &child_thread_ids {
                let entry = match server.thread_store.read_state(*child_thread_id).await? {
                    Some(state) => {
                        let (state_name, final_state) = if state.archived {
                            ("closed", true)
                        } else if state.active_turn_id.is_some() {
                            ("running", false)
                        } else if let Some(status) = state.last_turn_status {
                            let label = match status {
                                omne_protocol::TurnStatus::Completed => "completed",
                                omne_protocol::TurnStatus::Interrupted => "interrupted",
                                omne_protocol::TurnStatus::Failed => "failed",
                                omne_protocol::TurnStatus::Cancelled => "cancelled",
                                omne_protocol::TurnStatus::Stuck => "stuck",
                            };
                            (label, true)
                        } else if state.paused {
                            ("paused", false)
                        } else {
                            ("idle", false)
                        };
                        if final_state {
                            any_final = true;
                        }
                        serde_json::json!({
                            "state": state_name,
                            "active_turn_id": state.active_turn_id,
                            "last_turn_id": state.last_turn_id,
                            "last_turn_status": state.last_turn_status,
                            "archived": state.archived,
                            "paused": state.paused,
                        })
                    }
                    None => {
                        any_final = true;
                        serde_json::json!({
                            "state": "not_found",
                        })
                    }
                };
                statuses.insert(child_thread_id.to_string(), entry);
            }

            if any_final {
                break;
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                timed_out = true;
                break;
            }
            let remaining = deadline.saturating_duration_since(now);
            tokio::time::sleep(std::cmp::min(
                remaining,
                std::time::Duration::from_millis(100),
            ))
            .await;
        }

        Ok(serde_json::json!({
            "tool_id": tool_id,
            "status": statuses,
            "timed_out": timed_out,
            "timeout_ms": timeout_ms,
        }))
    }
    .await;

    match outcome {
        Ok(result) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: None,
                    error: None,
                    result: Some(result.clone()),
                })
                .await?;
            Ok(result)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: None,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_subagent_close_tool(
    server: &super::Server,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<ApprovalId>,
    args: SubagentCloseArgs,
) -> anyhow::Result<Value> {
    let approval_params = serde_json::json!({
        "id": args.id.clone(),
        "reason": args.reason.clone(),
        "force": true,
    });
    let gate = gate_subagent_lifecycle_action(
        server,
        thread_id,
        turn_id,
        approval_id,
        SubagentLifecycleAction::Close,
        &approval_params,
    )
    .await?;
    let (tool_id, thread_rt) = match gate {
        SubagentLifecycleGateOutcome::Allowed { tool_id, thread_rt } => (tool_id, thread_rt),
        SubagentLifecycleGateOutcome::Return(value) => return Ok(value),
    };

    let outcome: anyhow::Result<Value> = async {
        let child_thread_id: ThreadId = args.id.parse()?;
        let reason = args.reason.clone().or_else(|| {
            Some(format!(
                "subagent close requested by parent thread {thread_id}"
            ))
        });
        let response = crate::handle_thread_archive(
            &std::sync::Arc::new(server.clone()),
            crate::ThreadArchiveParams {
                thread_id: child_thread_id,
                force: true,
                reason,
            },
        )
        .await?;

        let mut result =
            serde_json::to_value(response).context("serialize thread/archive response")?;
        if let Value::Object(map) = &mut result {
            map.insert("tool_id".to_string(), Value::String(tool_id.to_string()));
        }
        Ok(result)
    }
    .await;

    match outcome {
        Ok(result) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Completed,
                    structured_error: crate::structured_error_from_result_value(&result),
                    error: None,
                    result: Some(result.clone()),
                })
                .await?;
            Ok(result)
        }
        Err(err) => {
            thread_rt
                .append_event(omne_protocol::ThreadEventKind::ToolCompleted {
                    tool_id,
                    status: omne_protocol::ToolStatus::Failed,
                    structured_error: None,
                    error: Some(err.to_string()),
                    result: None,
                })
                .await?;
            Err(err)
        }
    }
}

async fn handle_thread_state_tool(
    server: &super::Server,
    args: ThreadStateArgs,
) -> anyhow::Result<Value> {
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

async fn handle_thread_usage_tool(
    server: &super::Server,
    args: ThreadUsageArgs,
) -> anyhow::Result<Value> {
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

async fn handle_thread_events_tool(
    server: &super::Server,
    args: ThreadEventsArgs,
) -> anyhow::Result<Value> {
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

#[cfg(test)]
mod web_tool_boundary_tests {
    use super::*;

    #[test]
    fn validate_http_url_rejects_unsupported_scheme() {
        let err = validate_http_url("file:///tmp/image.png").expect_err("unsupported scheme");
        assert!(err.to_string().contains("unsupported url scheme"));
    }

    #[test]
    fn block_local_host_names() {
        assert!(is_blocked_web_host_name("localhost"));
        assert!(is_blocked_web_host_name("LOCALHOST"));
        assert!(is_blocked_web_host_name("dev.localhost"));
        assert!(!is_blocked_web_host_name("example.com"));
    }

    #[test]
    fn block_local_ip_ranges() {
        assert!(is_blocked_web_ip("127.0.0.1".parse().expect("ipv4")));
        assert!(is_blocked_web_ip("10.0.0.8".parse().expect("ipv4")));
        assert!(is_blocked_web_ip("::1".parse().expect("ipv6")));
        assert!(!is_blocked_web_ip("8.8.8.8".parse().expect("ipv4")));
    }

    #[tokio::test]
    async fn reject_loopback_literal_urls() {
        let url = reqwest::Url::parse("http://127.0.0.1/test").expect("url");
        let err = validate_web_outbound_url(&url)
            .await
            .expect_err("loopback should be denied");
        assert!(err.to_string().contains("blocked local address"));
    }

    #[tokio::test]
    async fn reject_localhost_urls_without_dns_lookup() {
        let url = reqwest::Url::parse("http://localhost/test").expect("url");
        let err = validate_web_outbound_url(&url)
            .await
            .expect_err("localhost should be denied");
        assert!(err.to_string().contains("blocked local host"));
    }

    #[test]
    fn resolve_web_redirect_url_rejects_non_http_scheme() {
        let base = reqwest::Url::parse("https://example.com/a").expect("base");
        let err = resolve_web_redirect_url(&base, "file:///etc/passwd")
            .expect_err("file redirects must fail");
        assert!(err.to_string().contains("unsupported redirect url scheme"));
    }
}
