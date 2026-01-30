#[derive(Parser)]
#[command(name = "pm-app-server")]
#[command(about = "CodePM v0.2.0 app-server (JSON-RPC over stdio)", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Override project data root directory (default: `./.codepm_data/`).
    #[arg(long)]
    pm_root: Option<PathBuf>,

    /// Listen on a Unix socket instead of stdio (daemon mode).
    #[arg(long, value_name = "PATH")]
    listen: Option<PathBuf>,

    /// Paths to execpolicy rule files to evaluate (repeatable).
    #[arg(long = "execpolicy-rules", value_name = "PATH")]
    execpolicy_rules: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Generate TypeScript protocol types to an output directory.
    GenerateTs(GenerateOutArgs),
    /// Generate JSON Schema files to an output directory.
    GenerateJsonSchema(GenerateOutArgs),
}

#[cfg(test)]
mod hardening_mode_tests {
    use super::HardeningMode;

    #[test]
    fn hardening_mode_defaults_to_best_effort() {
        assert_eq!(HardeningMode::parse(None).unwrap(), HardeningMode::BestEffort);
    }

    #[test]
    fn hardening_mode_parses_off() {
        assert_eq!(
            HardeningMode::parse(Some("off")).unwrap(),
            HardeningMode::Off
        );
    }

    #[test]
    fn hardening_mode_parses_best_effort() {
        assert_eq!(
            HardeningMode::parse(Some("best_effort")).unwrap(),
            HardeningMode::BestEffort
        );
    }

    #[test]
    fn hardening_mode_rejects_invalid_value() {
        assert!(HardeningMode::parse(Some("wat")).is_err());
        assert!(HardeningMode::parse(Some("best-effort")).is_err());
        assert!(HardeningMode::parse(Some("")).is_err());
        assert!(HardeningMode::parse(Some(" ")).is_err());
    }
}

#[derive(clap::Args)]
struct GenerateOutArgs {
    /// Output directory.
    #[arg(long = "out", value_name = "DIR")]
    out_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_INVALID_PARAMS: i64 = -32602;
const JSONRPC_INTERNAL_ERROR: i64 = -32603;
const CODE_PM_NOT_INITIALIZED: i64 = -32_000;
const CODE_PM_ALREADY_INITIALIZED: i64 = -32_001;

#[cfg(feature = "notify")]
type NotifyHub = Option<notify_kit::Hub>;

#[cfg(not(feature = "notify"))]
type NotifyHub = ();

#[cfg(test)]
fn default_notify_hub() -> NotifyHub {
    #[cfg(feature = "notify")]
    {
        None
    }
    #[cfg(not(feature = "notify"))]
    {
        ()
    }
}

fn init_notify_hub() -> anyhow::Result<NotifyHub> {
    #[cfg(feature = "notify")]
    {
        crate::notify_integration::init_notify_hub_from_env()
    }
    #[cfg(not(feature = "notify"))]
    {
        Ok(())
    }
}

struct DiskWarningState {
    last_checked_at: Option<tokio::time::Instant>,
    last_reported_at: Option<tokio::time::Instant>,
}

#[derive(Clone)]
struct Server {
    cwd: PathBuf,
    notify_tx: broadcast::Sender<String>,
    notify_hub: NotifyHub,
    thread_store: ThreadStore,
    threads: Arc<tokio::sync::Mutex<HashMap<ThreadId, Arc<ThreadRuntime>>>>,
    processes: Arc<tokio::sync::Mutex<HashMap<ProcessId, ProcessEntry>>>,
    mcp: Arc<tokio::sync::Mutex<McpManager>>,
    disk_warning: Arc<tokio::sync::Mutex<HashMap<ThreadId, DiskWarningState>>>,
    exec_policy: pm_execpolicy::Policy,
}

impl Server {
    async fn get_or_load_thread(&self, thread_id: ThreadId) -> anyhow::Result<Arc<ThreadRuntime>> {
        let mut threads = self.threads.lock().await;
        if let Some(rt) = threads.get(&thread_id) {
            return Ok(rt.clone());
        }

        let handle = self
            .thread_store
            .resume_thread(thread_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;

        let rt = Arc::new(ThreadRuntime::new(
            handle,
            self.notify_tx.clone(),
            self.notify_hub.clone(),
        ));
        threads.insert(thread_id, rt.clone());
        Ok(rt)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProcessStatus {
    Running,
    Exited,
    Abandoned,
}

#[derive(Clone, Debug, Serialize)]
struct ProcessInfo {
    process_id: ProcessId,
    thread_id: ThreadId,
    turn_id: Option<TurnId>,
    argv: Vec<String>,
    cwd: String,
    started_at: String,
    status: ProcessStatus,
    exit_code: Option<i32>,
    stdout_path: String,
    stderr_path: String,
    last_update_at: String,
}

#[derive(Clone)]
struct ProcessEntry {
    info: Arc<tokio::sync::Mutex<ProcessInfo>>,
    cmd_tx: mpsc::Sender<ProcessCommand>,
}

enum ProcessCommand {
    Interrupt { reason: Option<String> },
    Kill { reason: Option<String> },
}

struct ThreadRuntime {
    handle: tokio::sync::Mutex<pm_core::ThreadHandle>,
    active_turn: tokio::sync::Mutex<Option<ActiveTurn>>,
    notify_tx: broadcast::Sender<String>,
    #[cfg_attr(not(feature = "notify"), allow(dead_code))]
    notify_hub: NotifyHub,
}

fn validate_context_refs(refs: &[pm_protocol::ContextRef]) -> anyhow::Result<()> {
    for ctx in refs {
        match ctx {
            pm_protocol::ContextRef::File(file) => {
                if file.path.trim().is_empty() {
                    anyhow::bail!("context_refs.file.path must be non-empty");
                }
                if let Some(start_line) = file.start_line {
                    if start_line == 0 {
                        anyhow::bail!("context_refs.file.start_line must be >= 1");
                    }
                }
                if let Some(end_line) = file.end_line {
                    if end_line == 0 {
                        anyhow::bail!("context_refs.file.end_line must be >= 1");
                    }
                    let Some(start_line) = file.start_line else {
                        anyhow::bail!("context_refs.file.end_line requires start_line");
                    };
                    if end_line < start_line {
                        anyhow::bail!("context_refs.file.end_line must be >= start_line");
                    }
                }
                if let Some(max_bytes) = file.max_bytes {
                    if max_bytes == 0 {
                        anyhow::bail!("context_refs.file.max_bytes must be >= 1");
                    }
                }
            }
            pm_protocol::ContextRef::Diff(diff) => {
                if let Some(max_bytes) = diff.max_bytes {
                    if max_bytes == 0 {
                        anyhow::bail!("context_refs.diff.max_bytes must be >= 1");
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_turn_attachments(attachments: &[pm_protocol::TurnAttachment]) -> anyhow::Result<()> {
    for attachment in attachments {
        match attachment {
            pm_protocol::TurnAttachment::Image(image) => {
                match &image.source {
                    pm_protocol::AttachmentSource::Path { path } => {
                        if path.trim().is_empty() {
                            anyhow::bail!("attachments.image.source.path must be non-empty");
                        }
                    }
                    pm_protocol::AttachmentSource::Url { url } => {
                        if url.trim().is_empty() {
                            anyhow::bail!("attachments.image.source.url must be non-empty");
                        }
                    }
                }
                if let Some(media_type) = image.media_type.as_deref() {
                    if media_type.trim().is_empty() {
                        anyhow::bail!("attachments.image.media_type must be non-empty when provided");
                    }
                }
            }
            pm_protocol::TurnAttachment::File(file) => {
                match &file.source {
                    pm_protocol::AttachmentSource::Path { path } => {
                        if path.trim().is_empty() {
                            anyhow::bail!("attachments.file.source.path must be non-empty");
                        }
                    }
                    pm_protocol::AttachmentSource::Url { url } => {
                        if url.trim().is_empty() {
                            anyhow::bail!("attachments.file.source.url must be non-empty");
                        }
                    }
                }
                if file.media_type.trim().is_empty() {
                    anyhow::bail!("attachments.file.media_type must be non-empty");
                }
                if let Some(filename) = file.filename.as_deref() {
                    if filename.trim().is_empty() {
                        anyhow::bail!("attachments.file.filename must be non-empty when provided");
                    }
                }
            }
        }
    }
    Ok(())
}

impl ThreadRuntime {
    fn new(
        handle: pm_core::ThreadHandle,
        notify_tx: broadcast::Sender<String>,
        notify_hub: NotifyHub,
    ) -> Self {
        Self {
            handle: tokio::sync::Mutex::new(handle),
            active_turn: tokio::sync::Mutex::new(None),
            notify_tx,
            notify_hub,
        }
    }

    fn emit_notification<T>(&self, method: &'static str, params: &T)
    where
        T: Serialize,
    {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        if let Ok(line) = serde_json::to_string(&payload) {
            let _ = self.notify_tx.send(line);
        }
    }

    fn emit_event_notifications(&self, event: &ThreadEvent) {
        self.emit_notification("thread/event", event);

        match &event.kind {
            pm_protocol::ThreadEventKind::TurnStarted { .. } => {
                self.emit_notification("turn/started", event);
            }
            pm_protocol::ThreadEventKind::TurnCompleted { .. } => {
                self.emit_notification("turn/completed", event);
            }
            pm_protocol::ThreadEventKind::ToolStarted { .. }
            | pm_protocol::ThreadEventKind::ProcessStarted { .. }
            | pm_protocol::ThreadEventKind::ApprovalRequested { .. } => {
                self.emit_notification("item/started", event);
            }
            pm_protocol::ThreadEventKind::ToolCompleted { .. }
            | pm_protocol::ThreadEventKind::ProcessExited { .. }
            | pm_protocol::ThreadEventKind::ApprovalDecided { .. }
            | pm_protocol::ThreadEventKind::AssistantMessage { .. } => {
                self.emit_notification("item/completed", event);
            }
            pm_protocol::ThreadEventKind::AgentStep { .. } => {
                self.emit_notification("item/completed", event);
                self.emit_notification("agent/step", event);
            }
            _ => {}
        }

        self.emit_notify_hub(event);
    }

    fn emit_notify_hub(&self, event: &ThreadEvent) {
        let _ = event;
        #[cfg(feature = "notify")]
        {
            let Some(hub) = &self.notify_hub else {
                return;
            };
            let Some(ev) = crate::notify_integration::map_thread_event_to_notify_event(event)
            else {
                return;
            };
            hub.notify(ev);
        }
    }

    async fn start_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        input: String,
        context_refs: Option<Vec<pm_protocol::ContextRef>>,
        attachments: Option<Vec<pm_protocol::TurnAttachment>>,
        priority: pm_protocol::TurnPriority,
    ) -> anyhow::Result<TurnId> {
        let mut handle = self.handle.lock().await;
        let state = handle.state();
        if state.archived {
            anyhow::bail!("thread is archived");
        }
        if state.paused {
            anyhow::bail!("thread is paused");
        }
        if state.active_turn_id.is_some() {
            anyhow::bail!("turn already active");
        }

        let context_refs = match context_refs {
            Some(refs) if refs.is_empty() => None,
            other => other,
        };
        if let Some(refs) = context_refs.as_deref() {
            validate_context_refs(refs)?;
        }

        let attachments = match attachments {
            Some(attachments) if attachments.is_empty() => None,
            other => other,
        };
        if let Some(attachments) = attachments.as_deref() {
            validate_turn_attachments(attachments)?;
        }

        let turn_id = TurnId::new();
        let input_for_event = input.clone();
        let event = handle
            .append(pm_protocol::ThreadEventKind::TurnStarted {
                turn_id,
                input: input_for_event,
                context_refs,
                attachments,
                priority,
            })
            .await?;
        drop(handle);
        self.emit_event_notifications(&event);

        let cancel = CancellationToken::new();
        {
            let mut active = self.active_turn.lock().await;
            *active = Some(ActiveTurn {
                turn_id,
                cancel: cancel.clone(),
                interrupt_reason: None,
            });
        }

        tokio::task::spawn_local(async move {
            self.run_turn(server, turn_id, cancel, input, priority).await;
        });

        Ok(turn_id)
    }

    async fn append_event(
        &self,
        kind: pm_protocol::ThreadEventKind,
    ) -> anyhow::Result<ThreadEvent> {
        let mut handle = self.handle.lock().await;
        let event = handle.append(kind).await?;
        drop(handle);
        self.emit_event_notifications(&event);
        Ok(event)
    }

    async fn interrupt_turn(&self, turn_id: TurnId, reason: Option<String>) -> anyhow::Result<()> {
        let cancel = {
            let mut active = self.active_turn.lock().await;
            let Some(active) = active.as_mut() else {
                anyhow::bail!("no active turn");
            };
            if active.turn_id != turn_id {
                anyhow::bail!("turn is not active");
            }
            if active.interrupt_reason.is_none() {
                active.interrupt_reason = reason.clone();
            }
            active.cancel.clone()
        };

        let mut handle = self.handle.lock().await;
        if handle.state().active_turn_interrupt_requested {
            cancel.cancel();
            return Ok(());
        }
        let event = handle
            .append(pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason })
            .await?;
        drop(handle);
        self.emit_event_notifications(&event);

        cancel.cancel();
        Ok(())
    }

    async fn run_turn(
        self: Arc<Self>,
        server: Arc<Server>,
        turn_id: TurnId,
        cancel: CancellationToken,
        input: String,
        priority: pm_protocol::TurnPriority,
    ) {
        let agent_fut =
            agent::run_agent_turn(server.clone(), self.clone(), turn_id, input, cancel.clone(), priority);

        let (status, reason) = tokio::select! {
            _ = cancel.cancelled() => {
                let reason = {
                    let active = self.active_turn.lock().await;
                    active.as_ref().and_then(|a| a.interrupt_reason.clone())
                };
                (TurnStatus::Interrupted, reason.or_else(|| Some("turn interrupted".to_string())))
            },
            result = agent_fut => {
                match result {
                    Ok(_completion) => (TurnStatus::Completed, None),
                    Err(err) => {
                        let status = classify_agent_turn_error(&err);
                        (status, Some(err.to_string()))
                    }
                }
            },
        };
        let reason_for_report = reason.clone();

        let mut handle = self.handle.lock().await;
        let thread_id = handle.thread_id();
        if let Ok(event) = handle
            .append(pm_protocol::ThreadEventKind::TurnCompleted {
                turn_id,
                status,
                reason,
            })
            .await
        {
            self.emit_event_notifications(&event);
        }
        drop(handle);

        if matches!(status, TurnStatus::Stuck) {
            if let Err(err) = maybe_write_stuck_report(
                server.as_ref(),
                thread_id,
                turn_id,
                reason_for_report.as_deref(),
            )
            .await
            {
                tracing::debug!(
                    thread_id = %thread_id,
                    turn_id = %turn_id,
                    error = %err,
                    "stuck report write failed"
                );
            }
        }

        let _ = run_stop_hooks(
            server.as_ref(),
            thread_id,
            turn_id,
            status,
            reason_for_report.as_deref(),
        )
        .await;

        let mut active = self.active_turn.lock().await;
        if active.as_ref().is_some_and(|a| a.turn_id == turn_id) {
            *active = None;
        }
    }
}

fn classify_agent_turn_error(err: &anyhow::Error) -> TurnStatus {
    for cause in err.chain() {
        if let Some(agent_err) = cause.downcast_ref::<agent::AgentTurnError>() {
            return match agent_err {
                agent::AgentTurnError::Cancelled => TurnStatus::Interrupted,
                agent::AgentTurnError::BudgetExceeded { .. }
                | agent::AgentTurnError::TokenBudgetExceeded { .. }
                | agent::AgentTurnError::OpenAiRequestTimedOut
                | agent::AgentTurnError::LoopDetected { .. } => TurnStatus::Stuck,
            };
        }
    }
    TurnStatus::Failed
}

struct ActiveTurn {
    turn_id: TurnId,
    cancel: CancellationToken,
    interrupt_reason: Option<String>,
}
