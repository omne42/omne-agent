use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use globset::Glob;
use walkdir::WalkDir;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSymbol {
    pub path: String,
    pub kind: String,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSymbolsOutcome {
    pub symbols: Vec<RepoSymbol>,
    pub truncated_files: bool,
    pub truncated_symbols: bool,
    pub files_scanned: usize,
    pub files_parsed: usize,
    pub files_skipped_too_large: usize,
    pub files_skipped_binary: usize,
    pub files_failed_parse: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSymbolsRequest {
    pub root: PathBuf,
    pub include_glob: String,
    pub max_files: usize,
    pub max_bytes_per_file: u64,
    pub max_symbols: usize,
}

pub struct RustSymbolCollector {
    parser: tree_sitter::Parser,
}

impl RustSymbolCollector {
    pub fn new() -> anyhow::Result<Self> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .context("set tree-sitter language (rust)")?;
        Ok(Self { parser })
    }

    pub fn collect_for_file(
        &mut self,
        path: &str,
        source: &str,
        out: &mut Vec<RepoSymbol>,
        max_symbols: usize,
    ) -> bool {
        let Some(tree) = self.parser.parse(source, None) else {
            return false;
        };
        let mut module_stack = implicit_module_stack_for_rust_path(path);
        collect_rust_symbols(
            tree.root_node(),
            source,
            path,
            &mut module_stack,
            out,
            max_symbols,
        );
        true
    }
}

pub fn collect_repo_symbols(req: RepoSymbolsRequest) -> anyhow::Result<RepoSymbolsOutcome> {
    let include_matcher = Glob::new(&req.include_glob)
        .with_context(|| format!("invalid glob pattern: {}", req.include_glob))?
        .compile_matcher();
    let mut collector = RustSymbolCollector::new()?;

    let mut symbols = Vec::<RepoSymbol>::new();
    let mut truncated_files = false;
    let mut truncated_symbols = false;
    let mut files_scanned = 0usize;
    let mut files_parsed = 0usize;
    let mut files_skipped_too_large = 0usize;
    let mut files_skipped_binary = 0usize;
    let mut files_failed_parse = 0usize;

    for entry in WalkDir::new(&req.root)
        .follow_links(false)
        .into_iter()
        .filter_entry(omne_fs_policy::should_walk_entry)
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if files_scanned >= req.max_files {
            truncated_files = true;
            break;
        }

        let rel = entry.path().strip_prefix(&req.root).unwrap_or(entry.path());
        if omne_fs_policy::is_secret_rel_path(rel) {
            continue;
        }
        if !include_matcher.is_match(rel) {
            continue;
        }

        files_scanned += 1;

        let meta = entry.metadata()?;
        if meta.len() > req.max_bytes_per_file {
            files_skipped_too_large += 1;
            continue;
        }

        let bytes = match std::fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if bytes.contains(&0) {
            files_skipped_binary += 1;
            continue;
        }

        let Ok(source) = std::str::from_utf8(&bytes) else {
            files_failed_parse += 1;
            continue;
        };

        let rel_str = rel.to_string_lossy().to_string();
        if !collector.collect_for_file(&rel_str, source, &mut symbols, req.max_symbols) {
            files_failed_parse += 1;
            continue;
        }
        files_parsed += 1;

        if symbols.len() >= req.max_symbols {
            truncated_symbols = true;
            break;
        }
    }

    Ok(RepoSymbolsOutcome {
        symbols,
        truncated_files,
        truncated_symbols,
        files_scanned,
        files_parsed,
        files_skipped_too_large,
        files_skipped_binary,
        files_failed_parse,
    })
}

fn implicit_module_stack_for_rust_path(path: &str) -> Vec<String> {
    let path = Path::new(path);
    let mut components = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();

    let Some(file_name) = components.pop() else {
        return Vec::new();
    };
    let file_path = Path::new(file_name);
    if file_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return Vec::new();
    }

    let in_src_tree = matches!(components.first(), Some(component) if *component == "src");
    if in_src_tree {
        components.remove(0);
        if matches!(
            components.first().copied(),
            Some("bin" | "tests" | "examples" | "benches")
        ) {
            return Vec::new();
        }
    } else if !components.is_empty() {
        return Vec::new();
    }

    let Some(file_stem) = file_path.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };

    let mut module_stack = components
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if !matches!(file_stem, "lib" | "main" | "mod" | "build") {
        module_stack.push(file_stem.to_string());
    }
    module_stack
}

fn collect_rust_symbols(
    node: tree_sitter::Node<'_>,
    source: &str,
    path: &str,
    module_stack: &mut Vec<String>,
    out: &mut Vec<RepoSymbol>,
    max_symbols: usize,
) {
    if out.len() >= max_symbols {
        return;
    }

    let kind = node.kind();

    let mut entered_module = false;
    if kind == "mod_item"
        && let Some(name_node) = node.child_by_field_name("name")
        && let Some(name) = source.get(name_node.byte_range())
    {
        let full = if module_stack.is_empty() {
            name.to_string()
        } else {
            format!("{}::{name}", module_stack.join("::"))
        };
        let start_line = node.start_position().row.saturating_add(1);
        let end_line = node.end_position().row.saturating_add(1);
        out.push(RepoSymbol {
            path: path.to_string(),
            kind: "mod".to_string(),
            name: full,
            start_line,
            end_line,
        });
        module_stack.push(name.to_string());
        entered_module = true;
    }

    if matches!(
        kind,
        "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
    ) && let Some(name_node) = node.child_by_field_name("name")
        && let Some(name) = source.get(name_node.byte_range())
    {
        let prefix = if module_stack.is_empty() {
            String::new()
        } else {
            format!("{}::", module_stack.join("::"))
        };
        let symbol_kind = match kind {
            "function_item" => "fn",
            "struct_item" => "struct",
            "enum_item" => "enum",
            "trait_item" => "trait",
            "type_item" => "type",
            "const_item" => "const",
            "static_item" => "static",
            _ => kind,
        };
        let start_line = node.start_position().row.saturating_add(1);
        let end_line = node.end_position().row.saturating_add(1);
        out.push(RepoSymbol {
            path: path.to_string(),
            kind: symbol_kind.to_string(),
            name: format!("{prefix}{name}"),
            start_line,
            end_line,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if out.len() >= max_symbols {
            break;
        }
        collect_rust_symbols(child, source, path, module_stack, out, max_symbols);
    }

    if entered_module {
        debug_assert!(module_stack.pop().is_some(), "module stack underflow");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_nested_rust_symbols() {
        let mut collector = RustSymbolCollector::new().expect("collector");
        let source = r#"
mod outer {
    pub struct Foo;
    mod inner {
        fn work() {}
    }
}
"#;
        let mut out = Vec::new();
        let ok = collector.collect_for_file("src/lib.rs", source, &mut out, 100);
        assert!(ok);
        assert!(out.iter().any(|s| s.kind == "mod" && s.name == "outer"));
        assert!(
            out.iter()
                .any(|s| s.kind == "struct" && s.name == "outer::Foo")
        );
        assert!(
            out.iter()
                .any(|s| s.kind == "mod" && s.name == "outer::inner")
        );
        assert!(
            out.iter()
                .any(|s| s.kind == "fn" && s.name == "outer::inner::work")
        );
    }

    #[test]
    fn collect_repo_symbols_scans_rust_sources() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("lib.rs"), "mod a { fn f() {} }\n")?;
        std::fs::write(root.join(".env"), "secret")?;

        let out = collect_repo_symbols(RepoSymbolsRequest {
            root,
            include_glob: "**/*.rs".to_string(),
            max_files: 100,
            max_bytes_per_file: 1024 * 1024,
            max_symbols: 1000,
        })?;
        assert!(out.files_scanned >= 1);
        assert!(out.symbols.iter().any(|s| s.name == "a"));
        assert!(out.symbols.iter().any(|s| s.name == "a::f"));
        Ok(())
    }

    #[test]
    fn collect_repo_symbols_uses_file_backed_module_namespaces() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("src/foo"))?;
        std::fs::write(root.join("src/lib.rs"), "mod foo;\n")?;
        std::fs::write(root.join("src/foo.rs"), "pub fn top() {}\nmod bar;\n")?;
        std::fs::write(root.join("src/foo/bar.rs"), "pub struct Baz;\n")?;

        let out = collect_repo_symbols(RepoSymbolsRequest {
            root,
            include_glob: "**/*.rs".to_string(),
            max_files: 100,
            max_bytes_per_file: 1024 * 1024,
            max_symbols: 1000,
        })?;

        assert!(
            out.symbols
                .iter()
                .any(|s| s.kind == "mod" && s.name == "foo")
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.kind == "fn" && s.name == "foo::top")
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.kind == "mod" && s.name == "foo::bar")
        );
        assert!(
            out.symbols
                .iter()
                .any(|s| s.kind == "struct" && s.name == "foo::bar::Baz")
        );
        Ok(())
    }
}
