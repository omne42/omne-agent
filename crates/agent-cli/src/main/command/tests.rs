#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_frontmatter_handles_crlf() -> anyhow::Result<()> {
        let raw = "---\r\nversion: 1\r\nmode: coder\r\n---\r\nbody\r\n";
        let (yaml, body) = split_frontmatter(raw)?;
        assert!(yaml.contains("version: 1"));
        assert_eq!(body, "body\r\n");
        Ok(())
    }

    #[test]
    fn render_template_replaces_value() -> anyhow::Result<()> {
        let mut declared = BTreeSet::new();
        declared.insert("name".to_string());
        let mut vars = BTreeMap::new();
        vars.insert("name".to_string(), "ok".to_string());
        let rendered = render_template("hello {{name}}", &declared, &vars)?;
        assert_eq!(rendered, "hello ok");
        Ok(())
    }

    #[test]
    fn render_template_rejects_whitespace() {
        let mut declared = BTreeSet::new();
        declared.insert("name".to_string());
        let mut vars = BTreeMap::new();
        vars.insert("name".to_string(), "ok".to_string());
        let err = render_template("{{ name }}", &declared, &vars).unwrap_err();
        assert!(err.to_string().contains("whitespace"));
    }

    #[test]
    fn parse_workflow_tasks_extracts_task_sections() -> anyhow::Result<()> {
        let body = "Intro\n\n## Task: t1 First\nhello\n\n## Task: t2\nworld\n";
        let tasks = parse_workflow_tasks(body)?;
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "t1");
        assert_eq!(tasks[0].title, "First");
        assert!(tasks[0].body.contains("hello"));
        assert_eq!(tasks[1].id, "t2");
        assert_eq!(tasks[1].title, "");
        assert!(tasks[1].body.contains("world"));
        Ok(())
    }
}
