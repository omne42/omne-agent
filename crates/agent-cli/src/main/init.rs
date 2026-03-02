use std::path::Path;

async fn run_init(args: InitArgs) -> anyhow::Result<()> {
    let target_dir = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let target_dir = tokio::fs::canonicalize(&target_dir)
        .await
        .unwrap_or(target_dir);

    let interactive =
        !args.yes && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    let mut enable_project_config = args.enable_project_config;
    let mut create_config_local = args.create_config_local;
    let mut create_env = true;
    let mut create_spec_dir = true;
    let disable_spec_templates = args.no_spec_templates || args.minimal;
    let mut create_command_templates = !args.no_command_templates && !disable_spec_templates;
    let mut create_workspace_template = !args.no_workspace_template && !disable_spec_templates;
    let mut create_hooks_template = !args.no_hooks_template && !disable_spec_templates;
    let mut create_modes_template = !args.no_modes_template && !disable_spec_templates;

    if interactive {
        eprintln!("omne init");
        eprintln!("dir: {}", target_dir.display());
        enable_project_config = prompt_yes_no(
            "Enable project config now? (recommended: no by default)",
            enable_project_config,
        )?;
        create_config_local = prompt_yes_no(
            "Create .omne_data/config_local.toml template? (recommended: only when needed)",
            create_config_local,
        )?;
        create_env = prompt_yes_no("Create .omne_data/.env template?", create_env)?;
        create_spec_dir = prompt_yes_no("Create .omne_data/spec/ directory?", create_spec_dir)?;
        if create_spec_dir && !disable_spec_templates {
            create_command_templates = prompt_yes_no(
                "Create default command templates under .omne_data/spec/commands/?",
                create_command_templates,
            )?;
            create_workspace_template = prompt_yes_no(
                "Create .omne_data/spec/workspace.yaml template?",
                create_workspace_template,
            )?;
            create_hooks_template = prompt_yes_no(
                "Create .omne_data/spec/hooks.yaml template?",
                create_hooks_template,
            )?;
            create_modes_template = prompt_yes_no(
                "Create .omne_data/spec/modes.yaml template?",
                create_modes_template,
            )?;
        } else if !create_spec_dir {
            create_command_templates = false;
            create_workspace_template = false;
            create_hooks_template = false;
            create_modes_template = false;
        }
    }

    let omne_data_dir = target_dir.join(".omne_data");
    tokio::fs::create_dir_all(&omne_data_dir).await?;

    if create_spec_dir {
        let spec_dir = omne_data_dir.join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;
        let mut created_templates = Vec::<PathBuf>::new();
        let commands_dir = spec_dir.join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;
        if create_command_templates {
            created_templates
                .extend(write_default_command_templates(&commands_dir, args.force).await?);
        }
        if create_workspace_template {
            if let Some(path) = write_default_workspace_template(&spec_dir, args.force).await? {
                created_templates.push(path);
            }
        }
        if create_hooks_template {
            if let Some(path) = write_default_hooks_template(&spec_dir, args.force).await? {
                created_templates.push(path);
            }
        }
        if create_modes_template {
            if let Some(path) = write_default_modes_template(&spec_dir, args.force).await? {
                created_templates.push(path);
            }
        }
        if !created_templates.is_empty() {
            eprintln!(
                "created spec templates: {}",
                created_templates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    tokio::fs::create_dir_all(omne_data_dir.join("tmp")).await?;
    tokio::fs::create_dir_all(omne_data_dir.join("data")).await?;
    tokio::fs::create_dir_all(omne_data_dir.join("repos")).await?;
    tokio::fs::create_dir_all(omne_data_dir.join("reference")).await?;
    tokio::fs::create_dir_all(omne_data_dir.join("locks")).await?;
    tokio::fs::create_dir_all(omne_data_dir.join("logs")).await?;
    tokio::fs::create_dir_all(omne_data_dir.join("threads")).await?;

    write_omne_gitignore(&omne_data_dir).await?;
    write_omne_config_toml(&omne_data_dir, enable_project_config, args.force).await?;
    if create_config_local {
        write_omne_config_local_toml(&omne_data_dir, enable_project_config, args.force).await?;
    }
    if create_env {
        write_omne_env_template(&omne_data_dir, args.force).await?;
    }

    eprintln!("created: {}", omne_data_dir.display());
    Ok(())
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> anyhow::Result<bool> {
    let default_hint = if default_yes { "Y/n" } else { "y/N" };
    eprint!("{prompt} [{default_hint}]: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return Ok(default_yes);
    }
    match answer.as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => Ok(default_yes),
    }
}

async fn write_omne_gitignore(omne_data_dir: &Path) -> anyhow::Result<()> {
    let path = omne_data_dir.join(".gitignore");

    let desired = [
        "# OmneAgent runtime (do not commit)",
        "tmp/",
        "data/",
        "repos/",
        "reference/",
        "threads/",
        "locks/",
        "logs/",
        "daemon.sock",
        "",
        "# Local overrides (do not commit)",
        "config_local.toml",
        "",
        "# Secrets (do not commit)",
        ".env",
        "",
    ]
    .join("\n");

    if tokio::fs::try_exists(&path).await? {
        let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let mut out = existing;
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for line in desired.lines() {
            if line.is_empty() {
                if !out.ends_with("\n\n") {
                    out.push('\n');
                }
                continue;
            }
            if !out.lines().any(|it| it.trim() == line.trim()) {
                out.push_str(line);
                out.push('\n');
            }
        }
        tokio::fs::write(&path, out).await?;
        return Ok(());
    }

    tokio::fs::write(&path, desired).await?;
    Ok(())
}

async fn write_omne_config_toml(
    omne_data_dir: &Path,
    enable_project_config: bool,
    force: bool,
) -> anyhow::Result<()> {
    let path = omne_data_dir.join("config.toml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(());
    }

    let enabled = if enable_project_config {
        "true"
    } else {
        "false"
    };
    let contents = format!(
        r#"# OmneAgent project config (v0.2.x)
#
# This file is safe to commit. Secrets belong in `.omne_data/.env` (gitignored).
# For per-machine overrides, use `.omne_data/config_local.toml` (gitignored).
#
# When disabled, OmneAgent ignores project-level overrides from `config.toml` and `.env`.

[project_config]
enabled = {enabled}

[ui]
# Show model thinking/reasoning deltas in clients (default: true).
# show_thinking = true

[openai]
# provider = "openai-codex-apikey" # selects a profile under [openai.providers]
# model = "gpt-4.1"                # default model for this project
#
# # Provider profiles: auth/base_url/model whitelist live here.
# # (Secrets belong in `.omne_data/.env`.)
# #
# # [openai.providers.openai-codex-apikey]
# # base_url = "https://api.openai.com/v1"
# # # optional: provider-default model (used when thread/env/project model is unset)
# # # default_model = "gpt-4.1"
# # # optional: enforce a model allowlist
# # # model_whitelist = ["gpt-4.1", "gpt-4o-mini"]
# # # optional: override capability flags (useful for OpenAI-compatible providers)
# # # [openai.providers.openai-codex-apikey.capabilities]
# # # tools = true
# # # vision = true
# # # reasoning = true
# # # json_schema = true
# # # streaming = true
# # [openai.providers.openai-codex-apikey.auth]
# # type = "api_key_env"
# # # optional: override env keys (default: ["OPENAI_API_KEY","OMNE_OPENAI_API_KEY"])
# # # keys = ["OPENAI_API_KEY"]
# #
# # Auth plugin (Node-friendly): command must print JSON: {{"api_key":"..."}} or {{"token":"..."}}
# # provider = "openai-auth-command"
# # [openai.providers.openai-auth-command]
# # base_url = "https://api.openai.com/v1"
# # [openai.providers.openai-auth-command.auth]
# # type = "command"
# # command = ["node", "./.omne_data/openai-auth.mjs"]
#
# # Per-model thinking intensity (defaults to medium):
# # supported: unsupported/small/medium/high/xhigh
# # [openai.models."*"]
# # thinking = "medium"
# # [openai.models."codex-mini-latest"]
# # thinking = "xhigh"
"#
    );

    tokio::fs::write(&path, contents).await?;
    Ok(())
}

async fn write_omne_config_local_toml(
    omne_data_dir: &Path,
    enable_project_config: bool,
    force: bool,
) -> anyhow::Result<()> {
    let path = omne_data_dir.join("config_local.toml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(());
    }

    let enabled = if enable_project_config {
        "true"
    } else {
        "false"
    };
    let contents = format!(
        r#"# OmneAgent local project config (v0.2.x)
#
# This file is gitignored and should not be committed.
# When present, OmneAgent uses it instead of `.omne_data/config.toml`.
#
# When disabled, OmneAgent ignores project-level overrides from `config_local.toml` and `.env`.

[project_config]
enabled = {enabled}

[ui]
# Show model thinking/reasoning deltas in clients (default: true).
# show_thinking = true

[openai]
# provider = "openai-codex-apikey" # selects a profile under [openai.providers]
# model = "gpt-4.1"                # default model for this project
#
# # Provider profiles: auth/base_url/model whitelist live here.
# # (Secrets belong in `.omne_data/.env`.)
# #
# # [openai.providers.openai-codex-apikey]
# # base_url = "https://api.openai.com/v1"
# # # optional: provider-default model (used when thread/env/project model is unset)
# # # default_model = "gpt-4.1"
# # # optional: enforce a model allowlist
# # # model_whitelist = ["gpt-4.1", "gpt-4o-mini"]
# # # optional: override capability flags (useful for OpenAI-compatible providers)
# # # [openai.providers.openai-codex-apikey.capabilities]
# # # tools = true
# # # vision = true
# # # reasoning = true
# # # json_schema = true
# # # streaming = true
# # [openai.providers.openai-codex-apikey.auth]
# # type = "api_key_env"
# # # optional: override env keys (default: ["OPENAI_API_KEY","OMNE_OPENAI_API_KEY"])
# # # keys = ["OPENAI_API_KEY"]
# #
# # Auth plugin (Node-friendly): command must print JSON: {{"api_key":"..."}} or {{"token":"..."}}
# # provider = "openai-auth-command"
# # [openai.providers.openai-auth-command]
# # base_url = "https://api.openai.com/v1"
# # [openai.providers.openai-auth-command.auth]
# # type = "command"
# # command = ["node", "./.omne_data/openai-auth.mjs"]
#
# # Per-model thinking intensity (defaults to medium):
# # supported: unsupported/small/medium/high/xhigh
# # [openai.models."*"]
# # thinking = "medium"
# # [openai.models."codex-mini-latest"]
# # thinking = "xhigh"
"#
    );

    tokio::fs::write(&path, contents).await?;
    Ok(())
}

async fn write_omne_env_template(omne_data_dir: &Path, force: bool) -> anyhow::Result<()> {
    let path = omne_data_dir.join(".env");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(());
    }

    let contents = r#"# Secrets (do not commit)
OPENAI_API_KEY=
#
# Optional overrides:
# OMNE_OPENAI_PROVIDER=openai-codex-apikey
# OMNE_OPENAI_BASE_URL=https://api.openai.com/v1
# OMNE_OPENAI_MODEL=gpt-4.1
"#;

    tokio::fs::write(&path, contents).await?;
    Ok(())
}

fn default_command_templates() -> [(&'static str, &'static str); 2] {
    [
        (
            "plan.md",
            r#"---
version: 1
name: plan
mode: architect
subagent-fork: false
inputs:
  - name: goal
    required: true
---
Goal: {{goal}}

Produce a minimal, verifiable execution plan:

1. State assumptions and constraints.
2. Propose 3-6 concrete steps.
3. For each step, include acceptance criteria and verification commands.
4. Highlight top risks and mitigation.
"#,
        ),
        (
            "fanout-review.md",
            r#"---
version: 1
name: fanout-review
mode: reviewer
subagent-fork: true
inputs:
  - name: objective
    required: true
---
Objective: {{objective}}

Run a focused fan-out review and return blockers first.

## Task: api-contract Validate protocol and backward compatibility
Review protocol structs, schema versions, and serialization behavior.

## Task: runtime-safety Validate runtime and error handling
Check critical paths for failure propagation, cancellation, and recovery.

## Task: tests-coverage Validate regression coverage
Check whether changed behavior has targeted tests and edge-case coverage.
"#,
        ),
    ]
}

async fn write_default_command_templates(
    commands_dir: &Path,
    force: bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut created = Vec::<PathBuf>::new();
    for (filename, contents) in default_command_templates() {
        let path = commands_dir.join(filename);
        if !force && tokio::fs::try_exists(&path).await? {
            continue;
        }
        tokio::fs::write(&path, contents).await?;
        created.push(path);
    }
    Ok(created)
}

async fn write_default_workspace_template(
    spec_dir: &Path,
    force: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let path = spec_dir.join("workspace.yaml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(None);
    }
    let contents = r#"# Workspace lifecycle hooks (v0.2.x).
# Fill argv arrays as needed; hook execution still goes through
# mode/sandbox/execpolicy/approvals and remains non-interactive.
hooks: {}
# hooks:
#   setup: ["cargo", "--version"]
#   run: ["cargo", "test", "--workspace"]
#   archive: ["git", "status", "--porcelain=v1"]
"#;
    tokio::fs::write(&path, contents).await?;
    Ok(Some(path))
}

async fn write_default_hooks_template(
    spec_dir: &Path,
    force: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let path = spec_dir.join("hooks.yaml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(None);
    }
    let contents = r#"# Hook dispatch config (v0.2.x).
# Hooks are advisory; failures are recorded but do not become permission bypass.
version: 1
hooks:
  session_start: []
  pre_tool_use: []
  post_tool_use: []
  stop: []
"#;
    tokio::fs::write(&path, contents).await?;
    Ok(Some(path))
}

async fn write_default_modes_template(
    spec_dir: &Path,
    force: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let path = spec_dir.join("modes.yaml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(None);
    }
    let contents = r#"# Mode config (v1). Built-in modes remain available; entries here override/add.
version: 1
modes: {}
# modes:
#   docs-only:
#     description: "Read and docs edits only."
#     ui:
#       show_thinking: false
#     permissions:
#       read: { decision: allow }
#       edit:
#         decision: prompt
#         allow_globs: ["docs/**"]
#         deny_globs: [".git/**", ".omne_data/**", "**/.env"]
#       command: { decision: deny }
#       process:
#         inspect: { decision: deny }
#         kill: { decision: deny }
#         interact: { decision: deny }
#       artifact: { decision: allow }
#       browser: { decision: deny }
#       subagent:
#         spawn:
#           decision: deny
"#;
    tokio::fs::write(&path, contents).await?;
    Ok(Some(path))
}

#[cfg(test)]
mod init_tests {
    use super::*;

    async fn create_temp_dir(prefix: &str) -> anyhow::Result<PathBuf> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(dir)
    }

    #[tokio::test]
    async fn write_default_command_templates_creates_files() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-templates").await?;
        let commands_dir = tmp.join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;

        let created = write_default_command_templates(&commands_dir, false).await?;
        assert_eq!(created.len(), 2);

        let plan: String = tokio::fs::read_to_string(commands_dir.join("plan.md")).await?;
        assert!(plan.contains("name: plan"));
        assert!(plan.contains("Goal: {{goal}}"));

        let fanout: String =
            tokio::fs::read_to_string(commands_dir.join("fanout-review.md")).await?;
        assert!(fanout.contains("name: fanout-review"));
        assert!(fanout.contains("## Task: api-contract"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn write_default_command_templates_respects_force_flag() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-force").await?;
        let commands_dir = tmp.join("commands");
        tokio::fs::create_dir_all(&commands_dir).await?;

        write_default_command_templates(&commands_dir, false).await?;
        tokio::fs::write(commands_dir.join("plan.md"), "custom").await?;

        let created_without_force = write_default_command_templates(&commands_dir, false).await?;
        assert!(created_without_force.is_empty());
        let plan_unchanged = tokio::fs::read_to_string(commands_dir.join("plan.md")).await?;
        assert_eq!(plan_unchanged, "custom");

        let created_with_force = write_default_command_templates(&commands_dir, true).await?;
        assert_eq!(created_with_force.len(), 2);
        let plan_overwritten: String =
            tokio::fs::read_to_string(commands_dir.join("plan.md")).await?;
        assert!(plan_overwritten.contains("name: plan"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn write_default_workspace_template_creates_expected_file() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-workspace-template").await?;
        let spec_dir = tmp.join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        let created = write_default_workspace_template(&spec_dir, false).await?;
        let expected = spec_dir.join("workspace.yaml");
        assert_eq!(created.as_deref(), Some(expected.as_path()));

        let content = tokio::fs::read_to_string(spec_dir.join("workspace.yaml")).await?;
        assert!(content.contains("hooks: {}"));
        assert!(content.contains("setup"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn write_default_hooks_template_respects_force_flag() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-hooks-template").await?;
        let spec_dir = tmp.join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        write_default_hooks_template(&spec_dir, false).await?;
        tokio::fs::write(spec_dir.join("hooks.yaml"), "custom").await?;

        let created_without_force = write_default_hooks_template(&spec_dir, false).await?;
        assert!(created_without_force.is_none());
        let unchanged = tokio::fs::read_to_string(spec_dir.join("hooks.yaml")).await?;
        assert_eq!(unchanged, "custom");

        let created_with_force = write_default_hooks_template(&spec_dir, true).await?;
        let expected = spec_dir.join("hooks.yaml");
        assert_eq!(created_with_force.as_deref(), Some(expected.as_path()));
        let rewritten = tokio::fs::read_to_string(spec_dir.join("hooks.yaml")).await?;
        assert!(rewritten.contains("version: 1"));
        assert!(rewritten.contains("session_start"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn write_default_modes_template_creates_expected_file() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-modes-template").await?;
        let spec_dir = tmp.join("spec");
        tokio::fs::create_dir_all(&spec_dir).await?;

        let created = write_default_modes_template(&spec_dir, false).await?;
        let expected = spec_dir.join("modes.yaml");
        assert_eq!(created.as_deref(), Some(expected.as_path()));

        let content = tokio::fs::read_to_string(spec_dir.join("modes.yaml")).await?;
        assert!(content.contains("version: 1"));
        assert!(content.contains("modes: {}"));
        assert!(content.contains("docs-only"));
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_init_no_spec_templates_skips_template_files() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-no-spec-templates").await?;
        run_init(InitArgs {
            dir: Some(tmp.clone()),
            force: false,
            yes: true,
            enable_project_config: false,
            create_config_local: false,
            no_command_templates: false,
            no_workspace_template: false,
            no_hooks_template: false,
            no_modes_template: false,
            no_spec_templates: true,
            minimal: false,
        })
        .await?;

        let spec_dir = tmp.join(".omne_data").join("spec");
        assert!(tokio::fs::try_exists(&spec_dir).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("workspace.yaml").as_path()).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("hooks.yaml").as_path()).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("modes.yaml").as_path()).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("commands").join("plan.md").as_path()).await?);
        assert!(!tokio::fs::try_exists(
            spec_dir.join("commands").join("fanout-review.md").as_path()
        )
        .await?);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }

    #[tokio::test]
    async fn run_init_minimal_skips_template_files() -> anyhow::Result<()> {
        let tmp = create_temp_dir("omne-init-minimal").await?;
        run_init(InitArgs {
            dir: Some(tmp.clone()),
            force: false,
            yes: true,
            enable_project_config: false,
            create_config_local: false,
            no_command_templates: false,
            no_workspace_template: false,
            no_hooks_template: false,
            no_modes_template: false,
            no_spec_templates: false,
            minimal: true,
        })
        .await?;

        let spec_dir = tmp.join(".omne_data").join("spec");
        assert!(tokio::fs::try_exists(&spec_dir).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("workspace.yaml").as_path()).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("hooks.yaml").as_path()).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("modes.yaml").as_path()).await?);
        assert!(!tokio::fs::try_exists(spec_dir.join("commands").join("plan.md").as_path()).await?);
        assert!(!tokio::fs::try_exists(
            spec_dir.join("commands").join("fanout-review.md").as_path()
        )
        .await?);

        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(())
    }
}
