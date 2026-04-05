fn stream_label(stream: ProcessStream) -> &'static str {
    match stream {
        ProcessStream::Stdout => "stdout",
        ProcessStream::Stderr => "stderr",
    }
}

async fn resolve_process_info(
    server: &Server,
    process_id: ProcessId,
) -> anyhow::Result<ProcessInfo> {
    let entry = server.processes.lock().await.get(&process_id).cloned();

    if let Some(entry) = entry {
        let info = entry.info.lock().await;
        return Ok(info.clone());
    }

    let processes = handle_process_list(server, ProcessListParams { thread_id: None }).await?;
    processes
        .into_iter()
        .find(|p| p.process_id == process_id)
        .ok_or_else(|| anyhow::anyhow!("process not found: {}", process_id))
}

static PROCESS_EXEC_GATEWAY: OnceLock<omne_execution_gateway::ExecGateway> = OnceLock::new();

fn process_exec_gateway() -> &'static omne_execution_gateway::ExecGateway {
    PROCESS_EXEC_GATEWAY.get_or_init(|| {
        let mut policy = omne_execution_gateway::GatewayPolicy::default();
        // omne-agent process tools legitimately execute non-allowlisted programs.
        policy.enforce_allowlisted_program_for_mutation = false;
        // Preserve current OMNE_HARDENING=off behavior.
        policy.allow_isolation_none = true;
        omne_execution_gateway::ExecGateway::with_policy(policy)
    })
}

fn process_exec_gateway_required_isolation() -> policy_meta::ExecutionIsolation {
    match hardening_mode() {
        HardeningMode::Off => policy_meta::ExecutionIsolation::None,
        HardeningMode::BestEffort => policy_meta::ExecutionIsolation::BestEffort,
    }
}

fn process_exec_gateway_denied_reason(
    argv: &[String],
    cwd: &Path,
    thread_root: &Path,
) -> Option<String> {
    let request = process_exec_gateway_request(argv, cwd, thread_root)?;
    match process_exec_gateway().preflight(&request) {
        Ok(_) => None,
        Err(err) => {
            let (_, error) = err.into_parts();
            Some(process_exec_gateway_error_reason(&error))
        }
    }
}

fn process_exec_gateway_request(
    argv: &[String],
    cwd: &Path,
    thread_root: &Path,
) -> Option<omne_execution_gateway::ExecRequest> {
    let (program, args) = argv.split_first()?;
    let cwd = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        thread_root.join(cwd)
    };
    Some(omne_execution_gateway::ExecRequest::new(
        program.clone(),
        args.iter().cloned(),
        cwd,
        process_exec_gateway_required_isolation(),
        thread_root.to_path_buf(),
    ))
}

fn prepare_process_exec_gateway_command(
    argv: &[String],
    cwd: &Path,
    thread_root: &Path,
    sandbox_policy: policy_meta::WriteScope,
    command: &mut std::process::Command,
) -> omne_execution_gateway::ExecResult<()> {
    if sandbox_policy == policy_meta::WriteScope::FullAccess
        && cwd.is_absolute()
        && !cwd.starts_with(thread_root)
    {
        return Ok(());
    }
    let request = process_exec_gateway_request(argv, cwd, thread_root).ok_or_else(|| {
        omne_execution_gateway::ExecError::PolicyDenied("argv must not be empty".into())
    })?;
    let mut gateway_command = std::process::Command::new(command.get_program());
    gateway_command.args(command.get_args());
    if let Some(current_dir) = command.get_current_dir() {
        gateway_command.current_dir(current_dir);
    }
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            gateway_command.env(key, value);
        } else {
            gateway_command.env_remove(key);
        }
    }
    let (_event, result) = process_exec_gateway().prepare_command(&request, gateway_command);
    result.map(|_| ())
}

enum ProcessExecBoundaryDeny {
    SandboxPolicyReadOnly,
    SandboxNetworkDenied,
    GatewayDenied(String),
}

fn process_exec_boundary_denial(
    argv: &[String],
    cwd: &Path,
    thread_root: &Path,
    sandbox_policy: policy_meta::WriteScope,
    sandbox_network_access: omne_protocol::SandboxNetworkAccess,
) -> Option<ProcessExecBoundaryDeny> {
    if sandbox_policy == policy_meta::WriteScope::ReadOnly {
        return Some(ProcessExecBoundaryDeny::SandboxPolicyReadOnly);
    }

    if sandbox_network_access == omne_protocol::SandboxNetworkAccess::Deny
        && omne_process_runtime::command_uses_network(argv)
    {
        return Some(ProcessExecBoundaryDeny::SandboxNetworkDenied);
    }

    let request = process_exec_gateway_request(argv, cwd, thread_root)?;
    match process_exec_gateway().preflight(&request) {
        Ok(_) => None,
        Err(err)
            if sandbox_policy == policy_meta::WriteScope::FullAccess
                && matches!(
                    err.as_ref(),
                    omne_execution_gateway::ExecError::CwdOutsideWorkspace { .. }
                ) =>
        {
            None
        }
        Err(err) => {
            let (_, error) = err.into_parts();
            Some(ProcessExecBoundaryDeny::GatewayDenied(
                process_exec_gateway_error_reason(&error),
            ))
        }
    }
}

fn process_exec_gateway_error_reason(err: &omne_execution_gateway::ExecError) -> String {
    match err {
        omne_execution_gateway::ExecError::IsolationNotSupported {
            requested,
            supported,
        } => format!(
            "execution boundary denied command: isolation_not_supported (required={}, supported={})",
            requested.as_str(),
            supported.as_str()
        ),
        omne_execution_gateway::ExecError::WorkspaceRootInvalid { path } => format!(
            "execution boundary denied command: workspace_root_invalid ({})",
            path.display()
        ),
        omne_execution_gateway::ExecError::CwdOutsideWorkspace {
            cwd,
            workspace_root,
        } => format!(
            "execution boundary denied command: cwd_outside_workspace (cwd={}, workspace_root={})",
            cwd.display(),
            workspace_root.display()
        ),
        omne_execution_gateway::ExecError::PolicyDefaultIsolationMismatch {
            requested,
            policy_default,
        } => format!(
            "execution boundary denied command: policy_default_isolation_mismatch (requested={}, policy_default={})",
            requested.as_str(),
            policy_default.as_str()
        ),
        omne_execution_gateway::ExecError::PreparedCommandMismatch {
            requested_program,
            requested_args,
            actual_program,
            actual_args,
            ..
        } => format!(
            "execution boundary denied command: prepared_command_mismatch (requested_program={requested_program:?}, requested_args={requested_args:?}, actual_program={actual_program:?}, actual_args={actual_args:?})"
        ),
        omne_execution_gateway::ExecError::Sandbox(message) => {
            format!("execution boundary denied command: sandbox ({message})")
        }
        omne_execution_gateway::ExecError::PolicyDenied(message) => {
            format!("execution boundary denied command: policy_denied ({message})")
        }
        omne_execution_gateway::ExecError::Spawn(err) => {
            format!("execution boundary denied command: spawn_error ({err})")
        }
    }
}

struct LoadedProcessExecPolicy {
    policy: omne_execpolicy::Policy,
    matches: Vec<ExecRuleMatch>,
    decision: ExecDecision,
    justification: Option<String>,
}

enum ProcessExecPolicyLoadError {
    Mode { error: String, details: String },
    Thread { error: String, details: String },
}

#[derive(Clone, Copy)]
enum ProcessExecApprovalRequirement {
    Prompt,
    PromptStrict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessExecApprovalSource {
    ExecPolicy,
    ExecveWrapper,
}

impl ProcessExecApprovalSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExecPolicy => "execpolicy",
            Self::ExecveWrapper => "execve-wrapper",
        }
    }
}

fn process_exec_approval_requirement_label(
    approval_requirement: ProcessExecApprovalRequirement,
) -> &'static str {
    match approval_requirement {
        ProcessExecApprovalRequirement::Prompt => "prompt",
        ProcessExecApprovalRequirement::PromptStrict => "prompt_strict",
    }
}

fn build_process_exec_approval_params(
    argv: &[String],
    cwd: &str,
    timeout_ms: Option<u64>,
    approval: Option<(ProcessExecApprovalSource, ProcessExecApprovalRequirement)>,
) -> Value {
    let mut params = serde_json::json!({
        "argv": argv.to_vec(),
        "cwd": cwd,
    });
    if let Some(timeout_ms) = timeout_ms {
        params["timeout_ms"] = serde_json::json!(timeout_ms);
    }
    if let Some((source, requirement)) = approval {
        params["approval"] = serde_json::json!({
            "source": source.as_str(),
            "requirement": process_exec_approval_requirement_label(requirement),
        });
    }
    params
}

enum ProcessExecGovernance {
    Allowed,
    NeedsApproval {
        approval_id: omne_protocol::ApprovalId,
    },
    Denied(ProcessExecGovernanceDenied),
}

enum ProcessExecGovernanceDenied {
    SandboxPolicyReadOnly,
    SandboxNetworkDenied,
    GatewayDenied(String),
    UnknownMode {
        available: String,
        load_error: Option<String>,
    },
    ModeDenied {
        mode_decision: ModeDecisionAudit,
    },
    ExecPolicyLoad {
        error: String,
        details: String,
        rules: Vec<String>,
    },
    ExecPolicyForbidden {
        matches: Vec<ExecRuleMatch>,
        justification: Option<String>,
    },
    ApprovalDenied {
        remembered: bool,
    },
}

impl From<ProcessExecBoundaryDeny> for ProcessExecGovernanceDenied {
    fn from(value: ProcessExecBoundaryDeny) -> Self {
        match value {
            ProcessExecBoundaryDeny::SandboxPolicyReadOnly => Self::SandboxPolicyReadOnly,
            ProcessExecBoundaryDeny::SandboxNetworkDenied => Self::SandboxNetworkDenied,
            ProcessExecBoundaryDeny::GatewayDenied(reason) => Self::GatewayDenied(reason),
        }
    }
}

impl From<ProcessExecAuthorizationDenied> for ProcessExecGovernanceDenied {
    fn from(value: ProcessExecAuthorizationDenied) -> Self {
        match value {
            ProcessExecAuthorizationDenied::UnknownMode {
                available,
                load_error,
            } => Self::UnknownMode {
                available,
                load_error,
            },
            ProcessExecAuthorizationDenied::ModeDenied { mode_decision } => {
                Self::ModeDenied { mode_decision }
            }
            ProcessExecAuthorizationDenied::ExecPolicyLoad {
                error,
                details,
                rules,
            } => Self::ExecPolicyLoad {
                error,
                details,
                rules,
            },
            ProcessExecAuthorizationDenied::ExecPolicyForbidden {
                matches,
                justification,
            } => Self::ExecPolicyForbidden {
                matches,
                justification,
            },
            ProcessExecAuthorizationDenied::ApprovalDenied { remembered } => {
                Self::ApprovalDenied { remembered }
            }
        }
    }
}

fn process_exec_governance_denied_reason(
    denied: &ProcessExecGovernanceDenied,
    action_label: &str,
) -> String {
    match denied {
        ProcessExecGovernanceDenied::SandboxPolicyReadOnly => {
            format!("sandbox_policy=read_only forbids {action_label}")
        }
        ProcessExecGovernanceDenied::SandboxNetworkDenied => {
            "sandbox_network_access=deny blocked this command via best-effort argv network classification"
                .to_string()
        }
        ProcessExecGovernanceDenied::GatewayDenied(reason) => reason.clone(),
        ProcessExecGovernanceDenied::UnknownMode { .. } => "unknown mode".to_string(),
        ProcessExecGovernanceDenied::ModeDenied { .. } => {
            format!("mode denies {action_label}")
        }
        ProcessExecGovernanceDenied::ExecPolicyLoad { error, .. } => error.clone(),
        ProcessExecGovernanceDenied::ExecPolicyForbidden { .. } => {
            "execpolicy forbids this command".to_string()
        }
        ProcessExecGovernanceDenied::ApprovalDenied { remembered } => {
            approval_denied_error(*remembered).to_string()
        }
    }
}

struct ProcessExecGovernanceContext<'a> {
    cwd: &'a Path,
    sandbox_policy: policy_meta::WriteScope,
    sandbox_network_access: omne_protocol::SandboxNetworkAccess,
    authorization: ProcessExecAuthorizationContext<'a>,
}

enum ProcessExecAuthorization {
    Allowed,
    NeedsApproval {
        approval_id: omne_protocol::ApprovalId,
    },
    Denied(ProcessExecAuthorizationDenied),
}

enum ProcessExecAuthorizationDenied {
    UnknownMode {
        available: String,
        load_error: Option<String>,
    },
    ModeDenied {
        mode_decision: ModeDecisionAudit,
    },
    ExecPolicyLoad {
        error: String,
        details: String,
        rules: Vec<String>,
    },
    ExecPolicyForbidden {
        matches: Vec<ExecRuleMatch>,
        justification: Option<String>,
    },
    ApprovalDenied {
        remembered: bool,
    },
}

type ProcessExecFallback = fn(&[String]) -> ExecDecision;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnmatchedCommandPolicy {
    Prompt,
    Allow,
}

fn unmatched_command_allow_fallback(_: &[String]) -> ExecDecision {
    ExecDecision::Allow
}

impl UnmatchedCommandPolicy {
    fn fallback(self) -> Option<ProcessExecFallback> {
        match self {
            Self::Prompt => None,
            Self::Allow => Some(unmatched_command_allow_fallback),
        }
    }
}

struct ProcessExecAuthorizationContext<'a> {
    thread_root: &'a Path,
    thread_store: &'a ThreadStore,
    thread_rt: &'a Arc<ThreadRuntime>,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<omne_protocol::ApprovalId>,
    approval_policy: omne_protocol::ApprovalPolicy,
    mode_name: &'a str,
    action: &'static str,
    exec_policy: &'a omne_execpolicy::Policy,
    thread_execpolicy_rules: &'a [String],
    argv: &'a [String],
    unmatched_command_policy: UnmatchedCommandPolicy,
}

async fn load_effective_process_exec_policy(
    thread_root: &Path,
    base_policy: &omne_execpolicy::Policy,
    mode: &omne_core::modes::ModeDef,
    thread_execpolicy_rules: &[String],
    argv: &[String],
    unmatched_command_policy: UnmatchedCommandPolicy,
) -> Result<LoadedProcessExecPolicy, ProcessExecPolicyLoadError> {
    let mut effective_exec_policy = base_policy.clone();
    if !mode.command_execpolicy_rules.is_empty() {
        let mode_exec_policy =
            match load_mode_exec_policy(thread_root, &mode.command_execpolicy_rules).await {
                Ok(policy) => policy,
                Err(err) => {
                    return Err(ProcessExecPolicyLoadError::Mode {
                        error: "failed to load mode execpolicy rules".to_string(),
                        details: err.to_string(),
                    });
                }
            };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &mode_exec_policy);
    }
    if !thread_execpolicy_rules.is_empty() {
        let thread_exec_policy =
            match load_mode_exec_policy(thread_root, thread_execpolicy_rules).await {
                Ok(policy) => policy,
                Err(err) => {
                    return Err(ProcessExecPolicyLoadError::Thread {
                        error: "failed to load thread execpolicy rules".to_string(),
                        details: err.to_string(),
                    });
                }
            };
        effective_exec_policy = merge_exec_policies(&effective_exec_policy, &thread_exec_policy);
    }
    let matches = match unmatched_command_policy.fallback() {
        Some(fallback) => effective_exec_policy.matches_for_command(argv, Some(&fallback)),
        None => effective_exec_policy.matches_for_command(argv, None),
    };
    let decision = match matches.iter().map(ExecRuleMatch::decision).max() {
        Some(ExecDecision::Forbidden) => ExecDecision::Forbidden,
        Some(ExecDecision::PromptStrict) => ExecDecision::PromptStrict,
        Some(ExecDecision::Allow) => ExecDecision::Allow,
        Some(ExecDecision::Prompt) | None => ExecDecision::Prompt,
    };
    let justification = matches.iter().find_map(|m| match m {
        ExecRuleMatch::PrefixRuleMatch {
            decision: ExecDecision::Forbidden,
            justification,
            ..
        } => justification.clone(),
        _ => None,
    });
    Ok(LoadedProcessExecPolicy {
        policy: effective_exec_policy,
        matches,
        decision,
        justification,
    })
}

async fn authorize_process_exec<F, G>(
    ctx: &ProcessExecAuthorizationContext<'_>,
    base_decision_for_mode: F,
    approval_params_for_requirement: G,
) -> anyhow::Result<ProcessExecAuthorization>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
    G: Fn(ProcessExecApprovalRequirement) -> Value,
{
    let catalog = omne_core::modes::ModeCatalog::load(ctx.thread_root).await;
    let mode = match catalog.mode(ctx.mode_name).cloned() {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            return Ok(ProcessExecAuthorization::Denied(
                ProcessExecAuthorizationDenied::UnknownMode {
                    available,
                    load_error: catalog.load_error.clone(),
                },
            ));
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(&mode, ctx.action, base_decision_for_mode(&mode));
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        return Ok(ProcessExecAuthorization::Denied(
            ProcessExecAuthorizationDenied::ModeDenied { mode_decision },
        ));
    }

    let effective_exec_policy = match load_effective_process_exec_policy(
        ctx.thread_root,
        ctx.exec_policy,
        &mode,
        ctx.thread_execpolicy_rules,
        ctx.argv,
        ctx.unmatched_command_policy,
    )
    .await
    {
        Ok(policy) => policy,
        Err(ProcessExecPolicyLoadError::Mode { error, details }) => {
            return Ok(ProcessExecAuthorization::Denied(
                ProcessExecAuthorizationDenied::ExecPolicyLoad {
                    error,
                    details,
                    rules: mode.command_execpolicy_rules.clone(),
                },
            ));
        }
        Err(ProcessExecPolicyLoadError::Thread { error, details }) => {
            return Ok(ProcessExecAuthorization::Denied(
                ProcessExecAuthorizationDenied::ExecPolicyLoad {
                    error,
                    details,
                    rules: ctx.thread_execpolicy_rules.to_vec(),
                },
            ));
        }
    };

    if effective_exec_policy.decision == ExecDecision::Forbidden {
        return Ok(ProcessExecAuthorization::Denied(
            ProcessExecAuthorizationDenied::ExecPolicyForbidden {
                matches: effective_exec_policy.matches,
                justification: effective_exec_policy.justification,
            },
        ));
    }

    let approval_requirement = match effective_exec_policy.decision {
        ExecDecision::PromptStrict => Some(ProcessExecApprovalRequirement::PromptStrict),
        ExecDecision::Prompt => Some(ProcessExecApprovalRequirement::Prompt),
        ExecDecision::Allow if mode_decision.decision == omne_core::modes::Decision::Prompt => {
            Some(ProcessExecApprovalRequirement::Prompt)
        }
        ExecDecision::Allow => None,
        ExecDecision::Forbidden => None,
    };

    let Some(approval_requirement) = approval_requirement else {
        return Ok(ProcessExecAuthorization::Allowed);
    };

    let approval_params = approval_params_for_requirement(approval_requirement);
    match gate_approval_with_deps(
        ctx.thread_store,
        &effective_exec_policy.policy,
        ctx.thread_rt,
        ctx.thread_id,
        ctx.turn_id,
        ctx.approval_policy,
        ApprovalRequest {
            approval_id: ctx.approval_id,
            action: ctx.action,
            params: &approval_params,
        },
    )
    .await?
    {
        ApprovalGate::Approved => Ok(ProcessExecAuthorization::Allowed),
        ApprovalGate::Denied { remembered } => Ok(ProcessExecAuthorization::Denied(
            ProcessExecAuthorizationDenied::ApprovalDenied { remembered },
        )),
        ApprovalGate::NeedsApproval { approval_id } => {
            Ok(ProcessExecAuthorization::NeedsApproval { approval_id })
        }
    }
}

async fn evaluate_process_exec_governance<F, G>(
    ctx: &ProcessExecGovernanceContext<'_>,
    base_decision_for_mode: F,
    approval_params_for_requirement: G,
) -> anyhow::Result<ProcessExecGovernance>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
    G: Fn(ProcessExecApprovalRequirement) -> Value,
{
    if let Some(denial) = process_exec_boundary_denial(
        ctx.authorization.argv,
        ctx.cwd,
        ctx.authorization.thread_root,
        ctx.sandbox_policy,
        ctx.sandbox_network_access,
    ) {
        return Ok(ProcessExecGovernance::Denied(denial.into()));
    }

    match authorize_process_exec(
        &ctx.authorization,
        base_decision_for_mode,
        approval_params_for_requirement,
    )
    .await?
    {
        ProcessExecAuthorization::Allowed => Ok(ProcessExecGovernance::Allowed),
        ProcessExecAuthorization::NeedsApproval { approval_id } => {
            Ok(ProcessExecGovernance::NeedsApproval { approval_id })
        }
        ProcessExecAuthorization::Denied(denied) => {
            Ok(ProcessExecGovernance::Denied(denied.into()))
        }
    }
}

enum ProcessModeGate {
    Allowed { mode_decision: ModeDecisionAudit },
    Denied(Box<Value>),
}

struct ProcessModeApprovalContext<'a> {
    thread_rt: &'a Arc<ThreadRuntime>,
    thread_root: &'a Path,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    approval_id: Option<omne_protocol::ApprovalId>,
    approval_policy: omne_protocol::ApprovalPolicy,
    mode_name: &'a str,
    action: &'static str,
    tool_id: omne_protocol::ToolId,
    approval_params: &'a Value,
}

async fn enforce_process_mode_gate<F>(
    ctx: &ProcessModeApprovalContext<'_>,
    base_decision_for_mode: F,
) -> anyhow::Result<ProcessModeGate>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
{
    let catalog = omne_core::modes::ModeCatalog::load(ctx.thread_root).await;
    let mode = match catalog.mode(ctx.mode_name).cloned() {
        Some(mode) => mode,
        None => {
            let available = catalog.mode_names().collect::<Vec<_>>().join(", ");
            let result = process_unknown_mode_denied_response(
                ctx.tool_id,
                ctx.thread_id,
                ctx.mode_name,
                available,
                catalog.load_error.clone(),
            )?;
            emit_process_tool_denied(
                ctx.thread_rt,
                ctx.tool_id,
                ctx.turn_id,
                ctx.action,
                ctx.approval_params,
                "unknown mode".to_string(),
                result.clone(),
            )
            .await?;
            return Ok(ProcessModeGate::Denied(Box::new(result)));
        }
    };

    let mode_decision =
        resolve_mode_decision_audit(&mode, ctx.action, base_decision_for_mode(&mode));
    if mode_decision.decision == omne_core::modes::Decision::Deny {
        let result =
            process_mode_denied_response(ctx.tool_id, ctx.thread_id, ctx.mode_name, mode_decision)?;
        emit_process_tool_denied(
            ctx.thread_rt,
            ctx.tool_id,
            ctx.turn_id,
            ctx.action,
            ctx.approval_params,
            format!("mode denies {}", ctx.action),
            result.clone(),
        )
        .await?;
        return Ok(ProcessModeGate::Denied(Box::new(result)));
    }

    Ok(ProcessModeGate::Allowed { mode_decision })
}

async fn enforce_process_mode_and_approval<F>(
    server: &Server,
    ctx: ProcessModeApprovalContext<'_>,
    base_decision_for_mode: F,
) -> anyhow::Result<Option<Value>>
where
    F: Fn(&omne_core::modes::ModeDef) -> omne_core::modes::Decision,
{
    let mode_decision = match enforce_process_mode_gate(&ctx, base_decision_for_mode).await? {
        ProcessModeGate::Denied(result) => return Ok(Some(*result)),
        ProcessModeGate::Allowed { mode_decision, .. } => mode_decision,
    };

    if mode_decision.decision == omne_core::modes::Decision::Prompt {
        match gate_approval(
            server,
            ctx.thread_rt,
            ctx.thread_id,
            ctx.turn_id,
            ctx.approval_policy,
            ApprovalRequest {
                approval_id: ctx.approval_id,
                action: ctx.action,
                params: ctx.approval_params,
            },
        )
        .await?
        {
            ApprovalGate::Approved => {}
            ApprovalGate::Denied { remembered } => {
                let result = process_denied_response(ctx.tool_id, ctx.thread_id, Some(remembered))?;
                emit_process_tool_denied(
                    ctx.thread_rt,
                    ctx.tool_id,
                    ctx.turn_id,
                    ctx.action,
                    ctx.approval_params,
                    approval_denied_error(remembered).to_string(),
                    result.clone(),
                )
                .await?;
                return Ok(Some(result));
            }
            ApprovalGate::NeedsApproval { approval_id } => {
                let result = process_needs_approval_response(ctx.thread_id, approval_id)?;
                return Ok(Some(result));
            }
        }
    }

    Ok(None)
}

async fn emit_process_tool_denied(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
    turn_id: Option<TurnId>,
    action: &str,
    params: &Value,
    error: String,
    result: Value,
) -> anyhow::Result<()> {
    emit_tool_denied(
        thread_rt,
        tool_id,
        turn_id,
        action,
        Some(params.clone()),
        error,
        result,
    )
    .await
}

async fn emit_process_tool_denied_response(
    thread_rt: &Arc<ThreadRuntime>,
    tool_id: omne_protocol::ToolId,
    turn_id: Option<TurnId>,
    action: &str,
    params: &Value,
    error: String,
    result: Value,
) -> anyhow::Result<Value> {
    emit_process_tool_denied(
        thread_rt,
        tool_id,
        turn_id,
        action,
        params,
        error,
        result.clone(),
    )
    .await?;
    Ok(result)
}

fn process_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    remembered: Option<bool>,
) -> anyhow::Result<Value> {
    denied_response_with_remembered(
        tool_id,
        remembered,
        "serialize process denied response",
        |tool_id, remembered, structured_error, error_code| {
            omne_app_server_protocol::ProcessDeniedResponse {
                tool_id,
                denied: true,
                thread_id,
                remembered,
                structured_error,
                error_code,
            }
        },
    )
}

fn process_needs_approval_response(
    thread_id: ThreadId,
    approval_id: omne_protocol::ApprovalId,
) -> anyhow::Result<Value> {
    needs_approval_response_json(
        approval_id,
        "serialize process needs_approval response",
        |approval_id| omne_app_server_protocol::ProcessNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        },
    )
}

fn process_allowed_tools_denied_response(
    tool_id: omne_protocol::ToolId,
    tool: &str,
    allowed_tools: &Option<Vec<String>>,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("allowed_tools_denied", |message| {
        message.try_with_value_arg("tool", tool)?;
        Ok(())
    })?;
    let response = omne_app_server_protocol::ProcessAllowedToolsDeniedResponse {
        tool_id,
        denied: true,
        tool: tool.to_string(),
        allowed_tools: allowed_tools.clone().unwrap_or_default(),
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process allowed_tools denied response")
}

fn process_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    mode_name: &str,
    mode_decision: ModeDecisionAudit,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("mode_denied", |message| {
        message.try_with_value_arg("mode", mode_name)?;
        message.try_with_value_arg("decision_source", mode_decision.decision_source)?;
        message.try_with_value_arg("tool_override_hit", mode_decision.tool_override_hit)?;
        Ok(())
    })?;
    let response = omne_app_server_protocol::ProcessModeDeniedResponse {
        tool_id,
        denied: true,
        thread_id,
        mode: mode_name.to_string(),
        decision: map_mode_decision_for_protocol!(
            mode_decision.decision,
            omne_app_server_protocol::ProcessModeDecision
        ),
        decision_source: mode_decision.decision_source.to_string(),
        tool_override_hit: mode_decision.tool_override_hit,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process mode denied response")
}

fn process_unknown_mode_denied_response(
    tool_id: omne_protocol::ToolId,
    thread_id: ThreadId,
    mode_name: &str,
    available: String,
    load_error: Option<String>,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("mode_unknown", |message| {
        message.try_with_value_arg("mode", mode_name)?;
        message.try_with_value_arg("available", available.as_str())?;
        if let Some(load_error) = load_error.as_deref() {
            message.try_with_value_arg("load_error", load_error)?;
        }
        Ok(())
    })?;
    let response = omne_app_server_protocol::ProcessUnknownModeDeniedResponse {
        tool_id,
        denied: true,
        thread_id,
        mode: mode_name.to_string(),
        decision: omne_app_server_protocol::ProcessModeDecision::Deny,
        available,
        load_error,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process unknown mode denied response")
}

fn process_sandbox_policy_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_policy: policy_meta::WriteScope,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error("sandbox_policy_denied")?;
    let response = omne_app_server_protocol::ProcessSandboxPolicyDeniedResponse {
        tool_id,
        denied: true,
        sandbox_policy,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process sandbox_policy denied response")
}

fn process_sandbox_network_denied_response(
    tool_id: omne_protocol::ToolId,
    sandbox_network_access: omne_protocol::SandboxNetworkAccess,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error("sandbox_network_denied")?;
    let response = omne_app_server_protocol::ProcessSandboxNetworkDeniedResponse {
        tool_id,
        denied: true,
        sandbox_network_access,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process sandbox_network denied response")
}

fn process_execpolicy_denied_response(
    tool_id: omne_protocol::ToolId,
    decision: ExecDecision,
    matched_rules: &[ExecRuleMatch],
    justification: Option<String>,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error("execpolicy_denied")?;
    let response = omne_app_server_protocol::ProcessExecPolicyDeniedResponse {
        tool_id,
        denied: true,
        decision: to_protocol_execpolicy_decision(decision),
        matched_rules: to_protocol_execpolicy_matches(matched_rules),
        justification,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process execpolicy denied response")
}

fn process_execpolicy_load_denied_response(
    tool_id: omne_protocol::ToolId,
    mode_name: &str,
    error: &str,
    details: String,
) -> anyhow::Result<Value> {
    let structured_error = catalog_structured_error_with("execpolicy_load_denied", |message| {
        message.try_with_value_arg("mode", mode_name)?;
        message.try_with_value_arg("error", error)?;
        message.try_with_value_arg("details", details.as_str())?;
        Ok(())
    })?;
    let response = omne_app_server_protocol::ProcessExecPolicyLoadDeniedResponse {
        tool_id,
        denied: true,
        mode: mode_name.to_string(),
        error: error.to_string(),
        details,
        structured_error: Some(structured_error.clone()),
        error_code: structured_error_code(&structured_error),
    };
    serde_json::to_value(response).context("serialize process execpolicy load denied response")
}

fn to_protocol_execpolicy_decision(
    decision: ExecDecision,
) -> omne_app_server_protocol::ExecPolicyDecision {
    match decision {
        ExecDecision::Allow => omne_app_server_protocol::ExecPolicyDecision::Allow,
        ExecDecision::Prompt => omne_app_server_protocol::ExecPolicyDecision::Prompt,
        ExecDecision::PromptStrict => omne_app_server_protocol::ExecPolicyDecision::PromptStrict,
        ExecDecision::Forbidden => omne_app_server_protocol::ExecPolicyDecision::Forbidden,
    }
}

fn to_protocol_execpolicy_matches(
    matched_rules: &[ExecRuleMatch],
) -> Vec<omne_app_server_protocol::ExecPolicyRuleMatch> {
    matched_rules
        .iter()
        .map(|rule| match rule {
            ExecRuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision,
                justification,
            } => omne_app_server_protocol::ExecPolicyRuleMatch::PrefixRuleMatch {
                matched_prefix: matched_prefix.clone(),
                decision: to_protocol_execpolicy_decision(*decision),
                justification: justification.clone(),
            },
            ExecRuleMatch::HeuristicsRuleMatch { command, decision } => {
                omne_app_server_protocol::ExecPolicyRuleMatch::HeuristicsRuleMatch {
                    command: command.clone(),
                    decision: to_protocol_execpolicy_decision(*decision),
                }
            }
        })
        .collect()
}
