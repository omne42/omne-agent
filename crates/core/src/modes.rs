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
                    errors.push(format!("{err:#}"));
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
            ".omne/**".to_string(),
            ".omne/**".to_string(),
            "**/.env".to_string(),
            ".omne_data/config_local.toml".to_string(),
            ".omne_data/config.toml".to_string(),
            ".omne_data/spec/**".to_string(),
            ".omne_data/tmp/**".to_string(),
            ".omne_data/threads/**".to_string(),
            ".omne_data/locks/**".to_string(),
            ".omne_data/logs/**".to_string(),
            ".omne_data/data/**".to_string(),
            ".omne_data/repos/**".to_string(),
            ".omne_data/reference/**".to_string(),
        ];
        let coder_spawn_allowed_modes = Some(vec![
            "architect".to_string(),
            "reviewer".to_string(),
            "builder".to_string(),
            "code".to_string(),
            "chat".to_string(),
            "roleplay".to_string(),
            "author".to_string(),
            "doc_organizer".to_string(),
            "chatter".to_string(),
            "default".to_string(),
            "codder".to_string(),
            "coder".to_string(),
        ]);

        modes.insert(
            "architect".to_string(),
            ModeDef::builtin(
                "Read code/docs; edits require approvals and are limited to docs + a few root files.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        vec![
                            "docs/**".to_string(),
                            "AGENTS.md".to_string(),
                            "CHANGELOG.md".to_string(),
                        ],
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                    subagent: SubagentPermissions {
                        spawn: SubagentSpawnPermissions {
                            decision: Decision::Prompt,
                            allowed_modes: coder_spawn_allowed_modes.clone(),
                            max_threads: None,
                        },
                    },
                },
            ),
        );

        modes.insert(
            "code".to_string(),
            ModeDef::builtin(
                "Coding scenario mode; same permissions as coder.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Prompt,
                    subagent: SubagentPermissions {
                        spawn: SubagentSpawnPermissions {
                            decision: Decision::Prompt,
                            allowed_modes: coder_spawn_allowed_modes.clone(),
                            max_threads: None,
                        },
                    },
                },
            ),
        );

        modes.insert(
            "chat".to_string(),
            ModeDef::builtin(
                "Chat scenario mode; no code execution or file edits.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Deny,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Deny,
                    process: ProcessPermissions {
                        inspect: Decision::Deny,
                        kill: Decision::Deny,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Allow,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "roleplay".to_string(),
            ModeDef::builtin(
                "Roleplay scenario mode; conversational with no execution/edit permissions.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Deny,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Deny,
                    process: ProcessPermissions {
                        inspect: Decision::Deny,
                        kill: Decision::Deny,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Allow,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "author".to_string(),
            ModeDef::builtin(
                "Author scenario mode; focused writing permissions for docs/markdown.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        vec![
                            "docs/**".to_string(),
                            "**/*.md".to_string(),
                            "**/*.mdx".to_string(),
                            "README.md".to_string(),
                            "CHANGELOG.md".to_string(),
                        ],
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Deny,
                    process: ProcessPermissions {
                        inspect: Decision::Deny,
                        kill: Decision::Deny,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Allow,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "doc_organizer".to_string(),
            ModeDef::builtin(
                "Documentation organizer mode; docs-focused editing with prompts.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        vec![
                            "docs/**".to_string(),
                            "**/*.md".to_string(),
                            "**/*.mdx".to_string(),
                            "README.md".to_string(),
                            "CHANGELOG.md".to_string(),
                            "AGENTS.md".to_string(),
                        ],
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Deny,
                    process: ProcessPermissions {
                        inspect: Decision::Deny,
                        kill: Decision::Deny,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Allow,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );
        if let Some(mode) = modes.get("author").cloned() {
            modes.insert("作者".to_string(), mode);
        }
        if let Some(mode) = modes.get("doc_organizer").cloned() {
            modes.insert("文档整理者".to_string(), mode);
        }

        modes.insert(
            "chatter".to_string(),
            ModeDef::builtin(
                "Role profile for light conversation and browsing; no edits or commands.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Deny,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Deny,
                    process: ProcessPermissions {
                        inspect: Decision::Deny,
                        kill: Decision::Deny,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Allow,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "default".to_string(),
            ModeDef::builtin(
                "Role profile for balanced assistance; edits and commands require approvals.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Deny,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Prompt,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "codder".to_string(),
            ModeDef::builtin(
                "Role profile for implementation-heavy work; broad coding permissions.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Prompt,
                    subagent: SubagentPermissions {
                        spawn: SubagentSpawnPermissions {
                            decision: Decision::Prompt,
                            allowed_modes: coder_spawn_allowed_modes,
                            max_threads: None,
                        },
                    },
                },
            ),
        );

        modes.insert(
            "coder".to_string(),
            ModeDef::builtin(
                "Implement changes; edits and commands require approvals by default.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Prompt,
                    subagent: SubagentPermissions {
                        spawn: SubagentSpawnPermissions {
                            decision: Decision::Prompt,
                            allowed_modes: Some(vec![
                                "architect".to_string(),
                                "reviewer".to_string(),
                                "builder".to_string(),
                            ]),
                            max_threads: None,
                        },
                    },
                },
            ),
        );

        modes.insert(
            "reviewer".to_string(),
            ModeDef::builtin(
                "Read-only review; commands require approvals; no edits.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Deny,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "builder".to_string(),
            ModeDef::builtin(
                "Run checks/tests; no edits; commands require approvals.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Deny,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "debugger".to_string(),
            ModeDef::builtin(
                "Debug failures; inspect state and patch with approvals.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs.clone(),
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Prompt,
                    subagent: SubagentPermissions::default(),
                },
            ),
        );

        modes.insert(
            "merger".to_string(),
            ModeDef::builtin(
                "Integrate and finalize changes; edits and commands require approvals.",
                ModePermissions {
                    read: Decision::Allow,
                    edit: builtin_edit_permissions(
                        Decision::Prompt,
                        Vec::new(),
                        default_deny_globs,
                    ),
                    command: Decision::Prompt,
                    process: ProcessPermissions {
                        inspect: Decision::Allow,
                        kill: Decision::Prompt,
                        interact: Decision::Deny,
                    },
                    artifact: Decision::Allow,
                    browser: Decision::Deny,
                    subagent: SubagentPermissions::default(),
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
    pub ui: ModeUi,
    pub command_execpolicy_rules: Vec<String>,
    pub tool_overrides: BTreeMap<String, Decision>,
}

impl ModeDef {
    fn builtin(description: &str, permissions: ModePermissions) -> Self {
        Self {
            description: description.to_string(),
            permissions,
            ui: ModeUi::default(),
            command_execpolicy_rules: Vec::new(),
            tool_overrides: BTreeMap::new(),
        }
    }

    fn try_from_raw(name: &str, raw: RawModeDef) -> anyhow::Result<Self> {
        let command_execpolicy_rules = raw
            .permissions
            .command
            .as_ref()
            .map(|command| command.execpolicy_rules.clone())
            .unwrap_or_default();
        let permissions = ModePermissions::try_from_raw(name, raw.permissions)?;
        let tool_overrides = raw
            .tool_overrides
            .unwrap_or_default()
            .into_iter()
            .filter(|o| !o.tool.trim().is_empty())
            .map(|o| (o.tool, o.decision))
            .collect::<BTreeMap<_, _>>();
        let ui = raw.ui.unwrap_or_default().into_ui();

        Ok(Self {
            description: raw.description.unwrap_or_default(),
            permissions,
            ui,
            command_execpolicy_rules,
            tool_overrides,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModeUi {
    pub show_thinking: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ModePermissions {
    pub read: Decision,
    pub edit: EditPermissions,
    pub command: Decision,
    pub process: ProcessPermissions,
    pub artifact: Decision,
    pub browser: Decision,
    pub subagent: SubagentPermissions,
}

impl ModePermissions {
    fn try_from_raw(mode_name: &str, raw: RawPermissions) -> anyhow::Result<Self> {
        let read = raw.read.map(|v| v.decision).unwrap_or(Decision::Deny);
        let edit = if let Some(edit) = raw.edit {
            EditPermissions::new(edit.decision, edit.allow_globs, edit.deny_globs)?
        } else {
            EditPermissions::new(Decision::Deny, Vec::new(), Vec::new())?
        };
        let command = raw
            .command
            .as_ref()
            .map(|v| v.decision)
            .unwrap_or(Decision::Deny);

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
        let subagent = raw
            .subagent
            .map(SubagentPermissions::try_from_raw)
            .transpose()
            .with_context(|| format!("mode {mode_name}: parse subagent permissions"))?
            .unwrap_or_default();

        Ok(Self {
            read,
            edit,
            command,
            process,
            artifact,
            browser,
            subagent,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProcessPermissions {
    pub inspect: Decision,
    pub kill: Decision,
    pub interact: Decision,
}

#[derive(Debug, Clone, Default)]
pub struct SubagentPermissions {
    pub spawn: SubagentSpawnPermissions,
}

impl SubagentPermissions {
    fn try_from_raw(raw: RawSubagentPermissions) -> anyhow::Result<Self> {
        let spawn = raw
            .spawn
            .map(SubagentSpawnPermissions::try_from_raw)
            .transpose()?
            .unwrap_or_default();
        Ok(Self { spawn })
    }
}

#[derive(Debug, Clone)]
pub struct SubagentSpawnPermissions {
    pub decision: Decision,
    pub allowed_modes: Option<Vec<String>>,
    pub max_threads: Option<usize>,
}

impl Default for SubagentSpawnPermissions {
    fn default() -> Self {
        Self {
            decision: Decision::Deny,
            allowed_modes: None,
            max_threads: None,
        }
    }
}

impl SubagentSpawnPermissions {
    fn try_from_raw(raw: RawSubagentSpawn) -> anyhow::Result<Self> {
        let allowed_modes = raw.allowed_modes.map(|modes| {
            let mut out = Vec::<String>::new();
            let mut seen = std::collections::BTreeSet::<String>::new();
            for mode in modes {
                let mode = mode.trim();
                if mode.is_empty() {
                    continue;
                }
                if seen.insert(mode.to_string()) {
                    out.push(mode.to_string());
                }
            }
            out
        });
        if raw.max_threads.is_some_and(|value| value > 64) {
            anyhow::bail!("subagent.spawn.max_threads must be <= 64");
        }

        Ok(Self {
            decision: raw.decision,
            allowed_modes,
            max_threads: raw.max_threads,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EditPermissions {
    pub decision: Decision,
    pub allow_globs: Vec<String>,
    pub deny_globs: Vec<String>,
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
            allow_globs,
            deny_globs,
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

fn builtin_edit_permissions(
    decision: Decision,
    allow_globs: Vec<String>,
    deny_globs: Vec<String>,
) -> EditPermissions {
    match EditPermissions::new(decision, allow_globs, deny_globs) {
        Ok(edit) => edit,
        Err(err) => {
            tracing::error!(
                error = %err,
                "failed to compile built-in mode globs; falling back to deny-all edit permissions"
            );
            EditPermissions {
                decision: Decision::Deny,
                allow_globs: Vec::new(),
                deny_globs: Vec::new(),
                allow: None,
                deny: GlobSet::default(),
            }
        }
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
    let env = std::env::var("OMNE_MODES_FILE")
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

    let spec_dir = thread_root.join(".omne_data").join("spec");
    let yaml = spec_dir.join("modes.yaml");
    if yaml.exists() {
        return Some((ConfigSource::Project, yaml));
    }
    let yml = spec_dir.join("modes.yml");
    if yml.exists() {
        return Some((ConfigSource::Project, yml));
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
        return normalize_relative_path(rel);
    }

    normalize_relative_path(input)
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
    ui: Option<RawModeUi>,
    #[serde(default)]
    tool_overrides: Option<Vec<RawToolOverride>>,
}

#[derive(Debug, Deserialize, Default)]
struct RawModeUi {
    #[serde(default)]
    show_thinking: Option<bool>,
}

impl RawModeUi {
    fn into_ui(self) -> ModeUi {
        ModeUi {
            show_thinking: self.show_thinking,
        }
    }
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
    command: Option<RawCommand>,
    #[serde(default)]
    process: Option<RawProcess>,
    #[serde(default)]
    artifact: Option<RawDecision>,
    #[serde(default)]
    browser: Option<RawDecision>,
    #[serde(default)]
    subagent: Option<RawSubagentPermissions>,
}

#[derive(Debug, Deserialize)]
struct RawDecision {
    decision: Decision,
}

#[derive(Debug, Deserialize)]
struct RawCommand {
    decision: Decision,
    #[serde(default)]
    execpolicy_rules: Vec<String>,
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

#[derive(Debug, Deserialize)]
struct RawSubagentPermissions {
    #[serde(default)]
    spawn: Option<RawSubagentSpawn>,
}

#[derive(Debug, Deserialize)]
struct RawSubagentSpawn {
    decision: Decision,
    #[serde(default)]
    allowed_modes: Option<Vec<String>>,
    #[serde(default)]
    max_threads: Option<usize>,
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
        tokio::fs::create_dir_all(root.join(".omne_data/spec")).await?;
        tokio::fs::write(
            root.join(".omne_data/spec/modes.yaml"),
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
      command:
        decision: deny
        execpolicy_rules: ["rules/mode.rules"]
      process:
        inspect: { decision: deny }
        kill: { decision: deny }
        interact: { decision: deny }
      artifact: { decision: allow }
      browser: { decision: deny }
      subagent:
        spawn:
          decision: allow
          allowed_modes: ["reviewer"]
          max_threads: 2
"#,
        )
        .await?;

        let catalog = ModeCatalog::load(root).await;
        assert!(matches!(catalog.source, ModeCatalogSource::Project(_)));
        assert!(catalog.mode("coder").is_some());
        assert!(catalog.mode("code").is_some());
        assert!(catalog.mode("chat").is_some());
        assert!(catalog.mode("roleplay").is_some());
        assert!(catalog.mode("author").is_some());
        assert!(catalog.mode("doc_organizer").is_some());
        assert!(catalog.mode("作者").is_some());
        assert!(catalog.mode("文档整理者").is_some());
        assert!(catalog.mode("chatter").is_some());
        assert!(catalog.mode("default").is_some());
        assert!(catalog.mode("codder").is_some());
        assert!(catalog.mode("debugger").is_some());
        assert!(catalog.mode("merger").is_some());
        let docs_only = catalog.mode("docs-only").expect("custom mode present");
        assert_eq!(docs_only.permissions.command, Decision::Deny);
        assert_eq!(
            docs_only.command_execpolicy_rules,
            vec!["rules/mode.rules".to_string()]
        );
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
        assert_eq!(
            docs_only.permissions.subagent.spawn.decision,
            Decision::Allow
        );
        assert_eq!(
            docs_only.permissions.subagent.spawn.allowed_modes,
            Some(vec!["reviewer".to_string()])
        );
        assert_eq!(docs_only.permissions.subagent.spawn.max_threads, Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn invalid_subagent_max_threads_reports_load_error() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        tokio::fs::create_dir_all(root.join(".omne_data/spec")).await?;
        tokio::fs::write(
            root.join(".omne_data/spec/modes.yaml"),
            r#"
version: 1
modes:
  bad:
    description: "invalid max_threads"
    permissions:
      subagent:
        spawn:
          decision: allow
          max_threads: 65
"#,
        )
        .await?;

        let catalog = ModeCatalog::load(root).await;
        assert!(catalog.mode("bad").is_none());
        let load_error = catalog.load_error.unwrap_or_default();
        assert!(load_error.contains("subagent.spawn.max_threads must be <= 64"));
        Ok(())
    }
}
