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

    if interactive {
        eprintln!("pm init");
        eprintln!("dir: {}", target_dir.display());
        enable_project_config = prompt_yes_no(
            "Enable project config now? (recommended: no by default)",
            enable_project_config,
        )?;
        create_config_local = prompt_yes_no(
            "Create .codepm_data/config_local.toml template? (recommended: only when needed)",
            create_config_local,
        )?;
        create_env = prompt_yes_no("Create .codepm_data/.env template?", create_env)?;
        create_spec_dir = prompt_yes_no("Create .codepm_data/spec/ directory?", create_spec_dir)?;
    }

    let codepm_data_dir = target_dir.join(".codepm_data");
    tokio::fs::create_dir_all(&codepm_data_dir).await?;

    if create_spec_dir {
        tokio::fs::create_dir_all(codepm_data_dir.join("spec")).await?;
    }
    tokio::fs::create_dir_all(codepm_data_dir.join("tmp")).await?;
    tokio::fs::create_dir_all(codepm_data_dir.join("data")).await?;
    tokio::fs::create_dir_all(codepm_data_dir.join("repos")).await?;
    tokio::fs::create_dir_all(codepm_data_dir.join("locks")).await?;
    tokio::fs::create_dir_all(codepm_data_dir.join("logs")).await?;
    tokio::fs::create_dir_all(codepm_data_dir.join("threads")).await?;

    write_codepm_gitignore(&codepm_data_dir).await?;
    write_codepm_config_toml(&codepm_data_dir, enable_project_config, args.force).await?;
    if create_config_local {
        write_codepm_config_local_toml(&codepm_data_dir, enable_project_config, args.force).await?;
    }
    if create_env {
        write_codepm_env_template(&codepm_data_dir, args.force).await?;
    }

    eprintln!("created: {}", codepm_data_dir.display());
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

async fn write_codepm_gitignore(codepm_data_dir: &Path) -> anyhow::Result<()> {
    let path = codepm_data_dir.join(".gitignore");

    let desired = [
        "# CodePM runtime (do not commit)",
        "tmp/",
        "data/",
        "repos/",
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

async fn write_codepm_config_toml(
    codepm_data_dir: &Path,
    enable_project_config: bool,
    force: bool,
) -> anyhow::Result<()> {
    let path = codepm_data_dir.join("config.toml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(());
    }

    let enabled = if enable_project_config {
        "true"
    } else {
        "false"
    };
    let contents = format!(
        r#"# CodePM project config (v0.2.x)
#
# This file is safe to commit. Secrets belong in `.codepm_data/.env` (gitignored).
# For per-machine overrides, use `.codepm_data/config_local.toml` (gitignored).
#
# When disabled, CodePM ignores project-level overrides from `config.toml` and `.env`.

[project_config]
enabled = {enabled}

[openai]
# provider = "openai-codex-apikey" # selects a profile under [openai.providers]
# model = "gpt-4.1"                # default model for this project
#
# # Provider profiles: auth/base_url/model whitelist live here.
# # (Secrets belong in `.codepm_data/.env`.)
# #
# # [openai.providers.openai-codex-apikey]
# # base_url = "https://api.openai.com/v1"
# # # optional: provider-default model (used when thread/env/project model is unset)
# # # default_model = "gpt-4.1"
# # # optional: enforce a model allowlist
# # # model_whitelist = ["gpt-4.1", "gpt-4o-mini"]
# # [openai.providers.openai-codex-apikey.auth]
# # type = "api_key_env"
# # # optional: override env keys (default: ["OPENAI_API_KEY","CODE_PM_OPENAI_API_KEY"])
# # # keys = ["OPENAI_API_KEY"]
# #
# # Auth plugin (Node-friendly): command must print JSON: {{"api_key":"..."}} or {{"token":"..."}}
# # provider = "openai-auth-command"
# # [openai.providers.openai-auth-command]
# # base_url = "https://api.openai.com/v1"
# # [openai.providers.openai-auth-command.auth]
# # type = "command"
# # command = ["node", "./.codepm_data/openai-auth.mjs"]
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

async fn write_codepm_config_local_toml(
    codepm_data_dir: &Path,
    enable_project_config: bool,
    force: bool,
) -> anyhow::Result<()> {
    let path = codepm_data_dir.join("config_local.toml");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(());
    }

    let enabled = if enable_project_config {
        "true"
    } else {
        "false"
    };
    let contents = format!(
        r#"# CodePM local project config (v0.2.x)
#
# This file is gitignored and should not be committed.
# When present, CodePM uses it instead of `.codepm_data/config.toml`.
#
# When disabled, CodePM ignores project-level overrides from `config_local.toml` and `.env`.

[project_config]
enabled = {enabled}

[openai]
# provider = "openai-codex-apikey" # selects a profile under [openai.providers]
# model = "gpt-4.1"                # default model for this project
#
# # Provider profiles: auth/base_url/model whitelist live here.
# # (Secrets belong in `.codepm_data/.env`.)
# #
# # [openai.providers.openai-codex-apikey]
# # base_url = "https://api.openai.com/v1"
# # # optional: provider-default model (used when thread/env/project model is unset)
# # # default_model = "gpt-4.1"
# # # optional: enforce a model allowlist
# # # model_whitelist = ["gpt-4.1", "gpt-4o-mini"]
# # [openai.providers.openai-codex-apikey.auth]
# # type = "api_key_env"
# # # optional: override env keys (default: ["OPENAI_API_KEY","CODE_PM_OPENAI_API_KEY"])
# # # keys = ["OPENAI_API_KEY"]
# #
# # Auth plugin (Node-friendly): command must print JSON: {{"api_key":"..."}} or {{"token":"..."}}
# # provider = "openai-auth-command"
# # [openai.providers.openai-auth-command]
# # base_url = "https://api.openai.com/v1"
# # [openai.providers.openai-auth-command.auth]
# # type = "command"
# # command = ["node", "./.codepm_data/openai-auth.mjs"]
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

async fn write_codepm_env_template(codepm_data_dir: &Path, force: bool) -> anyhow::Result<()> {
    let path = codepm_data_dir.join(".env");
    if !force && tokio::fs::try_exists(&path).await? {
        return Ok(());
    }

    let contents = r#"# Secrets (do not commit)
OPENAI_API_KEY=
#
# Optional overrides:
# CODE_PM_OPENAI_PROVIDER=openai-codex-apikey
# CODE_PM_OPENAI_BASE_URL=https://api.openai.com/v1
# CODE_PM_OPENAI_MODEL=gpt-4.1
"#;

    tokio::fs::write(&path, contents).await?;
    Ok(())
}
