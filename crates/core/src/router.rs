use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use omne_protocol::ModelRoutingRuleSource;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub enum RouterConfigSource {
    Env,
    Project,
}

#[derive(Debug, Clone)]
pub struct LoadedRouterConfig {
    pub source: RouterConfigSource,
    pub path: PathBuf,
    pub config: RouterConfig,
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub project_override: Option<ProjectOverride>,
    pub role_defaults: BTreeMap<String, String>,
    pub keyword_rules: Vec<KeywordRule>,
}

#[derive(Debug, Clone)]
pub struct ProjectOverride {
    pub model: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KeywordRule {
    pub id: String,
    pub keywords: Vec<String>,
    pub min_context_tokens: Option<u64>,
    pub model: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRouteDecision {
    pub selected_model: String,
    pub rule_source: ModelRoutingRuleSource,
    pub reason: Option<String>,
    pub rule_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRouterFileV1 {
    version: u32,
    #[serde(default)]
    project_override: Option<RawProjectOverride>,
    #[serde(default)]
    role_defaults: BTreeMap<String, String>,
    #[serde(default)]
    keyword_rules: Vec<RawKeywordRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProjectOverride {
    model: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeywordRule {
    id: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    min_context_tokens: Option<u64>,
    model: String,
    #[serde(default)]
    reason: Option<String>,
}

fn select_router_config_path(
    thread_root: &Path,
    env_router_file: Option<&str>,
) -> Option<(RouterConfigSource, PathBuf)> {
    let env = env_router_file
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    if let Some(path) = env {
        let path = if path.is_absolute() {
            path
        } else {
            thread_root.join(path)
        };
        return Some((RouterConfigSource::Env, path));
    }

    let spec_dir = thread_root.join(".omne_data").join("spec");
    let yaml = spec_dir.join("router.yaml");
    if yaml.exists() {
        return Some((RouterConfigSource::Project, yaml));
    }
    let yml = spec_dir.join("router.yml");
    if yml.exists() {
        return Some((RouterConfigSource::Project, yml));
    }
    let json = spec_dir.join("router.json");
    if json.exists() {
        return Some((RouterConfigSource::Project, json));
    }

    None
}

fn parse_router_file(contents: &str, path: &Path) -> anyhow::Result<RawRouterFileV1> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => serde_json::from_str(contents)
            .with_context(|| format!("parse router json {}", path.display())),
        _ => serde_yaml::from_str(contents)
            .with_context(|| format!("parse router yaml {}", path.display())),
    }
}

fn normalize_role(name: &str) -> anyhow::Result<String> {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        anyhow::bail!("router role name must not be empty");
    }
    Ok(name)
}

fn normalize_model(model: &str) -> anyhow::Result<String> {
    let model = model.trim().to_string();
    if model.is_empty() {
        anyhow::bail!("router model must not be empty");
    }
    Ok(model)
}

fn normalize_keywords(keywords: &[String]) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for keyword in keywords {
        let keyword = keyword.trim().to_lowercase();
        if keyword.is_empty() {
            continue;
        }
        out.push(keyword);
    }
    if out.is_empty() {
        anyhow::bail!("router keyword_rules.keywords must contain at least one keyword");
    }
    Ok(out)
}

fn router_from_raw(raw: RawRouterFileV1) -> anyhow::Result<RouterConfig> {
    if raw.version != 1 {
        bail!("unsupported router version: {} (expected 1)", raw.version);
    }

    let project_override = match raw.project_override {
        Some(override_) => Some(ProjectOverride {
            model: normalize_model(&override_.model)?,
            reason: override_.reason.filter(|s| !s.trim().is_empty()),
        }),
        None => None,
    };

    let mut role_defaults = BTreeMap::<String, String>::new();
    for (role, model) in raw.role_defaults {
        role_defaults.insert(normalize_role(&role)?, normalize_model(&model)?);
    }

    let mut keyword_rules = Vec::<KeywordRule>::new();
    for rule in raw.keyword_rules {
        let id = rule.id.trim().to_string();
        if id.is_empty() {
            bail!("router keyword_rules.id must not be empty");
        }
        let min_context_tokens = rule.min_context_tokens.filter(|v| *v > 0);
        if rule.min_context_tokens.is_some() && min_context_tokens.is_none() {
            bail!("router keyword_rules.min_context_tokens must be > 0");
        }
        if rule.keywords.is_empty() && min_context_tokens.is_none() {
            bail!("router keyword_rules must set keywords and/or min_context_tokens");
        }
        keyword_rules.push(KeywordRule {
            id,
            keywords: if rule.keywords.is_empty() {
                Vec::new()
            } else {
                normalize_keywords(&rule.keywords)?
            },
            min_context_tokens,
            model: normalize_model(&rule.model)?,
            reason: rule.reason.filter(|s| !s.trim().is_empty()),
        });
    }

    Ok(RouterConfig {
        project_override,
        role_defaults,
        keyword_rules,
    })
}

async fn load_router_config_impl(
    thread_root: &Path,
    env_router_file: Option<&str>,
) -> anyhow::Result<Option<LoadedRouterConfig>> {
    let Some((source, path)) = select_router_config_path(thread_root, env_router_file) else {
        return Ok(None);
    };

    if matches!(source, RouterConfigSource::Env) && !path.exists() {
        bail!("router file not found: {}", path.display());
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read router config {}", path.display()))?;
    let parsed = parse_router_file(&raw, &path)?;
    let config = router_from_raw(parsed)?;

    Ok(Some(LoadedRouterConfig {
        source,
        path,
        config,
    }))
}

pub async fn load_router_config(thread_root: &Path) -> anyhow::Result<Option<LoadedRouterConfig>> {
    let env_router_file = std::env::var("OMNE_ROUTER_FILE").ok();
    load_router_config_impl(thread_root, env_router_file.as_deref()).await
}

pub fn route_model(
    router: Option<&RouterConfig>,
    role: Option<&str>,
    input: &str,
    global_default_model: &str,
    forced: bool,
    context_tokens_estimate: u64,
) -> ModelRouteDecision {
    if forced {
        return ModelRouteDecision {
            selected_model: global_default_model.to_string(),
            rule_source: ModelRoutingRuleSource::Subagent,
            reason: Some("model forced by thread config".to_string()),
            rule_id: None,
        };
    }

    let Some(router) = router else {
        return ModelRouteDecision {
            selected_model: global_default_model.to_string(),
            rule_source: ModelRoutingRuleSource::GlobalDefault,
            reason: None,
            rule_id: None,
        };
    };

    if let Some(project_override) = &router.project_override {
        return ModelRouteDecision {
            selected_model: project_override.model.clone(),
            rule_source: ModelRoutingRuleSource::ProjectOverride,
            reason: project_override.reason.clone(),
            rule_id: None,
        };
    }

    let input = input.trim();
    if !router.keyword_rules.is_empty() {
        let input_lower = input.to_lowercase();
        for rule in &router.keyword_rules {
            let keywords_ok = rule.keywords.is_empty()
                || rule
                    .keywords
                    .iter()
                    .any(|keyword| input_lower.contains(keyword));
            let context_ok = rule
                .min_context_tokens
                .is_none_or(|min| context_tokens_estimate >= min);
            if keywords_ok && context_ok {
                let mut reason_parts = Vec::<String>::new();
                if let Some(reason) = rule.reason.clone() {
                    reason_parts.push(reason);
                }
                if let Some(min) = rule.min_context_tokens {
                    reason_parts.push(format!(
                        "context_tokens_estimate={context_tokens_estimate} >= min_context_tokens={min}"
                    ));
                }
                let reason = if reason_parts.is_empty() {
                    None
                } else {
                    Some(reason_parts.join("; "))
                };
                return ModelRouteDecision {
                    selected_model: rule.model.clone(),
                    rule_source: ModelRoutingRuleSource::KeywordRule,
                    reason,
                    rule_id: Some(rule.id.clone()),
                };
            }
        }
    }

    if let Some(role) = role {
        let role = role.trim().to_lowercase();
        if !role.is_empty()
            && let Some(model) = router.role_defaults.get(&role)
        {
            return ModelRouteDecision {
                selected_model: model.clone(),
                rule_source: ModelRoutingRuleSource::RoleDefault,
                reason: Some(format!("role default: {role}")),
                rule_id: None,
            };
        }
    }

    ModelRouteDecision {
        selected_model: global_default_model.to_string(),
        rule_source: ModelRoutingRuleSource::GlobalDefault,
        reason: None,
        rule_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loads_router_yaml_from_project_spec_dir() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        tokio::fs::create_dir_all(root.join(".omne_data/spec")).await?;
        tokio::fs::write(
            root.join(".omne_data/spec/router.yaml"),
            r#"
version: 1
role_defaults:
  coder: gpt-4.1-mini
keyword_rules:
  - id: long-context
    keywords: ["rag"]
    model: gpt-4.1
"#,
        )
        .await?;

        let loaded = load_router_config(root)
            .await?
            .expect("router config loaded");
        assert!(matches!(loaded.source, RouterConfigSource::Project));
        assert_eq!(
            loaded.config.role_defaults.get("coder").map(String::as_str),
            Some("gpt-4.1-mini")
        );
        Ok(())
    }

    #[tokio::test]
    async fn env_router_file_missing_fails_closed() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let result = load_router_config_impl(tmp.path(), Some("nope/router.yaml")).await;

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn route_forced_beats_everything() {
        let router = RouterConfig {
            project_override: Some(ProjectOverride {
                model: "gpt-4.1".to_string(),
                reason: None,
            }),
            role_defaults: BTreeMap::from([("coder".to_string(), "gpt-4.1-mini".to_string())]),
            keyword_rules: vec![KeywordRule {
                id: "k".to_string(),
                keywords: vec!["urgent".to_string()],
                min_context_tokens: None,
                model: "gpt-4.1".to_string(),
                reason: None,
            }],
        };

        let decision = route_model(Some(&router), Some("coder"), "urgent", "gpt-4.1", true, 0);
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::Subagent);
        assert_eq!(decision.selected_model, "gpt-4.1");
    }

    #[test]
    fn route_project_override_beats_keyword_and_role() {
        let router = RouterConfig {
            project_override: Some(ProjectOverride {
                model: "gpt-4.1".to_string(),
                reason: Some("forced".to_string()),
            }),
            role_defaults: BTreeMap::from([("coder".to_string(), "gpt-4.1-mini".to_string())]),
            keyword_rules: vec![KeywordRule {
                id: "k".to_string(),
                keywords: vec!["urgent".to_string()],
                min_context_tokens: None,
                model: "gpt-4.1-mini".to_string(),
                reason: None,
            }],
        };

        let decision = route_model(
            Some(&router),
            Some("coder"),
            "urgent",
            "gpt-4.1-mini",
            false,
            0,
        );
        assert_eq!(
            decision.rule_source,
            ModelRoutingRuleSource::ProjectOverride
        );
        assert_eq!(decision.selected_model, "gpt-4.1");
        assert_eq!(decision.reason.as_deref(), Some("forced"));
    }

    #[test]
    fn route_keyword_beats_role_default() {
        let router = RouterConfig {
            project_override: None,
            role_defaults: BTreeMap::from([("coder".to_string(), "gpt-4.1-mini".to_string())]),
            keyword_rules: vec![KeywordRule {
                id: "k".to_string(),
                keywords: vec!["urgent".to_string()],
                min_context_tokens: None,
                model: "gpt-4.1".to_string(),
                reason: Some("needs reasoning".to_string()),
            }],
        };

        let decision = route_model(
            Some(&router),
            Some("coder"),
            "This is URGENT",
            "gpt-4.1-mini",
            false,
            0,
        );
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::KeywordRule);
        assert_eq!(decision.selected_model, "gpt-4.1");
        assert_eq!(decision.rule_id.as_deref(), Some("k"));
    }

    #[test]
    fn route_context_threshold_rule_beats_role_default() {
        let router = RouterConfig {
            project_override: None,
            role_defaults: BTreeMap::from([("coder".to_string(), "gpt-4.1-mini".to_string())]),
            keyword_rules: vec![KeywordRule {
                id: "long-context-threshold".to_string(),
                keywords: Vec::new(),
                min_context_tokens: Some(10),
                model: "gpt-4.1".to_string(),
                reason: None,
            }],
        };

        let decision = route_model(
            Some(&router),
            Some("coder"),
            "hello",
            "gpt-4.1-mini",
            false,
            12,
        );
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::KeywordRule);
        assert_eq!(decision.selected_model, "gpt-4.1");
        assert_eq!(decision.rule_id.as_deref(), Some("long-context-threshold"));
        assert!(
            decision
                .reason
                .unwrap_or_default()
                .contains("min_context_tokens=10")
        );
    }

    #[test]
    fn route_keyword_rule_requires_min_context_tokens_when_set() {
        let router = RouterConfig {
            project_override: None,
            role_defaults: BTreeMap::from([("coder".to_string(), "gpt-4.1-mini".to_string())]),
            keyword_rules: vec![KeywordRule {
                id: "k".to_string(),
                keywords: vec!["urgent".to_string()],
                min_context_tokens: Some(10),
                model: "gpt-4.1".to_string(),
                reason: None,
            }],
        };

        let decision = route_model(
            Some(&router),
            Some("coder"),
            "urgent",
            "gpt-4.1-mini",
            false,
            9,
        );
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::RoleDefault);

        let decision = route_model(
            Some(&router),
            Some("coder"),
            "hello",
            "gpt-4.1-mini",
            false,
            12,
        );
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::RoleDefault);

        let decision = route_model(
            Some(&router),
            Some("coder"),
            "urgent",
            "gpt-4.1-mini",
            false,
            12,
        );
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::KeywordRule);
    }

    #[test]
    fn route_role_default_when_no_keyword_match() {
        let router = RouterConfig {
            project_override: None,
            role_defaults: BTreeMap::from([("coder".to_string(), "gpt-4.1-mini".to_string())]),
            keyword_rules: Vec::new(),
        };

        let decision = route_model(Some(&router), Some("Coder"), "hello", "gpt-4.1", false, 0);
        assert_eq!(decision.rule_source, ModelRoutingRuleSource::RoleDefault);
        assert_eq!(decision.selected_model, "gpt-4.1-mini");
    }
}
