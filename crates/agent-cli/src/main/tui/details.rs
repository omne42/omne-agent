    fn build_process_inspect_text(resp: &ProcessInspectResponse) -> String {
        let mut out = String::new();
        out.push_str(&format!("process_id: {}\n", resp.process.process_id));
        out.push_str(&format!("thread_id: {}\n", resp.process.thread_id));
        out.push_str(&format!(
            "status: {}\n",
            process_status_str(resp.process.status)
        ));
        if let Some(turn_id) = resp.process.turn_id {
            out.push_str(&format!("turn_id: {turn_id}\n"));
        }
        out.push_str(&format!("started_at: {}\n", resp.process.started_at));
        out.push_str(&format!("last_update_at: {}\n", resp.process.last_update_at));
        if let Some(exit_code) = resp.process.exit_code {
            out.push_str(&format!("exit_code: {exit_code}\n"));
        }
        out.push_str(&format!("cwd: {}\n", resp.process.cwd));
        out.push_str(&format!("argv: {}\n", resp.process.argv.join(" ")));
        out.push_str(&format!("stdout_path: {}\n", resp.process.stdout_path));
        out.push_str(&format!("stderr_path: {}\n", resp.process.stderr_path));

        out.push_str("\n# stdout\n\n");
        out.push_str(resp.stdout_tail.trim_end());
        out.push_str("\n\n# stderr\n\n");
        out.push_str(resp.stderr_tail.trim_end());
        out
    }

    fn build_artifact_read_text(resp: &ArtifactReadResponse) -> String {
        let mut out = String::new();
        out.push_str(&format!("artifact_id: {}\n", resp.metadata.artifact_id));
        out.push_str(&format!("artifact_type: {}\n", resp.metadata.artifact_type));
        out.push_str(&format!("summary: {}\n", resp.metadata.summary));
        out.push_str(&format!("version: {}\n", resp.version));
        out.push_str(&format!("latest_version: {}\n", resp.latest_version));
        out.push_str(&format!("historical: {}\n", resp.historical));
        out.push_str(&format!(
            "metadata_source: {}\n",
            resp.metadata_source.as_str()
        ));
        if let Some(metadata_fallback_reason) = resp.metadata_fallback_reason {
            out.push_str(&format!(
                "metadata_fallback_reason: {}\n",
                metadata_fallback_reason.as_str()
            ));
        }
        out.push_str(&format!("bytes: {}\n", resp.bytes));
        out.push_str(&format!("truncated: {}\n", resp.truncated));

        if let Some(payload) = resp.fan_in_summary.as_ref() {
            let pending_approvals = payload
                .tasks
                .iter()
                .filter(|task| task.pending_approval.is_some())
                .count();
            let dependency_blocked = payload
                .tasks
                .iter()
                .filter(|task| task.dependency_blocked)
                .count();
            out.push_str("\n# Fan-in Summary (structured)\n\n");
            out.push_str(&format!("schema_version: {}\n", payload.schema_version));
            out.push_str(&format!("thread_id: {}\n", payload.thread_id));
            out.push_str(&format!("task_count: {}\n", payload.task_count));
            out.push_str(&format!(
                "scheduling: env_max_concurrent_subagents={} effective_concurrency_limit={} priority_aging_rounds={}\n",
                payload.scheduling.env_max_concurrent_subagents,
                payload.scheduling.effective_concurrency_limit,
                payload.scheduling.priority_aging_rounds
            ));
            out.push_str(&format!("pending_approvals: {}\n", pending_approvals));
            out.push_str(&format!("dependency_blocked: {}\n", dependency_blocked));
            if !payload.tasks.is_empty() {
                out.push_str("\n## tasks\n");
                for task in payload.tasks.iter().take(8) {
                    out.push_str(&format!(
                        "- task_id={} status={} title={}\n",
                        task.task_id, task.status, task.title
                    ));
                    if let Some(reason) = task.reason.as_deref().filter(|value| !value.is_empty()) {
                        out.push_str(&format!("  reason: {reason}\n"));
                    }
                    if task.dependency_blocked {
                        out.push_str("  dependency_blocked: true\n");
                    }
                    if let Some(blocker_task_id) = task
                        .dependency_blocker_task_id
                        .as_deref()
                        .filter(|value| !value.is_empty())
                    {
                        out.push_str(&format!("  dependency_blocker_task_id: {blocker_task_id}\n"));
                    }
                    if let Some(blocker_status) = task
                        .dependency_blocker_status
                        .as_deref()
                        .filter(|value| !value.is_empty())
                    {
                        out.push_str(&format!("  dependency_blocker_status: {blocker_status}\n"));
                    }
                    if let Some(pending) = task.pending_approval.as_ref() {
                        out.push_str(&format!(
                            "  pending_approval: action={} approval_id={}\n",
                            pending.action, pending.approval_id
                        ));
                        if let Some(approve_cmd) =
                            pending.approve_cmd.as_deref().filter(|value| !value.is_empty())
                        {
                            out.push_str(&format!("  approve_cmd: {approve_cmd}\n"));
                        }
                        if let Some(deny_cmd) =
                            pending.deny_cmd.as_deref().filter(|value| !value.is_empty())
                        {
                            out.push_str(&format!("  deny_cmd: {deny_cmd}\n"));
                        }
                    }
                }
                if payload.tasks.len() > 8 {
                    out.push_str(&format!("- ... {} more tasks\n", payload.tasks.len() - 8));
                }
            }
        }

        if let Some(payload) = resp.fan_out_linkage_issue.as_ref() {
            out.push_str("\n# Fan-out Linkage Issue (structured)\n\n");
            out.push_str(&format!("schema_version: {}\n", payload.schema_version));
            out.push_str(&format!(
                "fan_in_summary_artifact_id: {}\n",
                crate::normalize_fan_in_summary_artifact_id(
                    payload.fan_in_summary_artifact_id.as_str()
                )
            ));
            out.push_str(&format!("issue_truncated: {}\n", payload.issue_truncated));
            if let Some(summary) = crate::format_fan_out_linkage_issue_detail_from_payload(
                payload,
                resp.metadata.artifact_id,
            ) {
                out.push_str(&format!("summary: {summary}\n"));
            }
            if !payload.issue.trim().is_empty() {
                out.push_str("\n## issue\n");
                out.push_str(&format!("- {}\n", payload.issue.trim()));
            }
        }

        if let Some(payload) = resp.fan_out_linkage_issue_clear.as_ref() {
            out.push_str("\n# Fan-out Linkage Issue Clear (structured)\n\n");
            out.push_str(&format!("schema_version: {}\n", payload.schema_version));
            out.push_str(&format!(
                "fan_in_summary_artifact_id: {}\n",
                crate::normalize_fan_in_summary_artifact_id(
                    payload.fan_in_summary_artifact_id.as_str()
                )
            ));
            let summary = crate::format_fan_out_linkage_issue_clear_detail_from_payload(
                payload,
                resp.metadata.artifact_id,
            );
            out.push_str(&format!("summary: {summary}\n"));
        }

        if let Some(payload) = resp.fan_out_result.as_ref() {
            out.push_str("\n# Fan-out Result (structured)\n\n");
            out.push_str(&format!("schema_version: {}\n", payload.schema_version));
            out.push_str(&format!("task_id: {}\n", payload.task_id));
            out.push_str(&format!("thread_id: {}\n", payload.thread_id));
            out.push_str(&format!("turn_id: {}\n", payload.turn_id));
            out.push_str(&format!("status: {}\n", payload.status));
            out.push_str(&format!("workspace_mode: {}\n", payload.workspace_mode));
            if let Some(workspace_cwd) = payload.workspace_cwd.as_deref().filter(|value| !value.is_empty()) {
                out.push_str(&format!("workspace_cwd: {workspace_cwd}\n"));
            }
            if let Some(reason) = payload.reason.as_deref().filter(|value| !value.is_empty()) {
                out.push_str(&format!("reason: {reason}\n"));
            }

            if let Some(patch) = payload.isolated_write_patch.as_ref() {
                out.push_str("\n## isolated_write_patch\n");
                if let Some(artifact_type) = patch.artifact_type.as_deref().filter(|value| !value.is_empty()) {
                    out.push_str(&format!("- artifact_type: {artifact_type}\n"));
                }
                if let Some(artifact_id) = patch.artifact_id.as_deref().filter(|value| !value.is_empty()) {
                    out.push_str(&format!("- artifact_id: {artifact_id}\n"));
                }
                if let Some(truncated) = patch.truncated {
                    out.push_str(&format!("- truncated: {truncated}\n"));
                }
                if let Some(read_cmd) = patch.read_cmd.as_deref().filter(|value| !value.is_empty()) {
                    out.push_str(&format!("- read_cmd: {read_cmd}\n"));
                }
                if let Some(workspace_cwd) =
                    patch.workspace_cwd.as_deref().filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- workspace_cwd: {workspace_cwd}\n"));
                }
                if let Some(error) = patch.error.as_deref().filter(|value| !value.is_empty()) {
                    out.push_str(&format!("- error: {error}\n"));
                }
            }

            if let Some(handoff) = payload.isolated_write_handoff.as_ref() {
                out.push_str("\n## isolated_write_handoff\n");
                if let Some(workspace_cwd) =
                    handoff.workspace_cwd.as_deref().filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- workspace_cwd: {workspace_cwd}\n"));
                }
                if !handoff.status_argv.is_empty() {
                    out.push_str(&format!("- status_argv: {}\n", handoff.status_argv.join(" ")));
                }
                if !handoff.diff_argv.is_empty() {
                    out.push_str(&format!("- diff_argv: {}\n", handoff.diff_argv.join(" ")));
                }
                if let Some(apply_patch_hint) = handoff
                    .apply_patch_hint
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- apply_patch_hint: {apply_patch_hint}\n"));
                }
                if let Some(patch) = handoff.patch.as_ref() {
                    if let Some(artifact_id) = patch.artifact_id.as_deref().filter(|value| !value.is_empty()) {
                        out.push_str(&format!("- patch_artifact_id: {artifact_id}\n"));
                    } else if patch.error.as_deref().is_some_and(|value| !value.is_empty()) {
                        out.push_str("- patch: error\n");
                    }
                }
            }

            if let Some(auto_apply) = payload.isolated_write_auto_apply.as_ref() {
                out.push_str("\n## isolated_write_auto_apply\n");
                out.push_str(&format!("- enabled: {}\n", auto_apply.enabled));
                out.push_str(&format!("- attempted: {}\n", auto_apply.attempted));
                out.push_str(&format!("- applied: {}\n", auto_apply.applied));
                if let Some(workspace_cwd) = auto_apply
                    .workspace_cwd
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- workspace_cwd: {workspace_cwd}\n"));
                }
                if let Some(target_workspace_cwd) = auto_apply
                    .target_workspace_cwd
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- target_workspace_cwd: {target_workspace_cwd}\n"));
                }
                if !auto_apply.check_argv.is_empty() {
                    out.push_str(&format!(
                        "- check_argv: {}\n",
                        auto_apply.check_argv.join(" ")
                    ));
                }
                if !auto_apply.apply_argv.is_empty() {
                    out.push_str(&format!(
                        "- apply_argv: {}\n",
                        auto_apply.apply_argv.join(" ")
                    ));
                }
                if let Some(patch_artifact_id) = auto_apply
                    .patch_artifact_id
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- patch_artifact_id: {patch_artifact_id}\n"));
                }
                if let Some(patch_read_cmd) = auto_apply
                    .patch_read_cmd
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- patch_read_cmd: {patch_read_cmd}\n"));
                }
                if let Some(failure_stage) = auto_apply
                    .failure_stage
                    .as_ref()
                {
                    out.push_str(&format!("- failure_stage: {}\n", failure_stage.as_str()));
                }
                if let Some(recovery_hint) = auto_apply
                    .recovery_hint
                    .as_deref()
                    .filter(|value| !value.is_empty())
                {
                    out.push_str(&format!("- recovery_hint: {recovery_hint}\n"));
                }
                if !auto_apply.recovery_commands.is_empty() {
                    out.push_str("- recovery_commands:\n");
                    for command in &auto_apply.recovery_commands {
                        if command.argv.is_empty() {
                            out.push_str(&format!("  - {}\n", command.label));
                        } else {
                            out.push_str(&format!(
                                "  - {}: {}\n",
                                command.label,
                                command.argv.join(" ")
                            ));
                        }
                    }
                }
                if let Some(error) = auto_apply.error.as_deref().filter(|value| !value.is_empty()) {
                    out.push_str(&format!("- error: {error}\n"));
                }
            }
        }

        out.push_str("\n# Content\n\n");
        out.push_str(resp.text.trim_end());
        out
    }

    fn process_status_str(value: ProcessStatus) -> &'static str {
        match value {
            ProcessStatus::Running => "running",
            ProcessStatus::Exited => "exited",
            ProcessStatus::Abandoned => "abandoned",
        }
    }
