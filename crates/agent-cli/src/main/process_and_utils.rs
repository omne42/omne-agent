async fn run_process_follow(
    app: &mut App,
    process_id: ProcessId,
    stderr: bool,
    mut offset: u64,
    max_bytes: Option<u64>,
    poll_ms: u64,
    approval_id: Option<ApprovalId>,
) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(poll_ms.max(50));
    loop {
        let (text, next_offset, eof) = app
            .process_follow(process_id, stderr, offset, max_bytes, approval_id)
            .await?;
        offset = next_offset;
        if !text.is_empty() {
            print!("{text}");
            std::io::stdout().flush().ok();
        }

        if eof {
            let status = app.process_status(process_id).await?;
            if status != "running" {
                return Ok(());
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

fn render_event(event: &ThreadEvent) {
    let ts = event
        .timestamp
        .format(&time::format_description::well_known::Rfc3339);
    let ts = ts.unwrap_or_else(|_| "<time>".to_string());
    match &event.kind {
        pm_protocol::ThreadEventKind::ThreadCreated { cwd } => {
            println!("[{ts}] thread created cwd={cwd}");
        }
        pm_protocol::ThreadEventKind::ThreadArchived { reason } => {
            println!(
                "[{ts}] thread archived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnarchived { reason } => {
            println!(
                "[{ts}] thread unarchived reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadPaused { reason } => {
            println!(
                "[{ts}] thread paused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadUnpaused { reason } => {
            println!(
                "[{ts}] thread unpaused reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnStarted { turn_id, input, .. } => {
            println!("[{ts}] turn started {turn_id}");
            println!("user: {input}");
        }
        pm_protocol::ThreadEventKind::ModelRouted {
            turn_id,
            selected_model,
            rule_source,
            reason,
            rule_id,
        } => {
            println!(
                "[{ts}] model routed {turn_id} model={selected_model} source={rule_source:?} rule_id={} reason={}",
                rule_id.as_deref().unwrap_or(""),
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnInterruptRequested { turn_id, reason } => {
            println!(
                "[{ts}] turn interrupt requested {turn_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::TurnCompleted {
            turn_id,
            status,
            reason,
        } => {
            println!(
                "[{ts}] turn completed {turn_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ThreadConfigUpdated {
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots,
            sandbox_network_access,
            mode,
            model,
            openai_base_url,
            allowed_tools,
        } => {
            println!(
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} sandbox_writable_roots={sandbox_writable_roots:?} sandbox_network_access={sandbox_network_access:?} mode={} model={} openai_base_url={} allowed_tools={allowed_tools:?}",
                mode.as_deref().unwrap_or(""),
                model.as_deref().unwrap_or(""),
                openai_base_url.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ApprovalRequested {
            approval_id,
            action,
            ..
        } => {
            println!("[{ts}] approval requested {approval_id} action={action}");
        }
        pm_protocol::ThreadEventKind::ApprovalDecided {
            approval_id,
            decision,
            remember,
            reason,
        } => {
            println!(
                "[{ts}] approval decided {approval_id} decision={decision:?} remember={remember} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ToolStarted { tool, .. } => {
            println!("[{ts}] tool started {tool}");
        }
        pm_protocol::ThreadEventKind::ToolCompleted { status, error, .. } => {
            println!(
                "[{ts}] tool completed status={status:?} error={}",
                error.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::AgentStep {
            turn_id,
            step,
            model,
            response_id,
            text,
            tool_calls,
            tool_results,
            ..
        } => {
            println!(
                "[{ts}] step {step} turn_id={turn_id} model={model} response_id={response_id} tool_calls={} tool_results={}",
                tool_calls.len(),
                tool_results.len()
            );
            if let Some(text) = text.as_deref().filter(|s| !s.trim().is_empty()) {
                println!("{text}");
            }
        }
        pm_protocol::ThreadEventKind::AssistantMessage { text, model, .. } => {
            if let Some(model) = model {
                println!("[{ts}] assistant (model={model}):");
            } else {
                println!("[{ts}] assistant:");
            }
            println!("{text}");
        }
        pm_protocol::ThreadEventKind::ProcessStarted {
            process_id, argv, ..
        } => {
            println!("[{ts}] process started {process_id} argv={argv:?}");
        }
        pm_protocol::ThreadEventKind::ProcessInterruptRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process interrupt requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessKillRequested {
            process_id, reason, ..
        } => {
            println!(
                "[{ts}] process kill requested {process_id} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::ProcessExited {
            process_id,
            exit_code,
            reason,
        } => {
            println!(
                "[{ts}] process exited {process_id} exit_code={} reason={}",
                exit_code
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                reason.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::CheckpointCreated {
            checkpoint_id,
            label,
            snapshot_ref,
            ..
        } => {
            println!(
                "[{ts}] checkpoint created {checkpoint_id} label={} snapshot_ref={snapshot_ref}",
                label.as_deref().unwrap_or("")
            );
        }
        pm_protocol::ThreadEventKind::CheckpointRestored {
            checkpoint_id,
            status,
            reason,
            ..
        } => {
            println!(
                "[{ts}] checkpoint restored {checkpoint_id} status={status:?} reason={}",
                reason.as_deref().unwrap_or("")
            );
        }
    }
}

struct App {
    rpc: pm_jsonrpc::Client,
    notifications: Option<tokio::sync::mpsc::Receiver<pm_jsonrpc::Notification>>,
}

struct RepoSearchRequest {
    thread_id: ThreadId,
    query: String,
    is_regex: bool,
    include_glob: Option<String>,
    max_matches: Option<usize>,
    max_bytes_per_file: Option<u64>,
    max_files: Option<usize>,
    root: Option<String>,
    approval_id: Option<ApprovalId>,
}

struct RepoIndexRequest {
    thread_id: ThreadId,
    include_glob: Option<String>,
    max_files: Option<usize>,
    root: Option<String>,
    approval_id: Option<ApprovalId>,
}

struct RepoSymbolsRequest {
    thread_id: ThreadId,
    include_glob: Option<String>,
    max_files: Option<usize>,
    max_bytes_per_file: Option<u64>,
    max_symbols: Option<usize>,
    root: Option<String>,
    approval_id: Option<ApprovalId>,
}

struct McpListServersRequest {
    thread_id: ThreadId,
    approval_id: Option<ApprovalId>,
}

struct McpListToolsRequest {
    thread_id: ThreadId,
    server: String,
    approval_id: Option<ApprovalId>,
}

struct McpListResourcesRequest {
    thread_id: ThreadId,
    server: String,
    approval_id: Option<ApprovalId>,
}

struct McpCallRequest {
    thread_id: ThreadId,
    server: String,
    tool: String,
    arguments: Option<Value>,
    approval_id: Option<ApprovalId>,
}

fn split_special_directives(
    input: &str,
) -> anyhow::Result<(
    String,
    Vec<pm_protocol::ContextRef>,
    Vec<pm_protocol::TurnAttachment>,
)> {
    let mut refs = Vec::<pm_protocol::ContextRef>::new();
    let mut attachments = Vec::<pm_protocol::TurnAttachment>::new();
    let lines = input.lines().collect::<Vec<_>>();

    let mut idx = 0usize;
    let mut did_parse = false;
    while idx < lines.len() {
        let line = lines[idx];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            idx += 1;
            continue;
        }

        if trimmed == "@file" {
            anyhow::bail!("@file requires a path");
        }
        if trimmed.starts_with("@file ") || trimmed.starts_with("@file\t") {
            let spec = trimmed["@file".len()..].trim();
            let (path, start_line, end_line) = parse_file_ref_spec(spec)?;
            refs.push(pm_protocol::ContextRef::File(pm_protocol::ContextRefFile {
                path,
                start_line,
                end_line,
                max_bytes: None,
            }));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed.starts_with("@diff") && trimmed != "@diff" {
            anyhow::bail!("@diff does not accept arguments");
        }
        if trimmed == "@diff" {
            refs.push(pm_protocol::ContextRef::Diff(pm_protocol::ContextRefDiff { max_bytes: None }));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed == "@image" {
            anyhow::bail!("@image requires a path or url");
        }
        if trimmed.starts_with("@image ") || trimmed.starts_with("@image\t") {
            let spec = trimmed["@image".len()..].trim();
            let source = if spec.starts_with("http://") || spec.starts_with("https://") {
                pm_protocol::AttachmentSource::Url {
                    url: spec.to_string(),
                }
            } else {
                pm_protocol::AttachmentSource::Path {
                    path: spec.to_string(),
                }
            };
            attachments.push(pm_protocol::TurnAttachment::Image(
                pm_protocol::TurnAttachmentImage {
                    source,
                    media_type: None,
                },
            ));
            did_parse = true;
            idx += 1;
            continue;
        }

        if trimmed == "@pdf" {
            anyhow::bail!("@pdf requires a path or url");
        }
        if trimmed.starts_with("@pdf ") || trimmed.starts_with("@pdf\t") {
            let spec = trimmed["@pdf".len()..].trim();
            let source = if spec.starts_with("http://") || spec.starts_with("https://") {
                pm_protocol::AttachmentSource::Url {
                    url: spec.to_string(),
                }
            } else {
                pm_protocol::AttachmentSource::Path {
                    path: spec.to_string(),
                }
            };
            attachments.push(pm_protocol::TurnAttachment::File(
                pm_protocol::TurnAttachmentFile {
                    source,
                    media_type: "application/pdf".to_string(),
                    filename: None,
                },
            ));
            did_parse = true;
            idx += 1;
            continue;
        }

        break;
    }

    if !did_parse {
        return Ok((input.to_string(), refs, attachments));
    }

    Ok((lines[idx..].join("\n"), refs, attachments))
}

fn parse_file_ref_spec(spec: &str) -> anyhow::Result<(String, Option<u64>, Option<u64>)> {
    let spec = spec.trim();
    if spec.is_empty() {
        anyhow::bail!("file ref is empty");
    }

    let mut parts = spec.split(':').collect::<Vec<_>>();
    let last = parts.pop().unwrap_or_default().trim();
    let Ok(last_num) = last.parse::<u64>() else {
        return Ok((spec.to_string(), None, None));
    };

    if last_num == 0 {
        anyhow::bail!("line numbers must be >= 1");
    }

    let prev = parts.last().copied().unwrap_or_default().trim();
    let prev_num = prev.parse::<u64>().ok();

    let (path, start_line, end_line) = match prev_num {
        Some(prev_num) => {
            if prev_num == 0 {
                anyhow::bail!("line numbers must be >= 1");
            }
            parts.pop();
            let path = parts.join(":").trim().to_string();
            (path, Some(prev_num), Some(last_num))
        }
        None => {
            let path = parts.join(":").trim().to_string();
            (path, Some(last_num), None)
        }
    };

    if path.is_empty() {
        anyhow::bail!("@file path must be non-empty");
    }
    if let (Some(start), Some(end)) = (start_line, end_line) {
        if end < start {
            anyhow::bail!("end_line must be >= start_line");
        }
    }

    Ok((path, start_line, end_line))
}

impl App {
    async fn connect(cli: &Cli) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        let pm_root = cli
            .pm_root
            .clone()
            .or_else(|| std::env::var_os("CODE_PM_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| cwd.join(".codepm_data"));

        let server = cli.app_server.clone().unwrap_or_else(|| {
            default_app_server_path().unwrap_or_else(|| PathBuf::from("pm-app-server"))
        });

        let socket_path = pm_root.join("daemon.sock");

        let mut rpc = match pm_jsonrpc::Client::connect_unix(&socket_path).await {
            Ok(client) => client,
            Err(_) => {
                let mut argv: Vec<OsString> = Vec::new();
                argv.push("--pm-root".into());
                argv.push(pm_root.into_os_string());
                for path in &cli.execpolicy_rules {
                    argv.push("--execpolicy-rules".into());
                    argv.push(path.clone().into_os_string());
                }
                pm_jsonrpc::Client::spawn(server, argv).await?
            }
        };
        let _ = rpc.request("initialize", serde_json::json!({})).await?;
        let _ = rpc.request("initialized", serde_json::json!({})).await?;
        let notifications = rpc.take_notifications();
        Ok(Self { rpc, notifications })
    }

    async fn rpc(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        Ok(self.rpc.request(method, params).await?)
    }

    fn take_notifications(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pm_jsonrpc::Notification>> {
        self.notifications.take()
    }

    fn rpc_handle(&self) -> pm_jsonrpc::ClientHandle {
        self.rpc.handle()
    }

    async fn thread_start(&mut self, cwd: Option<String>) -> anyhow::Result<Value> {
        self.rpc("thread/start", serde_json::json!({ "cwd": cwd }))
            .await
    }

    async fn thread_resume(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/resume",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_fork(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc("thread/fork", serde_json::json!({ "thread_id": thread_id }))
            .await
    }

    async fn thread_spawn(
        &mut self,
        thread_id: ThreadId,
        input: String,
        model: Option<String>,
        openai_base_url: Option<String>,
    ) -> anyhow::Result<Value> {
        #[derive(Debug, Deserialize)]
        struct ForkResult {
            thread_id: ThreadId,
            log_path: String,
            last_seq: u64,
        }

        let forked = self.thread_fork(thread_id).await?;
        let forked: ForkResult = serde_json::from_value(forked).context("parse thread/fork")?;

        if model.is_some() || openai_base_url.is_some() {
            let _ = self
                .rpc(
                    "thread/configure",
                    serde_json::json!({
                        "thread_id": forked.thread_id,
                        "approval_policy": null,
                        "sandbox_policy": null,
                        "mode": null,
                        "model": model,
                        "openai_base_url": openai_base_url,
                    }),
                )
                .await?;
        }

        let turn_id = self
            .turn_start(
                forked.thread_id,
                input,
                Some(pm_protocol::TurnPriority::Background),
            )
            .await?;
        Ok(serde_json::json!({
            "thread_id": forked.thread_id,
            "turn_id": turn_id,
            "log_path": forked.log_path,
            "last_seq": forked.last_seq,
        }))
    }

    async fn thread_archive(
        &mut self,
        thread_id: ThreadId,
        force: bool,
        reason: Option<String>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/archive",
            serde_json::json!({
                "thread_id": thread_id,
                "force": force,
                "reason": reason,
            }),
        )
        .await
    }

    async fn thread_unarchive(
        &mut self,
        thread_id: ThreadId,
        reason: Option<String>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/unarchive",
            serde_json::json!({
                "thread_id": thread_id,
                "reason": reason,
            }),
        )
        .await
    }

    async fn thread_pause(
        &mut self,
        thread_id: ThreadId,
        reason: Option<String>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/pause",
            serde_json::json!({
                "thread_id": thread_id,
                "reason": reason,
            }),
        )
        .await
    }

    async fn thread_unpause(
        &mut self,
        thread_id: ThreadId,
        reason: Option<String>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/unpause",
            serde_json::json!({
                "thread_id": thread_id,
                "reason": reason,
            }),
        )
        .await
    }

    async fn thread_delete(&mut self, thread_id: ThreadId, force: bool) -> anyhow::Result<Value> {
        self.rpc(
            "thread/delete",
            serde_json::json!({ "thread_id": thread_id, "force": force }),
        )
        .await
    }

    async fn thread_clear_artifacts(
        &mut self,
        thread_id: ThreadId,
        force: bool,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/clear_artifacts",
            serde_json::json!({ "thread_id": thread_id, "force": force }),
        )
        .await
    }

    async fn thread_disk_usage(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/disk_usage",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_disk_report(
        &mut self,
        thread_id: ThreadId,
        top_files: Option<usize>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/disk_report",
            serde_json::json!({ "thread_id": thread_id, "top_files": top_files }),
        )
        .await
    }

    async fn thread_diff(
        &mut self,
        thread_id: ThreadId,
        approval_id: Option<ApprovalId>,
        max_bytes: Option<u64>,
        wait_seconds: Option<u64>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "thread/diff",
                serde_json::json!({
                    "thread_id": thread_id,
                    "approval_id": approval_id,
                    "max_bytes": max_bytes,
                    "wait_seconds": wait_seconds,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("thread/diff", &v)?;
        Ok(v)
    }

    async fn thread_patch(
        &mut self,
        thread_id: ThreadId,
        approval_id: Option<ApprovalId>,
        max_bytes: Option<u64>,
        wait_seconds: Option<u64>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "thread/patch",
                serde_json::json!({
                    "thread_id": thread_id,
                    "approval_id": approval_id,
                    "max_bytes": max_bytes,
                    "wait_seconds": wait_seconds,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("thread/patch", &v)?;
        Ok(v)
    }

    async fn thread_hook_run(
        &mut self,
        thread_id: ThreadId,
        hook: CliWorkspaceHookName,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<Value> {
        let hook = match hook {
            CliWorkspaceHookName::Setup => "setup",
            CliWorkspaceHookName::Run => "run",
            CliWorkspaceHookName::Archive => "archive",
        };
        let v = self
            .rpc(
                "thread/hook_run",
                serde_json::json!({
                    "thread_id": thread_id,
                    "hook": hook,
                    "approval_id": approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("thread/hook_run", &v)?;
        Ok(v)
    }

    async fn thread_events(
        &mut self,
        thread_id: ThreadId,
        since_seq: u64,
        max_events: Option<usize>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/events",
            serde_json::json!({
                "thread_id": thread_id,
                "since_seq": since_seq,
                "max_events": max_events,
            }),
        )
        .await
    }

    async fn thread_loaded(&mut self) -> anyhow::Result<Value> {
        self.rpc("thread/loaded", serde_json::json!({})).await
    }

    async fn thread_list(&mut self) -> anyhow::Result<Value> {
        self.rpc("thread/list", serde_json::json!({})).await
    }

    async fn thread_list_meta(&mut self, include_archived: bool) -> anyhow::Result<Value> {
        self.rpc(
            "thread/list_meta",
            serde_json::json!({ "include_archived": include_archived }),
        )
        .await
    }

    async fn thread_attention(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/attention",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_state(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/state",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_config_explain(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/config/explain",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn thread_models(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc("thread/models", serde_json::json!({ "thread_id": thread_id }))
            .await
    }

    async fn checkpoint_create(
        &mut self,
        thread_id: ThreadId,
        label: Option<String>,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "thread/checkpoint/create",
            serde_json::json!({ "thread_id": thread_id, "label": label }),
        )
        .await
    }

    async fn checkpoint_list(&mut self, thread_id: ThreadId) -> anyhow::Result<Value> {
        self.rpc(
            "thread/checkpoint/list",
            serde_json::json!({ "thread_id": thread_id }),
        )
        .await
    }

    async fn checkpoint_restore(
        &mut self,
        thread_id: ThreadId,
        checkpoint_id: pm_protocol::CheckpointId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "thread/checkpoint/restore",
                serde_json::json!({
                    "thread_id": thread_id,
                    "checkpoint_id": checkpoint_id,
                    "approval_id": approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("thread/checkpoint/restore", &v)?;
        Ok(v)
    }

    async fn thread_configure(&mut self, args: ThreadConfigureArgs) -> anyhow::Result<()> {
        let approval_policy: Option<ApprovalPolicy> = args.approval_policy.map(Into::into);
        let sandbox_policy: Option<SandboxPolicy> = args.sandbox_policy.map(Into::into);
        let sandbox_network_access: Option<pm_protocol::SandboxNetworkAccess> =
            args.sandbox_network_access.map(Into::into);
        let _ = self
            .rpc(
                "thread/configure",
                serde_json::json!({
                    "thread_id": args.thread_id,
                    "approval_policy": approval_policy,
                    "sandbox_policy": sandbox_policy,
                    "sandbox_writable_roots": args.sandbox_writable_roots,
                    "sandbox_network_access": sandbox_network_access,
                    "mode": args.mode,
                    "model": args.model,
                    "openai_base_url": args.openai_base_url,
                }),
            )
            .await?;
        Ok(())
    }

    async fn turn_start(
        &mut self,
        thread_id: ThreadId,
        input: String,
        priority: Option<pm_protocol::TurnPriority>,
    ) -> anyhow::Result<TurnId> {
        let (input, context_refs, attachments) = split_special_directives(&input)?;
        let v = self
            .rpc(
                "turn/start",
                serde_json::json!({
                    "thread_id": thread_id,
                    "input": input,
                    "context_refs": context_refs,
                    "attachments": attachments,
                    "priority": priority,
                }),
            )
            .await?;
        serde_json::from_value(v["turn_id"].clone()).context("turn_id missing in result")
    }

    async fn turn_interrupt(
        &mut self,
        thread_id: ThreadId,
        turn_id: TurnId,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let _ = self
            .rpc(
                "turn/interrupt",
                serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "reason": reason,
                }),
            )
            .await?;
        Ok(())
    }

    async fn thread_subscribe(
        &mut self,
        thread_id: ThreadId,
        since_seq: u64,
        max_events: Option<usize>,
        wait_ms: Option<u64>,
    ) -> anyhow::Result<SubscribeResponse> {
        let v = self
            .rpc(
                "thread/subscribe",
                serde_json::json!({
                    "thread_id": thread_id,
                    "since_seq": since_seq,
                    "max_events": max_events,
                    "wait_ms": wait_ms,
                }),
            )
            .await?;
        Ok(serde_json::from_value(v)?)
    }

    async fn approval_list(
        &mut self,
        thread_id: ThreadId,
        include_decided: bool,
    ) -> anyhow::Result<Value> {
        self.rpc(
            "approval/list",
            serde_json::json!({
                "thread_id": thread_id,
                "include_decided": include_decided,
            }),
        )
        .await
    }

    async fn approval_decide(
        &mut self,
        thread_id: ThreadId,
        approval_id: ApprovalId,
        decision: ApprovalDecision,
        remember: bool,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let _ = self
            .rpc(
                "approval/decide",
                serde_json::json!({
                    "thread_id": thread_id,
                    "approval_id": approval_id,
                    "decision": decision,
                    "remember": remember,
                    "reason": reason,
                }),
            )
            .await?;
        Ok(())
    }

    async fn process_list(&mut self, thread_id: Option<ThreadId>) -> anyhow::Result<Value> {
        self.rpc(
            "process/list",
            serde_json::json!({
                "thread_id": thread_id,
            }),
        )
        .await
    }

    async fn process_inspect(
        &mut self,
        process_id: ProcessId,
        max_lines: Option<usize>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
            "process/inspect",
            serde_json::json!({
                "process_id": process_id,
                "max_lines": max_lines,
                "approval_id": approval_id,
            }),
            )
            .await?;
        ensure_approval_and_denial_handled("process/inspect", &v)?;
        Ok(v)
    }

    async fn process_tail(
        &mut self,
        process_id: ProcessId,
        stderr: bool,
        max_lines: Option<usize>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<String> {
        let stream = if stderr { "stderr" } else { "stdout" };
        let v = self
            .rpc(
                "process/tail",
                serde_json::json!({
                    "process_id": process_id,
                    "stream": stream,
                    "max_lines": max_lines,
                    "approval_id": approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("process/tail", &v)?;
        Ok(v["text"].as_str().unwrap_or("").to_string())
    }

    async fn process_follow(
        &mut self,
        process_id: ProcessId,
        stderr: bool,
        since_offset: u64,
        max_bytes: Option<u64>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<(String, u64, bool)> {
        let stream = if stderr { "stderr" } else { "stdout" };
        let v = self
            .rpc(
                "process/follow",
                serde_json::json!({
                    "process_id": process_id,
                    "stream": stream,
                    "since_offset": since_offset,
                    "max_bytes": max_bytes,
                    "approval_id": approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("process/follow", &v)?;

        let text = v["text"].as_str().unwrap_or("").to_string();
        let next_offset = v["next_offset"].as_u64().unwrap_or(since_offset);
        let eof = v["eof"].as_bool().unwrap_or(true);
        Ok((text, next_offset, eof))
    }

    async fn process_status(&mut self, process_id: ProcessId) -> anyhow::Result<String> {
        let v = self.process_inspect(process_id, Some(0), None).await?;
        Ok(v["process"]["status"]
            .as_str()
            .unwrap_or("unknown")
            .to_string())
    }

    async fn process_kill(
        &mut self,
        process_id: ProcessId,
        reason: Option<String>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<()> {
        let v = self
            .rpc(
                "process/kill",
                serde_json::json!({
                    "process_id": process_id,
                    "reason": reason,
                    "approval_id": approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("process/kill", &v)?;
        Ok(())
    }

    async fn process_interrupt(
        &mut self,
        process_id: ProcessId,
        reason: Option<String>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<()> {
        let v = self
            .rpc(
                "process/interrupt",
                serde_json::json!({
                    "process_id": process_id,
                    "reason": reason,
                    "approval_id": approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("process/interrupt", &v)?;
        Ok(())
    }

    async fn repo_search(&mut self, req: RepoSearchRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "repo/search",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                    "root": req.root,
                    "query": req.query,
                    "is_regex": req.is_regex,
                    "include_glob": req.include_glob,
                    "max_matches": req.max_matches,
                    "max_bytes_per_file": req.max_bytes_per_file,
                    "max_files": req.max_files,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("repo/search", &v)?;
        Ok(v)
    }

    async fn repo_index(&mut self, req: RepoIndexRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "repo/index",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                    "root": req.root,
                    "include_glob": req.include_glob,
                    "max_files": req.max_files,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("repo/index", &v)?;
        Ok(v)
    }

    async fn repo_symbols(&mut self, req: RepoSymbolsRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "repo/symbols",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                    "root": req.root,
                    "include_glob": req.include_glob,
                    "max_files": req.max_files,
                    "max_bytes_per_file": req.max_bytes_per_file,
                    "max_symbols": req.max_symbols,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("repo/symbols", &v)?;
        Ok(v)
    }

    async fn mcp_list_servers(&mut self, req: McpListServersRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "mcp/list_servers",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("mcp/list_servers", &v)?;
        Ok(v)
    }

    async fn mcp_list_tools(&mut self, req: McpListToolsRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "mcp/list_tools",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                    "server": req.server,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("mcp/list_tools", &v)?;
        Ok(v)
    }

    async fn mcp_list_resources(&mut self, req: McpListResourcesRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "mcp/list_resources",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                    "server": req.server,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("mcp/list_resources", &v)?;
        Ok(v)
    }

    async fn mcp_call(&mut self, req: McpCallRequest) -> anyhow::Result<Value> {
        let v = self
            .rpc(
                "mcp/call",
                serde_json::json!({
                    "thread_id": req.thread_id,
                    "approval_id": req.approval_id,
                    "server": req.server,
                    "tool": req.tool,
                    "arguments": req.arguments,
                }),
            )
            .await?;
        ensure_approval_and_denial_handled("mcp/call", &v)?;
        Ok(v)
    }

    async fn artifact_list(
        &mut self,
        thread_id: ThreadId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
            "artifact/list",
            serde_json::json!({
                "thread_id": thread_id,
                "approval_id": approval_id,
            }),
            )
            .await?;
        ensure_approval_and_denial_handled("artifact/list", &v)?;
        Ok(v)
    }

    async fn artifact_read(
        &mut self,
        thread_id: ThreadId,
        artifact_id: pm_protocol::ArtifactId,
        max_bytes: Option<u64>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
            "artifact/read",
            serde_json::json!({
                "thread_id": thread_id,
                "artifact_id": artifact_id,
                "max_bytes": max_bytes,
                "approval_id": approval_id,
            }),
            )
            .await?;
        ensure_approval_and_denial_handled("artifact/read", &v)?;
        Ok(v)
    }

    async fn artifact_delete(
        &mut self,
        thread_id: ThreadId,
        artifact_id: pm_protocol::ArtifactId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<Value> {
        let v = self
            .rpc(
            "artifact/delete",
            serde_json::json!({
                "thread_id": thread_id,
                "artifact_id": artifact_id,
                "approval_id": approval_id,
            }),
            )
            .await?;
        ensure_approval_and_denial_handled("artifact/delete", &v)?;
        Ok(v)
    }
}

fn default_app_server_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join(app_server_exe_name());
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

fn app_server_exe_name() -> &'static str {
    if cfg!(windows) {
        "pm-app-server.exe"
    } else {
        "pm-app-server"
    }
}

fn ensure_approval_and_denial_handled(action: &str, value: &Value) -> anyhow::Result<()> {
    let Some(obj) = value.as_object() else {
        return Ok(());
    };

    if obj
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let thread_id = obj
            .get("thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing thread_id>");
        let approval_id = obj
            .get("approval_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing approval_id>");
        anyhow::bail!(
            "{action} needs approval: pm approval decide --thread-id {thread_id} --approval-id {approval_id} --approve (then re-run with --approval-id {approval_id})"
        );
    }

    if obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false) {
        let detail = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
        anyhow::bail!("{action} denied: {detail}");
    }

    Ok(())
}

#[cfg(test)]
mod special_directives_tests {
    use super::*;

    #[test]
    fn split_special_directives_noop_without_directives() -> anyhow::Result<()> {
        let input = "\n\nhello\nworld\n";
        let (remaining, refs, attachments) = split_special_directives(input)?;
        assert_eq!(remaining, input);
        assert!(refs.is_empty());
        assert!(attachments.is_empty());
        Ok(())
    }

    #[test]
    fn split_special_directives_parses_file_and_diff() -> anyhow::Result<()> {
        let input = "@file crates/core/src/redaction.rs:1:3\n@diff\n\nplease help\n";
        let (remaining, refs, attachments) = split_special_directives(input)?;
        assert_eq!(remaining, "please help");
        assert_eq!(refs.len(), 2);
        assert!(attachments.is_empty());
        assert!(matches!(
            &refs[0],
            pm_protocol::ContextRef::File(pm_protocol::ContextRefFile {
                path,
                start_line: Some(1),
                end_line: Some(3),
                ..
            }) if path == "crates/core/src/redaction.rs"
        ));
        assert!(matches!(&refs[1], pm_protocol::ContextRef::Diff(_)));
        Ok(())
    }

    #[test]
    fn split_special_directives_rejects_diff_args() {
        let err = split_special_directives("@diff nope\nx").unwrap_err();
        assert!(err.to_string().contains("@diff"));
    }

    #[test]
    fn split_special_directives_rejects_file_without_path() {
        let err = split_special_directives("@file\nx").unwrap_err();
        assert!(err.to_string().contains("@file"));
    }

    #[test]
    fn split_special_directives_parses_image_and_pdf() -> anyhow::Result<()> {
        let input = "@image assets/example.png\n@pdf https://example.com/file.pdf\n\nhello";
        let (remaining, refs, attachments) = split_special_directives(input)?;
        assert_eq!(remaining, "hello");
        assert!(refs.is_empty());
        assert!(matches!(
            &attachments[0],
            pm_protocol::TurnAttachment::Image(pm_protocol::TurnAttachmentImage {
                source: pm_protocol::AttachmentSource::Path { path },
                ..
            }) if path == "assets/example.png"
        ));
        assert!(matches!(
            &attachments[1],
            pm_protocol::TurnAttachment::File(pm_protocol::TurnAttachmentFile {
                source: pm_protocol::AttachmentSource::Url { url },
                media_type,
                ..
            }) if url == "https://example.com/file.pdf" && media_type == "application/pdf"
        ));
        Ok(())
    }
}
