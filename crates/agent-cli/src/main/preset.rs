mod preset {
    use std::path::{Path, PathBuf};

    use anyhow::Context;
    use omne_protocol::{ApprovalPolicy, SandboxNetworkAccess, SandboxPolicy, ThreadId};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PresetFileV1 {
        version: u32,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        portability_warnings: Vec<String>,
        thread_config: PresetThreadConfig,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PresetThreadConfig {
        approval_policy: ApprovalPolicy,
        sandbox_policy: SandboxPolicy,
        sandbox_network_access: SandboxNetworkAccess,
        sandbox_writable_roots: Vec<String>,
        mode: String,
        model: String,
        openai_base_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allowed_tools: Option<Option<Vec<String>>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        execpolicy_rules: Vec<String>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PresetListItem {
        file: String,
        version: u32,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PresetListError {
        file: String,
        code: String,
        error: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PresetListResult {
        presets: Vec<PresetListItem>,
        errors: Vec<PresetListError>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PresetValidateItem {
        file: String,
        version: u32,
        name: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PresetValidateError {
        file: String,
        code: String,
        error: String,
    }

    #[derive(Debug, Clone, Serialize)]
    struct PresetValidateResult {
        ok: bool,
        strict: bool,
        validated: Vec<PresetValidateItem>,
        errors: Vec<PresetValidateError>,
    }

    fn normalize_string(value: String, label: &str) -> anyhow::Result<String> {
        let value = value.trim().to_string();
        if value.is_empty() {
            anyhow::bail!("{label} must not be empty");
        }
        Ok(value)
    }

    fn normalize_string_opt(value: Option<String>) -> Option<String> {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn normalize_string_list(values: Vec<String>) -> Vec<String> {
        let mut out = Vec::<String>::new();
        let mut seen = std::collections::BTreeSet::<String>::new();
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

    fn build_export_portability_warnings(roots: &[String], thread_root: &Path) -> Vec<String> {
        let mut out = Vec::<String>::new();
        for root in roots {
            let root_path = Path::new(root);
            if root_path.is_absolute() && !root_path.starts_with(thread_root) {
                out.push(format!(
                    "sandbox_writable_roots contains path outside thread root: {root}"
                ));
            }
        }
        out
    }

    fn relativize_roots(roots: Vec<String>, thread_root: &Path) -> Vec<String> {
        let mut out = Vec::with_capacity(roots.len());
        for root in roots {
            let root_path = Path::new(&root);
            if let Ok(relative) = root_path.strip_prefix(thread_root) {
                if relative.as_os_str().is_empty() {
                    out.push(".".to_string());
                } else {
                    out.push(relative.to_string_lossy().to_string());
                }
            } else {
                out.push(root);
            }
        }
        out
    }

    async fn resolve_thread_root_for_export(
        app: &mut super::App,
        thread_id: ThreadId,
    ) -> anyhow::Result<Option<PathBuf>> {
        let state = app.thread_state(thread_id).await?;
        let Some(cwd) = state.cwd else {
            return Ok(None);
        };
        let canon = tokio::fs::canonicalize(&cwd)
            .await
            .with_context(|| format!("canonicalize thread cwd {cwd}"))?;
        Ok(Some(canon))
    }

    fn ensure_within_spec_dir(omne_root: &Path, file: &Path) -> anyhow::Result<()> {
        let spec_dir = omne_root.join("spec");
        if !spec_dir.exists() {
            anyhow::bail!("spec dir is missing: {} (run `omne init`?)", spec_dir.display());
        }

        let spec_dir = std::fs::canonicalize(&spec_dir)
            .with_context(|| format!("canonicalize {}", spec_dir.display()))?;
        let file =
            std::fs::canonicalize(file).with_context(|| format!("canonicalize {}", file.display()))?;
        if !file.starts_with(&spec_dir) {
            anyhow::bail!(
                "refusing to load preset outside spec dir: file={} (spec_dir={})",
                file.display(),
                spec_dir.display()
            );
        }
        Ok(())
    }

    async fn list_preset_paths(spec_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
        if !tokio::fs::try_exists(spec_dir).await? {
            anyhow::bail!("spec dir is missing: {} (run `omne init`?)", spec_dir.display());
        }

        let mut out = Vec::<PathBuf>::new();
        let default_yaml = spec_dir.join("preset.yaml");
        if tokio::fs::try_exists(&default_yaml).await? {
            out.push(default_yaml);
        }
        let default_yml = spec_dir.join("preset.yml");
        if tokio::fs::try_exists(&default_yml).await? {
            out.push(default_yml);
        }

        let presets_dir = spec_dir.join("presets");
        if tokio::fs::try_exists(&presets_dir).await? {
            let mut entries = tokio::fs::read_dir(&presets_dir)
                .await
                .with_context(|| format!("read dir {}", presets_dir.display()))?;
            while let Some(entry) = entries.next_entry().await? {
                let file_type = entry.file_type().await?;
                if !file_type.is_file() {
                    continue;
                }
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|v| v.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if ext == "yaml" || ext == "yml" {
                    out.push(path);
                }
            }
        }

        out.sort();
        out.dedup();
        Ok(out)
    }

    fn sanitize_preset(mut preset: PresetFileV1) -> anyhow::Result<PresetFileV1> {
        if preset.version != 1 {
            anyhow::bail!("unsupported preset version: {} (expected 1)", preset.version);
        }

        preset.name = normalize_string(preset.name, "preset.name")?;
        preset.description = normalize_string_opt(preset.description);
        preset.portability_warnings = normalize_string_list(preset.portability_warnings);

        preset.thread_config.mode =
            normalize_string(preset.thread_config.mode, "thread_config.mode")?;
        preset.thread_config.model =
            normalize_string(preset.thread_config.model, "thread_config.model")?;
        preset.thread_config.openai_base_url = normalize_string(
            preset.thread_config.openai_base_url,
            "thread_config.openai_base_url",
        )?;
        preset.thread_config.sandbox_writable_roots =
            normalize_string_list(preset.thread_config.sandbox_writable_roots);
        preset.thread_config.allowed_tools = match preset.thread_config.allowed_tools {
            None => None,
            Some(None) => Some(None),
            Some(Some(values)) => Some(Some(normalize_string_list(values))),
        };
        preset.thread_config.execpolicy_rules =
            normalize_string_list(preset.thread_config.execpolicy_rules);

        ensure_preset_has_no_secret_like_values(&preset)?;

        Ok(preset)
    }

    fn ensure_preset_has_no_secret_like_values(preset: &PresetFileV1) -> anyhow::Result<()> {
        fn check_value(label: &str, value: &str) -> anyhow::Result<()> {
            if let Some(reason) = detect_secret_like_value(value) {
                anyhow::bail!(
                    "{label} contains secret-like value ({reason}); preset files must not contain secrets or env placeholders"
                );
            }
            Ok(())
        }

        check_value("preset.name", &preset.name)?;
        if let Some(description) = preset.description.as_deref() {
            check_value("preset.description", description)?;
        }
        for (idx, warning) in preset.portability_warnings.iter().enumerate() {
            check_value(&format!("portability_warnings[{idx}]"), warning)?;
        }

        let cfg = &preset.thread_config;
        check_value("thread_config.mode", &cfg.mode)?;
        check_value("thread_config.model", &cfg.model)?;
        check_value("thread_config.openai_base_url", &cfg.openai_base_url)?;
        for (idx, root) in cfg.sandbox_writable_roots.iter().enumerate() {
            check_value(&format!("thread_config.sandbox_writable_roots[{idx}]"), root)?;
        }
        if let Some(Some(allowed_tools)) = cfg.allowed_tools.as_ref() {
            for (idx, tool) in allowed_tools.iter().enumerate() {
                check_value(&format!("thread_config.allowed_tools[{idx}]"), tool)?;
            }
        }
        for (idx, rule) in cfg.execpolicy_rules.iter().enumerate() {
            check_value(&format!("thread_config.execpolicy_rules[{idx}]"), rule)?;
        }

        Ok(())
    }

    fn detect_secret_like_value(value: &str) -> Option<&'static str> {
        fn has_long_prefixed_token(haystack: &str, prefix: &str, min_tail_len: usize) -> bool {
            let mut start = 0usize;
            while let Some(offset) = haystack[start..].find(prefix) {
                let idx = start + offset + prefix.len();
                let tail_len = haystack[idx..]
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
                    .count();
                if tail_len >= min_tail_len {
                    return true;
                }
                start = idx;
            }
            false
        }

        fn has_aws_access_key(haystack: &str) -> bool {
            let bytes = haystack.as_bytes();
            if bytes.len() < 20 {
                return false;
            }
            for i in 0..=bytes.len() - 20 {
                if &bytes[i..i + 4] != b"AKIA" {
                    continue;
                }
                if bytes[i + 4..i + 20]
                    .iter()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                {
                    return true;
                }
            }
            false
        }

        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        let lower = trimmed.to_ascii_lowercase();

        if lower.contains("{{env:") {
            return Some("env placeholder");
        }
        if trimmed.contains("-----BEGIN ") {
            return Some("pem/private key block");
        }
        if lower.starts_with("bearer ") && trimmed.len() > "bearer ".len() + 20 {
            return Some("bearer token");
        }
        if lower.contains("api_key=")
            || lower.contains("apikey=")
            || lower.contains("access_token=")
            || lower.contains("token=")
        {
            return Some("query-style credential");
        }
        if has_long_prefixed_token(&lower, "sk-", 20) {
            return Some("openai-style key");
        }
        if has_long_prefixed_token(&lower, "ghp_", 20)
            || has_long_prefixed_token(&lower, "gho_", 20)
            || has_long_prefixed_token(&lower, "ghu_", 20)
            || has_long_prefixed_token(&lower, "ghs_", 20)
            || has_long_prefixed_token(&lower, "ghr_", 20)
            || has_long_prefixed_token(&lower, "github_pat_", 20)
        {
            return Some("github token");
        }
        if has_long_prefixed_token(&lower, "xoxb-", 20)
            || has_long_prefixed_token(&lower, "xoxp-", 20)
            || has_long_prefixed_token(&lower, "xoxa-", 20)
            || has_long_prefixed_token(&lower, "xoxr-", 20)
            || has_long_prefixed_token(&lower, "xoxs-", 20)
        {
            return Some("slack token");
        }
        if has_aws_access_key(trimmed) {
            return Some("aws access key id");
        }

        None
    }

    async fn write_preset_file(out: &Path, preset: &PresetFileV1) -> anyhow::Result<()> {
        if let Some(parent) = out.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let yaml = serde_yaml::to_string(preset).context("serialize preset yaml")?;
        tokio::fs::write(out, yaml)
            .await
            .with_context(|| format!("write {}", out.display()))?;
        Ok(())
    }

    async fn read_preset_file(path: &Path) -> anyhow::Result<PresetFileV1> {
        let raw = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        let parsed = serde_yaml::from_str::<PresetFileV1>(&raw).context("parse preset yaml")?;
        sanitize_preset(parsed)
    }

    async fn apply_preset(
        app: &mut super::App,
        thread_id: ThreadId,
        preset: &PresetFileV1,
    ) -> anyhow::Result<()> {
        let cfg = &preset.thread_config;
        app.thread_configure_rpc(omne_app_server_protocol::ThreadConfigureParams {
            thread_id,
            approval_policy: Some(cfg.approval_policy),
            sandbox_policy: Some(cfg.sandbox_policy),
            sandbox_writable_roots: Some(cfg.sandbox_writable_roots.clone()),
            sandbox_network_access: Some(cfg.sandbox_network_access),
            mode: Some(cfg.mode.clone()),
            model: Some(cfg.model.clone()),
            thinking: None,
            show_thinking: None,
            openai_base_url: Some(cfg.openai_base_url.clone()),
            allowed_tools: cfg.allowed_tools.clone(),
            execpolicy_rules: Some(cfg.execpolicy_rules.clone()),
        })
        .await
    }

    async fn record_preset_provenance(
        app: &mut super::App,
        thread_id: ThreadId,
        file: &Path,
        preset: &PresetFileV1,
    ) -> Option<String> {
        let payload = serde_json::json!({
            "preset_name": preset.name,
            "preset_description": preset.description,
            "preset_file": file.display().to_string(),
        });
        let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
        let summary = format!(
            "preset applied: {} ({})",
            preset.name.trim(),
            file.display()
        );

        let result = app
            .artifact_write(omne_app_server_protocol::ArtifactWriteParams {
                thread_id,
                turn_id: None,
                approval_id: None,
                artifact_id: None,
                artifact_type: "preset_applied".to_string(),
                summary,
                text,
            })
            .await
            .ok()?;

        Some(result.artifact_id.to_string())
    }

    async fn export_preset(
        app: &mut super::App,
        thread_id: ThreadId,
        out: PathBuf,
        name: Option<String>,
        description: Option<String>,
        json: bool,
    ) -> anyhow::Result<()> {
        let explain = app.thread_config_explain(thread_id).await?;
        let original_sandbox_writable_roots = explain.effective.sandbox_writable_roots.clone();
        let mut thread_config = PresetThreadConfig {
            approval_policy: explain.effective.approval_policy,
            sandbox_policy: explain.effective.sandbox_policy,
            sandbox_network_access: explain.effective.sandbox_network_access,
            sandbox_writable_roots: original_sandbox_writable_roots.clone(),
            mode: explain.effective.mode,
            model: explain.effective.model,
            openai_base_url: explain.effective.openai_base_url,
            allowed_tools: Some(explain.effective.allowed_tools),
            execpolicy_rules: explain.effective.execpolicy_rules,
        };
        let mut portability_warnings = Vec::<String>::new();

        if let Some(thread_root) = resolve_thread_root_for_export(app, thread_id).await? {
            portability_warnings =
                build_export_portability_warnings(&original_sandbox_writable_roots, &thread_root);
            thread_config.sandbox_writable_roots =
                relativize_roots(thread_config.sandbox_writable_roots, &thread_root);
        }

        let name = normalize_string_opt(name).or_else(|| {
            out.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty())
        });
        let name =
            name.ok_or_else(|| anyhow::anyhow!("preset name is missing (pass --name or use a file name)"))?;

        let preset = PresetFileV1 {
            version: 1,
            name,
            description: normalize_string_opt(description),
            portability_warnings,
            thread_config,
        };

        write_preset_file(&out, &preset).await?;

        if json {
            let result = serde_json::json!({
                "ok": true,
                "thread_id": thread_id,
                "out": out.display().to_string(),
                "preset": preset,
            });
            super::print_json_or_pretty(true, &result)?;
        } else {
            println!("ok: wrote preset {}", out.display());
            for warning in &preset.portability_warnings {
                println!("warning: {warning}");
            }
        }

        Ok(())
    }

    async fn import_preset(
        cli: &super::Cli,
        app: &mut super::App,
        thread_id: ThreadId,
        file: Option<PathBuf>,
        name: Option<String>,
        json: bool,
    ) -> anyhow::Result<()> {
        let omne_root = super::resolve_pm_root(cli)?;
        let file = resolve_import_file(&omne_root, file, name).await?;
        ensure_within_spec_dir(&omne_root, &file)?;

        let preset = read_preset_file(&file).await?;
        apply_preset(app, thread_id, &preset).await?;
        let provenance_artifact_id =
            record_preset_provenance(app, thread_id, &file, &preset).await;

        if json {
            let result = serde_json::json!({
                "ok": true,
                "thread_id": thread_id,
                "file": file.display().to_string(),
                "preset": preset,
                "provenance_artifact_id": provenance_artifact_id,
            });
            super::print_json_or_pretty(true, &result)?;
        } else if let Some(artifact_id) = provenance_artifact_id {
            println!(
                "ok: applied preset {} (provenance: {})",
                file.display(),
                artifact_id
            );
        } else {
            println!("ok: applied preset {}", file.display());
        }

        Ok(())
    }

    pub(super) async fn run_preset_import(
        cli: &super::Cli,
        app: &mut super::App,
        thread_id: ThreadId,
        file: Option<PathBuf>,
        name: Option<String>,
        json: bool,
    ) -> anyhow::Result<()> {
        match import_preset(cli, app, thread_id, file, name, json).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if json {
                    let code = classify_preset_error(&err);
                    let result = preset_error_payload(code, &err);
                    super::print_json_or_pretty(true, &result)?;
                }
                Err(err)
            }
        }
    }

    pub(super) async fn run_preset_show(
        cli: &super::Cli,
        file: Option<PathBuf>,
        name: Option<String>,
        json: bool,
    ) -> anyhow::Result<()> {
        let result = async {
            let omne_root = super::resolve_pm_root(cli)?;
            let file = resolve_import_file(&omne_root, file, name).await?;
            ensure_within_spec_dir(&omne_root, &file)?;
            let preset = read_preset_file(&file).await?;
            Ok::<(PathBuf, PresetFileV1), anyhow::Error>((file, preset))
        }
        .await;

        match result {
            Ok((file, preset)) => {
                if json {
                    let result = serde_json::json!({
                        "ok": true,
                        "file": file.display().to_string(),
                        "preset": preset,
                    });
                    super::print_json_or_pretty(true, &result)?;
                } else {
                    println!("# {}", file.display());
                    let yaml = serde_yaml::to_string(&preset).context("serialize preset yaml")?;
                    print!("{yaml}");
                }
                Ok(())
            }
            Err(err) => {
                if json {
                    let code = classify_preset_error(&err);
                    let result = preset_error_payload(code, &err);
                    super::print_json_or_pretty(true, &result)?;
                }
                Err(err)
            }
        }
    }

    async fn resolve_import_file(
        omne_root: &Path,
        file: Option<PathBuf>,
        name: Option<String>,
    ) -> anyhow::Result<PathBuf> {
        match (file, name) {
            (Some(file), None) => Ok(file),
            (None, Some(name)) => resolve_preset_file_by_name(&omne_root.join("spec"), &name).await,
            (Some(_), Some(_)) => anyhow::bail!("pass either --file or --name, not both"),
            (None, None) => anyhow::bail!("missing preset selector: pass --file <path> or --name <name>"),
        }
    }

    async fn resolve_preset_file_by_name(spec_dir: &Path, name: &str) -> anyhow::Result<PathBuf> {
        let target = normalize_string(name.to_string(), "preset name")?;
        if target.contains('/') || target.contains('\\') {
            anyhow::bail!("preset name must not contain path separators: {target}");
        }

        let files = list_preset_paths(spec_dir).await?;
        let mut matches = Vec::<PathBuf>::new();
        let mut available = Vec::<String>::new();
        for file in files {
            match read_preset_file(&file).await {
                Ok(preset) => {
                    let stem = file
                        .file_stem()
                        .and_then(|v| v.to_str())
                        .unwrap_or_default()
                        .to_string();
                    available.push(preset.name.clone());
                    if preset.name == target || stem == target {
                        matches.push(file);
                    }
                }
                Err(_) => {
                    // Ignore parse-failed preset files when resolving by name.
                }
            }
        }

        available.sort();
        available.dedup();
        matches.sort();
        matches.dedup();

        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => {
                if available.is_empty() {
                    anyhow::bail!(
                        "preset `{target}` not found (no parseable presets under {}); run `omne preset list`",
                        spec_dir.display()
                    );
                }
                anyhow::bail!(
                    "preset `{target}` not found; available preset names: {}",
                    available.join(", ")
                );
            }
            _ => {
                let lines = matches
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::bail!("preset `{target}` is ambiguous; matched files: {lines}");
            }
        }
    }

    async fn resolve_validate_targets(
        omne_root: &Path,
        file: Option<PathBuf>,
        name: Option<String>,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let spec_dir = omne_root.join("spec");
        match (file, name) {
            (Some(file), None) => Ok(vec![file]),
            (None, Some(name)) => Ok(vec![resolve_preset_file_by_name(&spec_dir, &name).await?]),
            (Some(_), Some(_)) => anyhow::bail!("pass either --file or --name, not both"),
            (None, None) => list_preset_paths(&spec_dir).await,
        }
    }

    fn classify_preset_error(err: &anyhow::Error) -> &'static str {
        let msg = err.to_string().to_ascii_lowercase();
        if msg.contains("pass either --file or --name") {
            "selector_conflict"
        } else if msg.contains("missing preset selector") {
            "selector_missing"
        } else if msg.contains("must not contain path separators") {
            "invalid_name"
        } else if msg.contains("not found; available preset names")
            || msg.contains("not found (no parseable presets")
        {
            "name_not_found"
        } else if msg.contains("is ambiguous; matched files") {
            "name_ambiguous"
        } else if msg.contains("refusing to load preset outside spec dir") {
            "outside_spec_dir"
        } else if msg.contains("spec dir is missing") {
            "spec_dir_missing"
        } else if msg.contains("parse preset yaml") {
            "parse_yaml"
        } else if msg.contains("unsupported preset version") {
            "unsupported_version"
        } else if msg.contains("contains secret-like value") {
            "secret_like_value"
        } else if msg.contains("must not be empty") {
            "invalid_field"
        } else if msg.contains("canonicalize") || msg.contains("no such file or directory") {
            "file_access"
        } else {
            "validation_error"
        }
    }

    fn preset_error_payload(code: &str, err: &anyhow::Error) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "code": code,
            "message": format!("{err:#}"),
        })
    }

    fn maybe_print_preset_json_error(json: bool, err: &anyhow::Error) -> anyhow::Result<()> {
        if json {
            let code = classify_preset_error(err);
            let result = preset_error_payload(code, err);
            super::print_json_or_pretty(true, &result)?;
        }
        Ok(())
    }

    fn duplicate_name_errors(validated: &[PresetValidateItem]) -> Vec<PresetValidateError> {
        let mut by_name = std::collections::BTreeMap::<String, Vec<String>>::new();
        for item in validated {
            by_name
                .entry(item.name.clone())
                .or_default()
                .push(item.file.clone());
        }

        let mut errors = Vec::<PresetValidateError>::new();
        for (name, files) in by_name {
            if files.len() < 2 {
                continue;
            }
            for file in &files {
                let others = files
                    .iter()
                    .filter(|other| *other != file)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                errors.push(PresetValidateError {
                    file: file.clone(),
                    code: "duplicate_name".to_string(),
                    error: format!("duplicate preset name `{name}` also found in: {others}"),
                });
            }
        }
        errors
    }

    pub(super) async fn run_preset_list(cli: &super::Cli, json: bool) -> anyhow::Result<()> {
        let result = async {
            let omne_root = super::resolve_pm_root(cli)?;
            let spec_dir = omne_root.join("spec");
            let files = list_preset_paths(&spec_dir).await?;

            let mut presets = Vec::<PresetListItem>::new();
            let mut errors = Vec::<PresetListError>::new();
            for file in files {
                match read_preset_file(&file).await {
                    Ok(preset) => presets.push(PresetListItem {
                        file: file.display().to_string(),
                        version: preset.version,
                        name: preset.name,
                        description: preset.description,
                    }),
                    Err(err) => errors.push(PresetListError {
                        file: file.display().to_string(),
                        code: classify_preset_error(&err).to_string(),
                        error: format!("{err:#}"),
                    }),
                }
            }

            presets.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.file.cmp(&b.file)));
            let result = PresetListResult { presets, errors };

            if json {
                super::print_json_or_pretty(true, &serde_json::to_value(result)?)?;
            } else {
                if result.presets.is_empty() {
                    println!("(no presets)");
                } else {
                    for item in &result.presets {
                        if let Some(desc) = &item.description {
                            println!("{} v{} {} — {}", item.name, item.version, item.file, desc);
                        } else {
                            println!("{} v{} {}", item.name, item.version, item.file);
                        }
                    }
                }
                if !result.errors.is_empty() {
                    eprintln!("[preset/list parse errors: {}]", result.errors.len());
                    for item in result.errors.iter().take(3) {
                        eprintln!("- {}: {}", item.file, item.error);
                    }
                    if result.errors.len() > 3 {
                        eprintln!("- ... and {} more", result.errors.len() - 3);
                    }
                }
            }

            Ok::<(), anyhow::Error>(())
        }
        .await;

        match result {
            Ok(()) => Ok(()),
            Err(err) => {
                maybe_print_preset_json_error(json, &err)?;
                Err(err)
            }
        }
    }

    pub(super) async fn run_preset_validate(
        cli: &super::Cli,
        file: Option<PathBuf>,
        name: Option<String>,
        strict: bool,
        json: bool,
    ) -> anyhow::Result<()> {
        let result = async {
            let omne_root = super::resolve_pm_root(cli)?;
            let files = resolve_validate_targets(&omne_root, file, name).await?;

            let mut validated = Vec::<PresetValidateItem>::new();
            let mut errors = Vec::<PresetValidateError>::new();
            for file in files {
                if let Err(err) = ensure_within_spec_dir(&omne_root, &file) {
                    errors.push(PresetValidateError {
                        file: file.display().to_string(),
                        code: classify_preset_error(&err).to_string(),
                        error: format!("{err:#}"),
                    });
                    continue;
                }

                match read_preset_file(&file).await {
                    Ok(preset) => validated.push(PresetValidateItem {
                        file: file.display().to_string(),
                        version: preset.version,
                        name: preset.name,
                    }),
                    Err(err) => errors.push(PresetValidateError {
                        file: file.display().to_string(),
                        code: classify_preset_error(&err).to_string(),
                        error: format!("{err:#}"),
                    }),
                }
            }

            validated.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.file.cmp(&b.file)));
            if strict {
                errors.extend(duplicate_name_errors(&validated));
            }
            errors.sort_by(|a, b| a.file.cmp(&b.file));
            let result = PresetValidateResult {
                ok: errors.is_empty(),
                strict,
                validated,
                errors,
            };

            if json {
                super::print_json_or_pretty(true, &serde_json::to_value(&result)?)?;
            } else {
                if result.validated.is_empty() {
                    println!("(no presets validated)");
                } else {
                    for item in &result.validated {
                        println!("ok: {} v{} {}", item.name, item.version, item.file);
                    }
                }
                if !result.errors.is_empty() {
                    eprintln!("[preset/validate errors: {}]", result.errors.len());
                    for item in &result.errors {
                        eprintln!("- {}: {}", item.file, item.error);
                    }
                }
            }

            if result.ok {
                Ok(())
            } else {
                anyhow::bail!("preset validation failed: {} file(s) with errors", result.errors.len())
            }
        }
        .await;

        match result {
            Ok(()) => Ok(()),
            Err(err) => {
                maybe_print_preset_json_error(json, &err)?;
                Err(err)
            }
        }
    }

    async fn run_preset_export(
        app: &mut super::App,
        thread_id: ThreadId,
        out: PathBuf,
        name: Option<String>,
        description: Option<String>,
        json: bool,
    ) -> anyhow::Result<()> {
        match export_preset(app, thread_id, out, name, description, json).await {
            Ok(()) => Ok(()),
            Err(err) => {
                maybe_print_preset_json_error(json, &err)?;
                Err(err)
            }
        }
    }

    pub(super) async fn run_preset(
        cli: &super::Cli,
        app: &mut super::App,
        command: super::PresetCommand,
    ) -> anyhow::Result<()> {
        match command {
            super::PresetCommand::List { json } => run_preset_list(cli, json).await,
            super::PresetCommand::Export {
                thread_id,
                out,
                name,
                description,
                json,
            } => run_preset_export(app, thread_id, out, name, description, json).await,
            super::PresetCommand::Import {
                thread_id,
                file,
                name,
                json,
            } => run_preset_import(cli, app, thread_id, file, name, json).await,
            super::PresetCommand::Show { file, name, json } => {
                run_preset_show(cli, file, name, json).await
            }
            super::PresetCommand::Validate {
                file,
                name,
                strict,
                json,
            } => run_preset_validate(cli, file, name, strict, json).await,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn preset_rejects_unknown_fields() {
            let raw = r#"
version: 1
name: x
thread_config:
  approval_policy: auto_approve
  sandbox_policy: workspace_write
  sandbox_network_access: deny
  sandbox_writable_roots: []
  mode: coder
  model: gpt-4.1
  openai_base_url: https://api.openai.com/v1
oops: true
"#;
            let parsed = serde_yaml::from_str::<PresetFileV1>(raw);
            assert!(parsed.is_err());
        }

        #[test]
        fn sanitize_preset_trims_and_dedups_roots() -> anyhow::Result<()> {
            let raw = r#"
version: 1
name:  x
description: " "
portability_warnings: [" outside root ", "outside root", "outside root"]
thread_config:
  approval_policy: auto_approve
  sandbox_policy: workspace_write
  sandbox_network_access: deny
  sandbox_writable_roots: [" . ", " . "]
  mode: " coder "
  model: " gpt-4.1 "
  openai_base_url: " https://api.openai.com/v1 "
"#;
            let parsed = serde_yaml::from_str::<PresetFileV1>(raw)?;
            let preset = sanitize_preset(parsed)?;
            assert_eq!(preset.name, "x");
            assert_eq!(preset.description, None);
            assert_eq!(
                preset.portability_warnings,
                vec!["outside root".to_string()]
            );
            assert_eq!(preset.thread_config.mode, "coder");
            assert_eq!(preset.thread_config.model, "gpt-4.1");
            assert_eq!(
                preset.thread_config.openai_base_url,
                "https://api.openai.com/v1"
            );
            assert_eq!(
                preset.thread_config.sandbox_writable_roots,
                vec![".".to_string()]
            );
            Ok(())
        }

        #[test]
        fn sanitize_preset_rejects_secret_like_values() -> anyhow::Result<()> {
            let raw = r#"
version: 1
name: x
portability_warnings:
  - "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789"
thread_config:
  approval_policy: auto_approve
  sandbox_policy: workspace_write
  sandbox_network_access: deny
  sandbox_writable_roots: ["."]
  mode: coder
  model: "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789"
  openai_base_url: https://api.openai.com/v1
"#;
            let parsed = serde_yaml::from_str::<PresetFileV1>(raw)?;
            let err =
                sanitize_preset(parsed).expect_err("secret-like portability warning should fail");
            let msg = err.to_string();
            assert!(msg.contains("portability_warnings[0]"));
            assert!(msg.contains("secret-like value"));
            Ok(())
        }

        #[test]
        fn sanitize_preset_rejects_env_placeholders() -> anyhow::Result<()> {
            let raw = r#"
version: 1
name: x
description: "{{ENV:OPENAI_API_KEY}}"
thread_config:
  approval_policy: auto_approve
  sandbox_policy: workspace_write
  sandbox_network_access: deny
  sandbox_writable_roots: ["."]
  mode: coder
  model: gpt-4.1
  openai_base_url: https://api.openai.com/v1
"#;
            let parsed = serde_yaml::from_str::<PresetFileV1>(raw)?;
            let err = sanitize_preset(parsed).expect_err("env placeholder should be rejected");
            let msg = err.to_string();
            assert!(msg.contains("preset.description"));
            assert!(msg.contains("env placeholder"));
            Ok(())
        }

        #[tokio::test]
        async fn list_preset_paths_discovers_default_and_presets_dir_yaml() -> anyhow::Result<()> {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let tmp =
                std::env::temp_dir().join(format!("omne-preset-list-{}-{nonce}", std::process::id()));
            let spec_dir = tmp.join("spec");
            tokio::fs::create_dir_all(spec_dir.join("presets")).await?;
            tokio::fs::write(spec_dir.join("preset.yaml"), "x").await?;
            tokio::fs::write(spec_dir.join("presets").join("a.yaml"), "x").await?;
            tokio::fs::write(spec_dir.join("presets").join("b.yml"), "x").await?;
            tokio::fs::write(spec_dir.join("presets").join("skip.txt"), "x").await?;

            let files = list_preset_paths(&spec_dir).await?;
            let files = files
                .iter()
                .map(|p| p.strip_prefix(&spec_dir).unwrap().to_string_lossy().to_string())
                .collect::<Vec<_>>();
            assert_eq!(
                files,
                vec![
                    "preset.yaml".to_string(),
                    "presets/a.yaml".to_string(),
                    "presets/b.yml".to_string()
                ]
            );
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            Ok(())
        }

        #[tokio::test]
        async fn resolve_preset_file_by_name_matches_declared_name() -> anyhow::Result<()> {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let tmp = std::env::temp_dir().join(format!(
                "omne-preset-name-{}-{nonce}",
                std::process::id()
            ));
            let spec_dir = tmp.join("spec");
            tokio::fs::create_dir_all(spec_dir.join("presets")).await?;
            tokio::fs::write(
                spec_dir.join("presets").join("r1.yaml"),
                r#"version: 1
name: reviewer-safe
thread_config:
  approval_policy: manual
  sandbox_policy: read_only
  sandbox_network_access: deny
  sandbox_writable_roots: ["."]
  mode: reviewer
  model: gpt-4.1
  openai_base_url: https://api.openai.com/v1
"#,
            )
            .await?;

            let path = resolve_preset_file_by_name(&spec_dir, "reviewer-safe").await?;
            assert!(path.ends_with("r1.yaml"));
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            Ok(())
        }

        #[tokio::test]
        async fn resolve_preset_file_by_name_reports_ambiguous_matches() -> anyhow::Result<()> {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let tmp = std::env::temp_dir().join(format!(
                "omne-preset-ambiguous-{}-{nonce}",
                std::process::id()
            ));
            let spec_dir = tmp.join("spec");
            tokio::fs::create_dir_all(spec_dir.join("presets")).await?;
            tokio::fs::write(
                spec_dir.join("preset.yaml"),
                r#"version: 1
name: shared-name
thread_config:
  approval_policy: manual
  sandbox_policy: read_only
  sandbox_network_access: deny
  sandbox_writable_roots: ["."]
  mode: reviewer
  model: gpt-4.1
  openai_base_url: https://api.openai.com/v1
"#,
            )
            .await?;
            tokio::fs::write(
                spec_dir.join("presets").join("another.yaml"),
                r#"version: 1
name: shared-name
thread_config:
  approval_policy: manual
  sandbox_policy: read_only
  sandbox_network_access: deny
  sandbox_writable_roots: ["."]
  mode: reviewer
  model: gpt-4.1
  openai_base_url: https://api.openai.com/v1
"#,
            )
            .await?;

            let err = resolve_preset_file_by_name(&spec_dir, "shared-name")
                .await
                .expect_err("expected ambiguous preset name");
            assert!(err.to_string().contains("ambiguous"));
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            Ok(())
        }

        #[tokio::test]
        async fn resolve_validate_targets_without_selector_lists_all() -> anyhow::Result<()> {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let tmp = std::env::temp_dir().join(format!(
                "omne-preset-validate-targets-{}-{nonce}",
                std::process::id()
            ));
            let spec_dir = tmp.join("spec");
            tokio::fs::create_dir_all(spec_dir.join("presets")).await?;
            tokio::fs::write(spec_dir.join("preset.yaml"), "x").await?;
            tokio::fs::write(spec_dir.join("presets").join("a.yaml"), "x").await?;

            let files = resolve_validate_targets(&tmp, None, None).await?;
            assert_eq!(files.len(), 2);
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            Ok(())
        }

        #[test]
        fn duplicate_name_errors_reports_conflicts() {
            let validated = vec![
                PresetValidateItem {
                    file: "/tmp/a.yaml".to_string(),
                    version: 1,
                    name: "dup".to_string(),
                },
                PresetValidateItem {
                    file: "/tmp/b.yaml".to_string(),
                    version: 1,
                    name: "dup".to_string(),
                },
                PresetValidateItem {
                    file: "/tmp/c.yaml".to_string(),
                    version: 1,
                    name: "uniq".to_string(),
                },
            ];
            let errors = duplicate_name_errors(&validated);
            assert_eq!(errors.len(), 2);
            assert!(errors.iter().all(|err| err.code == "duplicate_name"));
            assert!(errors
                .iter()
                .all(|err| err.error.contains("duplicate preset name `dup`")));
        }

        #[test]
        fn classify_preset_error_detects_secret_like_value() {
            let err = anyhow::anyhow!(
                "thread_config.model contains secret-like value (openai-style key); preset files must not contain secrets or env placeholders"
            );
            assert_eq!(classify_preset_error(&err), "secret_like_value");
        }

        #[test]
        fn classify_preset_error_detects_parse_yaml() {
            let err = anyhow::anyhow!("parse preset yaml");
            assert_eq!(classify_preset_error(&err), "parse_yaml");
        }

        #[test]
        fn classify_preset_error_detects_selector_and_name_errors() {
            let selector_conflict = anyhow::anyhow!("pass either --file or --name, not both");
            assert_eq!(
                classify_preset_error(&selector_conflict),
                "selector_conflict"
            );

            let selector_missing =
                anyhow::anyhow!("missing preset selector: pass --file <path> or --name <name>");
            assert_eq!(classify_preset_error(&selector_missing), "selector_missing");

            let invalid_name = anyhow::anyhow!("preset name must not contain path separators: a/b");
            assert_eq!(classify_preset_error(&invalid_name), "invalid_name");

            let name_not_found =
                anyhow::anyhow!("preset `x` not found; available preset names: a, b");
            assert_eq!(classify_preset_error(&name_not_found), "name_not_found");

            let name_ambiguous =
                anyhow::anyhow!("preset `x` is ambiguous; matched files: a.yaml, b.yaml");
            assert_eq!(classify_preset_error(&name_ambiguous), "name_ambiguous");
        }

        #[test]
        fn preset_list_error_json_includes_code() {
            let item = PresetListError {
                file: "/tmp/preset.yaml".to_string(),
                code: "parse_yaml".to_string(),
                error: "parse preset yaml".to_string(),
            };
            let value = serde_json::to_value(item).expect("serialize PresetListError");
            assert_eq!(value.get("code").and_then(|v| v.as_str()), Some("parse_yaml"));
        }

        #[test]
        fn preset_error_payload_includes_ok_false_and_code() {
            let err = anyhow::anyhow!("parse preset yaml");
            let value = preset_error_payload("parse_yaml", &err);
            assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(false));
            assert_eq!(value.get("code").and_then(|v| v.as_str()), Some("parse_yaml"));
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .expect("message is string");
            assert!(message.contains("parse preset yaml"));
        }

        #[test]
        fn build_export_portability_warnings_flags_roots_outside_thread_root() {
            let thread_root = Path::new("/tmp/workspace");
            let roots = vec![
                "/tmp/workspace".to_string(),
                "/tmp/workspace/src".to_string(),
                "/etc".to_string(),
                "/opt/data".to_string(),
            ];
            let warnings = build_export_portability_warnings(&roots, thread_root);
            assert_eq!(warnings.len(), 2);
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains("outside thread root: /etc"))
            );
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains("outside thread root: /opt/data"))
            );
        }
    }
}

async fn run_preset(cli: &Cli, app: &mut App, command: PresetCommand) -> anyhow::Result<()> {
    preset::run_preset(cli, app, command).await
}

async fn run_preset_list(cli: &Cli, json: bool) -> anyhow::Result<()> {
    preset::run_preset_list(cli, json).await
}

async fn run_preset_show(
    cli: &Cli,
    file: Option<PathBuf>,
    name: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    preset::run_preset_show(cli, file, name, json).await
}

async fn run_preset_validate(
    cli: &Cli,
    file: Option<PathBuf>,
    name: Option<String>,
    strict: bool,
    json: bool,
) -> anyhow::Result<()> {
    preset::run_preset_validate(cli, file, name, strict, json).await
}
