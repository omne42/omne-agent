use super::*;

#[test]
fn format_fan_out_linkage_issue_from_structured_payload_includes_artifact_handles() {
    let artifact_id = ArtifactId::new();
    let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
        schema_version: "fan_out_linkage_issue.v1".to_string(),
        fan_in_summary_artifact_id: "fan-in-1".to_string(),
        issue: "fan-out linkage issue: blocked".to_string(),
        issue_truncated: true,
    };

    let text = format_fan_out_linkage_issue_from_structured_payload(&payload, artifact_id)
        .expect("linkage issue text");
    assert!(text.contains("fan-out linkage issue: blocked"));
    assert!(text.contains("fan_in_summary_artifact_id=fan-in-1"));
    assert!(text.contains("issue_truncated=true"));
    assert!(text.contains(&format!(
        "fan_out_linkage_issue artifact_id={}",
        artifact_id
    )));
}

#[test]
fn format_fan_out_linkage_issue_from_structured_payload_returns_none_when_issue_blank() {
    let artifact_id = ArtifactId::new();
    let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueStructuredData {
        schema_version: "fan_out_linkage_issue.v1".to_string(),
        fan_in_summary_artifact_id: "fan-in-1".to_string(),
        issue: "   ".to_string(),
        issue_truncated: false,
    };

    assert!(
        format_fan_out_linkage_issue_from_structured_payload(&payload, artifact_id).is_none()
    );
}

#[test]
fn format_fan_out_linkage_issue_clear_from_structured_payload_includes_artifact_handles() {
    let artifact_id = ArtifactId::new();
    let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
        schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
        fan_in_summary_artifact_id: "fan-in-1".to_string(),
    };

    let text = format_fan_out_linkage_issue_clear_from_structured_payload(&payload, artifact_id);
    assert!(text.contains("fan-out linkage issue cleared"));
    assert!(text.contains("fan_in_summary_artifact_id=fan-in-1"));
    assert!(text.contains(&format!(
        "fan_out_linkage_issue_clear artifact_id={}",
        artifact_id
    )));
}

#[test]
fn format_fan_out_linkage_issue_clear_from_structured_payload_handles_blank_summary_artifact_id()
{
    let artifact_id = ArtifactId::new();
    let payload = omne_app_server_protocol::ArtifactFanOutLinkageIssueClearStructuredData {
        schema_version: "fan_out_linkage_issue_clear.v1".to_string(),
        fan_in_summary_artifact_id: "   ".to_string(),
    };

    let text = format_fan_out_linkage_issue_clear_from_structured_payload(&payload, artifact_id);
    assert!(text.contains("fan_in_summary_artifact_id=-"));
}
