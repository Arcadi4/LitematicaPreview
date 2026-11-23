//! Self-update support for the standalone Windows and macOS builds.
//!
//! The transport and the native executable replacement belong to `self_update`.
//! This module owns the release policy: which host is trusted, how the release
//! metadata is read, how a release is compared with the running build, and when
//! an update is allowed at all.
//!
//! Every function here is blocking. The command layer should call them from a
//! worker thread and publish [`State`] transitions to the UI.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use self_update::{Checksum, Download};
use semver::{BuildMetadata, Version};
use serde::{Deserialize, Serialize};

mod platform;
#[cfg(windows)]
mod windows;

const GITHUB_OWNER: &str = "Arcadi4";
const GITHUB_REPO: &str = "LitematicaPreview";
const GITHUB_API: &str = "https://api.github.com";
const CLOUDFLARE_HOST: &str = "download.tanh.moe";
const CLOUDFLARE_LATEST: &str = "https://download.tanh.moe/litematica-preview/rune/latest.json";
const CLOUDFLARE_RELEASES: &str = "https://download.tanh.moe/litematica-preview/releases";

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const SUMS_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Temporary name for the extracted executable. It must differ from the
/// downloaded asset name: `self_update` extracts a plain asset into the same
/// temporary directory that holds the download, so reusing the asset name would
/// truncate the download before copying it.
#[cfg(windows)]
const STAGING_BINARY: &str = "LitematicaPreview-update.exe";
#[cfg(not(any(windows, target_os = "macos")))]
const STAGING_BINARY: &str = "LitematicaPreview-update";

/// Where a release is read from.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Github,
    Cloudflare,
}

/// How this copy is allowed to update itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// The signed standalone executable replaces itself in place.
    SelfUpdate,
    /// The AppImage launches the bundled AppImageUpdate GUI manually.
    AppImage,
    /// apt, an unpacked Linux tree, or a build that must not self-update.
    Disabled,
}

impl Mode {
    /// How this copy is allowed to update itself.
    ///
    /// The `managed-install` compile-time feature (apt) wins over everything:
    /// it never performs a network check and never launches AppImageUpdate. A
    /// Linux copy without that feature is an AppImage only when the AppImage
    /// runtime exported `APPIMAGE`; otherwise it is an unpacked copy and in-app
    /// updates stay disabled. Windows and macOS builds, including Homebrew
    /// installs, use the native self-update path.
    pub fn current() -> Self {
        #[cfg(feature = "managed-install")]
        {
            Self::Disabled
        }
        #[cfg(not(feature = "managed-install"))]
        {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                Self::SelfUpdate
            }
            #[cfg(target_os = "linux")]
            {
                if is_appimage_runtime() {
                    Self::AppImage
                } else {
                    Self::Disabled
                }
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
            {
                Self::Disabled
            }
        }
    }
}

/// Whether this process is really running from a mounted AppImage.
///
/// The environment alone is not enough: both runtime variables must be
/// non-empty, the `APPIMAGE` file and `APPDIR` directory must exist, and the
/// running executable must be rooted inside `APPDIR`. That keeps a stray
/// `APPIMAGE` variable from turning an apt or unpacked copy into an AppImage.
#[cfg(all(target_os = "linux", not(feature = "managed-install")))]
fn is_appimage_runtime() -> bool {
    let Some(appimage) = std::env::var_os("APPIMAGE").filter(|value| !value.is_empty()) else {
        return false;
    };
    let Some(appdir) = std::env::var_os("APPDIR").filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(appimage) = std::fs::canonicalize(std::path::PathBuf::from(appimage)) else {
        return false;
    };
    let Ok(appdir) = std::fs::canonicalize(std::path::PathBuf::from(appdir)) else {
        return false;
    };
    let Ok(executable) = std::env::current_exe() else {
        return false;
    };
    let Ok(executable) = std::fs::canonicalize(executable) else {
        return false;
    };
    appimage.is_file() && appdir.is_dir() && executable.starts_with(&appdir)
}

/// The state the UI can render while a check or install is in progress.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "value", rename_all = "camelCase")]
pub enum State {
    Idle,
    Checking,
    Current,
    Available(Release),
    Installing,
    Installed,
    Failed(String),
}

/// A release candidate returned by [`check`] and passed back to [`install`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// The bare semantic version, for example `0.8.0`.
    pub version: String,
    /// Markdown release notes, when the source publishes them.
    #[serde(default)]
    pub notes: String,
    /// Published asset size in bytes, when known. Cloudflare's manifest does
    /// not publish a size, so this is zero there.
    #[serde(default)]
    pub size: u64,
    /// The release's creation date, when the source publishes one.
    #[serde(default)]
    pub date: String,
    /// Unix timestamp of the check that produced this candidate.
    #[serde(default)]
    pub checked_at: u64,
    #[serde(default)]
    pub(crate) sha256: String,
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) asset: String,
    #[serde(default)]
    pub(crate) source: Source,
}

/// The version of the running build.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Looks for a newer release from `source`.
///
/// Returns `Ok(None)` when the copy is not allowed to self-update, when the
/// source has no release, or when its newest stable release is not strictly
/// newer than the running build. An older pre-Rune release is therefore reported
/// as "no update" rather than as an error or a migration prompt.
///
/// This performs network I/O on Windows and macOS only. On an AppImage, an apt
/// package, and an unpacked Linux copy it returns immediately.
pub fn check(source: Source) -> Result<Option<Release>, String> {
    if Mode::current() != Mode::SelfUpdate {
        return Ok(None);
    }
    match source {
        Source::Github => check_github(),
        Source::Cloudflare => check_cloudflare(),
    }
}

/// Downloads and installs `release`.
///
/// Windows verifies SHA-256 and the running executable's Authenticode publisher
/// before replacement. macOS verifies the DMG's SHA-256 and the bundle's
/// notarization and signing identity, then swaps the bundle atomically. This performs
/// network I/O and must be called from a worker thread.
pub fn install(release: &Release) -> Result<(), String> {
    match Mode::current() {
        Mode::SelfUpdate => {
            #[cfg(not(target_os = "macos"))]
            let version = validate_release(release)?;
            #[cfg(target_os = "macos")]
            {
                validate_release(release)?;
            }

            #[cfg(target_os = "macos")]
            {
                platform::install_macos(release)
            }
            #[cfg(not(target_os = "macos"))]
            {
                install_self_update(release, version.as_str())
            }
        }
        Mode::AppImage => Err(
            "This AppImage updates through AppImageUpdate; use the AppImage update action."
                .to_string(),
        ),
        Mode::Disabled => Err(
            "In-app updates are disabled for this installation. If it was installed with apt, \
             update it with apt."
                .to_string(),
        ),
    }
}

/// Starts the AppImageUpdate GUI bundled inside this AppImage.
///
/// This is a manual action: it does not check for updates, store state, schedule
/// anything, or perform network I/O itself. The bundled GUI is launched with the
/// running AppImage's path. It is unavailable on apt, standalone Linux, and every
/// non-Linux build.
pub fn appimage_update() -> Result<(), String> {
    match Mode::current() {
        Mode::AppImage => platform::appimage_update(),
        Mode::SelfUpdate => {
            Err("This installation updates itself; AppImageUpdate is not used.".to_string())
        }
        Mode::Disabled => Err(
            "AppImageUpdate is only used by AppImage builds. Update this installation with the \
             package manager or installer that provided it."
                .to_string(),
        ),
    }
}

fn check_github() -> Result<Option<Release>, String> {
    let updater = self_update::backends::github::Update::configure()
        .repo_owner(GITHUB_OWNER)
        .repo_name(GITHUB_REPO)
        .api_base_url(GITHUB_API)
        .bin_name("LitematicaPreview")
        .current_version(current_version())
        .timeout(CHECK_TIMEOUT)
        .retries(1)
        .no_confirm(true)
        .show_output(false)
        .build()
        .map_err(|error| format!("Could not prepare the GitHub update check: {error}"))?;

    let releases = match updater.get_latest_release() {
        Ok(releases) => releases,
        Err(self_update::Error::NoReleaseFound { .. }) => return Ok(None),
        Err(error) => return Err(format!("Could not check GitHub for updates: {error}")),
    };
    let Some(latest) = releases.latest() else {
        return Ok(None);
    };

    let version = normalize_version(latest.version())?;
    if !is_stable(&version) || !is_newer(&version, current_version())? {
        return Ok(None);
    }
    let asset_name = platform::asset_name(&version)?;

    let asset = latest
        .assets()
        .iter()
        .find(|asset| asset.name() == asset_name.as_str())
        .ok_or_else(|| {
            format!(
                "Litematica Preview {version} does not publish the {asset_name} update asset for \
                 this platform."
            )
        })?;
    validate_github_asset_url(asset.download_url())?;
    let sha256 = resolve_github_sha256(latest, asset.name())?;

    Ok(Some(Release {
        version,
        notes: latest.body().unwrap_or_default().to_string(),
        size: 0,
        date: latest.date().to_string(),
        checked_at: now_unix(),
        sha256,
        url: asset.download_url().to_string(),
        asset: asset.name().to_string(),
        source: Source::Github,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudflareManifest {
    version: String,
    assets: BTreeMap<String, String>,
}

fn check_cloudflare() -> Result<Option<Release>, String> {
    let bytes = download_bytes(CLOUDFLARE_LATEST, "application/json", CHECK_TIMEOUT)?;
    let manifest: CloudflareManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The Cloudflare update manifest is invalid: {error}"))?;
    let version = normalize_version(&manifest.version)?;
    if !is_stable(&version) || !is_newer(&version, current_version())? {
        return Ok(None);
    }
    let asset_name = platform::asset_name(&version)?;

    let sha256 = manifest.assets.get(asset_name.as_str()).ok_or_else(|| {
        format!("The Cloudflare update manifest for {version} does not list {asset_name}.")
    })?;
    if !is_sha256_hex(sha256) {
        return Err(format!(
            "The Cloudflare update manifest for {version} has an invalid SHA-256 for {asset_name}."
        ));
    }

    let url = cloudflare_asset_url(version.as_str(), asset_name.as_str())?;
    Ok(Some(Release {
        version,
        notes: String::new(),
        size: 0,
        date: String::new(),
        checked_at: now_unix(),
        sha256: sha256.to_ascii_lowercase(),
        url,
        asset: asset_name,
        source: Source::Cloudflare,
    }))
}

fn resolve_github_sha256(
    release: &self_update::Release,
    asset_name: &str,
) -> Result<String, String> {
    if let Some(digest) = release
        .assets()
        .iter()
        .find(|asset| asset.name() == asset_name)
        .and_then(|asset| asset.digest())
        && let Ok(Checksum::Sha256(hex)) = Checksum::parse_digest(digest)
        && is_sha256_hex(&hex)
    {
        return Ok(hex.to_ascii_lowercase());
    }

    let sums = release
        .assets()
        .iter()
        .find(|asset| matches!(asset.name(), "SHA256SUMS" | "SHA256SUMS.txt"))
        .ok_or_else(|| {
            format!(
                "Litematica Preview {} does not publish a SHA-256 digest or SHA256SUMS asset for \
                 {asset_name}.",
                release.version()
            )
        })?;
    validate_github_asset_url(sums.download_url())?;
    let bytes = download_bytes(
        sums.download_url(),
        "application/octet-stream",
        SUMS_TIMEOUT,
    )?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        format!(
            "The SHA256SUMS asset in Litematica Preview {} is not valid UTF-8.",
            release.version()
        )
    })?;
    let checksum = Checksum::from_sums_file(text, asset_name).map_err(|error| {
        format!("Could not read the SHA-256 for {asset_name} from SHA256SUMS: {error}")
    })?;
    match checksum {
        Checksum::Sha256(hex) if is_sha256_hex(&hex) => Ok(hex.to_ascii_lowercase()),
        _ => Err(format!(
            "The SHA256SUMS asset in Litematica Preview {} does not contain a SHA-256 digest for \
             {asset_name}.",
            release.version()
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn install_self_update(release: &Release, version: &str) -> Result<(), String> {
    let install_path = platform::install_path()?;
    let source_release = build_source_release(release, version)?;
    let asset_name = release.asset.clone();

    let mut builder = self_update::backends::custom::Update::configure();
    builder
        .source(StaticRelease {
            release: source_release,
        })
        .bin_name(STAGING_BINARY)
        .bin_path_in_archive(STAGING_BINARY)
        .bin_install_path(&install_path)
        .current_version(current_version())
        .release_tag(version)
        .asset_matcher(move |assets| {
            assets
                .iter()
                .find(|asset| asset.name() == asset_name.as_str())
                .cloned()
        })
        .verify_checksum(Checksum::Sha256(release.sha256.clone()))
        .no_confirm(true)
        .show_output(false)
        .timeout(DOWNLOAD_TIMEOUT)
        .retries(2);
    #[cfg(windows)]
    builder.verify_binary(move |candidate| {
        windows::verify_publisher(&install_path, candidate)
            .map_err(self_update::errors::Error::verification_rejected)
    });

    let updater = builder
        .build()
        .map_err(|error| format!("Could not prepare the update: {error}"))?;
    let status = updater
        .update()
        .map_err(|error| format!("The update could not be installed: {error}"))?;
    if !status.is_updated() {
        return Err(format!(
            "The updater did not install Litematica Preview {version}."
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
struct StaticRelease {
    release: self_update::Release,
}

#[cfg(not(target_os = "macos"))]
impl self_update::ReleaseSource for StaticRelease {
    fn get_releases(&self) -> self_update::Result<Vec<self_update::Release>> {
        Ok(vec![self.release.clone()])
    }
}

#[cfg(not(target_os = "macos"))]
fn build_source_release(release: &Release, version: &str) -> Result<self_update::Release, String> {
    let asset = self_update::ReleaseAsset::new(release.asset.clone(), release.url.clone())
        .with_digest(format!("sha256:{}", release.sha256));
    self_update::Release::builder()
        .version(version)
        .name(version)
        .body(release.notes.clone())
        .date(release.date.clone())
        .asset(asset)
        .build()
        .map_err(|error| format!("The update metadata is invalid: {error}"))
}

fn validate_release(release: &Release) -> Result<String, String> {
    let version = normalize_version(&release.version)?;
    if !is_stable(version.as_str()) {
        return Err(format!(
            "Litematica Preview {version} is not a stable release."
        ));
    }
    if !is_newer(version.as_str(), current_version())? {
        return Err(format!(
            "Litematica Preview {version} is not newer than the installed {}.",
            current_version()
        ));
    }

    let asset_name = platform::asset_name(version.as_str())?;
    if release.asset != asset_name {
        return Err(format!(
            "The update is for {}, but this build expects {asset_name}.",
            release.asset
        ));
    }
    if !is_sha256_hex(&release.sha256) {
        return Err("The update does not have a valid SHA-256 digest.".to_string());
    }

    match release.source {
        Source::Github => validate_github_asset_url(&release.url),
        Source::Cloudflare => {
            let expected = cloudflare_asset_url(version.as_str(), asset_name.as_str())?;
            if release.url != expected {
                return Err(
                    "The update URL does not match the official Cloudflare release path."
                        .to_string(),
                );
            }
            Ok(())
        }
    }?;

    Ok(version)
}

fn download_bytes(url: &str, accept: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    Download::from_url(url)
        .request_header("Accept", accept)
        .request_header("Cache-Control", "no-cache")
        .request_header(
            "User-Agent",
            format!("LitematicaPreview/{}", current_version()),
        )
        .timeout(timeout)
        .download_to(&mut bytes)
        .map_err(|error| format!("Could not download {url}: {error}"))?;
    Ok(bytes)
}

fn validate_github_asset_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url)
        .map_err(|error| format!("The GitHub update URL is invalid: {error}"))?;
    if parsed.scheme() != "https" {
        return Err("The GitHub update URL must use HTTPS.".to_string());
    }
    let host = parsed.host_str().unwrap_or_default();
    let path = parsed.path();
    let api_prefix = format!("/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/assets/");
    let web_prefix = format!("/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/");
    let official = (host == "api.github.com" && path.starts_with(api_prefix.as_str()))
        || (host == "github.com" && path.starts_with(web_prefix.as_str()));
    if !official {
        return Err(format!(
            "The update URL is not on the official GitHub host for {GITHUB_OWNER}/{GITHUB_REPO}."
        ));
    }
    Ok(())
}

fn cloudflare_asset_url(version: &str, asset: &str) -> Result<String, String> {
    let url = format!("{CLOUDFLARE_RELEASES}/v{version}/{asset}");
    let parsed = url::Url::parse(&url)
        .map_err(|error| format!("The Cloudflare update URL is invalid: {error}"))?;
    let expected_path = format!("/litematica-preview/releases/v{version}/{asset}");
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(CLOUDFLARE_HOST)
        || parsed.path() != expected_path.as_str()
    {
        return Err("The Cloudflare update URL is not on the official release path.".to_string());
    }
    Ok(url)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn parse_version(value: &str) -> Result<Version, String> {
    let value = value.trim();
    Version::parse(value.strip_prefix('v').unwrap_or(value))
        .map_err(|error| format!("Invalid release version {value}: {error}"))
}

fn normalize_version(value: &str) -> Result<String, String> {
    let mut version = parse_version(value)?;
    version.build = BuildMetadata::EMPTY;
    Ok(version.to_string())
}

fn is_stable(value: &str) -> bool {
    parse_version(value).is_ok_and(|version| version.pre.is_empty())
}

fn is_newer(candidate: &str, current: &str) -> Result<bool, String> {
    Ok(parse_version(candidate)?
        .cmp_precedence(&parse_version(current)?)
        .is_gt())
}
