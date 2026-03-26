use config_kit::{ConfigFormat, ConfigFormatSet, ConfigLoadOptions, try_load_typed_config_file};

const OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED_ENV: &str = "OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED";
const OMNE_TOOL_DYNAMIC_REGISTRY_PATH_ENV: &str = "OMNE_TOOL_DYNAMIC_REGISTRY_PATH";
const DEFAULT_DYNAMIC_TOOL_REGISTRY_REL_PATH: &str = ".omne_data/spec/tool_registry.json";

#[derive(Debug, Clone)]
pub(crate) struct DynamicToolSpec {
    pub name: String,
    pub description: String,
    pub mapped_tool: String,
    pub mapped_action: String,
    pub parameters: serde_json::Value,
    pub fixed_args: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
struct RawDynamicRegistry {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    tools: Vec<RawDynamicToolSpec>,
}

#[derive(Debug, serde::Deserialize)]
struct RawDynamicToolSpec {
    name: String,
    #[serde(default)]
    description: Option<String>,
    mapped_tool: String,
    #[serde(default)]
    parameters: Option<serde_json::Value>,
    #[serde(default)]
    fixed_args: Option<serde_json::Value>,
    #[serde(default)]
    read_only: Option<bool>,
}

fn parse_dynamic_registry_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub(crate) fn dynamic_tool_registry_enabled() -> bool {
    std::env::var(OMNE_TOOL_DYNAMIC_REGISTRY_ENABLED_ENV)
        .ok()
        .and_then(|raw| parse_dynamic_registry_bool(&raw))
        .unwrap_or(false)
}

fn resolve_dynamic_registry_paths(
    thread_root: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut out = Vec::<std::path::PathBuf>::new();

    if let Some(root) = thread_root {
        out.push(root.join(DEFAULT_DYNAMIC_TOOL_REGISTRY_REL_PATH));
    }

    let env_path = std::env::var(OMNE_TOOL_DYNAMIC_REGISTRY_PATH_ENV)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .map(std::path::PathBuf::from);
    if let Some(path) = env_path {
        let resolved = if path.is_absolute() {
            path
        } else if let Some(root) = thread_root {
            root.join(path)
        } else {
            path
        };
        out.push(resolved);
    }

    out
}

fn default_dynamic_tool_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true,
    })
}

fn dynamic_registry_load_options() -> ConfigLoadOptions {
    ConfigLoadOptions::new().with_default_format(ConfigFormat::Json)
}

fn normalize_dynamic_parameters(value: Option<serde_json::Value>) -> Option<serde_json::Value> {
    let value = value.unwrap_or_else(default_dynamic_tool_schema);
    if value.is_object() { Some(value) } else { None }
}

fn normalize_dynamic_fixed_args(value: Option<serde_json::Value>) -> Option<serde_json::Value> {
    let value = value.unwrap_or_else(|| serde_json::json!({}));
    if value.is_object() { Some(value) } else { None }
}

fn normalize_dynamic_tool_entry(
    raw: RawDynamicToolSpec,
    source_path: &std::path::Path,
) -> Option<DynamicToolSpec> {
    let name = raw.name.trim().to_string();
    if name.is_empty() {
        tracing::warn!(
            path = %source_path.display(),
            "skip dynamic tool with empty name"
        );
        return None;
    }
    if is_known_agent_tool_name(&name) {
        tracing::warn!(
            path = %source_path.display(),
            dynamic_tool = %name,
            "skip dynamic tool: name conflicts with built-in tool"
        );
        return None;
    }

    let mapped_tool = raw.mapped_tool.trim().to_string();
    let Some(mapped_action) = agent_tool_action(&mapped_tool) else {
        tracing::warn!(
            path = %source_path.display(),
            dynamic_tool = %name,
            mapped_tool = %mapped_tool,
            "skip dynamic tool: mapped tool is unknown"
        );
        return None;
    };

    if !is_plan_read_only_agent_tool(&mapped_tool) {
        tracing::warn!(
            path = %source_path.display(),
            dynamic_tool = %name,
            mapped_tool = %mapped_tool,
            "skip dynamic tool: only read-only mapped tools are supported in MVP"
        );
        return None;
    }

    let read_only = raw.read_only.unwrap_or(true);
    if !read_only {
        tracing::warn!(
            path = %source_path.display(),
            dynamic_tool = %name,
            "skip dynamic tool: read_only=false is not supported in MVP"
        );
        return None;
    }

    let Some(parameters) = normalize_dynamic_parameters(raw.parameters) else {
        tracing::warn!(
            path = %source_path.display(),
            dynamic_tool = %name,
            "skip dynamic tool: parameters must be a JSON object schema"
        );
        return None;
    };
    let Some(fixed_args) = normalize_dynamic_fixed_args(raw.fixed_args) else {
        tracing::warn!(
            path = %source_path.display(),
            dynamic_tool = %name,
            "skip dynamic tool: fixed_args must be a JSON object"
        );
        return None;
    };

    let description = raw
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Dynamic tool -> {mapped_tool} ({mapped_action})"));

    Some(DynamicToolSpec {
        name,
        description,
        mapped_tool,
        mapped_action: mapped_action.to_string(),
        parameters,
        fixed_args,
    })
}

fn load_dynamic_tool_registry_file(path: &std::path::Path) -> Vec<DynamicToolSpec> {
    let parsed = match try_load_typed_config_file::<RawDynamicRegistry>(
        path,
        dynamic_registry_load_options(),
        ConfigFormatSet::JSON,
    ) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => return Vec::new(),
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "failed to load dynamic tool registry");
            return Vec::new();
        }
    };

    if parsed.version.unwrap_or(1) != 1 {
        tracing::warn!(
            path = %path.display(),
            version = parsed.version.unwrap_or(0),
            "skip dynamic tool registry: unsupported version"
        );
        return Vec::new();
    }

    parsed
        .tools
        .into_iter()
        .filter_map(|tool| normalize_dynamic_tool_entry(tool, path))
        .collect()
}

pub(crate) fn load_dynamic_tool_specs(
    thread_root: Option<&std::path::Path>,
) -> Vec<DynamicToolSpec> {
    if !dynamic_tool_registry_enabled() {
        return Vec::new();
    }

    let mut out = Vec::<DynamicToolSpec>::new();
    let mut seen = std::collections::BTreeSet::<String>::new();
    for path in resolve_dynamic_registry_paths(thread_root) {
        for spec in load_dynamic_tool_registry_file(&path) {
            if seen.insert(spec.name.clone()) {
                out.push(spec);
            } else {
                tracing::warn!(
                    path = %path.display(),
                    dynamic_tool = %spec.name,
                    "skip duplicated dynamic tool name"
                );
            }
        }
    }
    out
}

pub(crate) fn find_dynamic_tool_spec(
    thread_root: Option<&std::path::Path>,
    tool_name: &str,
) -> Option<DynamicToolSpec> {
    load_dynamic_tool_specs(thread_root)
        .into_iter()
        .find(|spec| spec.name == tool_name)
}

#[cfg(test)]
mod dynamic_registry_tests {
    use super::*;

    #[test]
    fn normalize_dynamic_tool_entry_rejects_non_read_only_mapped_tool() {
        let source = std::path::Path::new("/tmp/tool_registry.json");
        let raw = RawDynamicToolSpec {
            name: "dyn_write".to_string(),
            description: None,
            mapped_tool: "file_write".to_string(),
            parameters: None,
            fixed_args: None,
            read_only: Some(true),
        };
        assert!(normalize_dynamic_tool_entry(raw, source).is_none());
    }

    #[test]
    fn normalize_dynamic_tool_entry_accepts_read_only_mapping() {
        let source = std::path::Path::new("/tmp/tool_registry.json");
        let raw = RawDynamicToolSpec {
            name: "dyn_read".to_string(),
            description: Some("read helper".to_string()),
            mapped_tool: "file_read".to_string(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            })),
            fixed_args: Some(serde_json::json!({ "root": "workspace" })),
            read_only: Some(true),
        };
        let spec = normalize_dynamic_tool_entry(raw, source).expect("dynamic tool");
        assert_eq!(spec.name, "dyn_read");
        assert_eq!(spec.mapped_tool, "file_read");
        assert_eq!(spec.mapped_action, "file/read");
    }

    #[test]
    fn load_dynamic_tool_registry_file_accepts_extensionless_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tool_registry");
        std::fs::write(
            &path,
            serde_json::json!({
                "version": 1,
                "tools": [
                    {
                        "name": "dyn_read",
                        "mapped_tool": "file_read",
                        "read_only": true
                    }
                ]
            })
            .to_string(),
        )
        .expect("write registry");

        let specs = load_dynamic_tool_registry_file(&path);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "dyn_read");
    }
}
