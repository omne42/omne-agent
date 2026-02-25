fn format_repo_search_artifact(
    root: &str,
    query: &str,
    is_regex: bool,
    include_glob: Option<&str>,
    outcome: &omne_repo_scan_runtime::RepoGrepOutcome,
) -> String {
    let mut out = String::new();
    out.push_str("# Repo Search\n\n");
    out.push_str("## Query\n");
    out.push_str(&format!("- root: `{root}`\n"));
    out.push_str(&format!("- query: `{}`\n", query.trim()));
    out.push_str(&format!("- is_regex: `{is_regex}`\n"));
    if let Some(glob) = include_glob {
        out.push_str(&format!("- include_glob: `{glob}`\n"));
    } else {
        out.push_str("- include_glob: (none)\n");
    }

    out.push_str("\n## Stats\n");
    let stats = serde_json::json!({
        "matches": outcome.matches.len(),
        "truncated": outcome.truncated,
        "files_scanned": outcome.files_scanned,
        "files_skipped_too_large": outcome.files_skipped_too_large,
        "files_skipped_binary": outcome.files_skipped_binary,
    });
    match serde_json::to_string_pretty(&stats) {
        Ok(json) => out.push_str(&format!("```json\n{json}\n```\n")),
        Err(_) => out.push_str(&format!("```json\n{}\n```\n", stats)),
    }

    out.push_str("\n## Results\n");
    out.push_str("```text\n");
    for m in &outcome.matches {
        out.push_str(&format!(
            "{}:{}: {}\n",
            m.path,
            m.line_number,
            m.line.replace('\n', " ")
        ));
    }
    if outcome.truncated {
        out.push_str("... (truncated)\n");
    }
    out.push_str("```\n");
    out
}

fn format_repo_index_artifact(
    root: &str,
    include_glob: Option<&str>,
    max_files: usize,
    outcome: &omne_repo_scan_runtime::RepoIndexOutcome,
) -> String {
    let mut out = String::new();
    out.push_str("# Repo Index\n\n");

    out.push_str("## Config\n");
    out.push_str(&format!("- root: `{root}`\n"));
    if let Some(glob) = include_glob {
        out.push_str(&format!("- include_glob: `{glob}`\n"));
    } else {
        out.push_str("- include_glob: (none)\n");
    }
    out.push_str(&format!("- max_files: `{max_files}`\n"));

    out.push_str("\n## Stats\n");
    let stats = serde_json::json!({
        "files_scanned": outcome.files_scanned,
        "truncated": outcome.truncated,
        "size_bytes": outcome.size_bytes,
        "paths_listed": outcome.paths.len(),
    });
    match serde_json::to_string_pretty(&stats) {
        Ok(json) => out.push_str(&format!("```json\n{json}\n```\n")),
        Err(_) => out.push_str(&format!("```json\n{}\n```\n", stats)),
    }

    out.push_str("\n## Sample Paths\n");
    out.push_str("```text\n");
    for path in &outcome.paths {
        out.push_str(path);
        out.push('\n');
    }
    if outcome.truncated {
        out.push_str("... (truncated)\n");
    }
    out.push_str("```\n");
    out
}

fn format_repo_symbols_artifact(
    root: &str,
    include_glob: &str,
    max_files: usize,
    max_bytes_per_file: u64,
    max_symbols: usize,
    outcome: &omne_repo_symbols_runtime::RepoSymbolsOutcome,
) -> String {
    let mut out = String::new();
    out.push_str("# Repo Symbols (Rust)\n\n");

    out.push_str("## Config\n");
    out.push_str(&format!("- root: `{root}`\n"));
    out.push_str(&format!("- include_glob: `{include_glob}`\n"));
    out.push_str(&format!("- max_files: `{max_files}`\n"));
    out.push_str(&format!("- max_bytes_per_file: `{max_bytes_per_file}`\n"));
    out.push_str(&format!("- max_symbols: `{max_symbols}`\n"));

    out.push_str("\n## Stats\n");
    let stats = serde_json::json!({
        "files_scanned": outcome.files_scanned,
        "files_parsed": outcome.files_parsed,
        "symbols": outcome.symbols.len(),
        "truncated_files": outcome.truncated_files,
        "truncated_symbols": outcome.truncated_symbols,
        "files_skipped_too_large": outcome.files_skipped_too_large,
        "files_skipped_binary": outcome.files_skipped_binary,
        "files_failed_parse": outcome.files_failed_parse,
    });
    match serde_json::to_string_pretty(&stats) {
        Ok(json) => out.push_str(&format!("```json\n{json}\n```\n")),
        Err(_) => out.push_str(&format!("```json\n{}\n```\n", stats)),
    }

    out.push_str("\n## Symbols\n");
    let mut by_path =
        std::collections::BTreeMap::<&str, Vec<&omne_repo_symbols_runtime::RepoSymbol>>::new();
    for sym in &outcome.symbols {
        by_path.entry(sym.path.as_str()).or_default().push(sym);
    }

    for (path, mut symbols) in by_path {
        symbols.sort_by_key(|sym| {
            (
                sym.start_line,
                sym.end_line,
                sym.kind.as_str(),
                sym.name.as_str(),
            )
        });
        out.push_str(&format!("\n### `{path}`\n\n"));
        for sym in symbols {
            out.push_str(&format!(
                "- `{}` `{}` (L{}-L{})\n",
                sym.kind, sym.name, sym.start_line, sym.end_line
            ));
        }
    }

    if outcome.truncated_files || outcome.truncated_symbols {
        out.push_str("\n---\n\n_truncated=true_\n");
    }

    out
}

