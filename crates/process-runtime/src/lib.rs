fn normalized_program_name(program: &str) -> String {
    let mut name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    if let Some(stripped) = name.strip_suffix(".exe") {
        name = stripped.to_string();
    }
    name
}

fn git_global_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-C" | "-c"
            | "--exec-path"
            | "--git-dir"
            | "--namespace"
            | "--super-prefix"
            | "--work-tree"
            | "--config-env"
    )
}

fn git_option_has_inline_value(arg: &str) -> bool {
    (arg.starts_with("-C") || arg.starts_with("-c")) && arg.len() > 2 || arg.contains('=')
}

fn git_subcommand(argv: &[String]) -> Option<&str> {
    let mut index = 1usize;
    while let Some(arg) = argv.get(index) {
        if arg == "--" {
            return argv.get(index + 1).map(String::as_str);
        }
        if !arg.starts_with('-') || arg == "-" {
            return Some(arg.as_str());
        }
        if git_global_option_takes_value(arg) && !git_option_has_inline_value(arg) {
            index += 1;
        }
        index += 1;
    }
    None
}

fn git_subcommand_uses_network(argv: &[String]) -> bool {
    git_subcommand(argv)
        .map(|subcommand| {
            matches!(
                subcommand,
                "clone" | "fetch" | "pull" | "push" | "ls-remote" | "submodule"
            )
        })
        .unwrap_or(false)
}

// This is a best-effort argv classifier used by omne-agent's network deny gate.
// It only covers commands that are clearly network-oriented from argv alone; it
// is not an OS-level network isolation primitive and should not be treated as one.
pub fn command_uses_network(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };

    let name = normalized_program_name(program);

    match name.as_str() {
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "ftp" | "telnet" | "nc" | "ncat" | "netcat"
        | "gh" => true,
        "git" => git_subcommand_uses_network(argv),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::command_uses_network;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn detects_common_network_programs() {
        assert!(command_uses_network(&argv(&["curl"])));
        assert!(command_uses_network(&argv(&["/usr/bin/wget"])));
        assert!(command_uses_network(&argv(&[
            "C:\\Windows\\System32\\SSH.EXE"
        ])));
    }

    #[test]
    fn detects_git_network_subcommands_only() {
        assert!(command_uses_network(&argv(&["git", "clone"])));
        assert!(command_uses_network(&argv(&["git", "fetch"])));
        assert!(command_uses_network(&argv(&["git", "-C", "repo", "fetch"])));
        assert!(command_uses_network(&argv(&[
            "git",
            "--git-dir=.git",
            "pull"
        ])));
        assert!(command_uses_network(&argv(&[
            "git",
            "-chttp.extraHeader=x",
            "push"
        ])));
        assert!(!command_uses_network(&argv(&["git", "status"])));
        assert!(!command_uses_network(&argv(&[
            "git", "-C", "repo", "status"
        ])));
        assert!(!command_uses_network(&argv(&["git"])));
    }

    #[test]
    fn non_network_commands_are_not_flagged() {
        assert!(!command_uses_network(&argv(&["ls"])));
        assert!(!command_uses_network(&argv(&["python"])));
        assert!(!command_uses_network(&[]));
    }

    #[test]
    fn opaque_path_invocations_are_not_classified_from_top_level_argv_alone() {
        assert!(!command_uses_network(&argv(&["./local-tool"])));
        assert!(!command_uses_network(&argv(&["tools/local-tool"])));
        assert!(!command_uses_network(&argv(&["C:\\tools\\local-tool.exe"])));
    }

    #[test]
    fn generic_launchers_are_not_classified_by_top_level_argv_shape() {
        assert!(!command_uses_network(&argv(&[
            "python",
            "-m",
            "http.server"
        ])));
        assert!(!command_uses_network(&argv(&["node", "server.js"])));
        assert!(!command_uses_network(&argv(&["bash", "script.sh"])));
    }
}
