#[derive(Debug, Clone, Serialize)]
struct CommandSummary {
    name: String,
    version: u32,
    mode: String,
    file: String,
}

fn workflow_spec_dir(pm_root: &std::path::Path) -> PathBuf {
    pm_root.join("spec").join("commands")
}

fn validate_workflow_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("command name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "invalid command name: {name} (allowed: [a-zA-Z0-9_-], no slashes)"
        );
    }
    Ok(())
}

fn is_valid_var_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn ensure_valid_var_name(name: &str, label: &str) -> anyhow::Result<()> {
    if is_valid_var_name(name) {
        return Ok(());
    }
    anyhow::bail!(
        "{label} must match [a-zA-Z0-9_-] with no whitespace: {name}"
    )
}

fn split_frontmatter(contents: &str) -> anyhow::Result<(&str, &str)> {
    let mut lines = contents.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    if first.trim_end() != "---" {
        anyhow::bail!("command file must start with YAML frontmatter (---)");
    }

    let mut yaml_end_offset = None::<usize>;
    let mut offset = first.len();
    for line in lines {
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]).trim_end();
        if trimmed == "---" {
            yaml_end_offset = Some(offset);
            offset += line.len();
            break;
        }
        offset += line.len();
    }
    let Some(yaml_end_offset) = yaml_end_offset else {
        anyhow::bail!("command file frontmatter is missing closing ---");
    };

    let yaml = &contents[first.len()..yaml_end_offset];
    let body = &contents[offset..];
    Ok((yaml, body))
}

fn normalize_string(value: String, label: &str) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    Ok(value)
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_unique_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = BTreeSet::<String>::new();
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

fn sanitize_frontmatter(
    mut fm: WorkflowFileFrontmatterV1,
    default_name: String,
) -> anyhow::Result<WorkflowFileFrontmatterV1> {
    if fm.version != 1 {
        anyhow::bail!("unsupported command version: {} (expected 1)", fm.version);
    }
    fm.name = normalize_optional_string(fm.name).or(Some(default_name));
    fm.mode = normalize_string(fm.mode, "mode")?;

    if let Some(tools) = fm.allowed_tools.take() {
        fm.allowed_tools = Some(normalize_unique_list(tools));
    }

    let mut seen_inputs = BTreeSet::<String>::new();
    for input in &mut fm.inputs {
        input.name = normalize_string(std::mem::take(&mut input.name), "inputs[].name")?;
        ensure_valid_var_name(&input.name, "inputs[].name")?;
        if !seen_inputs.insert(input.name.clone()) {
            anyhow::bail!("duplicate input name: {}", input.name);
        }
    }

    for step in &mut fm.context {
        if step.argv.is_empty() {
            anyhow::bail!("context argv must not be empty");
        }
        step.summary = normalize_string(std::mem::take(&mut step.summary), "context.summary")?;
        step.argv = step
            .argv
            .drain(..)
            .map(|v| normalize_string(v, "context.argv"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        if let Some(codes) = step.ok_exit_codes.as_mut() {
            if codes.is_empty() {
                anyhow::bail!("context.ok_exit_codes must not be empty");
            }
            codes.sort_unstable();
            codes.dedup();
        }
    }

    Ok(fm)
}

async fn load_workflow_file(cli: &Cli, name: &str) -> anyhow::Result<WorkflowFile> {
    validate_workflow_name(name)?;
    let pm_root = resolve_pm_root(cli)?;
    let dir = workflow_spec_dir(&pm_root);
    if !tokio::fs::try_exists(&dir).await? {
        anyhow::bail!(
            "commands dir is missing: {} (run `pm init`?)",
            dir.display()
        );
    }

    let path = dir.join(format!("{name}.md"));
    let path_canon = tokio::fs::canonicalize(&path)
        .await
        .with_context(|| format!("canonicalize {}", path.display()))?;
    let dir_canon = tokio::fs::canonicalize(&dir)
        .await
        .with_context(|| format!("canonicalize {}", dir.display()))?;
    if !path_canon.starts_with(&dir_canon) {
        anyhow::bail!(
            "refusing to load command outside spec dir: {}",
            path.display()
        );
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let (yaml, body) = split_frontmatter(&raw)?;

    let default_name = name.to_string();
    let fm = serde_yaml::from_str::<WorkflowFileFrontmatterV1>(yaml)
        .context("parse command frontmatter yaml")?;
    let fm = sanitize_frontmatter(fm, default_name)?;

    Ok(WorkflowFile {
        frontmatter: fm,
        body: body.to_string(),
    })
}

fn collect_vars(vars: &[CommandVar]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::<String, String>::new();
    for var in vars {
        if out.contains_key(&var.key) {
            anyhow::bail!("duplicate --var: {}", var.key);
        }
        out.insert(var.key.clone(), var.value.clone());
    }
    Ok(out)
}

fn render_template(
    template: &str,
    declared: &BTreeSet<String>,
    vars: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let (prefix, after_start) = rest.split_at(start);
        out.push_str(prefix);
        let Some(end) = after_start.find("}}") else {
            anyhow::bail!("unclosed template expression: missing '}}'");
        };
        let key = &after_start[2..end];
        if key.is_empty() {
            anyhow::bail!("empty template expression: {{}}");
        }
        if key.trim() != key {
            anyhow::bail!("template vars must not include whitespace: {key}");
        }
        if !is_valid_var_name(key) {
            anyhow::bail!("invalid template var name: {key}");
        }
        if !declared.contains(key) {
            anyhow::bail!("template references undeclared var: {key}");
        }
        let value = vars
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("template var missing value: {key}"))?;
        out.push_str(value);
        rest = &after_start[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

async fn wait_for_process_exit(
    app: &mut App,
    process_id: ProcessId,
    summary: &str,
    ok_exit_codes: &[i32],
) -> anyhow::Result<()> {
    let poll_interval = Duration::from_millis(250);
    loop {
        let resp = app.process_inspect(process_id, Some(0), None).await?;
        let status = resp["process"]["status"].as_str().unwrap_or("unknown");
        if status == "running" {
            tokio::time::sleep(poll_interval).await;
            continue;
        }

        let exit_code = resp["process"]["exit_code"].as_i64().map(|code| code as i32);
        if status == "abandoned" {
            anyhow::bail!("context step abandoned: summary={summary} process_id={process_id}");
        }
        if status != "exited" {
            anyhow::bail!(
                "context step ended with unexpected status: summary={summary} process_id={process_id} status={status}"
            );
        }
        let exit_code = exit_code.ok_or_else(|| {
            anyhow::anyhow!("context step missing exit_code: summary={summary} process_id={process_id}")
        })?;
        if !ok_exit_codes.contains(&exit_code) {
            anyhow::bail!(
                "context step failed: summary={summary} process_id={process_id} exit_code={exit_code} ok_exit_codes={ok_exit_codes:?}"
            );
        }
        return Ok(());
    }
}

async fn run_command_list(cli: &Cli, json: bool) -> anyhow::Result<()> {
    let pm_root = resolve_pm_root(cli)?;
    let dir = workflow_spec_dir(&pm_root);
    if !tokio::fs::try_exists(&dir).await? {
        anyhow::bail!(
            "commands dir is missing: {} (run `pm init`?)",
            dir.display()
        );
    }

    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .with_context(|| format!("read dir {}", dir.display()))?;
    let mut out = Vec::<CommandSummary>::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        let raw = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        let (yaml, _) = split_frontmatter(&raw)
            .with_context(|| format!("parse frontmatter {}", path.display()))?;
        let fm = serde_yaml::from_str::<WorkflowFileFrontmatterV1>(yaml)
            .with_context(|| format!("parse frontmatter yaml {}", path.display()))?;
        let fm = sanitize_frontmatter(fm, stem.to_string())
            .with_context(|| format!("sanitize {}", path.display()))?;

        let name = fm
            .name
            .clone()
            .unwrap_or_else(|| stem.to_string());
        out.push(CommandSummary {
            name,
            version: fm.version,
            mode: fm.mode.clone(),
            file: path.display().to_string(),
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    if json {
        print_json_or_pretty(true, &serde_json::to_value(out)?)?;
    } else {
        for entry in out {
            println!(
                "{} mode={} version={}",
                entry.name, entry.mode, entry.version
            );
        }
    }
    Ok(())
}

async fn run_command_show(cli: &Cli, name: &str, json: bool) -> anyhow::Result<()> {
    let wf = load_workflow_file(cli, name).await?;
    if json {
        let v = serde_json::json!({
            "frontmatter": wf.frontmatter,
            "body": wf.body,
        });
        print_json_or_pretty(true, &v)?;
        return Ok(());
    }

    println!("---");
    print!("{}", serde_yaml::to_string(&wf.frontmatter)?);
    println!("---");
    print!("{}", wf.body);
    if !wf.body.ends_with('\n') {
        println!();
    }
    Ok(())
}

async fn run_command_run(
    cli: &Cli,
    app: &mut App,
    command: &CommandCommand,
) -> anyhow::Result<()> {
    let CommandCommand::Run(args) = command else {
        anyhow::bail!("command execution is only supported via `pm command run`");
    };
    if args.fan_out_early_return && !args.fan_out {
        anyhow::bail!("--fan-out-early-return requires --fan-out");
    }

    let wf = load_workflow_file(cli, &args.name).await?;

    let mut declared = BTreeSet::<String>::new();
    let mut required = BTreeSet::<String>::new();
    for input in &wf.frontmatter.inputs {
        declared.insert(input.name.clone());
        if input.required {
            required.insert(input.name.clone());
        }
    }

    let vars = collect_vars(&args.vars)?;
    for k in vars.keys() {
        if !declared.contains(k) {
            anyhow::bail!("--var references undeclared input: {k}");
        }
    }
    for k in required {
        if !vars.contains_key(&k) {
            anyhow::bail!("missing required --var: {k}");
        }
    }

    let rendered_body = render_template(&wf.body, &declared, &vars)?;

    let thread_result = if let Some(thread_id) = args.thread_id {
        app.thread_resume(thread_id).await?
    } else {
        let cwd = args.cwd.as_ref().map(|p| p.display().to_string());
        app.thread_start(cwd).await?
    };

    let thread_id: ThreadId = serde_json::from_value(thread_result["thread_id"].clone())
        .context("thread_id missing in result")?;
    let mut configure = serde_json::Map::<String, Value>::new();
    configure.insert("thread_id".to_string(), serde_json::json!(thread_id));
    configure.insert("approval_policy".to_string(), Value::Null);
    configure.insert("sandbox_policy".to_string(), Value::Null);
    configure.insert("sandbox_writable_roots".to_string(), Value::Null);
    configure.insert("sandbox_network_access".to_string(), Value::Null);
    configure.insert(
        "mode".to_string(),
        serde_json::json!(wf.frontmatter.mode.clone()),
    );
    configure.insert("model".to_string(), Value::Null);
    configure.insert("openai_base_url".to_string(), Value::Null);
    if let Some(tools) = wf.frontmatter.allowed_tools.clone() {
        configure.insert("allowed_tools".to_string(), serde_json::json!(tools));
    }
    let _ = app.rpc("thread/configure", Value::Object(configure)).await?;

    let mut process_ids = Vec::<ProcessId>::new();
    for step in &wf.frontmatter.context {
        let summary = render_template(&step.summary, &declared, &vars)?;
        let argv = step
            .argv
            .iter()
            .map(|arg| render_template(arg, &declared, &vars))
            .collect::<anyhow::Result<Vec<_>>>()?;
        eprintln!("[command context] {summary}");

        let v = app
            .rpc(
                "process/start",
                serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": null,
                    "approval_id": null,
                    "argv": argv,
                    "cwd": null,
                }),
            )
            .await?;
        if v.get("needs_approval").and_then(|v| v.as_bool()).unwrap_or(false) {
            let approval_id = v
                .get("approval_id")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing approval_id>");
            anyhow::bail!(
                "context step needs approval: {approval_id} (thread={thread_id})"
            );
        }
        if v.get("denied").and_then(|v| v.as_bool()).unwrap_or(false) {
            anyhow::bail!("context step denied: {}", serde_json::to_string(&v)?);
        }
        let process_id: ProcessId = serde_json::from_value(
            v.get("process_id")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing process_id"))?,
        )
        .context("parse process_id")?;
        let ok_exit_codes = step.ok_exit_codes.as_deref().unwrap_or(&[0]);
        wait_for_process_exit(app, process_id, &summary, ok_exit_codes).await?;
        process_ids.push(process_id);
    }

    let mut fan_in_artifact_id: Option<pm_protocol::ArtifactId> = None;
    let mut fan_out_results = Vec::<WorkflowTaskResult>::new();
    let mut fan_out_scheduler: Option<FanOutScheduler> = None;
    if args.fan_out {
        let tasks = parse_workflow_tasks(&rendered_body)?;
        if !tasks.is_empty() {
            let artifact_id = pm_protocol::ArtifactId::new();
            if args.fan_out_early_return {
                let mut scheduler = FanOutScheduler::start(
                    app,
                    thread_id,
                    tasks,
                    artifact_id,
                    wf.frontmatter.subagent_fork,
                )
                .await?;
                scheduler.tick(app, thread_id).await?;
                fan_out_scheduler = Some(scheduler);
            } else {
                fan_out_results = run_workflow_fan_out(
                    app,
                    thread_id,
                    &tasks,
                    artifact_id,
                    wf.frontmatter.subagent_fork,
                )
                .await?;
                let _artifact_path =
                    write_fan_in_summary_artifact(app, thread_id, artifact_id, &fan_out_results)
                        .await?;
            }
            fan_in_artifact_id = Some(artifact_id);
        }
    }

    let mut input = String::new();
    if !process_ids.is_empty() {
        let ids = process_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        input.push_str(&format!(
            "Context steps executed. process_id(s)={ids}. Use `pm process inspect/tail/follow` for details.\n\n"
        ));
    }
    if let Some(artifact_id) = fan_in_artifact_id {
        if args.fan_out_early_return {
            input.push_str(&format!(
                "Fan-out tasks started (early return). fan_in_summary artifact_id={artifact_id} (updates while the main turn runs).\n"
            ));
            if let Some(scheduler) = fan_out_scheduler.as_ref() {
                for task in &scheduler.tasks {
                    input.push_str(&format!("- task_id={} title={}\n", task.id, task.title));
                }
            }
        } else {
            input.push_str(&format!(
                "Fan-out tasks completed. fan_in_summary artifact_id={artifact_id}.\n"
            ));
            for result in &fan_out_results {
                input.push_str(&format!(
                    "- task_id={} thread_id={} turn_id={} status={:?}\n",
                    result.task_id, result.thread_id, result.turn_id, result.status
                ));
            }
        }
        input.push('\n');
        input.push_str(&format!(
            "Use `pm artifact read {thread_id} {artifact_id}` for the full fan-in summary.\n\n"
        ));
    }
    input.push_str(&rendered_body);

    let ask_args = AskArgs {
        thread_id: Some(thread_id),
        cwd: None,
        approval_policy: None,
        sandbox_policy: None,
        mode: None,
        model: None,
        openai_base_url: None,
        input,
    };

    if let Some(scheduler) = fan_out_scheduler {
        let artifact_id = fan_in_artifact_id.unwrap_or_default();
        let scheduler = Arc::new(tokio::sync::Mutex::new(Some(scheduler)));

        let scheduler_for_tick = scheduler.clone();
        run_ask_with_tick(app, ask_args, move |app, parent_thread_id, _turn_id| {
            let scheduler_for_tick = scheduler_for_tick.clone();
            Box::pin(async move {
                let mut guard = scheduler_for_tick.lock().await;
                let Some(mut scheduler) = guard.take() else {
                    return Ok(());
                };
                drop(guard);

                scheduler.tick(app, parent_thread_id).await?;
                if scheduler.is_done() && !scheduler.final_summary_written {
                    let results = scheduler.results_ordered();
                    let outcome =
                        write_fan_in_summary_artifact(app, parent_thread_id, artifact_id, &results)
                            .await;
                    if let Err(err) = outcome {
                        eprintln!("[fan-out] final summary artifact write failed: {err}");
                    } else {
                        scheduler.final_summary_written = true;
                    }
                }

                let mut guard = scheduler_for_tick.lock().await;
                *guard = Some(scheduler);
                Ok(())
            })
        })
        .await?;

        let mut scheduler = scheduler
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow::anyhow!("fan-out scheduler missing"))?;
        while !scheduler.is_done() {
            scheduler.tick(app, thread_id).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if !scheduler.final_summary_written {
            let results = scheduler.results_ordered();
            let _artifact_path =
                write_fan_in_summary_artifact(app, thread_id, artifact_id, &results).await?;
        }
    } else {
        run_ask(app, ask_args).await?;
    }

    Ok(())
}

