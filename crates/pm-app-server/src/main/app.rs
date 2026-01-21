#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    if let Some(command) = args.command {
        match command {
            CliCommand::GenerateTs(output) => pm_app_server_protocol::generate_ts(&output.out_dir)?,
            CliCommand::GenerateJsonSchema(output) => {
                pm_app_server_protocol::generate_json_schema(&output.out_dir)?
            }
        }
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let pm_root = args
        .pm_root
        .or_else(|| std::env::var_os("CODE_PM_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| cwd.join(".code_pm"));

    let exec_policy = if args.execpolicy_rules.is_empty() {
        pm_execpolicy::Policy::empty()
    } else {
        pm_execpolicy::execpolicycheck::load_policies(&args.execpolicy_rules)?
    };

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    let server = Arc::new(Server {
        cwd,
        out_tx: out_tx.clone(),
        thread_store: ThreadStore::new(PmPaths::new(pm_root)),
        threads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        processes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        disk_warning: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        exec_policy,
    });

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            tokio::task::spawn_local(async move {
                let mut stdout = tokio::io::stdout();
                while let Some(line) = out_rx.recv().await {
                    if stdout.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    if stdout.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
            });

            let stdin = tokio::io::stdin();
            let mut lines = tokio::io::BufReader::new(stdin).lines();

            let mut initialized = false;

            while let Some(line) = lines.next_line().await? {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let request: JsonRpcRequest = match serde_json::from_str(line) {
                    Ok(req) => req,
                    Err(err) => {
                        eprintln!("app-server: invalid json: {err}");
                        continue;
                    }
                };

                let id = request.id.clone();
                let response = match request.method.as_str() {
            "initialize" => {
                if initialized {
                    JsonRpcResponse::err(
                        id,
                        CODE_PM_ALREADY_INITIALIZED,
                        "already initialized",
                        None,
                    )
                } else {
                    initialized = true;
                    JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "server": {
                                "name": "pm-app-server",
                                "version": env!("CARGO_PKG_VERSION"),
                            }
                        }),
                    )
                }
            }
            "initialized" => {
                if initialized {
                    JsonRpcResponse::ok(id, serde_json::json!({ "ok": true }))
                } else {
                    JsonRpcResponse::err(id, CODE_PM_NOT_INITIALIZED, "not initialized", None)
                }
            }
            _ if !initialized => {
                JsonRpcResponse::err(id, CODE_PM_NOT_INITIALIZED, "not initialized", None)
            }
            "thread/start" => match serde_json::from_value::<ThreadStartParams>(request.params) {
                Ok(params) => {
                    let cwd = params
                        .cwd
                        .map(PathBuf::from)
                        .unwrap_or_else(|| server.cwd.clone());
                    match server.thread_store.create_thread(cwd).await {
                        Ok(handle) => {
                            let thread_id = handle.thread_id();
                            let log_path = handle.log_path().display().to_string();
                            let last_seq = handle.last_seq().0;
                            let rt = Arc::new(ThreadRuntime::new(handle, server.out_tx.clone()));
                            server.threads.lock().await.insert(thread_id, rt);

                            JsonRpcResponse::ok(
                                id,
                                serde_json::json!({
                                    "thread_id": thread_id,
                                    "log_path": log_path,
                                    "last_seq": last_seq,
                                }),
                            )
                        }
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    }
                }
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/resume" => match serde_json::from_value::<ThreadResumeParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => {
                        let handle = rt.handle.lock().await;
                        JsonRpcResponse::ok(
                            id,
                            serde_json::json!({
                                "thread_id": handle.thread_id(),
                                "log_path": handle.log_path().display().to_string(),
                                "last_seq": handle.last_seq().0,
                            }),
                        )
                    }
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/fork" => match serde_json::from_value::<ThreadForkParams>(request.params) {
                Ok(params) => match handle_thread_fork(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/archive" => match serde_json::from_value::<ThreadArchiveParams>(request.params) {
                Ok(params) => match handle_thread_archive(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/unarchive" => {
                match serde_json::from_value::<ThreadUnarchiveParams>(request.params) {
                    Ok(params) => match handle_thread_unarchive(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/pause" => match serde_json::from_value::<ThreadPauseParams>(request.params) {
                Ok(params) => match handle_thread_pause(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/unpause" => {
                match serde_json::from_value::<ThreadUnpauseParams>(request.params) {
                    Ok(params) => match handle_thread_unpause(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/delete" => match serde_json::from_value::<ThreadDeleteParams>(request.params) {
                Ok(params) => match handle_thread_delete(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/clear_artifacts" => {
                match serde_json::from_value::<ThreadClearArtifactsParams>(request.params) {
                    Ok(params) => match handle_thread_clear_artifacts(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/list" => match server.thread_store.list_threads().await {
                Ok(threads) => JsonRpcResponse::ok(
                    id,
                    serde_json::json!({
                        "threads": threads,
                    }),
                ),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            "thread/list_meta" => match serde_json::from_value::<ThreadListMetaParams>(request.params)
            {
                Ok(params) => match handle_thread_list_meta(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/loaded" => {
                let mut threads = server
                    .threads
                    .lock()
                    .await
                    .keys()
                    .copied()
                    .collect::<Vec<_>>();
                threads.sort_unstable();
                JsonRpcResponse::ok(
                    id,
                    serde_json::json!({
                        "threads": threads,
                    }),
                )
            }
            "thread/events" => match serde_json::from_value::<ThreadEventsParams>(request.params) {
                Ok(params) => {
                    let since = EventSeq(params.since_seq);
                    match server
                        .thread_store
                        .read_events_since(params.thread_id, since)
                        .await
                    {
                        Ok(Some(mut events)) => {
                            let thread_last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);
                            let mut has_more = false;
                            if let Some(max_events) = params.max_events {
                                let max_events = max_events.clamp(1, 50_000);
                                if events.len() > max_events {
                                    events.truncate(max_events);
                                    has_more = true;
                                }
                            }

                            let last_seq = events.last().map(|e| e.seq.0).unwrap_or(since.0);
                            JsonRpcResponse::ok(
                                id,
                                serde_json::json!({
                                    "events": events,
                                    "last_seq": last_seq,
                                    "thread_last_seq": thread_last_seq,
                                    "has_more": has_more,
                                }),
                            )
                        }
                        Ok(None) => JsonRpcResponse::err(
                            id,
                            JSONRPC_INTERNAL_ERROR,
                            format!("thread not found: {}", params.thread_id),
                            None,
                        ),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    }
                }
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/subscribe" => {
                match serde_json::from_value::<ThreadSubscribeParams>(request.params) {
                    Ok(params) => match handle_thread_subscribe(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/state" => match serde_json::from_value::<ThreadStateParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => {
                        let handle = rt.handle.lock().await;
                        let state = handle.state();
                        let archived_at = state
                            .archived_at
                            .and_then(|ts| ts.format(&Rfc3339).ok());
                        let paused_at = state.paused_at.and_then(|ts| ts.format(&Rfc3339).ok());
                        JsonRpcResponse::ok(
                            id,
                            serde_json::json!({
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
                                "mode": state.mode,
                                "model": state.model,
                                "openai_base_url": state.openai_base_url,
                                "last_seq": handle.last_seq().0,
                                "active_turn_id": state.active_turn_id,
                                "active_turn_interrupt_requested": state.active_turn_interrupt_requested,
                                "last_turn_id": state.last_turn_id,
                                "last_turn_status": state.last_turn_status,
                                "last_turn_reason": state.last_turn_reason,
                            }),
                        )
                    }
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/attention" => {
                match serde_json::from_value::<ThreadAttentionParams>(request.params) {
                    Ok(params) => match handle_thread_attention(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/disk_usage" => {
                match serde_json::from_value::<ThreadDiskUsageParams>(request.params) {
                    Ok(params) => match handle_thread_disk_usage(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/disk_report" => {
                match serde_json::from_value::<ThreadDiskReportParams>(request.params) {
                    Ok(params) => match handle_thread_disk_report(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/hook_run" => match serde_json::from_value::<ThreadHookRunParams>(request.params)
            {
                Ok(params) => match handle_thread_hook_run(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "thread/configure" => {
                match serde_json::from_value::<ThreadConfigureParams>(request.params) {
                    Ok(params) => match handle_thread_configure(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "thread/config/explain" => {
                match serde_json::from_value::<ThreadConfigExplainParams>(request.params) {
                    Ok(params) => match handle_thread_config_explain(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "turn/start" => match serde_json::from_value::<TurnStartParams>(request.params) {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => match rt.start_turn(server.clone(), params.input).await {
                        Ok(turn_id) => JsonRpcResponse::ok(
                            id,
                            serde_json::json!({
                                "turn_id": turn_id,
                            }),
                        ),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "turn/interrupt" => match serde_json::from_value::<TurnInterruptParams>(request.params)
            {
                Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                    Ok(rt) => {
                        let kill_reason = params
                            .reason
                            .clone()
                            .or_else(|| Some("turn interrupted".to_string()));
                        match rt.interrupt_turn(params.turn_id, kill_reason.clone()).await {
                            Ok(()) => {
                                interrupt_processes_for_turn(
                                    &server,
                                    params.thread_id,
                                    params.turn_id,
                                    kill_reason.clone(),
                                )
                                .await;
                                let server = server.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                    kill_processes_for_turn(
                                        &server,
                                        params.thread_id,
                                        params.turn_id,
                                        kill_reason,
                                    )
                                    .await;
                                });
                                JsonRpcResponse::ok(id, serde_json::json!({ "ok": true }))
                            }
                            Err(err) => JsonRpcResponse::err(
                                id,
                                JSONRPC_INTERNAL_ERROR,
                                err.to_string(),
                                None,
                            ),
                        }
                    }
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/start" => match serde_json::from_value::<ProcessStartParams>(request.params) {
                Ok(params) => match handle_process_start(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/list" => match serde_json::from_value::<ProcessListParams>(request.params) {
                Ok(params) => match handle_process_list(&server, params).await {
                    Ok(processes) => JsonRpcResponse::ok(
                        id,
                        serde_json::json!({
                            "processes": processes,
                        }),
                    ),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/inspect" => {
                match serde_json::from_value::<ProcessInspectParams>(request.params) {
                    Ok(params) => match handle_process_inspect(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "process/kill" => match serde_json::from_value::<ProcessKillParams>(request.params) {
                Ok(params) => match handle_process_kill(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/interrupt" => {
                match serde_json::from_value::<ProcessInterruptParams>(request.params) {
                    Ok(params) => match handle_process_interrupt(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => JsonRpcResponse::err(
                            id,
                            JSONRPC_INTERNAL_ERROR,
                            err.to_string(),
                            None,
                        ),
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "process/tail" => match serde_json::from_value::<ProcessTailParams>(request.params) {
                Ok(params) => match handle_process_tail(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "process/follow" => match serde_json::from_value::<ProcessFollowParams>(request.params)
            {
                Ok(params) => match handle_process_follow(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/read" => match serde_json::from_value::<FileReadParams>(request.params) {
                Ok(params) => match handle_file_read(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/glob" => match serde_json::from_value::<FileGlobParams>(request.params) {
                Ok(params) => match handle_file_glob(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/grep" => match serde_json::from_value::<FileGrepParams>(request.params) {
                Ok(params) => match handle_file_grep(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/write" => match serde_json::from_value::<FileWriteParams>(request.params) {
                Ok(params) => match handle_file_write(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/patch" => match serde_json::from_value::<FilePatchParams>(request.params) {
                Ok(params) => match handle_file_patch(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/edit" => match serde_json::from_value::<FileEditParams>(request.params) {
                Ok(params) => match handle_file_edit(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "file/delete" => match serde_json::from_value::<FileDeleteParams>(request.params) {
                Ok(params) => match handle_file_delete(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "fs/mkdir" => match serde_json::from_value::<FsMkdirParams>(request.params) {
                Ok(params) => match handle_fs_mkdir(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/write" => match serde_json::from_value::<ArtifactWriteParams>(request.params)
            {
                Ok(params) => match handle_artifact_write(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/list" => match serde_json::from_value::<ArtifactListParams>(request.params) {
                Ok(params) => match handle_artifact_list(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/read" => match serde_json::from_value::<ArtifactReadParams>(request.params) {
                Ok(params) => match handle_artifact_read(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            "artifact/delete" => {
                match serde_json::from_value::<ArtifactDeleteParams>(request.params) {
                    Ok(params) => match handle_artifact_delete(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "approval/decide" => {
                match serde_json::from_value::<ApprovalDecideParams>(request.params) {
                    Ok(params) => match handle_approval_decide(&server, params).await {
                        Ok(result) => JsonRpcResponse::ok(id, result),
                        Err(err) => {
                            JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                        }
                    },
                    Err(err) => JsonRpcResponse::err(
                        id,
                        JSONRPC_INVALID_PARAMS,
                        "invalid params",
                        Some(serde_json::json!({ "error": err.to_string() })),
                    ),
                }
            }
            "approval/list" => match serde_json::from_value::<ApprovalListParams>(request.params) {
                Ok(params) => match handle_approval_list(&server, params).await {
                    Ok(result) => JsonRpcResponse::ok(id, result),
                    Err(err) => {
                        JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None)
                    }
                },
                Err(err) => JsonRpcResponse::err(
                    id,
                    JSONRPC_INVALID_PARAMS,
                    "invalid params",
                    Some(serde_json::json!({ "error": err.to_string() })),
                ),
            },
            _ => JsonRpcResponse::err(
                id,
                JSONRPC_METHOD_NOT_FOUND,
                "method not found",
                Some(serde_json::json!({ "method": request.method })),
            ),
        };

                let line = serde_json::to_string(&response)?;
                let _ = server.out_tx.send(line);
            }

            shutdown_running_processes(&server).await;
            Ok(())
        })
        .await
}
