use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct RoleDef {
    /// Mode name used when role-based allowed_tools checks need a permission source.
    pub permission_mode: String,
}

#[derive(Debug, Clone)]
pub struct RoleCatalog {
    roles: BTreeMap<String, RoleDef>,
}

impl RoleCatalog {
    pub fn builtin() -> Self {
        let mut roles = BTreeMap::<String, RoleDef>::new();

        add_role(&mut roles, "architect", "architect");
        add_role(&mut roles, "reviewer", "reviewer");
        add_role(&mut roles, "builder", "builder");
        add_role(&mut roles, "code", "code");
        add_role(&mut roles, "coder", "coder");
        add_role(&mut roles, "codder", "codder");
        add_role(&mut roles, "default", "default");
        add_role(&mut roles, "chatter", "chatter");
        add_role(&mut roles, "chat", "chat");
        add_role(&mut roles, "roleplay", "roleplay");
        add_role(&mut roles, "author", "author");
        add_role(&mut roles, "doc_organizer", "doc_organizer");
        add_role(&mut roles, "doc-organizer", "doc_organizer");
        add_role(&mut roles, "作者", "author");
        add_role(&mut roles, "文档整理者", "doc_organizer");

        Self { roles }
    }

    pub fn role(&self, name: &str) -> Option<&RoleDef> {
        self.roles.get(name)
    }

    pub fn role_names(&self) -> impl Iterator<Item = &str> {
        self.roles.keys().map(String::as_str)
    }

    pub fn permission_mode_name(&self, role_name: &str) -> Option<&str> {
        self.role(role_name)
            .map(|role| role.permission_mode.as_str())
    }
}

fn add_role(roles: &mut BTreeMap<String, RoleDef>, name: &str, permission_mode: &str) {
    roles.insert(
        name.to_string(),
        RoleDef {
            permission_mode: permission_mode.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_role_catalog_includes_core_roles() {
        let catalog = RoleCatalog::builtin();
        assert!(catalog.role("chatter").is_some());
        assert!(catalog.role("default").is_some());
        assert!(catalog.role("codder").is_some());
        assert!(catalog.role("coder").is_some());
        assert!(catalog.role("作者").is_some());
        assert!(catalog.role("文档整理者").is_some());
    }

    #[test]
    fn builtin_role_catalog_resolves_doc_organizer_alias() {
        let catalog = RoleCatalog::builtin();
        assert_eq!(
            catalog.permission_mode_name("doc-organizer"),
            Some("doc_organizer")
        );
    }
}
