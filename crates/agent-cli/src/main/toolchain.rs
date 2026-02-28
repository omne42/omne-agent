use std::process::Stdio;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ToolchainBootstrapStatus {
    Present,
    InstalledBundled,
    MissingWithoutFeature,
    FeatureMismatchMissingBinary,
    InstallFailed,
}

impl ToolchainBootstrapStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::InstalledBundled => "installed_bundled",
            Self::MissingWithoutFeature => "missing_without_feature",
            Self::FeatureMismatchMissingBinary => "feature_mismatch_missing_binary",
            Self::InstallFailed => "install_failed",
        }
    }

    fn is_success(self) -> bool {
        matches!(self, Self::Present | Self::InstalledBundled)
    }
}

#[derive(Debug, Clone, Serialize)]
struct ToolchainBootstrapItem {
    tool: String,
    status: ToolchainBootstrapStatus,
    detail: Option<String>,
    source: Option<String>,
    destination: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ToolchainBootstrapResult {
    schema_version: u32,
    target_triple: String,
    managed_dir: String,
    bundled_dir: Option<String>,
    items: Vec<ToolchainBootstrapItem>,
}

#[derive(Debug, Deserialize)]
struct BundledFeaturesFile {
    features: Option<Vec<String>>,
}

async fn run_toolchain_bootstrap(args: ToolchainBootstrapArgs) -> anyhow::Result<()> {
    let result = bootstrap_toolchain(&args)?;
    if args.json {
        let value = serde_json::to_value(&result).context("serialize toolchain/bootstrap response")?;
        print_json_or_pretty(true, &value)?;
    } else {
        println!(
            "toolchain bootstrap: target={} managed_dir={} bundled_dir={}",
            result.target_triple,
            result.managed_dir,
            result.bundled_dir.as_deref().unwrap_or("-")
        );
        for item in &result.items {
            let detail = item
                .detail
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            println!("- {}: {}{}", item.tool, item.status.as_str(), detail);
        }
    }

    if args.strict && has_bootstrap_failure(&result.items) {
        anyhow::bail!("toolchain bootstrap did not satisfy strict mode requirements");
    }
    Ok(())
}

fn has_bootstrap_failure(items: &[ToolchainBootstrapItem]) -> bool {
    items.iter().any(|item| !item.status.is_success())
}

fn bootstrap_toolchain(args: &ToolchainBootstrapArgs) -> anyhow::Result<ToolchainBootstrapResult> {
    let target_triple = detect_target_triple(args.target_triple.as_deref())
        .ok_or_else(|| anyhow::anyhow!("unsupported platform/arch for target triple detection"))?;
    let managed_dir =
        resolve_managed_toolchain_dir(args.managed_dir.as_deref(), &target_triple).ok_or_else(
            || anyhow::anyhow!("cannot resolve managed toolchain directory (missing HOME/USERPROFILE)"),
        )?;
    let bundled_dir = resolve_bundled_toolchain_dir(args.bundled_dir.as_deref(), &target_triple);
    let bundled_features = load_bundled_features(bundled_dir.as_deref());
    let binary_ext = target_binary_ext(&target_triple);

    let items = vec![
        bootstrap_one_tool(
            "git",
            "git-cli",
            binary_ext,
            bundled_dir.as_deref(),
            &bundled_features,
            &managed_dir,
        ),
        bootstrap_one_tool(
            "gh",
            "gh-cli",
            binary_ext,
            bundled_dir.as_deref(),
            &bundled_features,
            &managed_dir,
        ),
    ];

    Ok(ToolchainBootstrapResult {
        schema_version: 1,
        target_triple,
        managed_dir: managed_dir.display().to_string(),
        bundled_dir: bundled_dir.map(|path| path.display().to_string()),
        items,
    })
}

fn bootstrap_one_tool(
    tool: &str,
    feature_name: &str,
    binary_ext: &str,
    bundled_dir: Option<&std::path::Path>,
    bundled_features: &std::collections::BTreeSet<String>,
    managed_dir: &std::path::Path,
) -> ToolchainBootstrapItem {
    if command_available(tool) {
        return ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::Present,
            detail: None,
            source: None,
            destination: None,
        };
    }

    let destination = managed_dir.join(format!("{tool}{binary_ext}"));
    if destination.exists() {
        return ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::InstalledBundled,
            detail: Some("managed binary already exists".to_string()),
            source: None,
            destination: Some(destination.display().to_string()),
        };
    }

    let feature_enabled = bundled_features.contains(feature_name);
    if !feature_enabled {
        return ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::MissingWithoutFeature,
            detail: Some(format!("{feature_name} not available in bundled features")),
            source: None,
            destination: Some(destination.display().to_string()),
        };
    }

    let Some(source_dir) = bundled_dir else {
        return ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::FeatureMismatchMissingBinary,
            detail: Some("bundled toolchain directory is unavailable".to_string()),
            source: None,
            destination: Some(destination.display().to_string()),
        };
    };
    let source = source_dir.join(format!("{tool}{binary_ext}"));
    if !source.exists() {
        return ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::FeatureMismatchMissingBinary,
            detail: Some("bundled feature declared but binary is missing".to_string()),
            source: Some(source.display().to_string()),
            destination: Some(destination.display().to_string()),
        };
    }

    let install_result = install_bundled_tool(&source, &destination);
    match install_result {
        Ok(()) => ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::InstalledBundled,
            detail: None,
            source: Some(source.display().to_string()),
            destination: Some(destination.display().to_string()),
        },
        Err(err) => ToolchainBootstrapItem {
            tool: tool.to_string(),
            status: ToolchainBootstrapStatus::InstallFailed,
            detail: Some(err.to_string()),
            source: Some(source.display().to_string()),
            destination: Some(destination.display().to_string()),
        },
    }
}

fn install_bundled_tool(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::copy(source, destination)
        .with_context(|| format!("copy {} -> {}", source.display(), destination.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(destination)
            .with_context(|| format!("stat {}", destination.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(destination, perms)
            .with_context(|| format!("chmod {}", destination.display()))?;
    }
    Ok(())
}

fn command_available(command: &str) -> bool {
    let mut cmd = std::process::Command::new(command);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.status() {
        Ok(_) => true,
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    }
}

fn target_binary_ext(target_triple: &str) -> &'static str {
    if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

fn detect_target_triple(override_target: Option<&str>) -> Option<String> {
    if let Some(raw) = override_target {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin".to_string()),
        ("macos", "x86_64") => Some("x86_64-apple-darwin".to_string()),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu".to_string()),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu".to_string()),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc".to_string()),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc".to_string()),
        _ => None,
    }
}

fn resolve_managed_toolchain_dir(
    override_dir: Option<&std::path::Path>,
    target_triple: &str,
) -> Option<std::path::PathBuf> {
    if let Some(override_dir) = override_dir {
        return Some(override_dir.to_path_buf());
    }
    if let Ok(raw) = std::env::var("OMNE_MANAGED_TOOLCHAIN_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed));
        }
    }

    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))?;
    let mut out = std::path::PathBuf::from(home);
    out.push(".omne");
    out.push("toolchain");
    out.push(target_triple);
    out.push("bin");
    Some(out)
}

fn resolve_bundled_toolchain_dir(
    override_dir: Option<&std::path::Path>,
    target_triple: &str,
) -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = override_dir {
        candidates.push(path.to_path_buf());
    }
    if let Ok(raw) = std::env::var("OMNE_BUNDLED_TOOLCHAIN_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            candidates.push(std::path::PathBuf::from(trimmed));
        }
    }
    if let Ok(raw) = std::env::var("OMNE_PACKAGE_ROOT") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            candidates.push(
                std::path::Path::new(trimmed)
                    .join("vendor")
                    .join(target_triple)
                    .join("path"),
            );
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("path"));
            if let Some(parent) = exe_dir.parent() {
                candidates.push(parent.join("path"));
            }
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn load_bundled_features(
    bundled_dir: Option<&std::path::Path>,
) -> std::collections::BTreeSet<String> {
    let Some(bundled_dir) = bundled_dir else {
        return std::collections::BTreeSet::new();
    };
    let mut out = std::collections::BTreeSet::new();
    let features_path = bundled_dir
        .parent()
        .map(|parent| parent.join("features.json"));
    if let Some(features_path) = features_path {
        if let Ok(text) = std::fs::read_to_string(&features_path) {
            if let Ok(parsed) = serde_json::from_str::<BundledFeaturesFile>(&text) {
                for feature in parsed.features.unwrap_or_default() {
                    let normalized = feature.trim().to_ascii_lowercase();
                    if !normalized.is_empty() {
                        out.insert(normalized);
                    }
                }
            }
        }
    }
    if out.is_empty() {
        if bundled_dir.join("git").exists() || bundled_dir.join("git.exe").exists() {
            out.insert("git-cli".to_string());
        }
        if bundled_dir.join("gh").exists() || bundled_dir.join("gh.exe").exists() {
            out.insert("gh-cli".to_string());
        }
    }
    out
}

#[cfg(test)]
mod toolchain_tests {
    use super::*;

    #[test]
    fn detect_target_triple_prefers_override() {
        let detected = detect_target_triple(Some("custom-target")).expect("target");
        assert_eq!(detected, "custom-target");
    }

    #[test]
    fn bootstrap_failure_detects_non_success_status() {
        let items = vec![
            ToolchainBootstrapItem {
                tool: "git".to_string(),
                status: ToolchainBootstrapStatus::Present,
                detail: None,
                source: None,
                destination: None,
            },
            ToolchainBootstrapItem {
                tool: "gh".to_string(),
                status: ToolchainBootstrapStatus::MissingWithoutFeature,
                detail: None,
                source: None,
                destination: None,
            },
        ];
        assert!(has_bootstrap_failure(&items));
    }
}
