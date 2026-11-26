// Shared apt repository contract.
//
// The Linux package build embeds the rendered source list and the public
// keyring into the `.deb`; the publisher verifies the exact same bytes before
// it signs anything. Keep this module free of process orchestration so both
// sides can depend on it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Absolute path of the source list inside the installed package.
pub const APT_SOURCE_PATH: &str = "/etc/apt/sources.list.d/litematica-preview.list";
/// Absolute path of the repository keyring inside the installed package.
pub const APT_KEYRING_PATH: &str = "/usr/share/keyrings/litematica-preview-archive-keyring.gpg";
/// Suite/codename of the single published distribution.
pub const APT_DISTRIBUTION: &str = "stable";
/// Component that carries the package.
pub const APT_COMPONENT: &str = "main";
/// Debian architecture served by the repository.
pub const APT_ARCHITECTURE: &str = "amd64";
/// Debian package name served by the repository.
pub const APT_PACKAGE: &str = "litematica-preview";

/// Validates the public base URL of the repository and returns it without a
/// trailing slash.
///
/// The URL is the bucket root that serves `dists/` and `pool/`. It must be
/// plain HTTPS without credentials, query, fragment, or a path below the root.
pub fn normalize_apt_base_url(value: &str) -> Result<String> {
    let value = value.trim();
    let rest = match value.strip_prefix("https://") {
        Some(rest) => rest,
        None => bail!("The apt repository base URL must use https: {value}"),
    };
    if rest.is_empty() || value.chars().any(char::is_whitespace) {
        bail!("The apt repository base URL is not a URL: {value}");
    }
    if value.contains('?') || value.contains('#') {
        bail!("The apt repository base URL must not contain a query or fragment.");
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        bail!("The apt repository base URL is not a URL: {value}");
    }
    if authority.contains('@') {
        bail!("The apt repository base URL must not contain credentials.");
    }
    if !path.is_empty() && path != "/" {
        bail!("The apt repository base URL must point to the bucket root.");
    }
    Ok(format!("https://{authority}"))
}

/// Renders the source list embedded in the Debian package. The publisher
/// compares this exact string with the package contents.
pub fn render_apt_source(base_url: &str) -> Result<String> {
    let base_url = normalize_apt_base_url(base_url)?;
    Ok(format!(
        "deb [arch={APT_ARCHITECTURE} signed-by={APT_KEYRING_PATH}] {base_url} {APT_DISTRIBUTION} {APT_COMPONENT}\n"
    ))
}

/// Maps an absolute install path to its location inside an extracted package
/// root, for example `/etc/apt/...` to `<root>/etc/apt/...`.
pub fn embedded_path(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}

/// The signing key fingerprint comparison used by the publisher is
/// case-insensitive; fingerprints are hex and GnuPG prints them upper-case.
pub fn same_fingerprint(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// Fails unless `path` names a non-empty regular file.
pub fn require_file(path: &Path, describe: &str) -> Result<()> {
    let stats = std::fs::metadata(path)
        .with_context(|| format!("{describe} is missing: {}", path.display()))?;
    if !stats.is_file() {
        bail!("{describe} is not a file: {}", path.display());
    }
    if stats.len() == 0 {
        bail!("{describe} is empty: {}", path.display());
    }
    Ok(())
}
