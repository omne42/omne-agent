use super::*;

#[test]
fn fan_out_result_read_command_uses_artifact_read_cli() {
    let thread_id = ThreadId::new();
    let artifact_id = ArtifactId::new();
    let command = fan_out_result_read_command(thread_id, artifact_id);
    assert_eq!(
        command,
        format!("omne artifact read {} {}", thread_id, artifact_id)
    );
}

#[test]
fn collect_failed_task_error_reads_returns_parent_thread_commands() {
    let parent_thread_id = ThreadId::new();
    let error_artifact_id_a = ArtifactId::new();
    let error_artifact_id_b = ArtifactId::new();
    let mut results = vec![
        WorkflowTaskResult {
            task_id: "t-b".to_string(),
            title: "failed task b".to_string(),
            thread_id: Some(ThreadId::new()),
            turn_id: Some(TurnId::new()),
            result_artifact_id: None,
            result_artifact_error: Some("failed to write".to_string()),
            result_artifact_error_id: Some(error_artifact_id_b),
            status: TurnStatus::Failed,
            reason: Some("boom".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        },
        WorkflowTaskResult {
            task_id: "t-a".to_string(),
            title: "failed task a".to_string(),
            thread_id: Some(ThreadId::new()),
            turn_id: Some(TurnId::new()),
            result_artifact_id: None,
            result_artifact_error: Some("failed to write".to_string()),
            result_artifact_error_id: Some(error_artifact_id_a),
            status: TurnStatus::Failed,
            reason: Some("boom".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        },
    ];
    results.push(results[1].clone());

    let reads = collect_failed_task_error_reads(parent_thread_id, &results);
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0].0, "t-a");
    assert_eq!(
        reads[0].1,
        format!("omne artifact read {} {}", parent_thread_id, error_artifact_id_a)
    );
    assert_eq!(reads[1].0, "t-b");
    assert_eq!(
        reads[1].1,
        format!("omne artifact read {} {}", parent_thread_id, error_artifact_id_b)
    );
}

#[test]
fn collect_failed_task_reads_returns_sorted_unique_commands() {
    let thread_id_a = ThreadId::new();
    let thread_id_b = ThreadId::new();
    let artifact_id_a = ArtifactId::new();
    let artifact_id_b = ArtifactId::new();

    let mut results = vec![
        WorkflowTaskResult {
            task_id: "t-b".to_string(),
            title: "failed task b".to_string(),
            thread_id: Some(thread_id_b),
            turn_id: Some(TurnId::new()),
            result_artifact_id: Some(artifact_id_b),
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Failed,
            reason: Some("boom".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        },
        WorkflowTaskResult {
            task_id: "t-a".to_string(),
            title: "failed task a".to_string(),
            thread_id: Some(thread_id_a),
            turn_id: Some(TurnId::new()),
            result_artifact_id: Some(artifact_id_a),
            result_artifact_error: None,
            result_artifact_error_id: None,
            status: TurnStatus::Failed,
            reason: Some("boom".to_string()),
            dependency_blocked: false,
            assistant_text: None,
            pending_approval: None,
        },
    ];
    results.push(results[1].clone());

    let reads = collect_failed_task_reads(&results);
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0].0, "t-a");
    assert_eq!(
        reads[0].1,
        format!("omne artifact read {} {}", thread_id_a, artifact_id_a)
    );
    assert_eq!(reads[1].0, "t-b");
    assert_eq!(
        reads[1].1,
        format!("omne artifact read {} {}", thread_id_b, artifact_id_b)
    );
}
