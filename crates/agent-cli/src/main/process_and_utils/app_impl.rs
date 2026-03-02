macro_rules! define_rpc_value_passthrough {
    ($fn_name:ident, $method:literal, $params_ty:path) => {
        async fn $fn_name(&mut self, params: $params_ty) -> anyhow::Result<Value> {
            self.rpc_with_serialized_params($method, params).await
        }
    };
}

macro_rules! define_rpc_parsed_passthrough {
    ($fn_name:ident, $method:literal, $params_ty:path, $response_ty:path, $parser:path) => {
        async fn $fn_name(&mut self, params: $params_ty) -> anyhow::Result<$response_ty> {
            self.rpc_parsed($method, params, $parser).await
        }
    };
}

impl App {
    async fn connect(cli: &Cli) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;
        let omne_root = cli
            .omne_root
            .clone()
            .or_else(|| std::env::var_os("OMNE_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| cwd.join(".omne_data"));

        let server = cli.app_server.clone().unwrap_or_else(|| {
            default_app_server_path().unwrap_or_else(|| PathBuf::from("omne-app-server"))
        });

        let init_timeout = std::env::var("OMNE_RPC_INIT_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| Duration::from_millis(1500));

        let socket_path = omne_root.join("daemon.sock");
        let bypass_daemon = should_bypass_daemon(&socket_path, &server);

        let build_spawn_argv = || {
            let mut argv: Vec<OsString> = Vec::new();
            argv.push("--omne-root".into());
            argv.push(omne_root.clone().into_os_string());
            for path in &cli.execpolicy_rules {
                argv.push("--execpolicy-rules".into());
                argv.push(path.clone().into_os_string());
            }
            argv
        };

        let (mut rpc, used_daemon) = if bypass_daemon {
            (
                omne_jsonrpc::Client::spawn(server.clone(), build_spawn_argv()).await?,
                false,
            )
        } else {
            match omne_jsonrpc::Client::connect_unix(&socket_path).await {
                Ok(client) => (client, true),
                Err(_) => (
                    omne_jsonrpc::Client::spawn(server.clone(), build_spawn_argv()).await?,
                    false,
                ),
            }
        };

        let init_result = init_rpc(&mut rpc, init_timeout).await;
        if let Err(err) = init_result {
            if used_daemon {
                let mut fresh =
                    omne_jsonrpc::Client::spawn(server.clone(), build_spawn_argv()).await?;
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

    async fn rpc_with_serialized_params<T>(
        &mut self,
        method: &'static str,
        params: T,
    ) -> anyhow::Result<Value>
    where
        T: serde::Serialize,
    {
        let params =
            serde_json::to_value(params).with_context(|| format!("serialize {method} params"))?;
        self.rpc(method, params).await
    }

    async fn rpc_typed<P, R>(&mut self, method: &'static str, params: P) -> anyhow::Result<R>
    where
        P: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let value = self.rpc_with_serialized_params(method, params).await?;
        serde_json::from_value(value).with_context(|| format!("parse {method} response"))
    }

    async fn rpc_parsed<P, R, F>(
        &mut self,
        method: &'static str,
        params: P,
        parse: F,
    ) -> anyhow::Result<R>
    where
        P: serde::Serialize,
        F: FnOnce(&str, Value) -> anyhow::Result<R>,
    {
        let value = self.rpc_with_serialized_params(method, params).await?;
        parse(method, value)
    }

    fn ensure_ok(action: &str, ok: bool) -> anyhow::Result<()> {
        if ok {
            return Ok(());
        }
        anyhow::bail!("{action} failed: expected ok=true");
    }

    define_rpc_value_passthrough!(
        rpc_artifact_list_value,
        "artifact/list",
        omne_app_server_protocol::ArtifactListParams
    );
    define_rpc_value_passthrough!(
        rpc_artifact_read_value,
        "artifact/read",
        omne_app_server_protocol::ArtifactReadParams
    );
    define_rpc_value_passthrough!(
        rpc_artifact_versions_value,
        "artifact/versions",
        omne_app_server_protocol::ArtifactVersionsParams
    );
    define_rpc_value_passthrough!(
        rpc_process_inspect_value,
        "process/inspect",
        omne_app_server_protocol::ProcessInspectParams
    );
    define_rpc_value_passthrough!(
        rpc_process_kill_value,
        "process/kill",
        omne_app_server_protocol::ProcessKillParams
    );
    define_rpc_value_passthrough!(
        rpc_process_interrupt_value,
        "process/interrupt",
        omne_app_server_protocol::ProcessInterruptParams
    );

    fn take_notifications(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<omne_jsonrpc::Notification>> {
        self.notifications.take()
    }

    fn rpc_handle(&self) -> omne_jsonrpc::ClientHandle {
        self.rpc.handle()
    }

    async fn thread_start(
        &mut self,
        cwd: Option<String>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadStartResponse> {
        self.rpc_typed(
            "thread/start",
            omne_app_server_protocol::ThreadStartParams { cwd },
        )
        .await
    }

    async fn thread_resume(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadHandleResponse> {
        self.rpc_typed(
            "thread/resume",
            omne_app_server_protocol::ThreadResumeParams { thread_id },
        )
        .await
    }

    async fn thread_fork(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadHandleResponse> {
        self.rpc_typed(
            "thread/fork",
            omne_app_server_protocol::ThreadForkParams {
                thread_id,
                cwd: None,
            },
        )
        .await
    }

    async fn thread_spawn(
        &mut self,
        thread_id: ThreadId,
        input: String,
        model: Option<String>,
        openai_base_url: Option<String>,
    ) -> anyhow::Result<ThreadSpawnResponse> {
        let forked = self.thread_fork(thread_id).await?;

        if model.is_some() || openai_base_url.is_some() {
            self.thread_configure_rpc(omne_app_server_protocol::ThreadConfigureParams {
                thread_id: forked.thread_id,
                approval_policy: None,
                sandbox_policy: None,
                sandbox_writable_roots: None,
                sandbox_network_access: None,
                mode: None,
                model,
                thinking: None,
                show_thinking: None,
                openai_base_url,
                allowed_tools: None,
                execpolicy_rules: None,
            })
            .await?;
        }

        let turn_id = self
            .turn_start(
                forked.thread_id,
                input,
                Some(omne_protocol::TurnPriority::Background),
            )
            .await?;
        Ok(ThreadSpawnResponse {
            thread_id: forked.thread_id,
            turn_id,
            log_path: forked.log_path,
            last_seq: forked.last_seq,
        })
    }

    async fn thread_archive(
        &mut self,
        thread_id: ThreadId,
        force: bool,
        reason: Option<String>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadArchiveResponse> {
        self.rpc_typed(
            "thread/archive",
            omne_app_server_protocol::ThreadArchiveParams {
                thread_id,
                force,
                reason,
            },
        )
        .await
    }

    async fn thread_unarchive(
        &mut self,
        thread_id: ThreadId,
        reason: Option<String>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadUnarchiveResponse> {
        self.rpc_typed(
            "thread/unarchive",
            omne_app_server_protocol::ThreadUnarchiveParams { thread_id, reason },
        )
        .await
    }

    async fn thread_pause(
        &mut self,
        thread_id: ThreadId,
        reason: Option<String>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadPauseResponse> {
        self.rpc_typed(
            "thread/pause",
            omne_app_server_protocol::ThreadPauseParams { thread_id, reason },
        )
        .await
    }

    async fn thread_unpause(
        &mut self,
        thread_id: ThreadId,
        reason: Option<String>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadUnpauseResponse> {
        self.rpc_typed(
            "thread/unpause",
            omne_app_server_protocol::ThreadUnpauseParams { thread_id, reason },
        )
        .await
    }

    async fn thread_delete(
        &mut self,
        thread_id: ThreadId,
        force: bool,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadDeleteResponse> {
        self.rpc_typed(
            "thread/delete",
            omne_app_server_protocol::ThreadDeleteParams { thread_id, force },
        )
        .await
    }

    async fn thread_clear_artifacts(
        &mut self,
        thread_id: ThreadId,
        force: bool,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadClearArtifactsResponse> {
        self.rpc_typed(
            "thread/clear_artifacts",
            omne_app_server_protocol::ThreadClearArtifactsParams { thread_id, force },
        )
        .await
    }

    async fn thread_disk_usage(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadDiskUsageResponse> {
        self.rpc_typed(
            "thread/disk_usage",
            omne_app_server_protocol::ThreadDiskUsageParams { thread_id },
        )
        .await
    }

    async fn thread_disk_report(
        &mut self,
        thread_id: ThreadId,
        top_files: Option<usize>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadDiskReportResponse> {
        self.rpc_typed(
            "thread/disk_report",
            omne_app_server_protocol::ThreadDiskReportParams {
                thread_id,
                top_files,
            },
        )
        .await
    }

    async fn thread_diff(
        &mut self,
        thread_id: ThreadId,
        approval_id: Option<ApprovalId>,
        max_bytes: Option<u64>,
        wait_seconds: Option<u64>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse> {
        self.thread_git_snapshot_rpc(
            "thread/diff",
            omne_app_server_protocol::ThreadDiffParams {
                thread_id,
                turn_id: None,
                approval_id,
                max_bytes,
                wait_seconds,
            },
        )
        .await
    }

    async fn thread_patch(
        &mut self,
        thread_id: ThreadId,
        approval_id: Option<ApprovalId>,
        max_bytes: Option<u64>,
        wait_seconds: Option<u64>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse> {
        self.thread_git_snapshot_rpc(
            "thread/patch",
            omne_app_server_protocol::ThreadPatchParams {
                thread_id,
                turn_id: None,
                approval_id,
                max_bytes,
                wait_seconds,
            },
        )
        .await
    }

    async fn thread_git_snapshot_rpc<T>(
        &mut self,
        method: &'static str,
        params: T,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadGitSnapshotRpcResponse>
    where
        T: serde::Serialize,
    {
        self.rpc_parsed(method, params, parse_thread_git_snapshot_rpc_response)
            .await
    }

    async fn thread_hook_run(
        &mut self,
        thread_id: ThreadId,
        hook: CliWorkspaceHookName,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadHookRunResponse> {
        let hook = match hook {
            CliWorkspaceHookName::Setup => omne_app_server_protocol::WorkspaceHookName::Setup,
            CliWorkspaceHookName::Run => omne_app_server_protocol::WorkspaceHookName::Run,
            CliWorkspaceHookName::Archive => omne_app_server_protocol::WorkspaceHookName::Archive,
        };
        self.rpc_parsed(
            "thread/hook_run",
            omne_app_server_protocol::ThreadHookRunParams {
                thread_id,
                turn_id: None,
                approval_id,
                hook,
            },
            parse_thread_hook_run_rpc_response,
        )
        .await
    }

    async fn thread_events(
        &mut self,
        thread_id: ThreadId,
        since_seq: u64,
        max_events: Option<usize>,
        kinds: Option<Vec<omne_protocol::ThreadEventKindTag>>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadEventsResponse> {
        self.rpc_typed(
            "thread/events",
            omne_app_server_protocol::ThreadEventsParams {
                thread_id,
                since_seq,
                max_events,
                kinds,
            },
        )
        .await
    }

    async fn thread_loaded(
        &mut self,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadListResponse> {
        self.rpc_typed(
            "thread/loaded",
            omne_app_server_protocol::ThreadLoadedParams {},
        )
        .await
    }

    async fn thread_list(&mut self) -> anyhow::Result<omne_app_server_protocol::ThreadListResponse> {
        self.rpc_typed("thread/list", omne_app_server_protocol::ThreadListParams {})
            .await
    }

    async fn thread_list_meta(
        &mut self,
        include_archived: bool,
        include_attention_markers: bool,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadListMetaResponse> {
        self.rpc_typed(
            "thread/list_meta",
            omne_app_server_protocol::ThreadListMetaParams {
                include_archived,
                include_attention_markers,
            },
        )
        .await
    }

    async fn thread_attention(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadAttentionResponse> {
        self.rpc_typed(
            "thread/attention",
            omne_app_server_protocol::ThreadAttentionParams { thread_id },
        )
        .await
    }

    async fn thread_state(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadStateResponse> {
        self.rpc_typed(
            "thread/state",
            omne_app_server_protocol::ThreadStateParams { thread_id },
        )
        .await
    }

    async fn thread_usage(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadUsageResponse> {
        self.rpc_typed(
            "thread/usage",
            omne_app_server_protocol::ThreadUsageParams { thread_id },
        )
        .await
    }

    async fn thread_config_explain(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<ThreadConfigExplainResponse> {
        self.rpc_typed(
            "thread/config/explain",
            omne_app_server_protocol::ThreadConfigExplainParams {
                thread_id,
            },
        )
        .await
    }

    async fn thread_models(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadModelsResponse> {
        self.rpc_typed(
            "thread/models",
            omne_app_server_protocol::ThreadModelsParams { thread_id },
        )
        .await
    }

    async fn checkpoint_create(
        &mut self,
        thread_id: ThreadId,
        label: Option<String>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadCheckpointCreateResponse> {
        self.rpc_typed(
            "thread/checkpoint/create",
            omne_app_server_protocol::ThreadCheckpointCreateParams { thread_id, label },
        )
        .await
    }

    async fn checkpoint_list(
        &mut self,
        thread_id: ThreadId,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadCheckpointListResponse> {
        self.rpc_typed(
            "thread/checkpoint/list",
            omne_app_server_protocol::ThreadCheckpointListParams { thread_id },
        )
        .await
    }

    async fn checkpoint_restore(
        &mut self,
        thread_id: ThreadId,
        checkpoint_id: omne_protocol::CheckpointId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ThreadCheckpointRestoreResponse> {
        self.rpc_parsed(
            "thread/checkpoint/restore",
            omne_app_server_protocol::ThreadCheckpointRestoreParams {
                thread_id,
                checkpoint_id,
                turn_id: None,
                approval_id,
            },
            parse_checkpoint_restore_rpc_response,
        )
        .await
    }

    async fn thread_configure(&mut self, args: ThreadConfigureArgs) -> anyhow::Result<()> {
        if args.clear_allowed_tools && args.allowed_tools.is_some() {
            anyhow::bail!(
                "conflicting options: --allowed-tools cannot be used with --clear-allowed-tools"
            );
        }
        if args.clear_execpolicy_rules && args.execpolicy_rules.is_some() {
            anyhow::bail!(
                "conflicting options: --execpolicy-rules cannot be used with --clear-execpolicy-rules"
            );
        }
        let approval_policy: Option<ApprovalPolicy> = args.approval_policy.map(Into::into);
        let sandbox_policy: Option<SandboxPolicy> = args.sandbox_policy.map(Into::into);
        let sandbox_network_access: Option<omne_protocol::SandboxNetworkAccess> =
            args.sandbox_network_access.map(Into::into);
        let allowed_tools = if args.clear_allowed_tools {
            Some(None)
        } else {
            args.allowed_tools
                .map(normalize_string_list)
                .map(Some)
        };
        let execpolicy_rules = if args.clear_execpolicy_rules {
            Some(Vec::<String>::new())
        } else {
            args.execpolicy_rules.map(normalize_string_list)
        };
        self.thread_configure_rpc(omne_app_server_protocol::ThreadConfigureParams {
            thread_id: args.thread_id,
            approval_policy,
            sandbox_policy,
            sandbox_writable_roots: args.sandbox_writable_roots,
            sandbox_network_access,
            mode: args.mode,
            model: args.model,
            thinking: args.thinking,
            show_thinking: None,
            openai_base_url: args.openai_base_url,
            allowed_tools,
            execpolicy_rules,
        })
        .await
    }

    async fn thread_configure_rpc(
        &mut self,
        params: omne_app_server_protocol::ThreadConfigureParams,
    ) -> anyhow::Result<()> {
        let response: omne_app_server_protocol::ThreadConfigureResponse =
            self.rpc_typed("thread/configure", params).await?;
        Self::ensure_ok("thread/configure", response.ok)
    }

    async fn turn_start(
        &mut self,
        thread_id: ThreadId,
        input: String,
        priority: Option<omne_protocol::TurnPriority>,
    ) -> anyhow::Result<TurnId> {
        let (input, context_refs, attachments, directives) = split_special_directives(&input)?;
        let response: omne_app_server_protocol::TurnStartResponse = self
            .rpc_typed(
                "turn/start",
                omne_app_server_protocol::TurnStartParams {
                    thread_id,
                    input,
                    context_refs: Some(context_refs),
                    attachments: Some(attachments),
                    directives: Some(directives),
                    priority,
                },
            )
            .await?;
        Ok(response.turn_id)
    }

    async fn turn_interrupt(
        &mut self,
        thread_id: ThreadId,
        turn_id: TurnId,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        let response: omne_app_server_protocol::TurnInterruptResponse = self
            .rpc_typed(
                "turn/interrupt",
                omne_app_server_protocol::TurnInterruptParams {
                    thread_id,
                    turn_id,
                    reason,
                },
            )
            .await?;
        Self::ensure_ok("turn/interrupt", response.ok)
    }

    async fn thread_subscribe(
        &mut self,
        thread_id: ThreadId,
        since_seq: u64,
        max_events: Option<usize>,
        wait_ms: Option<u64>,
    ) -> anyhow::Result<SubscribeResponse> {
        self.rpc_typed(
            "thread/subscribe",
            omne_app_server_protocol::ThreadSubscribeParams {
                thread_id,
                since_seq,
                max_events,
                kinds: None,
                wait_ms,
            },
        )
        .await
    }

    async fn approval_list(
        &mut self,
        thread_id: ThreadId,
        include_decided: bool,
    ) -> anyhow::Result<omne_app_server_protocol::ApprovalListResponse> {
        self.rpc_typed(
            "approval/list",
            omne_app_server_protocol::ApprovalListParams {
                thread_id,
                include_decided,
            },
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
    ) -> anyhow::Result<omne_app_server_protocol::ApprovalDecideResponse> {
        self.rpc_typed(
            "approval/decide",
            omne_app_server_protocol::ApprovalDecideParams {
                thread_id,
                approval_id,
                decision,
                remember,
                reason,
            },
        )
        .await
    }

    async fn process_list(
        &mut self,
        thread_id: Option<ThreadId>,
    ) -> anyhow::Result<omne_app_server_protocol::ProcessListResponse> {
        self.rpc_parsed(
            "process/list",
            omne_app_server_protocol::ProcessListParams { thread_id },
            parse_process_rpc_response_typed,
        )
        .await
    }

    define_rpc_parsed_passthrough!(
        process_start,
        "process/start",
        omne_app_server_protocol::ProcessStartParams,
        omne_app_server_protocol::ProcessStartResponse,
        parse_process_rpc_response_typed
    );

    async fn process_inspect(
        &mut self,
        process_id: ProcessId,
        max_lines: Option<usize>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ProcessInspectResponse> {
        self.rpc_parsed(
            "process/inspect",
            omne_app_server_protocol::ProcessInspectParams {
                process_id,
                turn_id: None,
                approval_id,
                max_lines,
            },
            parse_process_rpc_response_typed,
        )
        .await
    }

    async fn process_tail(
        &mut self,
        process_id: ProcessId,
        stderr: bool,
        max_lines: Option<usize>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<String> {
        let stream = if stderr {
            omne_app_server_protocol::ProcessStream::Stderr
        } else {
            omne_app_server_protocol::ProcessStream::Stdout
        };
        let response: omne_app_server_protocol::ProcessTailResponse = self
            .rpc_parsed(
                "process/tail",
                omne_app_server_protocol::ProcessTailParams {
                    process_id,
                    turn_id: None,
                    approval_id,
                    stream,
                    max_lines,
                },
                parse_process_rpc_response_typed,
            )
            .await?;
        Ok(response.text)
    }

    async fn process_follow(
        &mut self,
        process_id: ProcessId,
        stderr: bool,
        since_offset: u64,
        max_bytes: Option<u64>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<(String, u64, bool)> {
        let stream = if stderr {
            omne_app_server_protocol::ProcessStream::Stderr
        } else {
            omne_app_server_protocol::ProcessStream::Stdout
        };
        let response: omne_app_server_protocol::ProcessFollowResponse = self
            .rpc_parsed(
                "process/follow",
                omne_app_server_protocol::ProcessFollowParams {
                    process_id,
                    turn_id: None,
                    approval_id,
                    stream,
                    since_offset,
                    max_bytes,
                },
                parse_process_rpc_response_typed,
            )
            .await?;

        Ok((response.text, response.next_offset, response.eof))
    }

    async fn process_status(&mut self, process_id: ProcessId) -> anyhow::Result<String> {
        let response = self.process_inspect(process_id, Some(0), None).await?;
        let status = match response.process.status {
            omne_app_server_protocol::ProcessStatus::Running => "running",
            omne_app_server_protocol::ProcessStatus::Exited => "exited",
            omne_app_server_protocol::ProcessStatus::Abandoned => "abandoned",
        };
        Ok(status.to_string())
    }

    async fn process_kill(
        &mut self,
        process_id: ProcessId,
        reason: Option<String>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<()> {
        let response: omne_app_server_protocol::ProcessSignalResponse = self
            .rpc_parsed(
                "process/kill",
                omne_app_server_protocol::ProcessKillParams {
                    process_id,
                    turn_id: None,
                    approval_id,
                    reason,
                },
                parse_process_rpc_response_typed,
            )
            .await?;
        Self::ensure_ok("process/kill", response.ok)
    }

    async fn process_interrupt(
        &mut self,
        process_id: ProcessId,
        reason: Option<String>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<()> {
        let response: omne_app_server_protocol::ProcessSignalResponse = self
            .rpc_parsed(
                "process/interrupt",
                omne_app_server_protocol::ProcessInterruptParams {
                    process_id,
                    turn_id: None,
                    approval_id,
                    reason,
                },
                parse_process_rpc_response_typed,
            )
            .await?;
        Self::ensure_ok("process/interrupt", response.ok)
    }

    define_rpc_parsed_passthrough!(
        repo_search,
        "repo/search",
        omne_app_server_protocol::RepoSearchParams,
        omne_app_server_protocol::RepoSearchResponse,
        parse_repo_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        repo_index,
        "repo/index",
        omne_app_server_protocol::RepoIndexParams,
        omne_app_server_protocol::RepoIndexResponse,
        parse_repo_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        repo_symbols,
        "repo/symbols",
        omne_app_server_protocol::RepoSymbolsParams,
        omne_app_server_protocol::RepoSymbolsResponse,
        parse_repo_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        mcp_list_servers,
        "mcp/list_servers",
        omne_app_server_protocol::McpListServersParams,
        McpListServersOrFailedResponse,
        parse_mcp_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        mcp_list_tools,
        "mcp/list_tools",
        omne_app_server_protocol::McpListToolsParams,
        McpActionOrFailedResponse,
        parse_mcp_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        mcp_list_resources,
        "mcp/list_resources",
        omne_app_server_protocol::McpListResourcesParams,
        McpActionOrFailedResponse,
        parse_mcp_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        mcp_call,
        "mcp/call",
        omne_app_server_protocol::McpCallParams,
        McpActionOrFailedResponse,
        parse_mcp_rpc_response_typed
    );
    define_rpc_parsed_passthrough!(
        artifact_write,
        "artifact/write",
        omne_app_server_protocol::ArtifactWriteParams,
        omne_app_server_protocol::ArtifactWriteResponse,
        parse_artifact_rpc_response_typed
    );

    async fn artifact_list(
        &mut self,
        thread_id: ThreadId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ArtifactListResponse> {
        self.rpc_parsed(
            "artifact/list",
            omne_app_server_protocol::ArtifactListParams {
                thread_id,
                turn_id: None,
                approval_id,
            },
            parse_artifact_rpc_response_typed,
        )
        .await
    }

    async fn artifact_read(
        &mut self,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
        version: Option<u32>,
        max_bytes: Option<u64>,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ArtifactReadResponse> {
        self.rpc_parsed(
            "artifact/read",
            omne_app_server_protocol::ArtifactReadParams {
                thread_id,
                turn_id: None,
                approval_id,
                artifact_id,
                version,
                max_bytes,
            },
            parse_artifact_rpc_response_typed,
        )
        .await
    }

    async fn artifact_versions(
        &mut self,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ArtifactVersionsResponse> {
        self.rpc_parsed(
            "artifact/versions",
            omne_app_server_protocol::ArtifactVersionsParams {
                thread_id,
                turn_id: None,
                approval_id,
                artifact_id,
            },
            parse_artifact_rpc_response_typed,
        )
        .await
    }

    async fn artifact_delete(
        &mut self,
        thread_id: ThreadId,
        artifact_id: omne_protocol::ArtifactId,
        approval_id: Option<ApprovalId>,
    ) -> anyhow::Result<omne_app_server_protocol::ArtifactDeleteResponse> {
        self.rpc_parsed(
            "artifact/delete",
            omne_app_server_protocol::ArtifactDeleteParams {
                thread_id,
                turn_id: None,
                approval_id,
                artifact_id,
            },
            parse_artifact_rpc_response_typed,
        )
        .await
    }
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = trimmed.to_string();
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}
