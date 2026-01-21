fn validate_strict_run_result(result: &pm_core::RunResult) -> anyhow::Result<()> {
    let failed: Vec<String> = result
        .prs
        .iter()
        .filter(|pr| matches!(pr.status, pm_core::PullRequestStatus::Failed))
        .map(|pr| pr.id.as_str().to_string())
        .collect();
    if !failed.is_empty() {
        anyhow::bail!(
            "session {} had failed tasks: {}",
            result.session.id,
            failed.join(", ")
        );
    }
    if let Some(error) = result.merge.error.as_deref() {
        anyhow::bail!("session {} merge failed: {}", result.session.id, error);
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct TaskInput {
    id: Option<String>,
    title: String,
    description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum TasksFile {
    List(Vec<TaskInput>),
    Object { tasks: Vec<TaskInput> },
}

