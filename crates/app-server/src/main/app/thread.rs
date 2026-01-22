async fn handle_thread_request(
    server: &Arc<Server>,
    id: serde_json::Value,
    method: &str,
    params: serde_json::Value,
) -> JsonRpcResponse {
    match method {
        "thread/start" => match serde_json::from_value::<ThreadStartParams>(params) {
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
                        let rt = Arc::new(ThreadRuntime::new(handle, server.notify_tx.clone()));
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
                    Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
                }
            }
            Err(err) => invalid_params(id, err),
        },
        "thread/resume" => match serde_json::from_value::<ThreadResumeParams>(params) {
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
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/fork" => match serde_json::from_value::<ThreadForkParams>(params) {
            Ok(params) => match handle_thread_fork(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/archive" => match serde_json::from_value::<ThreadArchiveParams>(params) {
            Ok(params) => match handle_thread_archive(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/unarchive" => match serde_json::from_value::<ThreadUnarchiveParams>(params) {
            Ok(params) => match handle_thread_unarchive(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/pause" => match serde_json::from_value::<ThreadPauseParams>(params) {
            Ok(params) => match handle_thread_pause(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/unpause" => match serde_json::from_value::<ThreadUnpauseParams>(params) {
            Ok(params) => match handle_thread_unpause(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/delete" => match serde_json::from_value::<ThreadDeleteParams>(params) {
            Ok(params) => match handle_thread_delete(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/clear_artifacts" => match serde_json::from_value::<ThreadClearArtifactsParams>(params) {
            Ok(params) => match handle_thread_clear_artifacts(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/list" => {
            let _ = params;
            match server.thread_store.list_threads().await {
                Ok(threads) => JsonRpcResponse::ok(id, serde_json::json!({ "threads": threads })),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            }
        }
        "thread/list_meta" => match serde_json::from_value::<ThreadListMetaParams>(params) {
            Ok(params) => match handle_thread_list_meta(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/loaded" => {
            let _ = params;
            let mut threads = server
                .threads
                .lock()
                .await
                .keys()
                .copied()
                .collect::<Vec<_>>();
            threads.sort_unstable();
            JsonRpcResponse::ok(id, serde_json::json!({ "threads": threads }))
        }
        "thread/events" => match serde_json::from_value::<ThreadEventsParams>(params) {
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
                    Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
                }
            }
            Err(err) => invalid_params(id, err),
        },
        "thread/subscribe" => match serde_json::from_value::<ThreadSubscribeParams>(params) {
            Ok(params) => match handle_thread_subscribe(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/state" => match serde_json::from_value::<ThreadStateParams>(params) {
            Ok(params) => match server.get_or_load_thread(params.thread_id).await {
                Ok(rt) => {
                    let handle = rt.handle.lock().await;
                    let state = handle.state();
                    let archived_at = state.archived_at.and_then(|ts| ts.format(&Rfc3339).ok());
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
                            "sandbox_writable_roots": state.sandbox_writable_roots,
                            "sandbox_network_access": state.sandbox_network_access,
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
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/attention" => match serde_json::from_value::<ThreadAttentionParams>(params) {
            Ok(params) => match handle_thread_attention(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/disk_usage" => match serde_json::from_value::<ThreadDiskUsageParams>(params) {
            Ok(params) => match handle_thread_disk_usage(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/disk_report" => match serde_json::from_value::<ThreadDiskReportParams>(params) {
            Ok(params) => match handle_thread_disk_report(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/diff" => match serde_json::from_value::<ThreadDiffParams>(params) {
            Ok(params) => match handle_thread_diff(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/hook_run" => match serde_json::from_value::<ThreadHookRunParams>(params) {
            Ok(params) => match handle_thread_hook_run(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/configure" => match serde_json::from_value::<ThreadConfigureParams>(params) {
            Ok(params) => match handle_thread_configure(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/config/explain" => match serde_json::from_value::<ThreadConfigExplainParams>(params) {
            Ok(params) => match handle_thread_config_explain(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        "thread/models" => match serde_json::from_value::<ThreadModelsParams>(params) {
            Ok(params) => match handle_thread_models(server, params).await {
                Ok(result) => JsonRpcResponse::ok(id, result),
                Err(err) => JsonRpcResponse::err(id, JSONRPC_INTERNAL_ERROR, err.to_string(), None),
            },
            Err(err) => invalid_params(id, err),
        },
        _ => {
            let _ = params;
            method_not_found(id, method)
        }
    }
}
