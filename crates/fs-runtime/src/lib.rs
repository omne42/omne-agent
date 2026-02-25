use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobOutcome {
    pub paths: Vec<String>,
    pub truncated: bool,
}

pub fn glob_read_only_paths(
    root_id: String,
    root: PathBuf,
    pattern: String,
    max_results: usize,
) -> anyhow::Result<GlobOutcome> {
    let mut secrets = safe_fs_tools::policy::SecretRules::default();
    secrets.deny_globs.extend(
        [
            ".omne_data/**",
            "**/.omne_data/**",
            ".omne/**",
            "**/.omne/**",
            "target/**",
            "**/target/**",
            "node_modules/**",
            "**/node_modules/**",
            "example/**",
            "**/example/**",
        ]
        .into_iter()
        .map(ToString::to_string),
    );

    let policy = safe_fs_tools::policy::SandboxPolicy {
        roots: vec![safe_fs_tools::policy::Root {
            id: root_id.clone(),
            path: root,
            mode: safe_fs_tools::policy::RootMode::ReadOnly,
        }],
        permissions: safe_fs_tools::policy::Permissions {
            glob: true,
            ..Default::default()
        },
        limits: safe_fs_tools::policy::Limits {
            max_results,
            ..Default::default()
        },
        secrets,
        traversal: Default::default(),
        paths: Default::default(),
    };
    let ctx =
        safe_fs_tools::ops::Context::new(policy).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let resp =
        safe_fs_tools::ops::glob_paths(&ctx, safe_fs_tools::ops::GlobRequest { root_id, pattern })
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let paths = resp
        .matches
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    Ok(GlobOutcome {
        paths,
        truncated: resp.truncated,
    })
}
