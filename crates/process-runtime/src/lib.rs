use std::path::Path;

pub fn command_uses_network(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };

    let mut name = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program.as_str())
        .to_ascii_lowercase();
    if let Some(stripped) = name.strip_suffix(".exe") {
        name = stripped.to_string();
    }

    match name.as_str() {
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "ftp" | "telnet" | "nc" | "ncat" | "netcat"
        | "gh" => true,
        "git" => argv
            .get(1)
            .map(|subcommand| {
                matches!(
                    subcommand.as_str(),
                    "clone" | "fetch" | "pull" | "push" | "ls-remote" | "submodule"
                )
            })
            .unwrap_or(false),
        _ => false,
    }
}
