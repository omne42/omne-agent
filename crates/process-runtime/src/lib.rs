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

fn shell_snippet_uses_network(command: &str, depth_remaining: usize) -> bool {
    let Some(tokens) = shlex::split(command) else {
        return false;
    };
    command_sequence_uses_network(&tokens, depth_remaining)
}

fn command_sequence_uses_network(argv: &[String], depth_remaining: usize) -> bool {
    let control_operators = ["&&", "||", ";", "|", "|&", "&"];
    let mut segment_start = 0usize;

    for (index, token) in argv.iter().enumerate() {
        if control_operators.contains(&token.as_str()) {
            if argv_uses_network(&argv[segment_start..index], depth_remaining) {
                return true;
            }
            segment_start = index + 1;
        }
    }

    argv_uses_network(&argv[segment_start..], depth_remaining)
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

fn env_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-u" | "--unset" | "--chdir" | "-C" | "--split-string" | "-S"
    )
}

fn env_wrapped_command<'a>(argv: &'a [String]) -> &'a [String] {
    let mut index = 1usize;
    while let Some(arg) = argv.get(index) {
        if arg == "--" {
            return argv.get(index + 1..).unwrap_or(&[]);
        }
        if arg.contains('=') && !arg.starts_with('-') {
            index += 1;
            continue;
        }
        if !arg.starts_with('-') || arg == "-" {
            return &argv[index..];
        }
        if env_option_takes_value(arg) && !arg.contains('=') {
            index += 1;
        }
        index += 1;
    }
    &[]
}

fn python_eval_uses_network(argv: &[String], depth_remaining: usize) -> bool {
    if depth_remaining == 0 {
        return false;
    }
    let mut index = 1usize;
    while let Some(arg) = argv.get(index) {
        match arg.as_str() {
            "-c" | "--command" => {
                let Some(code) = argv.get(index + 1) else {
                    return false;
                };
                let code = code.to_ascii_lowercase();
                return code.contains("http://")
                    || code.contains("https://")
                    || code.contains("import socket")
                    || code.contains("from socket")
                    || code.contains("import requests")
                    || code.contains("from requests")
                    || code.contains("import urllib")
                    || code.contains("from urllib")
                    || code.contains("import httpx")
                    || code.contains("from httpx")
                    || code.contains("import aiohttp")
                    || code.contains("from aiohttp")
                    || code.contains("import ftplib")
                    || code.contains("from ftplib")
                    || code.contains("import websocket")
                    || code.contains("from websocket")
                    || code.contains("import websockets")
                    || code.contains("from websockets");
            }
            "-m" | "--module" => return false,
            _ => {
                if !arg.starts_with('-') {
                    return false;
                }
            }
        }
        index += 1;
    }
    false
}

fn node_eval_uses_network(argv: &[String], depth_remaining: usize) -> bool {
    if depth_remaining == 0 {
        return false;
    }
    let mut index = 1usize;
    while let Some(arg) = argv.get(index) {
        match arg.as_str() {
            "-e" | "--eval" => {
                let Some(code) = argv.get(index + 1) else {
                    return false;
                };
                let code = code.to_ascii_lowercase();
                return code.contains("http://")
                    || code.contains("https://")
                    || code.contains("fetch(")
                    || code.contains("xmlhttprequest")
                    || code.contains("require('http')")
                    || code.contains("require(\"http\")")
                    || code.contains("require('https')")
                    || code.contains("require(\"https\")")
                    || code.contains("require('net')")
                    || code.contains("require(\"net\")")
                    || code.contains("require('tls')")
                    || code.contains("require(\"tls\")")
                    || code.contains("import http from")
                    || code.contains("import https from")
                    || code.contains("import net from")
                    || code.contains("import tls from");
            }
            _ => {
                if !arg.starts_with('-') {
                    return false;
                }
            }
        }
        index += 1;
    }
    false
}

fn shell_invocation_uses_network(argv: &[String], depth_remaining: usize) -> bool {
    if depth_remaining == 0 {
        return false;
    }
    let mut index = 1usize;
    while let Some(arg) = argv.get(index) {
        if arg == "--" {
            index += 1;
            continue;
        }
        if arg == "-c" || arg.ends_with('c') && arg.starts_with('-') {
            let Some(command) = argv.get(index + 1) else {
                return false;
            };
            return shell_snippet_uses_network(command, depth_remaining - 1);
        }
        if !arg.starts_with('-') || arg == "-" {
            return false;
        }
        index += 1;
    }
    false
}

fn argv_uses_network(argv: &[String], depth_remaining: usize) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };

    let name = normalized_program_name(program);

    match name.as_str() {
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "ftp" | "telnet" | "nc" | "ncat" | "netcat"
        | "gh" => true,
        "git" => git_subcommand_uses_network(argv),
        "env" => {
            let wrapped = env_wrapped_command(argv);
            !wrapped.is_empty() && argv_uses_network(wrapped, depth_remaining.saturating_sub(1))
        }
        "python" | "python3" => python_eval_uses_network(argv, depth_remaining),
        "node" | "nodejs" | "deno" | "bun" => node_eval_uses_network(argv, depth_remaining),
        "bash" | "rbash" | "sh" | "dash" | "zsh" | "ksh" => {
            shell_invocation_uses_network(argv, depth_remaining)
        }
        _ => false,
    }
}

// This is a best-effort argv classifier used by omne-agent's network deny gate.
// It only covers commands that are clearly network-oriented from argv alone; it
// is not an OS-level network isolation primitive and should not be treated as one.
pub fn command_uses_network(argv: &[String]) -> bool {
    argv_uses_network(argv, 4)
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
        assert!(!command_uses_network(&argv(&[
            "env", "FOO=bar", "printenv", "FOO"
        ])));
    }

    #[test]
    fn detects_wrapped_network_commands() {
        assert!(command_uses_network(&argv(&[
            "env",
            "FOO=bar",
            "curl",
            "https://example.com"
        ])));
        assert!(command_uses_network(&argv(&[
            "python",
            "-c",
            "import requests; requests.get('https://example.com')"
        ])));
        assert!(command_uses_network(&argv(&[
            "node",
            "-e",
            "fetch('https://example.com')"
        ])));
        assert!(command_uses_network(&argv(&[
            "bash",
            "-lc",
            "echo ok && curl https://example.com"
        ])));
        assert!(command_uses_network(&argv(&[
            "sh",
            "-lc",
            "git -C repo fetch origin"
        ])));
    }

    #[test]
    fn wrapped_non_network_commands_are_not_flagged() {
        assert!(!command_uses_network(&argv(&[
            "python",
            "-c",
            "print('hello')"
        ])));
        assert!(!command_uses_network(&argv(&[
            "node",
            "-e",
            "console.log('hello')"
        ])));
        assert!(!command_uses_network(&argv(&[
            "bash",
            "-lc",
            "echo ok && pwd"
        ])));
    }
}
