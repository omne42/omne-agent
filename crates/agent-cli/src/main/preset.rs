mod preset {
    use std::path::{Path, PathBuf};

    use anyhow::Context;
    use omne_agent_protocol::{ApprovalPolicy, SandboxNetworkAccess, SandboxPolicy, ThreadId};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PresetFileV1 {
        version: u32,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
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
    }

    #[derive(Debug, Deserialize)]
    struct ThreadConfigExplainOutput {
        effective: PresetThreadConfig,
    }

    #[derive(Debug, Deserialize)]
    struct ThreadStateOutput {
        #[serde(default)]
        cwd: Option<String>,
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
        let parsed = serde_json::from_value::<ThreadStateOutput>(state)
            .context("parse thread/state output")?;
        let Some(cwd) = parsed.cwd else {
            return Ok(None);
        };
        let canon = tokio::fs::canonicalize(&cwd)
            .await
            .with_context(|| format!("canonicalize thread cwd {cwd}"))?;
        Ok(Some(canon))
    }

    fn ensure_within_spec_dir(agent_root: &Path, file: &Path) -> anyhow::Result<()> {
        let spec_dir = agent_root.join("spec");
        if !spec_dir.exists() {
            anyhow::bail!("spec dir is missing: {} (run `omne-agent init`?)", spec_dir.display());
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

    fn sanitize_preset(mut preset: PresetFileV1) -> anyhow::Result<PresetFileV1> {
        if preset.version != 1 {
            anyhow::bail!("unsupported preset version: {} (expected 1)", preset.version);
        }

        preset.name = normalize_string(preset.name, "preset.name")?;
        preset.description = normalize_string_opt(preset.description);

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

        Ok(preset)
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
        let _ = app
            .rpc(
                "thread/configure",
                serde_json::json!({
                    "thread_id": thread_id,
                    "approval_policy": cfg.approval_policy,
                    "sandbox_policy": cfg.sandbox_policy,
                    "sandbox_writable_roots": cfg.sandbox_writable_roots,
                    "sandbox_network_access": cfg.sandbox_network_access,
                    "mode": cfg.mode,
                    "model": cfg.model,
                    "openai_base_url": cfg.openai_base_url,
                }),
            )
            .await?;
        Ok(())
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
        let mut parsed = serde_json::from_value::<ThreadConfigExplainOutput>(explain)
            .context("parse config-explain output")?;

        if let Some(thread_root) = resolve_thread_root_for_export(app, thread_id).await? {
            parsed.effective.sandbox_writable_roots =
                relativize_roots(parsed.effective.sandbox_writable_roots, &thread_root);
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
            thread_config: parsed.effective,
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
        }

        Ok(())
    }

    async fn import_preset(
        cli: &super::Cli,
        app: &mut super::App,
        thread_id: ThreadId,
        file: PathBuf,
        json: bool,
    ) -> anyhow::Result<()> {
        let agent_root = super::resolve_root(cli)?;
        ensure_within_spec_dir(&agent_root, &file)?;

        let preset = read_preset_file(&file).await?;
        apply_preset(app, thread_id, &preset).await?;

        if json {
            let result = serde_json::json!({
                "ok": true,
                "thread_id": thread_id,
                "file": file.display().to_string(),
                "preset": preset,
            });
            super::print_json_or_pretty(true, &result)?;
        } else {
            println!("ok: applied preset {}", file.display());
        }

        Ok(())
    }

    pub(super) async fn run_preset(
        cli: &super::Cli,
        app: &mut super::App,
        command: super::PresetCommand,
    ) -> anyhow::Result<()> {
        match command {
            super::PresetCommand::Export {
                thread_id,
                out,
                name,
                description,
                json,
            } => export_preset(app, thread_id, out, name, description, json).await,
            super::PresetCommand::Import {
                thread_id,
                file,
                json,
            } => import_preset(cli, app, thread_id, file, json).await,
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
    }
}

async fn run_preset(cli: &Cli, app: &mut App, command: PresetCommand) -> anyhow::Result<()> {
    preset::run_preset(cli, app, command).await
}
