const ROLE_PROMPT_ARCHITECT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/architect.md"
));
const ROLE_PROMPT_BUILDER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/builder.md"
));
const ROLE_PROMPT_CODER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/coder.md"
));
const ROLE_PROMPT_DEBUGGER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/debugger.md"
));
const ROLE_PROMPT_DESIGNER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/designer.md"
));
const ROLE_PROMPT_IDEATOR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/ideator.md"
));
const ROLE_PROMPT_LIBRARIAN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/librarian.md"
));
const ROLE_PROMPT_MERGER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/merger.md"
));
const ROLE_PROMPT_ORCHESTRATOR: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/orchestrator.md"
));
const ROLE_PROMPT_REVIEWER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/reviewer.md"
));
const ROLE_PROMPT_SECURITY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/security.md"
));
const ROLE_PROMPT_SKEPTIC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompt/roles/skeptic.md"
));

#[derive(Clone, Copy)]
struct RolePromptParts {
    yaml_frontmatter: Option<&'static str>,
    markdown_body: &'static str,
}

fn split_yaml_frontmatter(raw: &'static str) -> RolePromptParts {
    let raw = raw.trim_start_matches('\u{feff}');
    if !raw.starts_with("---") {
        return RolePromptParts {
            yaml_frontmatter: None,
            markdown_body: raw,
        };
    }

    let mut offset = 0usize;
    let mut lines = raw.split_inclusive('\n');
    let Some(first_line) = lines.next() else {
        return RolePromptParts {
            yaml_frontmatter: None,
            markdown_body: raw,
        };
    };
    let first_trimmed = first_line.trim_end_matches('\n').trim_end_matches('\r');
    if first_trimmed != "---" {
        return RolePromptParts {
            yaml_frontmatter: None,
            markdown_body: raw,
        };
    }

    offset += first_line.len();
    let yaml_start = offset;

    for line in lines {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed == "---" {
            let yaml_end = offset;
            let body_start = offset + line.len();
            let yaml = raw[yaml_start..yaml_end].trim();
            let yaml_frontmatter = if yaml.is_empty() { None } else { Some(yaml) };
            let markdown_body = &raw[body_start..];
            return RolePromptParts {
                yaml_frontmatter,
                markdown_body,
            };
        }
        offset += line.len();
    }

    RolePromptParts {
        yaml_frontmatter: None,
        markdown_body: raw,
    }
}

fn role_prompt_parts_for_mode(mode: &str) -> Option<RolePromptParts> {
    match mode {
        "architect" => Some(split_yaml_frontmatter(ROLE_PROMPT_ARCHITECT)),
        "builder" => Some(split_yaml_frontmatter(ROLE_PROMPT_BUILDER)),
        "coder" => Some(split_yaml_frontmatter(ROLE_PROMPT_CODER)),
        "debugger" => Some(split_yaml_frontmatter(ROLE_PROMPT_DEBUGGER)),
        "designer" => Some(split_yaml_frontmatter(ROLE_PROMPT_DESIGNER)),
        "ideator" => Some(split_yaml_frontmatter(ROLE_PROMPT_IDEATOR)),
        "librarian" => Some(split_yaml_frontmatter(ROLE_PROMPT_LIBRARIAN)),
        "merger" => Some(split_yaml_frontmatter(ROLE_PROMPT_MERGER)),
        "orchestrator" => Some(split_yaml_frontmatter(ROLE_PROMPT_ORCHESTRATOR)),
        "reviewer" => Some(split_yaml_frontmatter(ROLE_PROMPT_REVIEWER)),
        "security" => Some(split_yaml_frontmatter(ROLE_PROMPT_SECURITY)),
        "skeptic" => Some(split_yaml_frontmatter(ROLE_PROMPT_SKEPTIC)),
        _ => None,
    }
}

fn render_role_message_block(mode: &str) -> Option<String> {
    let role = role_prompt_parts_for_mode(mode)?;
    let mut out = String::new();
    out.push_str("@role <role>\n");
    out.push_str(&format!("name: {mode}\n\n"));

    let body = role.markdown_body.trim();
    if !body.is_empty() {
        out.push_str("## Role prompt\n\n");
        out.push_str(body);
        out.push_str("\n\n");
    }

    if let Some(yaml) = role.yaml_frontmatter {
        let yaml = yaml.trim();
        if !yaml.is_empty() {
            out.push_str("## Role permissions\n\n```yaml\n");
            out.push_str(yaml);
            out.push_str("\n```\n\n");
        }
    }

    out.push_str("</role>");
    Some(out)
}
