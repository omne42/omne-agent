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

fn git_subcommand_uses_network(subcommand: Option<&String>) -> bool {
    subcommand
        .map(|subcommand| {
            matches!(
                subcommand.as_str(),
                "clone" | "fetch" | "pull" | "push" | "ls-remote" | "submodule"
            )
        })
        .unwrap_or(false)
}

pub fn command_uses_network(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };

    let name = normalized_program_name(program);

    match name.as_str() {
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "ftp" | "telnet" | "nc" | "ncat" | "netcat"
        | "gh" => true,
        "git" => git_subcommand_uses_network(argv.get(1)),
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
        assert!(!command_uses_network(&argv(&["git", "status"])));
        assert!(!command_uses_network(&argv(&["git"])));
    }

    #[test]
    fn non_network_commands_are_not_flagged() {
        assert!(!command_uses_network(&argv(&["ls"])));
        assert!(!command_uses_network(&argv(&["python", "script.py"])));
        assert!(!command_uses_network(&[]));
    }
}
