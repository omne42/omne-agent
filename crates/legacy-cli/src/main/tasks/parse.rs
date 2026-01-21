async fn parse_tasks_override(args: &RunArgs) -> anyhow::Result<Option<Vec<pm_core::TaskSpec>>> {
    if args.tasks_file.is_some() && !args.task.is_empty() {
        anyhow::bail!("use only one of --tasks-file or --task");
    }

    let override_requested = args.tasks_file.is_some() || !args.task.is_empty();
    if !override_requested {
        return Ok(None);
    }

    let tasks = if let Some(path) = &args.tasks_file {
        let text = tokio::fs::read_to_string(path).await?;
        let parsed: TasksFile = serde_json::from_str(&text)?;
        match parsed {
            TasksFile::List(tasks) => tasks,
            TasksFile::Object { tasks } => tasks,
        }
    } else if !args.task.is_empty() {
        args.task
            .iter()
            .enumerate()
            .map(|(index, raw)| task_input_from_arg(raw, index))
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    if tasks.is_empty() {
        anyhow::bail!("tasks override provided but empty");
    }

    let mut seen: HashSet<pm_core::TaskId> = HashSet::new();
    let mut specs = Vec::with_capacity(tasks.len());
    for (index, task) in tasks.into_iter().enumerate() {
        let fallback = format!("t{}", index + 1);
        let id_raw = match task.id {
            Some(id) => {
                let trimmed = id.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("task id must not be empty (task index: {})", index + 1);
                }
                trimmed.to_string()
            }
            None => fallback,
        };
        let id = pm_core::TaskId::sanitize(&id_raw);

        if !seen.insert(id.clone()) {
            anyhow::bail!("duplicate task id: {}", id.as_str());
        }

        let title = task.title.trim().to_string();
        if title.is_empty() {
            anyhow::bail!("task title must not be empty (task id: {})", id.as_str());
        }

        specs.push(pm_core::TaskSpec {
            id,
            title,
            description: task.description.and_then(|d| {
                let trimmed = d.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }),
        });
    }

    Ok(Some(specs))
}

fn task_input_from_arg(raw: &str, index: usize) -> anyhow::Result<TaskInput> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("--task value must not be empty");
    }

    let (id, title) = match raw.split_once(':') {
        Some((id, title)) => {
            let id = id.trim();
            if id.is_empty() {
                anyhow::bail!("--task id must not be empty");
            }
            (Some(id.to_string()), title.trim().to_string())
        }
        None => (Some(format!("t{}", index + 1)), raw.to_string()),
    };

    Ok(TaskInput {
        id,
        title,
        description: None,
    })
}

struct TemplateArchitect;

#[async_trait::async_trait]
impl Architect for TemplateArchitect {
    async fn split(&self, session: &pm_core::Session) -> anyhow::Result<Vec<pm_core::TaskSpec>> {
        Ok(vec![pm_core::TaskSpec {
            id: pm_core::TaskId::sanitize("main"),
            title: format!("Implement {}", session.pr_name.as_str()),
            description: Some("Phase 1: template single-task split".to_string()),
        }])
    }
}
