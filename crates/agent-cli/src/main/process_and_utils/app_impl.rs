impl App {
    async fn connect(cli: &Cli) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        let agent_root = cli
            .root
            .clone()
            .or_else(|| std::env::var_os("OMNE_AGENT_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| cwd.join(".omne_agent_data"));

        let server = cli.app_server.clone().unwrap_or_else(|| {
            default_app_server_path().unwrap_or_else(|| PathBuf::from("omne-agent-app-server"))
        });

        let init_timeout = std::env::var("OMNE_AGENT_RPC_INIT_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(1500));

        let socket_path = agent_root.join("daemon.sock");
        let bypass_daemon = should_bypass_daemon(&socket_path, &server);

        let build_spawn_argv = || {
            let mut argv: Vec<OsString> = Vec::new();
            argv.push("--root".into());
            argv.push(agent_root.clone().into_os_string());
            for path in &cli.execpolicy_rules {
                argv.push("--execpolicy-rules".into());
                argv.push(path.clone().into_os_string());
            }
            argv
        };

        let (mut rpc, used_daemon) = if bypass_daemon {
            (
                mcp_jsonrpc::Client::spawn(server.clone(), build_spawn_argv()).await?,
                false,
            )
        } else {
            match mcp_jsonrpc::Client::connect_unix(&socket_path).await {
                Ok(client) => (client, true),
                Err(_) => (
                    mcp_jsonrpc::Client::spawn(server.clone(), build_spawn_argv()).await?,
                    false,
                ),
            }
        };

        let init_result = init_rpc(&mut rpc, init_timeout).await;
        if let Err(err) = init_result {
            if used_daemon {
                let mut fresh =
                    mcp_jsonrpc::Client::spawn(server.clone(), build_spawn_argv()).await?;
                init_rpc(&mut fresh, init_timeout).await?;
                rpc = fresh;
            } else {
                return Err(err);
            }
        }
        let notifications = rpc.take_notifications();
        Ok(Self { rpc, notifications })
    }

    async fn rpc(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        Ok(self.rpc.request(method, params).await?)
    }

    fn take_notifications(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<mcp_jsonrpc::Notification>> {
        self.notifications.take()
    }

    fn rpc_handle(&self) -> mcp_jsonrpc::ClientHandle {
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
                Some(omne_agent_protocol::TurnPriority::Background),
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
        checkpoint_id: omne_agent_protocol::CheckpointId,
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
        let sandbox_network_access: Option<omne_agent_protocol::SandboxNetworkAccess> =
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
                    "openai_provider": args.openai_provider,
                    "model": args.model,
                    "openai_base_url": args.openai_base_url,
                    "thinking": args.thinking,
                }),
            )
            .await?;
        Ok(())
    }

    async fn turn_start(
        &mut self,
        thread_id: ThreadId,
        input: String,
        priority: Option<omne_agent_protocol::TurnPriority>,
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
        artifact_id: omne_agent_protocol::ArtifactId,
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
        artifact_id: omne_agent_protocol::ArtifactId,
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
