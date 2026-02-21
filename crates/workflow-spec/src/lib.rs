use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// Return the canonical workflow spec directory under the omne root.
pub fn workflow_spec_dir(omne_root: &Path) -> PathBuf {
    omne_root.join("spec").join("commands")
}

/// Validate a workflow name used as `<name>.md` inside the spec dir.
pub fn validate_workflow_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("workflow name must not be empty");
    }
    if name.trim() != name {
        anyhow::bail!("workflow name must not contain leading/trailing whitespace");
    }
    if name.contains('/') || name.contains('\\') {
        anyhow::bail!("workflow name must not contain path separators");
    }
    if name.contains("..") {
        anyhow::bail!("workflow name must not contain `..`");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        anyhow::bail!("workflow name contains invalid characters: {name}");
    }
    Ok(())
}

/// Validate an input/template variable name.
pub fn ensure_valid_var_name(name: &str, label: &str) -> anyhow::Result<()> {
    if name.is_empty() {
        anyhow::bail!("{label} must not be empty");
    }
    let mut chars = name.chars();
    let first = chars.next().expect("checked non-empty");
    if !(first.is_ascii_alphabetic() || first == '_') {
        anyhow::bail!("{label} must start with [A-Za-z_]: {name}");
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        anyhow::bail!("{label} contains invalid characters: {name}");
    }
    Ok(())
}

/// Split markdown with YAML frontmatter (`---`) into `(yaml, body)`.
pub fn split_frontmatter(contents: &str) -> anyhow::Result<(&str, &str)> {
    let Some(first_newline) = contents.find('\n') else {
        anyhow::bail!("missing frontmatter start delimiter");
    };
    let first_line = contents[..first_newline]
        .trim_end_matches(['\r', '\n'])
        .trim_end();
    if first_line != "---" {
        anyhow::bail!("missing frontmatter start delimiter");
    }

    let yaml_start = first_newline + 1;
    let mut cursor = yaml_start;
    while cursor < contents.len() {
        let line_end = match contents[cursor..].find('\n') {
            Some(rel) => cursor + rel + 1,
            None => contents.len(),
        };
        let line = contents[cursor..line_end].trim_end_matches(['\r', '\n']);
        if line == "---" {
            let yaml = &contents[yaml_start..cursor];
            let body = &contents[line_end..];
            return Ok((yaml, body));
        }
        if line_end == contents.len() {
            break;
        }
        cursor = line_end;
    }

    anyhow::bail!("missing frontmatter end delimiter")
}

/// Trim, drop empties, and dedupe while preserving first-seen order.
pub fn normalize_unique_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::<String>::new();
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

/// Render `{{name}}` placeholders with strict validation.
pub fn render_template(
    template: &str,
    declared: &BTreeSet<String>,
    vars: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut cursor = 0usize;

    while let Some(open_rel) = template[cursor..].find("{{") {
        let open = cursor + open_rel;
        out.push_str(&template[cursor..open]);

        let after_open = open + 2;
        let Some(close_rel) = template[after_open..].find("}}") else {
            anyhow::bail!("unclosed template placeholder");
        };
        let close = after_open + close_rel;
        let raw_key = &template[after_open..close];
        if raw_key.trim() != raw_key {
            anyhow::bail!("template placeholder contains whitespace: {raw_key}");
        }
        if raw_key.is_empty() {
            anyhow::bail!("template placeholder must not be empty");
        }
        ensure_valid_var_name(raw_key, "template placeholder")?;
        if !declared.contains(raw_key) {
            anyhow::bail!("undeclared template variable: {raw_key}");
        }
        let value = vars
            .get(raw_key)
            .ok_or_else(|| anyhow::anyhow!("missing template variable: {raw_key}"))?;
        out.push_str(value);
        cursor = close + 2;
    }

    out.push_str(&template[cursor..]);
    Ok(out)
}
