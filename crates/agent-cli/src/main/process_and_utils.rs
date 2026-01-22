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
        pm_protocol::ThreadEventKind::TurnStarted { turn_id, input } => {
            println!("[{ts}] turn started {turn_id}");
            println!("user: {input}");
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
        } => {
            println!(
                "[{ts}] config approval_policy={approval_policy:?} sandbox_policy={sandbox_policy:?} sandbox_writable_roots={sandbox_writable_roots:?} sandbox_network_access={sandbox_network_access:?} mode={} model={} openai_base_url={}",
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
    }
}

struct App {
    rpc: pm_jsonrpc::Client,
    notifications: Option<tokio::sync::mpsc::UnboundedReceiver<pm_jsonrpc::Notification>>,
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
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<pm_jsonrpc::Notification>> {
        self.notifications.take()
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

        let turn_id = self.turn_start(forked.thread_id, input).await?;
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

    async fn turn_start(&mut self, thread_id: ThreadId, input: String) -> anyhow::Result<TurnId> {
        let v = self
            .rpc(
                "turn/start",
                serde_json::json!({ "thread_id": thread_id, "input": input }),
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
