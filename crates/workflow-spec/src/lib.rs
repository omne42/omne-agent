use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use structured_text_protocol::StructuredTextData;

pub const FAN_IN_SUMMARY_SCHEMA_V1: &str = "fan_in_summary.v1";
pub const FAN_OUT_LINKAGE_ISSUE_SCHEMA_V1: &str = "fan_out_linkage_issue.v1";
pub const FAN_OUT_LINKAGE_ISSUE_CLEAR_SCHEMA_V1: &str = "fan_out_linkage_issue_clear.v1";
pub const FAN_OUT_RESULT_SCHEMA_V1: &str = "fan_out_result.v1";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanInSummaryStructuredData {
    pub schema_version: String,
    pub thread_id: String,
    pub task_count: usize,
    pub scheduling: FanInSchedulingStructuredData,
    pub tasks: Vec<FanInTaskStructuredData>,
}

impl FanInSummaryStructuredData {
    pub fn new(
        thread_id: String,
        scheduling: FanInSchedulingStructuredData,
        tasks: Vec<FanInTaskStructuredData>,
    ) -> Self {
        Self {
            schema_version: FAN_IN_SUMMARY_SCHEMA_V1.to_string(),
            thread_id,
            task_count: tasks.len(),
            scheduling,
            tasks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanInSchedulingStructuredData {
    pub env_max_concurrent_subagents: usize,
    pub effective_concurrency_limit: usize,
    pub priority_aging_rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanInTaskStructuredData {
    pub task_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub dependency_blocked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_blocker_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_blocker_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_artifact_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_artifact_structured_error: Option<StructuredTextData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_artifact_error_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_artifact_diagnostics: Option<FanInResultArtifactDiagnosticsStructuredData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<FanInPendingApprovalStructuredData>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanInResultArtifactDiagnosticsStructuredData {
    pub scan_last_seq: u64,
    pub matched_completion_count: u64,
    pub pending_matching_tool_ids: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanInPendingApprovalStructuredData {
    pub approval_id: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approve_cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_cmd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutLinkageIssueStructuredData {
    pub schema_version: String,
    pub fan_in_summary_artifact_id: String,
    pub issue: String,
    pub issue_truncated: bool,
}

impl FanOutLinkageIssueStructuredData {
    pub fn new(fan_in_summary_artifact_id: String, issue: String, issue_truncated: bool) -> Self {
        Self {
            schema_version: FAN_OUT_LINKAGE_ISSUE_SCHEMA_V1.to_string(),
            fan_in_summary_artifact_id,
            issue,
            issue_truncated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutLinkageIssueClearStructuredData {
    pub schema_version: String,
    pub fan_in_summary_artifact_id: String,
}

impl FanOutLinkageIssueClearStructuredData {
    pub fn new(fan_in_summary_artifact_id: String) -> Self {
        Self {
            schema_version: FAN_OUT_LINKAGE_ISSUE_CLEAR_SCHEMA_V1.to_string(),
            fan_in_summary_artifact_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutResultIsolatedWritePatchStructuredData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_error: Option<StructuredTextData>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutResultIsolatedWriteHandoffStructuredData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_cwd: Option<String>,
    #[serde(default)]
    pub status_argv: Vec<String>,
    #[serde(default)]
    pub diff_argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_patch_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<FanOutResultIsolatedWritePatchStructuredData>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanOutResultIsolatedWriteAutoApplyFailureStage {
    Precondition,
    CapturePatch,
    CheckPatch,
    ApplyPatch,
    #[serde(other)]
    Unknown,
}

impl FanOutResultIsolatedWriteAutoApplyFailureStage {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Precondition => "precondition",
            Self::CapturePatch => "capture_patch",
            Self::CheckPatch => "check_patch",
            Self::ApplyPatch => "apply_patch",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutResultIsolatedWriteAutoApplyStructuredData {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub attempted: bool,
    #[serde(default)]
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_workspace_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub check_argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apply_argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_read_cmd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<FanOutResultIsolatedWriteAutoApplyFailureStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovery_commands: Vec<FanOutResultRecoveryCommandStructuredData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_error: Option<StructuredTextData>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutResultRecoveryCommandStructuredData {
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanOutResultStructuredData {
    pub schema_version: String,
    pub task_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub workspace_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolated_write_patch: Option<FanOutResultIsolatedWritePatchStructuredData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolated_write_handoff: Option<FanOutResultIsolatedWriteHandoffStructuredData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolated_write_auto_apply: Option<FanOutResultIsolatedWriteAutoApplyStructuredData>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl FanOutResultStructuredData {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: String,
        thread_id: String,
        turn_id: String,
        workspace_mode: String,
        workspace_cwd: Option<String>,
        isolated_write_patch: Option<FanOutResultIsolatedWritePatchStructuredData>,
        isolated_write_handoff: Option<FanOutResultIsolatedWriteHandoffStructuredData>,
        status: String,
        reason: Option<String>,
    ) -> Self {
        Self {
            schema_version: FAN_OUT_RESULT_SCHEMA_V1.to_string(),
            task_id,
            thread_id,
            turn_id,
            workspace_mode,
            workspace_cwd,
            isolated_write_patch,
            isolated_write_handoff,
            isolated_write_auto_apply: None,
            status,
            reason,
        }
    }
}

/// Return the canonical workflow spec directory under the omne root.
pub fn workflow_spec_dir(omne_root: &Path) -> PathBuf {
    omne_root.join("spec").join("commands")
}

/// Validate a workflow name used as `<name>.md` inside the spec dir.
pub fn validate_workflow_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("workflow name must not be empty");
    }
    if name.trim() != name {
        anyhow::bail!("workflow name must not contain leading/trailing whitespace");
    }
    if name.contains('/') || name.contains('\\') {
        anyhow::bail!("workflow name must not contain path separators");
    }
    if name.contains("..") {
        anyhow::bail!("workflow name must not contain `..`");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("workflow name contains invalid characters: {name}");
    }
    Ok(())
}

/// Validate an input/template variable name.
pub fn ensure_valid_var_name(name: &str, label: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        anyhow::bail!("{label} must not be empty");
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        anyhow::bail!("{label} must start with [A-Za-z_]: {name}");
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        anyhow::bail!("{label} contains invalid characters: {name}");
    }
    Ok(())
}

/// Split markdown with YAML frontmatter (`---`) into `(yaml, body)`.
pub fn split_frontmatter(contents: &str) -> anyhow::Result<(&str, &str)> {
    let Some(first_newline) = contents.find('\n') else {
        anyhow::bail!("missing frontmatter start delimiter");
    };
    let first_line = contents[..first_newline]
        .trim_end_matches(['\r', '\n'])
        .trim_end();
    if first_line != "---" {
        anyhow::bail!("missing frontmatter start delimiter");
    }

    let yaml_start = first_newline + 1;
    let mut cursor = yaml_start;
    while cursor < contents.len() {
        let line_end = match contents[cursor..].find('\n') {
            Some(rel) => cursor + rel + 1,
            None => contents.len(),
        };
        let line = contents[cursor..line_end].trim_end_matches(['\r', '\n']);
        if line == "---" {
            let yaml = &contents[yaml_start..cursor];
            let body = &contents[line_end..];
            return Ok((yaml, body));
        }
        if line_end == contents.len() {
            break;
        }
        cursor = line_end;
    }

    anyhow::bail!("missing frontmatter end delimiter")
}

/// Trim, drop empties, and dedupe while preserving first-seen order.
pub fn normalize_unique_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

/// Render `{{name}}` placeholders with strict validation.
pub fn render_template(
    template: &str,
    declared: &BTreeSet<String>,
    vars: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut cursor = 0usize;

    while let Some(open_rel) = template[cursor..].find("{{") {
        let open = cursor + open_rel;
        out.push_str(&template[cursor..open]);

        let after_open = open + 2;
        let Some(close_rel) = template[after_open..].find("}}") else {
            anyhow::bail!("unclosed template placeholder");
        };
        let close = after_open + close_rel;
        let raw_key = &template[after_open..close];
        if raw_key.trim() != raw_key {
            anyhow::bail!("template placeholder contains whitespace: {raw_key}");
        }
        if raw_key.is_empty() {
            anyhow::bail!("template placeholder must not be empty");
        }
        ensure_valid_var_name(raw_key, "template placeholder")?;
        if !declared.contains(raw_key) {
            anyhow::bail!("undeclared template variable: {raw_key}");
        }
        let value = vars
            .get(raw_key)
            .ok_or_else(|| anyhow::anyhow!("missing template variable: {raw_key}"))?;
        out.push_str(value);
        cursor = close + 2;
    }

    out.push_str(&template[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fan_in_summary_structured_data_new_sets_schema_and_task_count() {
        let payload = FanInSummaryStructuredData::new(
            "thread-1".to_string(),
            FanInSchedulingStructuredData {
                env_max_concurrent_subagents: 4,
                effective_concurrency_limit: 2,
                priority_aging_rounds: 3,
            },
            vec![FanInTaskStructuredData {
                task_id: "t1".to_string(),
                title: "task".to_string(),
                thread_id: Some("child-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                status: "Completed".to_string(),
                reason: None,
                dependency_blocked: false,
                dependency_blocker_task_id: None,
                dependency_blocker_status: None,
                result_artifact_id: None,
                result_artifact_error: None,
                result_artifact_structured_error: None,
                result_artifact_error_id: None,
                result_artifact_diagnostics: None,
                pending_approval: None,
            }],
        );
        assert_eq!(payload.schema_version, FAN_IN_SUMMARY_SCHEMA_V1);
        assert_eq!(payload.thread_id, "thread-1");
        assert_eq!(payload.task_count, 1);
        assert_eq!(payload.tasks.len(), 1);
        assert_eq!(payload.scheduling.effective_concurrency_limit, 2);
    }

    #[test]
    fn fan_out_linkage_issue_structured_data_new_sets_schema() {
        let payload = FanOutLinkageIssueStructuredData::new(
            "artifact-1".to_string(),
            "fan-out linkage issue: blocked".to_string(),
            false,
        );
        assert_eq!(payload.schema_version, FAN_OUT_LINKAGE_ISSUE_SCHEMA_V1);
        assert_eq!(payload.fan_in_summary_artifact_id, "artifact-1");
        assert_eq!(payload.issue, "fan-out linkage issue: blocked");
        assert!(!payload.issue_truncated);
    }

    #[test]
    fn fan_out_linkage_issue_clear_structured_data_new_sets_schema() {
        let payload = FanOutLinkageIssueClearStructuredData::new("artifact-1".to_string());
        assert_eq!(
            payload.schema_version,
            FAN_OUT_LINKAGE_ISSUE_CLEAR_SCHEMA_V1
        );
        assert_eq!(payload.fan_in_summary_artifact_id, "artifact-1");
    }

    #[test]
    fn fan_out_result_structured_data_new_sets_schema() {
        let payload = FanOutResultStructuredData::new(
            "t1".to_string(),
            "thread-1".to_string(),
            "turn-1".to_string(),
            "isolated_write".to_string(),
            Some("/tmp/subagent/repo".to_string()),
            Some(FanOutResultIsolatedWritePatchStructuredData {
                artifact_type: Some("patch".to_string()),
                artifact_id: Some("artifact-1".to_string()),
                truncated: Some(false),
                read_cmd: Some("omne artifact read thread-1 artifact-1".to_string()),
                workspace_cwd: None,
                error: None,
                structured_error: None,
            }),
            None,
            "completed".to_string(),
            None,
        );
        assert_eq!(payload.schema_version, FAN_OUT_RESULT_SCHEMA_V1);
        assert_eq!(payload.task_id, "t1");
        assert_eq!(payload.workspace_mode, "isolated_write");
        assert_eq!(
            payload
                .isolated_write_patch
                .as_ref()
                .and_then(|patch| patch.artifact_id.as_deref()),
            Some("artifact-1")
        );
        assert!(payload.isolated_write_auto_apply.is_none());
    }
}
