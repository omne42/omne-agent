#[cfg(test)]
mod special_directives_tests {
    use super::*;

    #[test]
    fn split_special_directives_noop_without_directives() -> anyhow::Result<()> {
        let input = "\n\nhello\nworld\n";
        let (remaining, refs, attachments, directives) = split_special_directives(input)?;
        assert_eq!(remaining, input);
        assert!(refs.is_empty());
        assert!(attachments.is_empty());
        assert!(directives.is_empty());
        Ok(())
    }

    #[test]
    fn split_special_directives_parses_file_and_diff() -> anyhow::Result<()> {
        let input = "@file crates/core/src/redaction.rs:1:3\n@diff\n\nplease help\n";
        let (remaining, refs, attachments, directives) = split_special_directives(input)?;
        assert_eq!(remaining, "please help");
        assert_eq!(refs.len(), 2);
        assert!(attachments.is_empty());
        assert!(directives.is_empty());
        assert!(matches!(
            &refs[0],
            omne_protocol::ContextRef::File(omne_protocol::ContextRefFile {
                path,
                start_line: Some(1),
                end_line: Some(3),
                ..
            }) if path == "crates/core/src/redaction.rs"
        ));
        assert!(matches!(&refs[1], omne_protocol::ContextRef::Diff(_)));
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
        let (remaining, refs, attachments, directives) = split_special_directives(input)?;
        assert_eq!(remaining, "hello");
        assert!(refs.is_empty());
        assert!(directives.is_empty());
        assert!(matches!(
            &attachments[0],
            omne_protocol::TurnAttachment::Image(omne_protocol::TurnAttachmentImage {
                source: omne_protocol::AttachmentSource::Path { path },
                ..
            }) if path == "assets/example.png"
        ));
        assert!(matches!(
            &attachments[1],
            omne_protocol::TurnAttachment::File(omne_protocol::TurnAttachmentFile {
                source: omne_protocol::AttachmentSource::Url { url },
                media_type,
                ..
            }) if url == "https://example.com/file.pdf" && media_type == "application/pdf"
        ));
        Ok(())
    }

    #[test]
    fn split_special_directives_parses_plan_directive() -> anyhow::Result<()> {
        let input = "/plan\n\nbuild this feature";
        let (remaining, refs, attachments, directives) = split_special_directives(input)?;
        assert_eq!(remaining, "build this feature");
        assert!(refs.is_empty());
        assert!(attachments.is_empty());
        assert!(matches!(
            directives.as_slice(),
            [omne_protocol::TurnDirective::Plan]
        ));
        Ok(())
    }

    #[test]
    fn split_special_directives_rejects_plan_args() {
        let err = split_special_directives("/plan now\nx").unwrap_err();
        assert!(err.to_string().contains("/plan"));
    }
}

#[cfg(test)]
mod rpc_response_parse_tests {
    use super::*;

    #[test]
    fn artifact_rpc_needs_approval_returns_actionable_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ArtifactNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        })
        .expect("serialize ArtifactNeedsApprovalResponse");

        let err = parse_artifact_rpc_response_typed::<omne_app_server_protocol::ArtifactWriteResponse>(
            "artifact/write",
            value,
        )
        .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("artifact/write needs approval"));
        assert!(message.contains(&approval_id.to_string()));
        assert!(message.contains(&format!(
            "omne approval decide {} {} --approve",
            thread_id, approval_id
        )));
        assert!(!message.contains("--thread-id"));
    }

    #[test]
    fn artifact_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ArtifactNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        })?;

        let outcome =
            parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactWriteResponse>(
                "artifact/write",
                value,
            )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn artifact_rpc_denied_returns_error() {
        let value = serde_json::to_value(omne_app_server_protocol::ArtifactDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            error_code: Some("allowed_tools_denied".to_string()),
            remembered: None,
        })
        .expect("serialize ArtifactDeniedResponse");

        let err = parse_artifact_rpc_response_typed::<omne_app_server_protocol::ArtifactWriteResponse>(
            "artifact/write",
            value,
        )
        .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("artifact/write denied"));
        assert!(message.contains("allowed_tools_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn artifact_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ArtifactDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            error_code: None,
            remembered: None,
        })?;

        let outcome =
            parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactWriteResponse>(
                "artifact/write",
                value,
            )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Denied { detail }
                if detail.get("denied") == Some(&serde_json::Value::Bool(true))
        ));
        Ok(())
    }

    #[test]
    fn artifact_rpc_ok_passthrough() -> anyhow::Result<()> {
        let tool_id = omne_protocol::ToolId::new();
        let artifact_id = omne_protocol::ArtifactId::new();
        let value = serde_json::json!({
            "tool_id": tool_id,
            "artifact_id": artifact_id,
            "created": true,
            "content_path": "/tmp/artifact.txt",
            "metadata_path": "/tmp/artifact.json",
            "metadata": {
                "artifact_id": artifact_id,
                "artifact_type": "report",
                "summary": "summary",
                "created_at": "1970-01-01T00:00:00Z",
                "updated_at": "1970-01-01T00:00:00Z",
                "version": 1,
                "content_path": "/tmp/artifact.txt",
                "size_bytes": 12
            }
        });

        let parsed: omne_app_server_protocol::ArtifactWriteResponse =
            parse_artifact_rpc_response_typed("artifact/write", value)?;
        assert_eq!(parsed.tool_id, tool_id);
        assert_eq!(parsed.artifact_id, artifact_id);
        assert_eq!(parsed.metadata.version, 1);
        Ok(())
    }

    #[test]
    fn artifact_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
        let tool_id = omne_protocol::ToolId::new();
        let artifact_id = omne_protocol::ArtifactId::new();
        let value = serde_json::json!({
            "tool_id": tool_id,
            "artifact_id": artifact_id,
            "created": true,
            "content_path": "/tmp/artifact.txt",
            "metadata_path": "/tmp/artifact.json",
            "metadata": {
                "artifact_id": artifact_id,
                "artifact_type": "report",
                "summary": "summary",
                "created_at": "1970-01-01T00:00:00Z",
                "updated_at": "1970-01-01T00:00:00Z",
                "version": 1,
                "content_path": "/tmp/artifact.txt",
                "size_bytes": 12
            }
        });

        let outcome =
            parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactWriteResponse>(
                "artifact/write",
                value,
            )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(omne_app_server_protocol::ArtifactWriteResponse {
                tool_id: t,
                artifact_id: a,
                created: true,
                ..
            }) if t == tool_id && a == artifact_id
        ));
        Ok(())
    }

    #[test]
    fn artifact_rpc_outcome_denied_false_is_not_misclassified() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ArtifactDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: false,
            error_code: None,
            remembered: None,
        })?;

        let outcome =
            parse_artifact_rpc_outcome::<omne_app_server_protocol::ArtifactDeniedResponse>(
                "artifact/write",
                value,
            )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(omne_app_server_protocol::ArtifactDeniedResponse {
                denied: false,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn repo_rpc_needs_approval_returns_actionable_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(omne_app_server_protocol::RepoNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        })
        .expect("serialize RepoNeedsApprovalResponse");

        let err =
            parse_repo_rpc_response_typed::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)
                .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("repo/search needs approval"));
        assert!(message.contains(&approval_id.to_string()));
    }

    #[test]
    fn repo_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(omne_app_server_protocol::RepoNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        })?;

        let outcome =
            parse_repo_rpc_outcome::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn repo_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::RepoDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            remembered: None,
            error_code: None,
        })?;

        let outcome =
            parse_repo_rpc_outcome::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Denied { detail }
                if detail.get("denied") == Some(&serde_json::Value::Bool(true))
        ));
        Ok(())
    }

    #[test]
    fn repo_rpc_denied_returns_error_includes_error_code() {
        let value = serde_json::to_value(omne_app_server_protocol::RepoDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            remembered: None,
            error_code: Some("mode_denied".to_string()),
        })
        .expect("serialize RepoDeniedResponse");

        let err =
            parse_repo_rpc_response_typed::<omne_app_server_protocol::RepoSearchResponse>(
                "repo/search",
                value,
            )
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("repo/search denied"));
        assert!(message.contains("mode_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn repo_rpc_ok_requires_typed_success_shape() {
        let value = serde_json::json!({
            "tool_id": "tool_123",
            "text": "hello",
        });
        let err =
            parse_repo_rpc_response_typed::<omne_app_server_protocol::RepoSearchResponse>("repo/search", value)
                .expect_err("expected error");
        assert!(err.to_string().contains("parse repo/search response"));
    }

    #[test]
    fn mcp_rpc_failed_response_passthrough() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::McpFailedResponse {
            tool_id: omne_protocol::ToolId::new(),
            failed: true,
            error: "server timeout".to_string(),
            server: "demo".to_string(),
        })
        .expect("serialize McpFailedResponse");

        let parsed: McpActionOrFailedResponse = parse_mcp_rpc_response_typed("mcp/call", value)?;
        match parsed {
            McpActionOrFailedResponse::Failed(response) => {
                assert!(response.failed);
                assert_eq!(response.server, "demo");
            }
            McpActionOrFailedResponse::Action(_) => anyhow::bail!("expected failed response"),
        }
        Ok(())
    }

    #[test]
    fn mcp_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(omne_app_server_protocol::McpNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        })?;

        let outcome = parse_mcp_rpc_outcome::<McpListServersOrFailedResponse>("mcp/list_servers", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn mcp_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::McpDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            remembered: None,
            error_code: None,
        })?;

        let outcome = parse_mcp_rpc_outcome::<McpListServersOrFailedResponse>("mcp/list_servers", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Denied { detail }
                if detail.get("denied") == Some(&serde_json::Value::Bool(true))
        ));
        Ok(())
    }

    #[test]
    fn mcp_rpc_denied_returns_error_includes_error_code() {
        let value = serde_json::to_value(omne_app_server_protocol::McpDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            remembered: None,
            error_code: Some("allowed_tools_denied".to_string()),
        })
        .expect("serialize McpDeniedResponse");

        let err = parse_mcp_rpc_response_typed::<McpListServersOrFailedResponse>(
            "mcp/list_servers",
            value,
        )
        .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("mcp/list_servers denied"));
        assert!(message.contains("allowed_tools_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn mcp_rpc_outcome_keeps_failed_passthrough() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::McpFailedResponse {
            tool_id: omne_protocol::ToolId::new(),
            failed: true,
            error: "server timeout".to_string(),
            server: "demo".to_string(),
        })?;

        let outcome = parse_mcp_rpc_outcome::<McpActionOrFailedResponse>("mcp/call", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(McpActionOrFailedResponse::Failed(response))
                if response.failed && response.server == "demo"
        ));
        Ok(())
    }

    #[test]
    fn mcp_rpc_ok_requires_typed_success_shape() {
        let value = serde_json::json!({
            "foo": "bar",
        });
        let err = parse_mcp_rpc_response_typed::<McpListServersOrFailedResponse>(
            "mcp/list_servers",
            value,
        )
        .expect_err("expected error");
        assert!(err.to_string().contains("parse mcp/list_servers response"));
    }

    #[test]
    fn process_rpc_denied_returns_error() {
        let value = serde_json::to_value(omne_app_server_protocol::ProcessDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            thread_id: omne_protocol::ThreadId::new(),
            remembered: None,
            error_code: Some("sandbox_policy_denied".to_string()),
        })
        .expect("serialize ProcessDeniedResponse");

        let err = parse_process_rpc_response_typed::<omne_app_server_protocol::ProcessTailResponse>(
            "process/tail",
            value,
        )
        .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("process/tail denied"));
        assert!(message.contains("sandbox_policy_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn process_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ProcessDeniedResponse {
            tool_id: omne_protocol::ToolId::new(),
            denied: true,
            thread_id: omne_protocol::ThreadId::new(),
            remembered: None,
            error_code: None,
        })?;

        let outcome = parse_process_rpc_outcome::<omne_app_server_protocol::ProcessTailResponse>(
            "process/tail",
            value,
        )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Denied { detail }
                if detail.get("denied") == Some(&serde_json::Value::Bool(true))
        ));
        Ok(())
    }

    #[test]
    fn process_rpc_ok_passthrough() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ProcessTailResponse {
            tool_id: omne_protocol::ToolId::new(),
            text: "hello".to_string(),
        })
        .expect("serialize ProcessTailResponse");

        let parsed: omne_app_server_protocol::ProcessTailResponse =
            parse_process_rpc_response_typed("process/tail", value)?;
        assert_eq!(parsed.text, "hello");
        Ok(())
    }

    #[test]
    fn process_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ProcessTailResponse {
            tool_id: omne_protocol::ToolId::new(),
            text: "hello".to_string(),
        })?;

        let outcome = parse_process_rpc_outcome::<omne_app_server_protocol::ProcessTailResponse>(
            "process/tail",
            value,
        )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(omne_app_server_protocol::ProcessTailResponse { text, .. })
                if text == "hello"
        ));
        Ok(())
    }

    #[test]
    fn process_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ProcessNeedsApprovalResponse {
            needs_approval: true,
            thread_id,
            approval_id,
        })?;

        let outcome = parse_process_rpc_outcome::<omne_app_server_protocol::ProcessTailResponse>(
            "process/tail",
            value,
        )?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn thread_git_snapshot_rpc_needs_approval_returns_actionable_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotNeedsApprovalResponse {
                needs_approval: true,
                thread_id,
                approval_id,
            },
        )
        .expect("serialize ThreadGitSnapshotNeedsApprovalResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/diff needs approval"));
        assert!(message.contains(&approval_id.to_string()));
    }

    #[test]
    fn thread_git_snapshot_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotNeedsApprovalResponse {
                needs_approval: true,
                thread_id,
                approval_id,
            },
        )?;

        let outcome = parse_thread_git_snapshot_rpc_outcome("thread/diff", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn thread_git_snapshot_rpc_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("sandbox_policy_denied".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                    omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                        omne_app_server_protocol::ProcessDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            thread_id,
                            remembered: None,
                            error_code: None,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/diff denied"));
        assert!(message.contains("sandbox_policy_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_patch_rpc_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("execpolicy_denied".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                    omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                        omne_app_server_protocol::ProcessDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            thread_id,
                            remembered: None,
                            error_code: None,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/patch denied"));
        assert!(message.contains("execpolicy_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_patch_rpc_artifact_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("allowed_tools_denied".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(
                        omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            tool: "artifact/write".to_string(),
                            allowed_tools: vec!["process/start".to_string()],
                            error_code: None,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/patch denied"));
        assert!(message.contains("allowed_tools_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_diff_rpc_artifact_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("allowed_tools_denied".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::AllowedToolsDenied(
                        omne_app_server_protocol::ArtifactAllowedToolsDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            tool: "artifact/write".to_string(),
                            allowed_tools: vec!["process/start".to_string()],
                            error_code: None,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/diff denied"));
        assert!(message.contains("allowed_tools_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_patch_rpc_artifact_mode_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("mode_denied".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(
                        omne_app_server_protocol::ArtifactModeDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            error_code: None,
                            mode: "artifact-deny".to_string(),
                            decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                            decision_source: "mode_permission".to_string(),
                            tool_override_hit: false,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/patch denied"));
        assert!(message.contains("mode_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_diff_rpc_artifact_mode_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("mode_denied".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::ModeDenied(
                        omne_app_server_protocol::ArtifactModeDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            error_code: None,
                            mode: "artifact-deny".to_string(),
                            decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                            decision_source: "mode_permission".to_string(),
                            tool_override_hit: false,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/diff", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/diff denied"));
        assert!(message.contains("mode_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_patch_rpc_artifact_unknown_mode_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("mode_unknown".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(
                        omne_app_server_protocol::ArtifactUnknownModeDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            error_code: None,
                            mode: "artifact-unknown".to_string(),
                            decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                            available: "other-mode".to_string(),
                            load_error: None,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err = parse_thread_git_snapshot_rpc_response("thread/patch", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/patch denied"));
        assert!(message.contains("mode_unknown"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_diff_rpc_artifact_unknown_mode_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
                denied: true,
                thread_id,
                error_code: Some("mode_unknown".to_string()),
                detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Artifact(
                    omne_app_server_protocol::ThreadArtifactDeniedDetail::UnknownModeDenied(
                        omne_app_server_protocol::ArtifactUnknownModeDeniedResponse {
                            tool_id: omne_protocol::ToolId::new(),
                            denied: true,
                            error_code: None,
                            mode: "artifact-unknown".to_string(),
                            decision: omne_app_server_protocol::ArtifactModeDecision::Deny,
                            available: "other-mode".to_string(),
                            load_error: None,
                        },
                    ),
                ),
            },
        )
        .expect("serialize ThreadGitSnapshotDeniedResponse");

        let err =
            parse_thread_git_snapshot_rpc_response("thread/diff", value).expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/diff denied"));
        assert!(message.contains("mode_unknown"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_git_snapshot_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadGitSnapshotDeniedResponse {
            denied: true,
            thread_id,
            error_code: None,
            detail: omne_app_server_protocol::ThreadGitSnapshotDeniedDetail::Process(
                omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                    omne_app_server_protocol::ProcessDeniedResponse {
                        tool_id: omne_protocol::ToolId::new(),
                        denied: true,
                        thread_id,
                        remembered: None,
                        error_code: None,
                    },
                ),
            ),
        })?;

        let outcome = parse_thread_git_snapshot_rpc_outcome("thread/diff", value)?;
        assert!(matches!(outcome, RpcGateOutcome::Denied { .. }));
        Ok(())
    }

    #[test]
    fn thread_git_snapshot_rpc_timed_out_passthrough() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
                omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse {
                    thread_id,
                    process_id: omne_protocol::ProcessId::new(),
                    stdout_path: "/tmp/stdout.log".to_string(),
                    stderr_path: "/tmp/stderr.log".to_string(),
                    timed_out: true,
                    wait_seconds: 5,
                },
            ),
        )
        .expect("serialize ThreadGitSnapshotRpcResponse::TimedOut");

        let parsed = parse_thread_git_snapshot_rpc_response("thread/diff", value)?;
        assert!(matches!(
            parsed,
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
                omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse { wait_seconds: 5, .. }
            )
        ));
        Ok(())
    }

    #[test]
    fn thread_git_snapshot_rpc_outcome_keeps_timed_out_passthrough() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
                omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse {
                    thread_id,
                    process_id: omne_protocol::ProcessId::new(),
                    stdout_path: "/tmp/stdout.log".to_string(),
                    stderr_path: "/tmp/stderr.log".to_string(),
                    timed_out: true,
                    wait_seconds: 5,
                },
            ),
        )?;

        let outcome = parse_thread_git_snapshot_rpc_outcome("thread/diff", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(
                omne_app_server_protocol::ThreadGitSnapshotRpcResponse::TimedOut(
                    omne_app_server_protocol::ThreadGitSnapshotTimedOutResponse { wait_seconds: 5, .. }
                )
            )
        ));
        Ok(())
    }

    #[test]
    fn thread_hook_run_rpc_needs_approval_returns_actionable_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value =
            serde_json::to_value(omne_app_server_protocol::ThreadHookRunNeedsApprovalResponse {
                needs_approval: true,
                thread_id,
                approval_id,
                hook: "run".to_string(),
            })
            .expect("serialize ThreadHookRunNeedsApprovalResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run needs approval"));
        assert!(message.contains(&approval_id.to_string()));
    }

    #[test]
    fn thread_hook_run_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value =
            serde_json::to_value(omne_app_server_protocol::ThreadHookRunNeedsApprovalResponse {
                needs_approval: true,
                thread_id,
                approval_id,
                hook: "run".to_string(),
            })?;

        let outcome = parse_thread_hook_run_rpc_outcome("thread/hook_run", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn thread_hook_run_rpc_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: Some("sandbox_policy_denied".to_string()),
            config_path: None,
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                omne_app_server_protocol::ProcessDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    thread_id,
                    remembered: None,
                    error_code: None,
                },
            ),
        })
        .expect("serialize ThreadHookRunDeniedResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run denied"));
        assert!(message.contains("sandbox_policy_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_hook_run_rpc_mode_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: Some("mode_denied".to_string()),
            config_path: None,
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::ModeDenied(
                omne_app_server_protocol::ProcessModeDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    thread_id,
                    mode: "hook-mode-deny".to_string(),
                    decision: omne_app_server_protocol::ProcessModeDecision::Deny,
                    decision_source: "mode_permission".to_string(),
                    tool_override_hit: false,
                    error_code: None,
                },
            ),
        })
        .expect("serialize ThreadHookRunDeniedResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run denied"));
        assert!(message.contains("mode_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_hook_run_rpc_mode_unknown_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: Some("mode_unknown".to_string()),
            config_path: None,
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::UnknownModeDenied(
                omne_app_server_protocol::ProcessUnknownModeDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    thread_id,
                    mode: "hook-mode".to_string(),
                    decision: omne_app_server_protocol::ProcessModeDecision::Deny,
                    available: "other-mode".to_string(),
                    load_error: None,
                    error_code: None,
                },
            ),
        })
        .expect("serialize ThreadHookRunDeniedResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run denied"));
        assert!(message.contains("mode_unknown"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_hook_run_rpc_allowed_tools_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: Some("allowed_tools_denied".to_string()),
            config_path: Some(".omne_data/spec/workspace.yaml".to_string()),
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::AllowedToolsDenied(
                omne_app_server_protocol::ProcessAllowedToolsDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    tool: "process/start".to_string(),
                    allowed_tools: vec!["repo/search".to_string()],
                    error_code: None,
                },
            ),
        })
        .expect("serialize ThreadHookRunDeniedResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run denied"));
        assert!(message.contains("allowed_tools_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_hook_run_rpc_execpolicy_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: Some("execpolicy_denied".to_string()),
            config_path: Some(".omne_data/spec/workspace.yaml".to_string()),
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyDenied(
                omne_app_server_protocol::ProcessExecPolicyDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    decision: omne_app_server_protocol::ExecPolicyDecision::Forbidden,
                    matched_rules: vec![],
                    justification: None,
                    error_code: None,
                },
            ),
        })
        .expect("serialize ThreadHookRunDeniedResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run denied"));
        assert!(message.contains("execpolicy_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_hook_run_rpc_execpolicy_load_denied_returns_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: Some("execpolicy_load_denied".to_string()),
            config_path: Some(".omne_data/spec/workspace.yaml".to_string()),
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::ExecPolicyLoadDenied(
                omne_app_server_protocol::ProcessExecPolicyLoadDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    mode: "coder".to_string(),
                    error: "failed to load thread execpolicy rules".to_string(),
                    details: "missing rules/missing.rules".to_string(),
                    error_code: None,
                },
            ),
        })
        .expect("serialize ThreadHookRunDeniedResponse");

        let err = parse_thread_hook_run_rpc_response("thread/hook_run", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/hook_run denied"));
        assert!(message.contains("execpolicy_load_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn thread_hook_run_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunDeniedResponse {
            denied: true,
            thread_id,
            hook: "run".to_string(),
            error_code: None,
            config_path: None,
            detail: omne_app_server_protocol::ThreadProcessDeniedDetail::Denied(
                omne_app_server_protocol::ProcessDeniedResponse {
                    tool_id: omne_protocol::ToolId::new(),
                    denied: true,
                    thread_id,
                    remembered: None,
                    error_code: None,
                },
            ),
        })?;

        let outcome = parse_thread_hook_run_rpc_outcome("thread/hook_run", value)?;
        assert!(matches!(outcome, RpcGateOutcome::Denied { .. }));
        Ok(())
    }

    #[test]
    fn thread_hook_run_rpc_ok_passthrough() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
            omne_app_server_protocol::ThreadHookRunResponse {
                ok: true,
                skipped: false,
                hook: "run".to_string(),
                reason: None,
                searched: None,
                config_path: None,
                argv: None,
                process_id: None,
                stdout_path: None,
                stderr_path: None,
            },
        ))
        .expect("serialize ThreadHookRunRpcResponse::Ok");

        let parsed = parse_thread_hook_run_rpc_response("thread/hook_run", value)?;
        assert!(parsed.ok);
        assert_eq!(parsed.hook, "run");
        Ok(())
    }

    #[test]
    fn thread_hook_run_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ThreadHookRunRpcResponse::Ok(
            omne_app_server_protocol::ThreadHookRunResponse {
                ok: true,
                skipped: false,
                hook: "run".to_string(),
                reason: None,
                searched: None,
                config_path: None,
                argv: None,
                process_id: None,
                stdout_path: None,
                stderr_path: None,
            },
        ))?;

        let outcome = parse_thread_hook_run_rpc_outcome("thread/hook_run", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(omne_app_server_protocol::ThreadHookRunResponse { ok: true, .. })
        ));
        Ok(())
    }

    #[test]
    fn checkpoint_restore_rpc_needs_approval_returns_actionable_error() {
        let thread_id = omne_protocol::ThreadId::new();
        let checkpoint_id = omne_protocol::CheckpointId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse {
                thread_id,
                checkpoint_id,
                needs_approval: true,
                approval_id,
                plan: omne_app_server_protocol::ThreadCheckpointPlan {
                    create: 1,
                    modify: 2,
                    delete: 3,
                },
            },
        )
        .expect("serialize ThreadCheckpointRestoreNeedsApprovalResponse");

        let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/checkpoint/restore needs approval"));
        assert!(message.contains(&approval_id.to_string()));
    }

    #[test]
    fn checkpoint_restore_rpc_outcome_classifies_needs_approval() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let checkpoint_id = omne_protocol::CheckpointId::new();
        let approval_id = omne_protocol::ApprovalId::new();
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreNeedsApprovalResponse {
                thread_id,
                checkpoint_id,
                needs_approval: true,
                approval_id,
                plan: omne_app_server_protocol::ThreadCheckpointPlan {
                    create: 1,
                    modify: 2,
                    delete: 3,
                },
            },
        )?;

        let outcome = parse_checkpoint_restore_rpc_outcome("thread/checkpoint/restore", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::NeedsApproval {
                thread_id: t,
                approval_id: a
            } if t == thread_id && a == approval_id
        ));
        Ok(())
    }

    #[test]
    fn checkpoint_restore_rpc_denied_returns_error() {
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: omne_protocol::ThreadId::new(),
                checkpoint_id: omne_protocol::CheckpointId::new(),
                denied: true,
                error_code: Some("mode_denied".to_string()),
                sandbox_policy: None,
                mode: Some("coder".to_string()),
                decision: None,
                available: None,
                load_error: None,
                sandbox_writable_roots: None,
            },
        )
        .expect("serialize ThreadCheckpointRestoreDeniedResponse");

        let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/checkpoint/restore denied"));
        assert!(message.contains("mode_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn checkpoint_restore_rpc_approval_denied_returns_error() {
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: omne_protocol::ThreadId::new(),
                checkpoint_id: omne_protocol::CheckpointId::new(),
                denied: true,
                error_code: Some("approval_denied".to_string()),
                sandbox_policy: None,
                mode: None,
                decision: None,
                available: None,
                load_error: None,
                sandbox_writable_roots: None,
            },
        )
        .expect("serialize ThreadCheckpointRestoreDeniedResponse");

        let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/checkpoint/restore denied"));
        assert!(message.contains("approval_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn checkpoint_restore_rpc_mode_unknown_returns_error() {
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: omne_protocol::ThreadId::new(),
                checkpoint_id: omne_protocol::CheckpointId::new(),
                denied: true,
                error_code: Some("mode_unknown".to_string()),
                sandbox_policy: None,
                mode: Some("checkpoint-restore-mode".to_string()),
                decision: Some(omne_app_server_protocol::ThreadCheckpointDecision::Deny),
                available: Some("other-mode".to_string()),
                load_error: None,
                sandbox_writable_roots: None,
            },
        )
        .expect("serialize ThreadCheckpointRestoreDeniedResponse");

        let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/checkpoint/restore denied"));
        assert!(message.contains("mode_unknown"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn checkpoint_restore_rpc_sandbox_policy_denied_returns_error() {
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: omne_protocol::ThreadId::new(),
                checkpoint_id: omne_protocol::CheckpointId::new(),
                denied: true,
                error_code: Some("sandbox_policy_denied".to_string()),
                sandbox_policy: Some(omne_protocol::SandboxPolicy::ReadOnly),
                mode: None,
                decision: None,
                available: None,
                load_error: None,
                sandbox_writable_roots: None,
            },
        )
        .expect("serialize ThreadCheckpointRestoreDeniedResponse");

        let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/checkpoint/restore denied"));
        assert!(message.contains("sandbox_policy_denied"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn checkpoint_restore_rpc_sandbox_writable_roots_unsupported_returns_error() {
        let value = serde_json::to_value(
            omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
                thread_id: omne_protocol::ThreadId::new(),
                checkpoint_id: omne_protocol::CheckpointId::new(),
                denied: true,
                error_code: Some("sandbox_writable_roots_unsupported".to_string()),
                sandbox_policy: None,
                mode: None,
                decision: None,
                available: None,
                load_error: None,
                sandbox_writable_roots: Some(vec![".".to_string()]),
            },
        )
        .expect("serialize ThreadCheckpointRestoreDeniedResponse");

        let err = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)
            .expect_err("expected error");
        let message = err.to_string();
        assert!(message.contains("thread/checkpoint/restore denied"));
        assert!(message.contains("sandbox_writable_roots_unsupported"));
        assert!(message.contains("[rpc error_code]"));
    }

    #[test]
    fn checkpoint_restore_rpc_outcome_classifies_denied() -> anyhow::Result<()> {
        let value = serde_json::to_value(omne_app_server_protocol::ThreadCheckpointRestoreDeniedResponse {
            thread_id: omne_protocol::ThreadId::new(),
            checkpoint_id: omne_protocol::CheckpointId::new(),
            denied: true,
            error_code: None,
            sandbox_policy: None,
            mode: Some("coder".to_string()),
            decision: None,
            available: None,
            load_error: None,
            sandbox_writable_roots: None,
        })?;

        let outcome = parse_checkpoint_restore_rpc_outcome("thread/checkpoint/restore", value)?;
        assert!(matches!(outcome, RpcGateOutcome::Denied { .. }));
        Ok(())
    }

    #[test]
    fn checkpoint_restore_rpc_ok_passthrough() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let checkpoint_id = omne_protocol::CheckpointId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadCheckpointRestoreResponse {
            thread_id,
            checkpoint_id,
            restored: true,
            plan: omne_app_server_protocol::ThreadCheckpointPlan {
                create: 0,
                modify: 0,
                delete: 0,
            },
            duration_ms: 42,
        })
        .expect("serialize ThreadCheckpointRestoreResponse");

        let parsed = parse_checkpoint_restore_rpc_response("thread/checkpoint/restore", value)?;
        assert!(parsed.restored);
        assert_eq!(parsed.duration_ms, 42);
        Ok(())
    }

    #[test]
    fn checkpoint_restore_rpc_outcome_ok_passthrough() -> anyhow::Result<()> {
        let thread_id = omne_protocol::ThreadId::new();
        let checkpoint_id = omne_protocol::CheckpointId::new();
        let value = serde_json::to_value(omne_app_server_protocol::ThreadCheckpointRestoreResponse {
            thread_id,
            checkpoint_id,
            restored: true,
            plan: omne_app_server_protocol::ThreadCheckpointPlan {
                create: 0,
                modify: 0,
                delete: 0,
            },
            duration_ms: 42,
        })?;

        let outcome = parse_checkpoint_restore_rpc_outcome("thread/checkpoint/restore", value)?;
        assert!(matches!(
            outcome,
            RpcGateOutcome::Ok(omne_app_server_protocol::ThreadCheckpointRestoreResponse {
                restored: true,
                duration_ms: 42,
                ..
            })
        ));
        Ok(())
    }
}
