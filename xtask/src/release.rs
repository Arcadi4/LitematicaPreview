use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{Read, Write as _},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::{Context, Result, bail};
use semver::Version;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

mod notes;

const BUCKET: &str = "apps-download";
const RELEASES_PREFIX: &str = "litematica-preview/releases";
const FEED_KEY: &str = "litematica-preview/rune/latest.json";
const CHECKSUM_FILE: &str = "SHA256SUMS.txt";

const WINDOWS_BINARY: &str = "LitematicaPreview-windows-x86_64.exe";

pub fn check(args: &[OsString]) -> Result<()> {
    let tag = tag_argument(args, "release-check")?;
    let version = crate::version()?;
    let tag_version = release_version(&tag, &version)?;
    let prerelease = !tag_version.pre.is_empty();
    let exists = release_lookup(&tag)?.is_some();

    write_github_output(&[
        ("exists", exists.to_string()),
        ("prerelease", prerelease.to_string()),
    ])?;
    println!("{tag}: exists={exists} prerelease={prerelease}");
    Ok(())
}

pub fn checksums(args: &[OsString]) -> Result<()> {
    let mut windows = false;
    let mut macos = false;
    let mut linux = false;
    for argument in args {
        match argument.to_string_lossy().as_ref() {
            "--win" | "--windows" => windows = true,
            "--mac" | "--macos" => macos = true,
            "--linux" => linux = true,
            other => {
                bail!("checksums does not accept `{other}`; expected --win, --mac, or --linux")
            }
        }
    }
    let filtered = windows || macos || linux;
    let version = crate::version()?;
    let artifacts = crate::root().join("artifacts");
    fs::create_dir_all(&artifacts)
        .with_context(|| format!("could not create {}", artifacts.display()))?;

    let names: Vec<String> = expected_assets(&version)
        .into_iter()
        .filter(|name| {
            !filtered
                || (windows && is_windows(name))
                || (macos && is_macos(name))
                || (linux && is_linux(name))
        })
        .collect();
    let hashes = hash_artifacts(&artifacts, &names)?;
    let sums = write_checksums(&artifacts, &hashes)?;

    for (name, hash) in &hashes {
        println!("{hash}  {name}");
    }
    println!("wrote {}", sums.display());
    Ok(())
}

pub fn publish(args: &[OsString]) -> Result<()> {
    let tag = tag_argument(args, "release-publish")?;
    let version = crate::version()?;
    release_version(&tag, &version)?;
    let prerelease = !version.pre.is_empty();

    let artifacts = crate::root().join("artifacts");
    fs::create_dir_all(&artifacts)
        .with_context(|| format!("could not create {}", artifacts.display()))?;
    let names = expected_assets(&version);

    let release = release_lookup(&tag)?;
    download_missing(&artifacts, &names, &tag, release.as_ref())?;
    let hashes = hash_artifacts(&artifacts, &names)?;
    let sums = write_checksums(&artifacts, &hashes)?;

    match release {
        Some(release) => reconcile_release(
            release, &tag, &artifacts, &names, &sums, &hashes, prerelease,
        )?,
        None => {
            let release_notes = notes::compose(&tag, &version)?;
            create_release(&tag, &artifacts, &names, &sums, &release_notes, prerelease)?;
            let release = release_lookup(&tag)?
                .with_context(|| format!("could not verify the newly published release {tag}"))?;
            verify_release(&release, &names, &hashes, &artifacts, prerelease)?;
        }
    }

    upload_release_objects(&tag, &artifacts, &names, &sums)?;

    if prerelease {
        println!("prerelease: stable update feed left unchanged");
    } else {
        publish_feed(&version, &artifacts)?;
    }
    Ok(())
}

pub fn publish_apt(args: &[OsString]) -> Result<()> {
    let tag = tag_argument(args, "release-apt")?;
    let version = release_version(&tag, &crate::version()?)?;
    if !version.pre.is_empty() {
        bail!("prereleases are not published to the stable apt repository");
    }
    if let Some(latest) = upstream_latest_stable()?
        && latest.cmp_precedence(&version).is_gt()
    {
        println!("apt repository left unchanged: upstream stable v{latest} is newer than {tag}");
        return Ok(());
    }
    crate::apt::run(&[
        OsString::from("--deb"),
        OsString::from(format!("artifacts/LitematicaPreview-{version}-x86_64.deb")),
    ])
}

fn expected_assets(version: &Version) -> Vec<String> {
    let version = version.to_string();
    let mut names = vec![
        format!("LitematicaPreview-{version}-x86_64-setup.exe"),
        format!("LitematicaPreview-{version}-x86_64-portable.zip"),
        format!("LitematicaPreview-{version}-x86_64.deb"),
        format!("LitematicaPreview-{version}-x86_64.AppImage"),
        format!("LitematicaPreview-{version}-x86_64.AppImage.zsync"),
        format!("LitematicaPreview-{version}-aarch64.dmg"),
        format!("LitematicaPreview-{version}-x86_64.dmg"),
        WINDOWS_BINARY.to_owned(),
    ];
    names.sort();
    names
}

fn is_windows(name: &str) -> bool {
    name.ends_with("-setup.exe") || name.ends_with("-portable.zip") || name == WINDOWS_BINARY
}

fn is_macos(name: &str) -> bool {
    name.ends_with(".dmg")
}

fn is_linux(name: &str) -> bool {
    name.ends_with(".deb") || name.ends_with(".AppImage") || name.ends_with(".AppImage.zsync")
}

fn tag_argument(args: &[OsString], command: &str) -> Result<String> {
    let mut tag = None;
    for argument in args {
        let value = argument.to_string_lossy();
        if value.starts_with('-') {
            bail!("{command} does not accept the option `{value}`");
        }
        if tag.replace(value.to_string()).is_some() {
            bail!("{command} accepts a single release tag");
        }
    }

    let tag = tag
        .or_else(|| std::env::var("GITHUB_REF_NAME").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{command} requires a release tag such as v1.2.3"))?;
    Ok(tag)
}

fn parse_tag(tag: &str) -> Result<Version> {
    let raw = tag
        .strip_prefix('v')
        .with_context(|| format!("release tag `{tag}` must start with `v`"))?;
    if raw.contains('+') {
        bail!("release tag `{tag}` must not contain build metadata");
    }
    Version::parse(raw).with_context(|| format!("release tag `{tag}` is not valid semver"))
}

fn release_version(tag: &str, expected: &Version) -> Result<Version> {
    let parsed = parse_tag(tag)?;
    if &parsed != expected {
        bail!("release tag `{tag}` does not match the app manifest version `{expected}`");
    }
    Ok(parsed)
}

fn write_github_output(values: &[(&str, String)]) -> Result<()> {
    match std::env::var_os("GITHUB_OUTPUT") {
        Some(path) => {
            let path = PathBuf::from(path);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("could not open GITHUB_OUTPUT {}", path.display()))?;
            for (key, value) in values {
                writeln!(file, "{key}={value}")
                    .with_context(|| format!("could not write {key} to {}", path.display()))?;
            }
        }
        None => {
            for (key, value) in values {
                println!("{key}={value}");
            }
        }
    }
    Ok(())
}

struct Release {
    prerelease: bool,
    draft: bool,
    assets: Vec<ReleaseAsset>,
}

struct ReleaseAsset {
    name: String,
    size: u64,
    digest: Option<String>,
}

fn release_lookup(tag: &str) -> Result<Option<Release>> {
    let endpoint = format!("repos/{}/releases/tags/{tag}", repository()?);
    let output = run_output(
        "gh",
        &[
            OsString::from("api"),
            OsString::from("--include"),
            OsString::from(endpoint),
        ],
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    let failure = failure_detail(&output);

    match http_status(&text) {
        Some(200) => {
            let body = http_body(&text)
                .with_context(|| format!("gh api returned no response body for release {tag}"))?;
            Ok(Some(parse_release(body, tag)?))
        }
        Some(404) => Ok(None),
        Some(status) => bail!(
            "GitHub release lookup for `{tag}` failed with HTTP {status}: {}",
            failure.trim()
        ),
        None if gh_http_error(&output, 404) => Ok(None),
        None => {
            if output.status.success() {
                bail!("gh api returned no HTTP status for release `{tag}`");
            }
            bail!(
                "GitHub release lookup for `{tag}` failed before a response: {}",
                failure.trim()
            );
        }
    }
}

fn parse_release(body: &str, tag: &str) -> Result<Release> {
    let value: Value = serde_json::from_str(body)
        .with_context(|| format!("could not parse the GitHub release for `{tag}`"))?;
    let prerelease = value
        .get("prerelease")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let draft = value.get("draft").and_then(Value::as_bool).unwrap_or(false);
    let mut assets = Vec::new();
    for asset in value
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| assets.as_slice())
        .unwrap_or_default()
    {
        let name = asset
            .get("name")
            .and_then(Value::as_str)
            .with_context(|| format!("GitHub release `{tag}` returned an asset without a name"))?;
        let size = asset.get("size").and_then(Value::as_u64).unwrap_or(0);
        let digest = asset
            .get("digest")
            .and_then(Value::as_str)
            .map(str::to_owned);
        assets.push(ReleaseAsset {
            name: name.to_owned(),
            size,
            digest,
        });
    }
    Ok(Release {
        prerelease,
        draft,
        assets,
    })
}

fn verify_release(
    release: &Release,
    names: &[String],
    hashes: &BTreeMap<String, String>,
    artifacts: &Path,
    prerelease: bool,
) -> Result<()> {
    if release.draft {
        bail!("refusing to mirror a draft GitHub release");
    }
    if release.prerelease != prerelease {
        bail!(
            "existing GitHub release has prerelease={} but tag requires prerelease={prerelease}",
            release.prerelease
        );
    }

    let expected: BTreeSet<&str> = names
        .iter()
        .map(String::as_str)
        .chain([CHECKSUM_FILE])
        .collect();
    let actual: BTreeSet<&str> = release
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .collect();
    if expected != actual {
        let missing: Vec<&str> = expected.difference(&actual).copied().collect();
        let extra: Vec<&str> = actual.difference(&expected).copied().collect();
        bail!(
            "existing GitHub release asset set does not match the release contract (missing: [{}], extra: [{}])",
            missing.join(", "),
            extra.join(", ")
        );
    }

    let sums_digest = sha256_file(&artifacts.join(CHECKSUM_FILE))?;
    for asset in &release.assets {
        let digest = if asset.name == CHECKSUM_FILE {
            sums_digest.as_str()
        } else {
            hashes
                .get(&asset.name)
                .map(String::as_str)
                .with_context(|| format!("no local digest for {}", asset.name))?
        };
        let local = artifacts.join(asset.name.as_str());
        let size = fs::metadata(&local)
            .with_context(|| format!("missing local artifact {}", local.display()))?
            .len();
        if asset.size != size {
            bail!(
                "release asset `{}` is {} bytes on GitHub but {} bytes locally",
                asset.name,
                asset.size,
                size
            );
        }
        let remote = asset
            .digest
            .as_deref()
            .and_then(|value| value.strip_prefix("sha256:"))
            .with_context(|| {
                format!("GitHub did not report a SHA-256 digest for {}", asset.name)
            })?;
        if !remote.eq_ignore_ascii_case(digest) {
            bail!(
                "release asset `{}` digest {remote} does not match the local file",
                asset.name
            );
        }
    }
    Ok(())
}

fn create_release(
    tag: &str,
    artifacts: &Path,
    names: &[String],
    sums: &Path,
    release_notes: &str,
    prerelease: bool,
) -> Result<()> {
    let mut notes = tempfile::Builder::new()
        .prefix("litematica-preview-notes-")
        .suffix(".md")
        .tempfile()
        .context("could not create a temporary release notes file")?;
    notes
        .write_all(release_notes.as_bytes())
        .context("could not write the release notes")?;
    notes.flush().context("could not flush the release notes")?;

    let mut args = vec![
        OsString::from("release"),
        OsString::from("create"),
        OsString::from(tag),
        OsString::from("--verify-tag"),
        OsString::from("--title"),
        OsString::from(format!("Litematica Preview {tag}")),
        OsString::from("--notes-file"),
        notes.path().as_os_str().to_owned(),
    ];
    if prerelease {
        args.push(OsString::from("--prerelease"));
    }
    for name in names {
        args.push(artifacts.join(name.as_str()).into_os_string());
    }
    args.push(sums.as_os_str().to_owned());

    run("gh", &args)?;
    println!("created GitHub release {tag}");
    Ok(())
}

fn download_missing(
    artifacts: &Path,
    names: &[String],
    tag: &str,
    release: Option<&Release>,
) -> Result<()> {
    for name in names {
        let path = artifacts.join(name.as_str());
        if path.is_file() && fs::metadata(&path)?.len() > 0 {
            continue;
        }
        let Some(release) = release else {
            bail!(
                "release artifact is missing: {} (build it before publishing)",
                path.display()
            );
        };
        if !release
            .assets
            .iter()
            .any(|asset| asset.name.as_str() == name.as_str())
        {
            bail!(
                "GitHub release {tag} is missing {name}, and it is not available locally; rebuild it before repairing"
            );
        }
        println!("downloading {name} from GitHub release {tag}");
        let args = [
            OsString::from("release"),
            OsString::from("download"),
            OsString::from(tag),
            OsString::from("--pattern"),
            OsString::from(name.as_str()),
            OsString::from("--dir"),
            artifacts.as_os_str().to_owned(),
            OsString::from("--clobber"),
        ];
        run("gh", &args)?;
        if !path.is_file() || fs::metadata(&path)?.len() == 0 {
            bail!("GitHub release {tag} did not provide {name}");
        }
    }
    Ok(())
}

fn reconcile_release(
    release: Release,
    tag: &str,
    artifacts: &Path,
    names: &[String],
    sums: &Path,
    hashes: &BTreeMap<String, String>,
    prerelease: bool,
) -> Result<()> {
    if release.draft {
        bail!("existing GitHub release is a draft; publish it before rerunning");
    }
    if release.prerelease != prerelease {
        bail!(
            "existing GitHub release has prerelease={} but tag requires prerelease={prerelease}",
            release.prerelease
        );
    }

    let expected: BTreeSet<&str> = names
        .iter()
        .map(String::as_str)
        .chain([CHECKSUM_FILE])
        .collect();
    let actual: BTreeSet<&str> = release
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .collect();
    if !actual.is_subset(&expected) {
        let extra: Vec<&str> = actual.difference(&expected).copied().collect();
        bail!(
            "existing GitHub release has unexpected assets: [{}]",
            extra.join(", ")
        );
    }

    let missing: Vec<&str> = expected.difference(&actual).copied().collect();
    if !missing.is_empty() {
        let mut args = vec![
            OsString::from("release"),
            OsString::from("upload"),
            OsString::from(tag),
        ];
        for name in &missing {
            if *name == CHECKSUM_FILE {
                args.push(sums.as_os_str().to_owned());
            } else {
                args.push(artifacts.join(*name).into_os_string());
            }
        }
        run("gh", &args)?;
        println!("uploaded missing release assets: {}", missing.join(", "));
        let release = release_lookup(tag)?
            .with_context(|| format!("GitHub release {tag} disappeared while repairing it"))?;
        return verify_release(&release, names, hashes, artifacts, prerelease);
    }

    verify_release(&release, names, hashes, artifacts, prerelease)
}

fn upload_release_objects(
    tag: &str,
    artifacts: &Path,
    names: &[String],
    sums: &Path,
) -> Result<()> {
    for name in names {
        let key = format!("{RELEASES_PREFIX}/{tag}/{name}");
        r2_put(&key, &artifacts.join(name.as_str()), content_type(name))?;
    }
    let key = format!("{RELEASES_PREFIX}/{tag}/{CHECKSUM_FILE}");
    r2_put(&key, sums, "text/plain; charset=utf-8")?;
    Ok(())
}

fn content_type(name: &str) -> &'static str {
    if name.ends_with(".zip") {
        "application/zip"
    } else if name.ends_with(".deb") {
        "application/vnd.debian.binary-package"
    } else if name.ends_with(".dmg") {
        "application/x-apple-diskimage"
    } else if name.ends_with(".exe") {
        "application/vnd.microsoft.portable-executable"
    } else {
        "application/octet-stream"
    }
}

fn publish_feed(version: &Version, artifacts: &Path) -> Result<()> {
    if let Some(latest) = upstream_latest_stable()?
        && latest.cmp(version).is_gt()
    {
        println!(
            "update feed left unchanged: upstream stable release v{latest} is newer than v{version}"
        );
        return Ok(());
    }
    if let Some(current) = read_feed()?
        && current.cmp(version).is_gt()
    {
        println!(
            "update feed left unchanged: it already advertises v{current}, newer than v{version}"
        );
        return Ok(());
    }

    let version_name = version.to_string();
    let feed_assets = [
        WINDOWS_BINARY.to_owned(),
        format!("LitematicaPreview-{version_name}-aarch64.dmg"),
        format!("LitematicaPreview-{version_name}-x86_64.dmg"),
    ];
    let mut assets = Map::new();
    for name in feed_assets {
        let hash = sha256_file(&artifacts.join(name.as_str()))?;
        assets.insert(name, Value::String(hash));
    }
    let feed = json!({
        "version": version.to_string(),
        "assets": assets,
    });

    let mut file = tempfile::Builder::new()
        .prefix("litematica-preview-latest-")
        .suffix(".json")
        .tempfile()
        .context("could not create a temporary update feed file")?;
    file.write_all(feed.to_string().as_bytes())
        .context("could not write the update feed")?;
    file.flush().context("could not flush the update feed")?;

    r2_put(FEED_KEY, file.path(), "application/json")?;
    println!("advanced stable update feed to v{version}");
    Ok(())
}

fn upstream_latest_stable() -> Result<Option<Version>> {
    let endpoint = format!("repos/{}/releases/latest", repository()?);
    let output = run_output(
        "gh",
        &[
            OsString::from("api"),
            OsString::from(endpoint),
            OsString::from("--jq"),
            OsString::from(".tag_name"),
        ],
    )?;
    if !output.status.success() {
        let detail = failure_detail(&output);
        if gh_http_error(&output, 404) {
            return Ok(None);
        }
        bail!(
            "could not read the latest upstream stable release: {}",
            detail.trim()
        );
    }
    let tag = String::from_utf8(output.stdout)
        .context("the latest upstream release tag is not UTF-8")?
        .trim()
        .to_owned();
    if tag.is_empty() {
        return Ok(None);
    }
    let version = parse_tag(&tag)
        .with_context(|| format!("upstream release tag `{tag}` is not a release tag"))?;
    Ok(Some(version))
}

fn gh_http_error(output: &Output, status: u16) -> bool {
    let suffix = format!("(HTTP {status})");
    !output.status.success()
        && String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line.starts_with("gh:") && line.trim_end().ends_with(&suffix))
}

fn read_feed() -> Result<Option<Version>> {
    let output = run_output(
        "cf",
        &[
            OsString::from("r2"),
            OsString::from("objects"),
            OsString::from("get"),
            OsString::from(FEED_KEY),
            OsString::from("--text"),
            OsString::from("--bucket-name"),
            OsString::from(BUCKET),
        ],
    )?;
    if !output.status.success() {
        let detail = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let lower = detail.to_ascii_lowercase();
        if lower.contains("404")
            || lower.contains("not found")
            || lower.contains("does not exist")
            || lower.contains("nosuchkey")
            || lower.contains("no such key")
        {
            return Ok(None);
        }
        bail!(
            "could not read the current stable update feed: {}",
            detail.trim()
        );
    }

    let text =
        String::from_utf8(output.stdout).context("the current stable update feed is not UTF-8")?;
    let value: Value = serde_json::from_str(text.trim())
        .context("the current stable update feed is not valid JSON")?;
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .context("the current stable update feed has no version")?;
    let version = Version::parse(version.trim())
        .with_context(|| format!("current update feed version `{version}` is not semver"))?;
    Ok(Some(version))
}

fn hash_artifacts(artifacts: &Path, names: &[String]) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for name in names {
        let path = artifacts.join(name.as_str());
        let metadata = fs::metadata(&path)
            .with_context(|| format!("missing release artifact {}", path.display()))?;
        if !metadata.is_file() || metadata.len() == 0 {
            bail!(
                "release artifact is not a non-empty file: {}",
                path.display()
            );
        }
        hashes.insert(name.clone(), sha256_file(&path)?);
    }
    Ok(hashes)
}

fn write_checksums(artifacts: &Path, hashes: &BTreeMap<String, String>) -> Result<PathBuf> {
    let mut content = String::new();
    for (name, hash) in hashes {
        writeln!(content, "{hash}  {name}").expect("writing to a String cannot fail");
    }
    let path = artifacts.join(CHECKSUM_FILE);
    fs::write(&path, content).with_context(|| format!("could not write {}", path.display()))?;
    Ok(path)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("could not read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}

fn r2_put(key: &str, path: &Path, content_type: &str) -> Result<()> {
    run(
        "cf",
        &[
            OsString::from("r2"),
            OsString::from("objects"),
            OsString::from("put"),
            OsString::from(key),
            OsString::from("--bucket-name"),
            OsString::from(BUCKET),
            OsString::from("--content-type"),
            OsString::from(content_type),
            OsString::from("--file"),
            path.as_os_str().to_owned(),
        ],
    )?;
    println!("uploaded {key}");
    Ok(())
}

fn repository() -> Result<String> {
    if let Some(repository) = std::env::var_os("GITHUB_REPOSITORY") {
        let repository = repository.to_string_lossy().trim().to_owned();
        if !repository.is_empty() {
            return Ok(repository);
        }
    }
    let output = run(
        "gh",
        &[
            OsString::from("repo"),
            OsString::from("view"),
            OsString::from("--json"),
            OsString::from("nameWithOwner"),
            OsString::from("--jq"),
            OsString::from(".nameWithOwner"),
        ],
    )?;
    let repository = String::from_utf8(output.stdout)
        .context("gh repo view returned a non-UTF-8 repository name")?
        .trim()
        .to_owned();
    if repository.is_empty() {
        bail!("cannot determine the GitHub repository; set GITHUB_REPOSITORY");
    }
    Ok(repository)
}

fn http_status(headers: &str) -> Option<u16> {
    let line = headers.lines().next()?;
    let mut parts = line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

fn http_body(response: &str) -> Option<&str> {
    response
        .split_once("\r\n\r\n")
        .or_else(|| response.split_once("\n\n"))
        .map(|(_, body)| body)
}

fn run(program: &str, args: &[OsString]) -> Result<Output> {
    let output = run_output(program, args)?;
    if !output.status.success() {
        bail!(
            "`{} {}` failed: {}",
            program,
            render_args(args),
            failure_detail(&output).trim()
        );
    }
    Ok(output)
}

fn run_output(program: &str, args: &[OsString]) -> Result<Output> {
    Command::new(program)
        .args(args)
        .current_dir(crate::root())
        .output()
        .with_context(|| format!("failed to run `{program}`; is it installed and on PATH?"))
}

fn render_args(args: &[OsString]) -> String {
    args.iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn failure_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !stdout.is_empty() {
        return stdout;
    }
    format!("exit status {}", output.status)
}
