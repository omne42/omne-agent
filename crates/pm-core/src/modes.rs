use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Prompt,
    Deny,
}

impl Decision {
    pub fn combine(self, other: Self) -> Self {
        std::cmp::max(self, other)
    }
}

#[derive(Debug, Clone)]
pub enum ModeCatalogSource {
    Builtin,
    Env(PathBuf),
    Project(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ModeCatalog {
    pub source: ModeCatalogSource,
    pub load_error: Option<String>,
    modes: BTreeMap<String, ModeDef>,
}

impl ModeCatalog {
    pub async fn load(thread_root: &Path) -> Self {
        let mut out = Self::builtin();

        let Some((source, path)) = select_config_path(thread_root) else {
            return out;
        };

        let raw = match tokio::fs::read_to_string(&path).await {
            Ok(contents) => contents,
            Err(err) => {
                out.source = source.with_path(path);
                out.load_error = Some(format!("read modes config: {err}"));
                return out;
            }
        };

        let parsed: RawModesFile = match serde_yaml::from_str(&raw) {
            Ok(v) => v,
            Err(err) => {
                out.source = source.with_path(path);
                out.load_error = Some(format!("parse modes config: {err}"));
                return out;
            }
        };

        if parsed.version != 1 {
            out.source = source.with_path(path);
            out.load_error = Some(format!(
                "unsupported modes config version: {} (expected 1)",
                parsed.version
            ));
            return out;
        }

        let mut errors = Vec::<String>::new();
        for (name, raw_mode) in parsed.modes {
            match ModeDef::try_from_raw(&name, raw_mode) {
                Ok(mode) => {
                    out.modes.insert(name, mode);
                }
                Err(err) => {
                    errors.push(err.to_string());
                }
            }
        }

        out.source = source.with_path(path);
        if !errors.is_empty() {
            out.load_error = Some(errors.join("; "));
        }
        out
    }

    pub fn builtin() -> Self {
        let mut modes = BTreeMap::<String, ModeDef>::new();

        let default_deny_globs = vec![
            ".git/**".to_string(),
            ".code_pm/**".to_string(),
            ".codepm/**".to_string(),
        ];

        modes.insert(
            "architect".to_string(),
            ModeDef::builtin(
                "Read code/docs; edits require approvals and are limited to docs + a few root files.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: EditPermissions::new(
                        Decision::Prompt,
                        vec![
                            "docs/**".to_string(),
                            "AGENTS.md".to_string(),
                            "CHANGELOG.md".to_string(),
                        ],
                        default_deny_globs.clone(),
                    )
                    .expect("builtin globs must compile"),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                },
            ),
        );

        modes.insert(
            "coder".to_string(),
            ModeDef::builtin(
                "Implement changes; edits and commands require approvals by default.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: EditPermissions::new(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs.clone(),
                    )
                    .expect("builtin globs must compile"),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Prompt,
                },
            ),
        );

        modes.insert(
            "reviewer".to_string(),
            ModeDef::builtin(
                "Read-only review; commands require approvals; no edits.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: EditPermissions::new(
                        Decision::Deny,
                        Vec::new(),
                        default_deny_globs.clone(),
                    )
                    .expect("builtin globs must compile"),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                },
            ),
        );

        modes.insert(
            "builder".to_string(),
            ModeDef::builtin(
                "Run checks/tests; no edits; commands require approvals.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: EditPermissions::new(Decision::Deny, Vec::new(), default_deny_globs)
                        .expect("builtin globs must compile"),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                },
            ),
        );

        Self {
            source: ModeCatalogSource::Builtin,
            load_error: None,
            modes,
        }
    }

    pub fn mode(&self, name: &str) -> Option<&ModeDef> {
        self.modes.get(name)
    }

    pub fn mode_names(&self) -> impl Iterator<Item = &str> {
        self.modes.keys().map(String::as_str)
    }
}

#[derive(Debug, Clone)]
pub struct ModeDef {
    pub description: String,
    pub permissions: ModePermissions,
    pub tool_overrides: BTreeMap<String, Decision>,
}

impl ModeDef {
    fn builtin(description: &str, permissions: ModePermissions) -> Self {
        Self {
            description: description.to_string(),
            permissions,
            tool_overrides: BTreeMap::new(),
        }
    }

    fn try_from_raw(name: &str, raw: RawModeDef) -> anyhow::Result<Self> {
        let permissions = ModePermissions::try_from_raw(name, raw.permissions)?;
        let tool_overrides = raw
            .tool_overrides
            .unwrap_or_default()
            .into_iter()
            .filter(|o| !o.tool.trim().is_empty())
            .map(|o| (o.tool, o.decision))
            .collect::<BTreeMap<_, _>>();

        Ok(Self {
            description: raw.description.unwrap_or_default(),
            permissions,
            tool_overrides,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ModePermissions {
    pub read: Decision,
    pub edit: EditPermissions,
    pub command: Decision,
    pub process: ProcessPermissions,
    pub artifact: Decision,
    pub browser: Decision,
}

impl ModePermissions {
    fn try_from_raw(mode_name: &str, raw: RawPermissions) -> anyhow::Result<Self> {
        let read = raw.read.map(|v| v.decision).unwrap_or(Decision::Deny);
        let edit = if let Some(edit) = raw.edit {
            EditPermissions::new(edit.decision, edit.allow_globs, edit.deny_globs)?
        } else {
            EditPermissions::new(Decision::Deny, Vec::new(), Vec::new())?
        };
        let command = raw.command.map(|v| v.decision).unwrap_or(Decision::Deny);

        let process = if let Some(process) = raw.process {
            let inspect = process
                .inspect
                .map(|v| v.decision)
                .unwrap_or(Decision::Deny);
            let kill = process.kill.map(|v| v.decision).unwrap_or(Decision::Deny);
            let interact = process
                .interact
                .map(|v| v.decision)
                .unwrap_or(Decision::Deny);
            if interact != Decision::Deny {
                anyhow::bail!("mode {mode_name}: process.interact must be deny");
            }
            ProcessPermissions {
                inspect,
                kill,
                interact,
            }
        } else {
            ProcessPermissions {
                inspect: Decision::Deny,
                kill: Decision::Deny,
                interact: Decision::Deny,
            }
        };

        let artifact = raw.artifact.map(|v| v.decision).unwrap_or(Decision::Deny);
        let browser = raw.browser.map(|v| v.decision).unwrap_or(Decision::Deny);

        Ok(Self {
            read,
            edit,
            command,
            process,
            artifact,
            browser,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProcessPermissions {
    pub inspect: Decision,
    pub kill: Decision,
    pub interact: Decision,
}

#[derive(Debug, Clone)]
pub struct EditPermissions {
    pub decision: Decision,
    allow: Option<GlobSet>,
    deny: GlobSet,
}

impl EditPermissions {
    pub fn new(
        decision: Decision,
        allow_globs: Vec<String>,
        deny_globs: Vec<String>,
    ) -> anyhow::Result<Self> {
        let allow = if allow_globs.is_empty() {
            None
        } else {
            Some(compile_globset(&allow_globs)?)
        };
        let deny = compile_globset(&deny_globs)?;
        Ok(Self {
            decision,
            allow,
            deny,
        })
    }

    pub fn decision_for_path(&self, rel_path: &Path) -> Decision {
        if self.decision == Decision::Deny {
            return Decision::Deny;
        }
        if self.deny.is_match(rel_path) {
            return Decision::Deny;
        }
        if let Some(allow) = &self.allow
            && !allow.is_match(rel_path)
        {
            return Decision::Deny;
        }
        self.decision
    }

    pub fn is_denied(&self, rel_path: &Path) -> bool {
        self.deny.is_match(rel_path)
    }
}

fn compile_globset(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob =
            Glob::new(pattern).with_context(|| format!("invalid glob pattern: {pattern}"))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

#[derive(Debug, Clone, Copy)]
enum ConfigSource {
    Env,
    Project,
}

impl ConfigSource {
    fn with_path(self, path: PathBuf) -> ModeCatalogSource {
        match self {
            Self::Env => ModeCatalogSource::Env(path),
            Self::Project => ModeCatalogSource::Project(path),
        }
    }
}

fn select_config_path(thread_root: &Path) -> Option<(ConfigSource, PathBuf)> {
    let env = std::env::var("CODE_PM_MODES_FILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    if let Some(path) = env {
        let path = if path.is_absolute() {
            path
        } else {
            thread_root.join(path)
        };
        return Some((ConfigSource::Env, path));
    }

    let project = thread_root.join(".codepm").join("modes.yaml");
    if project.exists() {
        return Some((ConfigSource::Project, project));
    }

    let fallback = thread_root.join("codepm.modes.yaml");
    if fallback.exists() {
        return Some((ConfigSource::Project, fallback));
    }

    None
}

pub fn normalize_relative_path(input: &Path) -> anyhow::Result<PathBuf> {
    if input.is_absolute() {
        anyhow::bail!("expected relative path, got {}", input.display());
    }
    if input
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        anyhow::bail!("parent traversal is not allowed: {}", input.display());
    }

    let mut out = PathBuf::new();
    for comp in input.components() {
        match comp {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

pub fn relative_path_under_root(root: &Path, input: &Path) -> anyhow::Result<PathBuf> {
    if input.is_absolute() {
        let rel = input
            .strip_prefix(root)
            .with_context(|| format!("path is outside workspace root: {}", input.display()))?;
        return Ok(normalize_relative_path(rel)?);
    }

    Ok(normalize_relative_path(input)?)
}

#[derive(Debug, Deserialize)]
struct RawModesFile {
    version: u32,
    #[serde(default)]
    modes: BTreeMap<String, RawModeDef>,
}

#[derive(Debug, Deserialize)]
struct RawModeDef {
    #[serde(default)]
    description: Option<String>,
    permissions: RawPermissions,
    #[serde(default)]
    tool_overrides: Option<Vec<RawToolOverride>>,
}

#[derive(Debug, Deserialize)]
struct RawToolOverride {
    tool: String,
    decision: Decision,
}

#[derive(Debug, Deserialize)]
struct RawPermissions {
    #[serde(default)]
    read: Option<RawDecision>,
    #[serde(default)]
    edit: Option<RawEdit>,
    #[serde(default)]
    command: Option<RawDecision>,
    #[serde(default)]
    process: Option<RawProcess>,
    #[serde(default)]
    artifact: Option<RawDecision>,
    #[serde(default)]
    browser: Option<RawDecision>,
}

#[derive(Debug, Deserialize)]
struct RawDecision {
    decision: Decision,
}

#[derive(Debug, Deserialize)]
struct RawEdit {
    decision: Decision,
    #[serde(default)]
    allow_globs: Vec<String>,
    #[serde(default)]
    deny_globs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawProcess {
    #[serde(default)]
    inspect: Option<RawDecision>,
    #[serde(default)]
    kill: Option<RawDecision>,
    #[serde(default)]
    interact: Option<RawDecision>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_normalizes_and_rejects_parent_traversal() {
        assert!(normalize_relative_path(Path::new("../nope")).is_err());
        assert!(normalize_relative_path(Path::new("a/../b")).is_err());
        assert_eq!(
            normalize_relative_path(Path::new("./a/./b")).unwrap(),
            PathBuf::from("a/b")
        );
    }

    #[tokio::test]
    async fn loads_project_modes_yaml_and_merges() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        tokio::fs::create_dir_all(root.join(".codepm")).await?;
        tokio::fs::write(
            root.join(".codepm/modes.yaml"),
            r#"
version: 1
modes:
  docs-only:
    description: "docs only"
    permissions:
      read: { decision: allow }
      edit:
        decision: prompt
        allow_globs: ["docs/**"]
        deny_globs: [".git/**"]
      command: { decision: deny }
      process:
        inspect: { decision: deny }
        kill: { decision: deny }
        interact: { decision: deny }
      artifact: { decision: allow }
      browser: { decision: deny }
"#,
        )
        .await?;

        let catalog = ModeCatalog::load(root).await;
        assert!(matches!(catalog.source, ModeCatalogSource::Project(_)));
        assert!(catalog.mode("coder").is_some());
        let docs_only = catalog.mode("docs-only").expect("custom mode present");
        assert_eq!(docs_only.permissions.command, Decision::Deny);
        let decision = docs_only
            .permissions
            .edit
            .decision_for_path(Path::new("docs/readme.md"));
        assert_eq!(decision, Decision::Prompt);
        let decision = docs_only
            .permissions
            .edit
            .decision_for_path(Path::new("src/lib.rs"));
        assert_eq!(decision, Decision::Deny);
        Ok(())
    }
}
