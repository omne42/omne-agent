use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

const DEFAULT_GITHUB_API_BASE: &str = "https://api.github.com";
const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 15;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainBootstrapStatus {
    Present,
    InstalledBundled,
    InstalledPublic,
    MissingWithoutFeature,
    FeatureMismatchMissingBinary,
    InstallFailed,
}

impl ToolchainBootstrapStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::InstalledBundled => "installed_bundled",
            Self::InstalledPublic => "installed_public",
            Self::MissingWithoutFeature => "missing_without_feature",
            Self::FeatureMismatchMissingBinary => "feature_mismatch_missing_binary",
            Self::InstallFailed => "install_failed",
        }
    }

    pub fn is_success(self) -> bool {
        matches!(
            self,
            Self::Present | Self::InstalledBundled | Self::InstalledPublic
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolchainBootstrapItem {
    pub tool: String,
    pub status: ToolchainBootstrapStatus,
    pub detail: Option<String>,
    pub source: Option<String>,
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolchainBootstrapResult {
    pub schema_version: u32,
    pub target_triple: String,
    pub managed_dir: String,
    pub bundled_dir: Option<String>,
    pub items: Vec<ToolchainBootstrapItem>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolchainBootstrapRequest {
    pub target_triple: Option<String>,
    pub bundled_dir: Option<PathBuf>,
    pub managed_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct BundledFeaturesFile {
    features: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
struct PublicBootstrapConfig {
    github_api_bases: Vec<String>,
    mirror_prefixes: Vec<String>,
    http_timeout: Duration,
}

impl PublicBootstrapConfig {
    fn from_env() -> Self {
        let github_api_bases = parse_csv_env("OMNE_TOOLCHAIN_GITHUB_API_BASES");
        let github_api_bases = if github_api_bases.is_empty() {
            vec![DEFAULT_GITHUB_API_BASE.to_string()]
        } else {
            github_api_bases
        };

        let mirror_prefixes = parse_csv_env("OMNE_TOOLCHAIN_MIRROR_PREFIXES");

        let http_timeout = std::env::var("OMNE_TOOLCHAIN_HTTP_TIMEOUT_SECONDS")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS));

        Self {
            github_api_bases,
            mirror_prefixes,
            http_timeout,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

pub fn has_bootstrap_failure(items: &[ToolchainBootstrapItem]) -> bool {
    items.iter().any(|item| !item.status.is_success())
}

pub async fn bootstrap_toolchain(
    request: &ToolchainBootstrapRequest,
) -> anyhow::Result<ToolchainBootstrapResult> {
    let target_triple = detect_target_triple(request.target_triple.as_deref())
        .ok_or_else(|| anyhow::anyhow!("unsupported platform/arch for target triple detection"))?;
    let managed_dir = resolve_managed_toolchain_dir(request.managed_dir.as_deref(), &target_triple)
        .ok_or_else(|| {
            anyhow::anyhow!("cannot resolve managed toolchain directory (missing HOME/USERPROFILE)")
        })?;
    let bundled_dir = resolve_bundled_toolchain_dir(request.bundled_dir.as_deref(), &target_triple);
    let bundled_features = load_bundled_features(bundled_dir.as_deref());
    let public_cfg = PublicBootstrapConfig::from_env();
    let http_client = reqwest::Client::builder()
        .timeout(public_cfg.http_timeout)
        .user_agent("omne-toolchain-bootstrap")
        .build()
        .context("build toolchain bootstrap http client")?;
    let binary_ext = target_binary_ext(&target_triple);

    let items = vec![
        bootstrap_one_tool(
            "git",
            "git-cli",
            &target_triple,
            binary_ext,
            bundled_dir.as_deref(),
            &bundled_features,
            &managed_dir,
            &public_cfg,
            &http_client,
        )
        .await,
        bootstrap_one_tool(
            "gh",
            "gh-cli",
            &target_triple,
            binary_ext,
            bundled_dir.as_deref(),
            &bundled_features,
            &managed_dir,
            &public_cfg,
            &http_client,
        )
        .await,
    ];

    Ok(ToolchainBootstrapResult {
        schema_version: 1,
        target_triple,
        managed_dir: managed_dir.display().to_string(),
        bundled_dir: bundled_dir.map(|path| path.display().to_string()),
        items,
    })
}

async fn bootstrap_one_tool(
    tool: &str,
    feature_name: &str,
    target_triple: &str,
    binary_ext: &str,
    bundled_dir: Option<&std::path::Path>,
    bundled_features: &std::collections::BTreeSet<String>,
    managed_dir: &std::path::Path,
    public_cfg: &PublicBootstrapConfig,
    http_client: &reqwest::Client,
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

    let mut detail_parts: Vec<String> = Vec::new();
    let mut source_for_error: Option<String> = None;

    let feature_enabled = bundled_features.contains(feature_name);
    if feature_enabled {
        match install_from_bundled(tool, binary_ext, bundled_dir, &destination) {
            Ok(source) => {
                return ToolchainBootstrapItem {
                    tool: tool.to_string(),
                    status: ToolchainBootstrapStatus::InstalledBundled,
                    detail: None,
                    source: Some(source),
                    destination: Some(destination.display().to_string()),
                };
            }
            Err(err) => {
                if let Some(source) = bundled_source_path(tool, binary_ext, bundled_dir) {
                    source_for_error = Some(source.display().to_string());
                }
                detail_parts.push(format!("bundled bootstrap failed: {err}"));
            }
        }
    } else {
        detail_parts.push(format!("{feature_name} not available in bundled features"));
    }

    if has_public_recipe(tool, target_triple) {
        match install_from_public(
            tool,
            target_triple,
            binary_ext,
            &destination,
            public_cfg,
            http_client,
        )
        .await
        {
            Ok(source) => {
                return ToolchainBootstrapItem {
                    tool: tool.to_string(),
                    status: ToolchainBootstrapStatus::InstalledPublic,
                    detail: None,
                    source: Some(source),
                    destination: Some(destination.display().to_string()),
                };
            }
            Err(err) => {
                detail_parts.push(format!("public upstream bootstrap failed: {err}"));
            }
        }
    } else {
        detail_parts.push(format!(
            "no public bootstrap recipe for target `{target_triple}`"
        ));
    }

    let status = if !feature_enabled && !has_public_recipe(tool, target_triple) {
        ToolchainBootstrapStatus::MissingWithoutFeature
    } else if feature_enabled {
        ToolchainBootstrapStatus::FeatureMismatchMissingBinary
    } else {
        ToolchainBootstrapStatus::InstallFailed
    };
    ToolchainBootstrapItem {
        tool: tool.to_string(),
        status,
        detail: Some(detail_parts.join("; ")),
        source: source_for_error,
        destination: Some(destination.display().to_string()),
    }
}

fn parse_csv_env(name: &str) -> Vec<String> {
    std::env::var(name)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn bundled_source_path(
    tool: &str,
    binary_ext: &str,
    bundled_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    bundled_dir.map(|dir| dir.join(format!("{tool}{binary_ext}")))
}

fn install_from_bundled(
    tool: &str,
    binary_ext: &str,
    bundled_dir: Option<&std::path::Path>,
    destination: &std::path::Path,
) -> anyhow::Result<String> {
    let Some(source) = bundled_source_path(tool, binary_ext, bundled_dir) else {
        anyhow::bail!("bundled toolchain directory is unavailable");
    };
    if !source.exists() {
        anyhow::bail!("bundled feature declared but binary is missing");
    }
    install_bundled_tool(&source, destination)?;
    Ok(source.display().to_string())
}

fn has_public_recipe(tool: &str, target_triple: &str) -> bool {
    match tool {
        "gh" => gh_asset_suffix_for_target(target_triple).is_some(),
        "git" => {
            target_triple == "x86_64-pc-windows-msvc" || target_triple == "aarch64-pc-windows-msvc"
        }
        _ => false,
    }
}

async fn install_from_public(
    tool: &str,
    target_triple: &str,
    binary_ext: &str,
    destination: &std::path::Path,
    cfg: &PublicBootstrapConfig,
    http_client: &reqwest::Client,
) -> anyhow::Result<String> {
    match tool {
        "gh" => {
            install_gh_from_public(target_triple, binary_ext, destination, cfg, http_client).await
        }
        "git" => install_git_from_public(target_triple, destination, cfg, http_client).await,
        _ => anyhow::bail!("unsupported tool for public bootstrap: {tool}"),
    }
}

async fn install_gh_from_public(
    target_triple: &str,
    binary_ext: &str,
    destination: &std::path::Path,
    cfg: &PublicBootstrapConfig,
    http_client: &reqwest::Client,
) -> anyhow::Result<String> {
    let suffix = gh_asset_suffix_for_target(target_triple).ok_or_else(|| {
        anyhow::anyhow!("gh public recipe unsupported on target `{target_triple}`")
    })?;
    let release = fetch_latest_github_release(http_client, cfg, "cli/cli").await?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(suffix))
        .ok_or_else(|| anyhow::anyhow!("cannot find gh release asset suffix `{suffix}`"))?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref())
        .ok_or_else(|| anyhow::anyhow!("missing sha256 digest in gh release metadata"))?;
    let (bytes, source_url) = download_with_candidates(
        http_client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
    )
    .await?;
    verify_sha256(&bytes, &expected_sha)?;
    let binary_name = format!("gh{binary_ext}");
    install_binary_from_archive(&asset.name, &bytes, &binary_name, "gh", destination)?;
    Ok(source_url)
}

async fn install_git_from_public(
    target_triple: &str,
    destination: &std::path::Path,
    cfg: &PublicBootstrapConfig,
    http_client: &reqwest::Client,
) -> anyhow::Result<String> {
    if target_triple != "x86_64-pc-windows-msvc" && target_triple != "aarch64-pc-windows-msvc" {
        anyhow::bail!("git public recipe unsupported on target `{target_triple}`");
    }
    let release = fetch_latest_github_release(http_client, cfg, "git-for-windows/git").await?;
    let asset = select_mingit_asset_for_target(&release.assets, target_triple)
        .ok_or_else(|| anyhow::anyhow!("cannot find MinGit asset for target `{target_triple}`"))?;
    let expected_sha = parse_sha256_digest(asset.digest.as_deref()).ok_or_else(|| {
        anyhow::anyhow!("missing sha256 digest in git-for-windows release metadata")
    })?;
    let (bytes, source_url) = download_with_candidates(
        http_client,
        &asset.browser_download_url,
        &cfg.mirror_prefixes,
    )
    .await?;
    verify_sha256(&bytes, &expected_sha)?;
    install_binary_from_archive(&asset.name, &bytes, "git.exe", "git", destination)?;
    Ok(source_url)
}

fn select_mingit_asset_for_target<'a>(
    assets: &'a [GithubReleaseAsset],
    target_triple: &str,
) -> Option<&'a GithubReleaseAsset> {
    match target_triple {
        "x86_64-pc-windows-msvc" => assets
            .iter()
            .find(|asset| {
                asset.name.starts_with("MinGit-") && asset.name.ends_with("-busybox-64-bit.zip")
            })
            .or_else(|| {
                assets.iter().find(|asset| {
                    asset.name.starts_with("MinGit-")
                        && asset.name.ends_with("-64-bit.zip")
                        && !asset.name.contains("busybox")
                })
            }),
        "aarch64-pc-windows-msvc" => assets
            .iter()
            .find(|asset| asset.name.starts_with("MinGit-") && asset.name.ends_with("-arm64.zip")),
        _ => None,
    }
}

fn gh_asset_suffix_for_target(target_triple: &str) -> Option<&'static str> {
    match target_triple {
        "x86_64-unknown-linux-gnu" => Some("_linux_amd64.tar.gz"),
        "aarch64-unknown-linux-gnu" => Some("_linux_arm64.tar.gz"),
        "x86_64-apple-darwin" => Some("_macOS_amd64.zip"),
        "aarch64-apple-darwin" => Some("_macOS_arm64.zip"),
        "x86_64-pc-windows-msvc" => Some("_windows_amd64.zip"),
        "aarch64-pc-windows-msvc" => Some("_windows_arm64.zip"),
        _ => None,
    }
}

async fn fetch_latest_github_release(
    http_client: &reqwest::Client,
    cfg: &PublicBootstrapConfig,
    repo: &str,
) -> anyhow::Result<GithubRelease> {
    let mut errors = Vec::new();
    for base in &cfg.github_api_bases {
        let base = base.trim().trim_end_matches('/');
        if base.is_empty() {
            continue;
        }
        let url = format!("{base}/repos/{repo}/releases/latest");
        match http_client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    errors.push(format!("{url} -> HTTP {}", resp.status()));
                    continue;
                }
                match resp.json::<GithubRelease>().await {
                    Ok(release) => return Ok(release),
                    Err(err) => {
                        errors.push(format!("{url} -> invalid json: {err}"));
                    }
                }
            }
            Err(err) => {
                errors.push(format!("{url} -> {err}"));
            }
        }
    }
    anyhow::bail!(
        "failed to fetch latest release metadata for {repo}: {}",
        errors.join(" | ")
    );
}

async fn download_with_candidates(
    http_client: &reqwest::Client,
    canonical_url: &str,
    mirror_prefixes: &[String],
) -> anyhow::Result<(Vec<u8>, String)> {
    let mut errors = Vec::new();
    for candidate in make_download_candidates(canonical_url, mirror_prefixes) {
        match http_client.get(&candidate).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    errors.push(format!("{candidate} -> HTTP {}", resp.status()));
                    continue;
                }
                match resp.bytes().await {
                    Ok(bytes) => return Ok((bytes.to_vec(), candidate)),
                    Err(err) => errors.push(format!("{candidate} -> read body failed: {err}")),
                }
            }
            Err(err) => errors.push(format!("{candidate} -> {err}")),
        }
    }
    anyhow::bail!(
        "all download candidates failed for {canonical_url}: {}",
        errors.join(" | ")
    );
}

fn make_download_candidates(canonical_url: &str, mirror_prefixes: &[String]) -> Vec<String> {
    let mut out = vec![canonical_url.to_string()];
    for raw_prefix in mirror_prefixes {
        let prefix = raw_prefix.trim();
        if prefix.is_empty() {
            continue;
        }
        let candidate = if prefix.contains("{url}") {
            prefix.replace("{url}", canonical_url)
        } else {
            format!("{prefix}{canonical_url}")
        };
        if !out.iter().any(|value| value == &candidate) {
            out.push(candidate);
        }
    }
    out
}

fn parse_sha256_digest(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    let value = raw.strip_prefix("sha256:")?.trim().to_ascii_lowercase();
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(value)
}

fn verify_sha256(content: &[u8], expected_hex: &str) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(content);
    let digest = hasher.finalize();
    let actual = digest
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect::<String>();
    if actual != expected_hex {
        anyhow::bail!("checksum mismatch: expected {expected_hex}, got {actual}");
    }
    Ok(())
}

fn install_binary_from_archive(
    asset_name: &str,
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    if asset_name.ends_with(".tar.gz") {
        install_from_tar_gz(content, binary_name, tool, destination)
    } else if asset_name.ends_with(".zip") {
        install_from_zip(content, binary_name, tool, destination)
    } else {
        anyhow::bail!("unsupported archive type for `{asset_name}`");
    }
}

fn install_from_tar_gz(
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(content));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        let path = entry
            .path()
            .context("read tar entry path")?
            .to_string_lossy()
            .replace('\\', "/");
        if is_binary_entry_match(&path, binary_name, tool) {
            write_binary_from_reader(&mut entry, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary `{binary_name}` not found in tar archive");
}

fn install_from_zip(
    content: &[u8],
    binary_name: &str,
    tool: &str,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(content))
        .context("open zip archive for tool bootstrap")?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .with_context(|| format!("open zip entry #{index}"))?;
        if entry.is_dir() {
            continue;
        }
        let path = entry.name().replace('\\', "/");
        if is_binary_entry_match(&path, binary_name, tool) {
            write_binary_from_reader(&mut entry, destination)?;
            return Ok(());
        }
    }
    anyhow::bail!("binary `{binary_name}` not found in zip archive");
}

fn is_binary_entry_match(path: &str, binary_name: &str, tool: &str) -> bool {
    if path.ends_with(&format!("/bin/{binary_name}")) {
        return true;
    }
    if tool == "git" && binary_name.eq_ignore_ascii_case("git.exe") {
        return path.ends_with("/cmd/git.exe")
            || path.ends_with("/mingw64/bin/git.exe")
            || path.ends_with("/usr/bin/git.exe")
            || path.ends_with("/bin/git.exe");
    }
    false
}

fn write_binary_from_reader(
    reader: &mut dyn Read,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = std::fs::File::create(destination)
        .with_context(|| format!("create {}", destination.display()))?;
    std::io::copy(reader, &mut file).with_context(|| format!("write {}", destination.display()))?;
    file.flush()
        .with_context(|| format!("flush {}", destination.display()))?;
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
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

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

    #[test]
    fn parse_sha256_digest_accepts_valid_value() {
        let digest = parse_sha256_digest(Some(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ));
        assert_eq!(
            digest.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }

    #[test]
    fn make_download_candidates_supports_placeholder_prefix() {
        let out = make_download_candidates(
            "https://github.com/org/repo/releases/download/v1/x.tar.gz",
            &[
                "https://mirror.example/{url}".to_string(),
                "https://proxy.example/".to_string(),
            ],
        );
        assert_eq!(
            out,
            vec![
                "https://github.com/org/repo/releases/download/v1/x.tar.gz".to_string(),
                "https://mirror.example/https://github.com/org/repo/releases/download/v1/x.tar.gz"
                    .to_string(),
                "https://proxy.example/https://github.com/org/repo/releases/download/v1/x.tar.gz"
                    .to_string()
            ]
        );
    }

    #[test]
    fn select_mingit_prefers_busybox_on_x64() {
        let assets = vec![
            GithubReleaseAsset {
                name: "MinGit-2.53.0-64-bit.zip".to_string(),
                browser_download_url: "https://example.invalid/a.zip".to_string(),
                digest: Some(
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        .to_string(),
                ),
            },
            GithubReleaseAsset {
                name: "MinGit-2.53.0-busybox-64-bit.zip".to_string(),
                browser_download_url: "https://example.invalid/b.zip".to_string(),
                digest: Some(
                    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                        .to_string(),
                ),
            },
        ];
        let selected = select_mingit_asset_for_target(&assets, "x86_64-pc-windows-msvc")
            .expect("selected asset");
        assert_eq!(selected.name, "MinGit-2.53.0-busybox-64-bit.zip");
    }

    #[tokio::test]
    async fn public_gh_install_from_mock_release_api() -> anyhow::Result<()> {
        let archive_name = "gh_9.9.9_linux_amd64.tar.gz";
        let archive_bytes = make_tar_gz_archive(&[(
            "gh_9.9.9_linux_amd64/bin/gh",
            b"#!/bin/sh\necho mock-gh\n".as_slice(),
            0o755,
        )])?;
        let digest = sha256_hex(&archive_bytes);

        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let base = format!("http://{addr}");
        let release_body = serde_json::json!({
            "assets": [{
                "name": archive_name,
                "browser_download_url": format!("{base}/asset/{archive_name}"),
                "digest": format!("sha256:{digest}")
            }]
        })
        .to_string()
        .into_bytes();

        let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
        routes.insert(
            "/api/repos/cli/cli/releases/latest".to_string(),
            release_body,
        );
        routes.insert(format!("/asset/{archive_name}"), archive_bytes);
        let handle = spawn_mock_http_server(listener, routes, 2);

        let cfg = PublicBootstrapConfig {
            github_api_bases: vec![format!("{base}/api")],
            mirror_prefixes: Vec::new(),
            http_timeout: Duration::from_secs(5),
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        let tmp = tempfile::tempdir()?;
        let destination = tmp.path().join("gh");

        let source =
            install_gh_from_public("x86_64-unknown-linux-gnu", "", &destination, &cfg, &client)
                .await?;
        assert_eq!(source, format!("{base}/asset/{archive_name}"));
        let installed = std::fs::read_to_string(&destination)?;
        assert!(installed.contains("mock-gh"));

        handle.join().expect("mock server thread join");
        Ok(())
    }

    fn sha256_hex(content: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content);
        let digest = hasher.finalize();
        digest
            .iter()
            .map(|value| format!("{value:02x}"))
            .collect::<String>()
    }

    fn make_tar_gz_archive(entries: &[(&str, &[u8], u32)]) -> anyhow::Result<Vec<u8>> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (path, body, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, *path, &mut Cursor::new(*body))
                .with_context(|| format!("append tar entry {path}"))?;
        }
        let encoder = builder.into_inner().context("finalize tar builder")?;
        let archive = encoder.finish().context("finalize gzip stream")?;
        Ok(archive)
    }

    fn spawn_mock_http_server(
        listener: TcpListener,
        routes: HashMap<String, Vec<u8>>,
        expected_requests: usize,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            for _ in 0..expected_requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buffer = [0_u8; 8192];
                let Ok(size) = stream.read(&mut buffer) else {
                    continue;
                };
                if size == 0 {
                    continue;
                }
                let request = String::from_utf8_lossy(&buffer[..size]);
                let request_line = request.lines().next().unwrap_or_default();
                let path = request_line.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = if let Some(body) = routes.get(path) {
                    ("200 OK", body.clone())
                } else {
                    ("404 Not Found", b"not found".to_vec())
                };
                let headers = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        })
    }
}
