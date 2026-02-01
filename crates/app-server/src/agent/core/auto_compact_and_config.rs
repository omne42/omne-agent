async fn auto_compact_summary(
    ctx: AutoCompactSummaryContext<'_>,
    cfg: AutoCompactSummaryConfig,
) -> anyhow::Result<bool> {
    let AutoCompactSummaryContext {
        server,
        thread_id,
        turn_id,
        model,
        llm,
        turn_priority,
        max_openai_request_duration,
        max_total_tokens,
        total_tokens_used,
        input_items,
    } = ctx;

    let transcript = render_items_for_summary(input_items, cfg.source_max_chars);
    if transcript.trim().is_empty() {
        return Ok(false);
    }

    let prompt = format!(
        "# Summarize session\n\nthread_id: {thread_id}\nturn_id: {turn_id}\n\n## Transcript\n\n{transcript}"
    );

    let messages = vec![
        ditto_llm::Message::system(SUMMARY_INSTRUCTIONS),
        ditto_llm::Message::user(prompt),
    ];
    let mut req = ditto_llm::GenerateRequest::from(messages);
    req.model = Some(model.to_string());
    req.tool_choice = Some(ditto_llm::ToolChoice::None);

    let _permit = LlmWorkerPool::global().acquire(turn_priority).await?;
    let resp = match tokio::time::timeout(max_openai_request_duration, llm.generate(req)).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) => return Ok(false),
        Err(_) => return Ok(false),
    };

    if max_total_tokens > 0
        && let Some(usage) = token_usage_json_from_ditto_usage(&resp.usage)
        && let Some(tokens) = usage_total_tokens(&usage)
    {
        *total_tokens_used = total_tokens_used.saturating_add(tokens);
        if *total_tokens_used > max_total_tokens {
            return Err(
                AgentTurnError::TokenBudgetExceeded {
                    used: *total_tokens_used,
                    limit: max_total_tokens,
                }
                .into(),
            );
        }
    }

    let summary_text = resp.text();
    let summary_text = summary_text.trim();
    if summary_text.is_empty() {
        return Ok(false);
    }
    let summary_text = omne_agent_core::redact_text(summary_text);
    let summary_text = truncate_chars(&summary_text, 20_000);

    let artifact_value = match crate::handle_artifact_write(
        server,
        crate::ArtifactWriteParams {
            thread_id,
            turn_id: Some(turn_id),
            approval_id: None,
            artifact_id: None,
            artifact_type: "summary".to_string(),
            summary: "Summary (auto compact)".to_string(),
            text: summary_text.clone(),
        },
    )
    .await
    {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };

    if artifact_value
        .get("needs_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || artifact_value
            .get("denied")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return Ok(false);
    }

    let artifact_id = artifact_value
        .get("artifact_id")
        .cloned()
        .and_then(|value| serde_json::from_value::<ArtifactId>(value).ok());

    let tail_count = cfg.tail_items.min(input_items.len());
    let mut tail = input_items
        .iter()
        .rev()
        .take(tail_count)
        .cloned()
        .collect::<Vec<_>>();
    tail.reverse();

    let mut system_text = String::new();
    system_text.push_str("# Context summary\n\n");
    system_text.push_str(summary_text.trim());
    if let Some(artifact_id) = artifact_id {
        system_text.push_str(&format!("\n\n(summary artifact_id: {artifact_id})"));
    }

    input_items.clear();
    input_items.push(serde_json::json!({
        "type": "message",
        "role": "system",
        "content": [{ "type": "input_text", "text": system_text }],
    }));
    input_items.extend(tail);

    Ok(true)
}

fn resolve_user_instructions_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OMNE_AGENT_USER_INSTRUCTIONS_FILE") {
        let path = path.trim();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    let home = home_dir()?;
    Some(home.join(".omne_agent_data").join("AGENTS.md"))
}

fn builtin_openai_provider_config(provider: &str) -> Option<ditto_llm::ProviderConfig> {
    match provider {
        "openai-codex-apikey" => Some(ditto_llm::ProviderConfig {
            base_url: Some(DEFAULT_OPENAI_BASE_URL.to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            http_headers: Default::default(),
            http_query_params: Default::default(),
            auth: Some(ditto_llm::ProviderAuth::ApiKeyEnv { keys: Vec::new() }),
            capabilities: None,
        }),
        "openai-auth-command" => Some(ditto_llm::ProviderConfig {
            base_url: Some(DEFAULT_OPENAI_BASE_URL.to_string()),
            default_model: None,
            model_whitelist: Vec::new(),
            http_headers: Default::default(),
            http_query_params: Default::default(),
            auth: Some(ditto_llm::ProviderAuth::Command { command: Vec::new() }),
            capabilities: None,
        }),
        _ => None,
    }
}

fn merge_provider_config(
    mut base: ditto_llm::ProviderConfig,
    overrides: &ditto_llm::ProviderConfig,
) -> ditto_llm::ProviderConfig {
    if let Some(base_url) = overrides
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        base.base_url = Some(base_url.to_string());
    }
    if let Some(default_model) = overrides
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        base.default_model = Some(default_model.to_string());
    }
    if !overrides.model_whitelist.is_empty() {
        base.model_whitelist =
            ditto_llm::normalize_string_list(overrides.model_whitelist.clone());
    }
    if !overrides.http_headers.is_empty() {
        base.http_headers.extend(overrides.http_headers.clone());
    }
    if !overrides.http_query_params.is_empty() {
        base.http_query_params
            .extend(overrides.http_query_params.clone());
    }
    if let Some(auth) = overrides.auth.clone() {
        base.auth = Some(auth);
    }
    if let Some(capabilities) = overrides.capabilities {
        base.capabilities = Some(capabilities);
    }
    base
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
}

#[derive(Debug, Default)]
struct SkillOverrides {
    model: Option<String>,
    thinking: Option<String>,
    model_sources: Vec<String>,
    thinking_sources: Vec<String>,
}

#[derive(Debug)]
struct LoadedSkillsFromInput {
    markdown: String,
    overrides: SkillOverrides,
}

#[derive(Debug, Default, serde::Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

#[derive(Debug)]
struct LoadedSkill {
    path: PathBuf,
    body: String,
    model: Option<String>,
    thinking: Option<String>,
}

fn split_skill_frontmatter(contents: &str) -> anyhow::Result<(Option<&str>, &str)> {
    let mut lines = contents.split_inclusive('\n');
    let first = lines.next().unwrap_or("");
    if first.trim_end() != "---" {
        return Ok((None, contents));
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
        anyhow::bail!("skill file frontmatter is missing closing ---");
    };

    let yaml = &contents[first.len()..yaml_end_offset];
    let body = &contents[offset..];
    Ok((Some(yaml), body))
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_optional_thinking(value: Option<String>) -> anyhow::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let value = value.to_ascii_lowercase();
    match value.as_str() {
        "unsupported" | "small" | "medium" | "high" | "xhigh" => Ok(Some(value)),
        other => anyhow::bail!(
            "invalid thinking: {other} (expected: unsupported|small|medium|high|xhigh)"
        ),
    }
}

async fn load_skill(name: &str, thread_root: PathBuf) -> anyhow::Result<Option<LoadedSkill>> {
    let mut roots = Vec::<PathBuf>::new();

    if let Ok(dir) = std::env::var("OMNE_AGENT_SKILLS_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            roots.push(PathBuf::from(dir));
        }
    }

    roots.push(thread_root.join(".omne_agent_data").join("spec").join("skills"));
    roots.push(thread_root.join(".codex").join("skills"));

    if let Some(home) = home_dir() {
        roots.push(home.join(".omne_agent_data").join("spec").join("skills"));
    }

    let candidates = [name.to_string(), name.to_ascii_lowercase()];
    for root in roots {
        for candidate in candidates.iter() {
            let path = root.join(candidate).join("SKILL.md");
            let raw = match tokio::fs::read_to_string(&path).await {
                Ok(contents) => contents,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
            };

            let (yaml, body) = split_skill_frontmatter(&raw)
                .with_context(|| format!("parse skill frontmatter {}", path.display()))?;
            let frontmatter = match yaml {
                Some(yaml) if !yaml.trim().is_empty() => serde_yaml::from_str::<SkillFrontmatter>(yaml)
                    .with_context(|| format!("parse skill frontmatter yaml {}", path.display()))?,
                _ => SkillFrontmatter::default(),
            };

            let model = normalize_optional_string(frontmatter.model);
            let thinking = normalize_optional_thinking(frontmatter.thinking)
                .with_context(|| format!("parse skill thinking {}", path.display()))?;
            let body = omne_agent_core::redact_text(body);

            return Ok(Some(LoadedSkill {
                path,
                body,
                model,
                thinking,
            }));
        }
    }

    Ok(None)
}

async fn load_skills_from_input(
    input: &str,
    thread_cwd: Option<&str>,
) -> anyhow::Result<Option<LoadedSkillsFromInput>> {

    let skill_names = parse_skill_names(input);
    if skill_names.is_empty() {
        return Ok(None);
    }

    let Some(thread_cwd) = thread_cwd else {
        return Ok(None);
    };

    let mut out = String::new();
    let mut overrides = SkillOverrides::default();

    for name in skill_names {
        if let Some(loaded) = load_skill(&name, PathBuf::from(thread_cwd)).await? {
            if let Some(model) = loaded.model.as_deref() {
                match overrides.model.as_deref() {
                    None => {
                        overrides.model = Some(model.to_string());
                        overrides.model_sources.push(name.clone());
                    }
                    Some(existing) if existing == model => {
                        overrides.model_sources.push(name.clone());
                    }
                    Some(existing) => {
                        anyhow::bail!(
                            "conflicting skill model overrides: existing={existing} new={model} (skill={name})"
                        );
                    }
                }
            }
            if let Some(thinking) = loaded.thinking.as_deref() {
                match overrides.thinking.as_deref() {
                    None => {
                        overrides.thinking = Some(thinking.to_string());
                        overrides.thinking_sources.push(name.clone());
                    }
                    Some(existing) if existing == thinking => {
                        overrides.thinking_sources.push(name.clone());
                    }
                    Some(existing) => {
                        anyhow::bail!(
                            "conflicting skill thinking overrides: existing={existing} new={thinking} (skill={name})"
                        );
                    }
                }
            }

            out.push_str("\n\n# Skill\n\n");
            out.push_str(&format!("_Name: `{}`_\n\n", name));
            out.push_str(&format!("_Source: {}_\n\n", loaded.path.display()));
            out.push_str(&loaded.body);
        } else {
            out.push_str("\n\n# Skill (missing)\n\n");
            out.push_str(&format!("_Name: `{}`_\n\n", name));
            out.push_str("_Reason: not found in configured skill directories._\n");
        }
    }

    Ok(Some(LoadedSkillsFromInput {
        markdown: out,
        overrides,
    }))
}

fn parse_skill_names(input: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = std::collections::HashSet::<String>::new();

    let chars = input.chars().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] != '$' {
            idx += 1;
            continue;
        }
        idx += 1;
        let start = idx;
        while idx < chars.len() {
            let c = chars[idx];
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                idx += 1;
                continue;
            }
            break;
        }
        if idx <= start {
            continue;
        }
        let name = chars[start..idx].iter().collect::<String>();
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }

    out
}


fn parse_env_usize(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn parse_env_u64(key: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn parse_bool_value(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|value| parse_bool_value(&value))
        .unwrap_or(default)
}

fn tool_is_read_only(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_read"
            | "file_glob"
            | "file_grep"
            | "process_inspect"
            | "process_tail"
            | "process_follow"
            | "artifact_list"
            | "artifact_read"
            | "thread_state"
            | "thread_events"
    )
}

fn usage_total_tokens(usage: &Value) -> Option<u64> {
    let total_tokens = usage.get("total_tokens").and_then(Value::as_u64);
    let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
    let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);

    total_tokens.or_else(|| match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => input.checked_add(output),
        (Some(input), None) => Some(input),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    })
}

async fn thread_total_tokens_used(
    thread_store: &omne_agent_core::ThreadStore,
    thread_id: ThreadId,
) -> anyhow::Result<u64> {
    Ok(thread_store
        .read_state(thread_id)
        .await?
        .map(|state| state.total_tokens_used)
        .unwrap_or(0))
}

async fn load_latest_summary_artifact(
    server: &super::Server,
    thread_id: ThreadId,
) -> anyhow::Result<Option<(ArtifactMetadata, String)>> {
    let dir = crate::user_artifacts_dir_for_thread(server, thread_id);
    let mut read_dir = match tokio::fs::read_dir(&dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", dir.display())),
    };

    let mut latest: Option<ArtifactMetadata> = None;
    loop {
        let Some(entry) = read_dir
            .next_entry()
            .await
            .with_context(|| format!("read {}", dir.display()))?
        else {
            break;
        };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.ends_with(".metadata.json") {
            continue;
        }

        let meta = match crate::read_artifact_metadata(&path).await {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.artifact_type != "summary" {
            continue;
        }

        let should_replace = match &latest {
            None => true,
            Some(prev) => meta
                .updated_at
                .unix_timestamp_nanos()
                .cmp(&prev.updated_at.unix_timestamp_nanos())
                .then_with(|| meta.artifact_id.cmp(&prev.artifact_id))
                .is_gt(),
        };
        if should_replace {
            latest = Some(meta);
        }
    }

    let Some(meta) = latest else {
        return Ok(None);
    };

    let (content_path, _metadata_path) = crate::user_artifact_paths(server, thread_id, meta.artifact_id);
    let text = tokio::fs::read_to_string(&content_path)
        .await
        .with_context(|| format!("read {}", content_path.display()))?;
    Ok(Some((meta, text)))
}
