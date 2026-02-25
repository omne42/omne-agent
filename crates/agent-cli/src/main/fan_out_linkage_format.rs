fn normalize_fan_in_summary_artifact_id(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() { "-" } else { trimmed }
}

fn format_fan_out_linkage_issue_detail_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData,
    artifact_id: omne_protocol::ArtifactId,
) -> Option<String> {
    let issue = payload.issue.trim();
    if issue.is_empty() {
        return None;
    }
    let fan_in_summary_artifact_id =
        normalize_fan_in_summary_artifact_id(payload.fan_in_summary_artifact_id.as_str());
    Some(format!(
        "{issue} fan_in_summary_artifact_id={} issue_truncated={} (see fan_out_linkage_issue artifact_id={artifact_id})",
        fan_in_summary_artifact_id,
        payload.issue_truncated
    ))
}

fn format_fan_out_linkage_issue_clear_detail_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData,
    artifact_id: omne_protocol::ArtifactId,
) -> String {
    let fan_in_summary_artifact_id =
        normalize_fan_in_summary_artifact_id(payload.fan_in_summary_artifact_id.as_str());
    format!(
        "fan-out linkage issue cleared fan_in_summary_artifact_id={} (see fan_out_linkage_issue_clear artifact_id={artifact_id})",
        fan_in_summary_artifact_id
    )
}

fn format_fan_out_linkage_issue_notice_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData,
) -> String {
    let fan_in_summary_artifact_id =
        normalize_fan_in_summary_artifact_id(payload.fan_in_summary_artifact_id.as_str());
    format!(
        "fan_out_linkage_issue schema={} fan_in_summary_artifact_id={} issue_truncated={}",
        payload.schema_version, fan_in_summary_artifact_id, payload.issue_truncated
    )
}

fn format_fan_out_linkage_issue_clear_notice_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData,
) -> String {
    let fan_in_summary_artifact_id =
        normalize_fan_in_summary_artifact_id(payload.fan_in_summary_artifact_id.as_str());
    format!(
        "fan_out_linkage_issue_clear schema={} fan_in_summary_artifact_id={}",
        payload.schema_version, fan_in_summary_artifact_id
    )
}

fn format_fan_out_result_notice_from_payload(
    payload: &omne_app_server_protocol::ArtifactFanOutResultStructuredData,
) -> String {
    let mut out = format!(
        "fan_out_result schema={} task_id={} status={} workspace_mode={}",
        payload.schema_version, payload.task_id, payload.status, payload.workspace_mode
    );
    if let Some(workspace_cwd) = payload.workspace_cwd.as_deref().filter(|value| !value.is_empty()) {
        out.push_str(" workspace_cwd=");
        out.push_str(workspace_cwd);
    }
    match payload.isolated_write_patch.as_ref() {
        None => out.push_str(" isolated_write_patch=none"),
        Some(patch) => {
            if patch.error.as_deref().is_some_and(|value| !value.is_empty()) {
                out.push_str(" isolated_write_patch=error");
            } else if let Some(artifact_id) = patch.artifact_id.as_deref().filter(|value| !value.is_empty()) {
                out.push_str(" isolated_write_patch_artifact_id=");
                out.push_str(artifact_id);
                if let Some(truncated) = patch.truncated {
                    out.push_str(" isolated_write_patch_truncated=");
                    out.push_str(if truncated { "true" } else { "false" });
                }
            } else {
                out.push_str(" isolated_write_patch=present");
            }
        }
    }
    if let Some(auto_apply) = payload.isolated_write_auto_apply.as_ref() {
        if auto_apply.applied {
            out.push_str(" isolated_write_auto_apply=applied");
        } else if auto_apply
            .error
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            out.push_str(" isolated_write_auto_apply=error");
            if let Some(stage) = auto_apply
                .failure_stage
                .as_ref()
            {
                out.push_str(" isolated_write_auto_apply_stage=");
                out.push_str(stage.as_str());
            }
            if let Some(patch_artifact_id) = auto_apply
                .patch_artifact_id
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                out.push_str(" isolated_write_auto_apply_patch_artifact_id=");
                out.push_str(patch_artifact_id);
            }
            if !auto_apply.recovery_commands.is_empty() {
                out.push_str(" isolated_write_auto_apply_recovery_commands=");
                out.push_str(auto_apply.recovery_commands.len().to_string().as_str());
            }
        } else if auto_apply.attempted {
            out.push_str(" isolated_write_auto_apply=attempted_not_applied");
        } else {
            out.push_str(" isolated_write_auto_apply=enabled_not_attempted");
        }
    }
    out
}
