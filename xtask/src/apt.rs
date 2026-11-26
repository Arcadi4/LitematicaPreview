// Publishes a Linux Debian package to the signed apt repository on Cloudflare R2.
//
// Usage: `cargo xtask apt-publish --deb <package.deb> [options]`.
//
// The publisher never creates keys and never deletes remote objects. Every run
// rebuilds a fresh latest-only local reprepro repository, verifies the signed
// `InRelease` with the keyring embedded in the package, then uploads the pool
// package and immutable by-hash indexes before `Release`, `Release.gpg`, and
// finally `InRelease`.

pub mod metadata;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use metadata::{
    APT_ARCHITECTURE, APT_COMPONENT, APT_DISTRIBUTION, APT_KEYRING_PATH, APT_PACKAGE,
    APT_SOURCE_PATH, embedded_path, normalize_apt_base_url, render_apt_source, require_file,
    same_fingerprint,
};

const DEFAULT_REPOSITORY: &str = "target/apt-repo";
const REPOSITORY_MARKER: &str = ".litematica-apt-repository";

/// Entry point for `cargo xtask apt-publish`.
pub fn run(args: &[OsString]) -> Result<()> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        let Some(flag) = argument.to_str() else {
            bail!(
                "Unsupported argument: {}. Run with --help for usage.",
                argument.to_string_lossy()
            );
        };
        match flag {
            "--" => {}
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            "--deb" => {
                let value = args
                    .get(index + 1)
                    .with_context(|| "--deb requires a path.".to_string())?;
                options.debs.push(root_path(PathBuf::from(value.clone())));
                index += 1;
            }
            "--repo" => {
                let value = args
                    .get(index + 1)
                    .with_context(|| "--repo requires a path.".to_string())?;
                options.repo = Some(root_path(PathBuf::from(value.clone())));
                index += 1;
            }
            "--allow-prerelease" => options.allow_prerelease = true,
            "--dry-run" => options.dry_run = true,
            other => bail!("Unsupported argument: {other}. Run with --help for usage."),
        }
        index += 1;
    }
    if options.debs.is_empty() {
        bail!("Pass at least one --deb package to publish.");
    }
    publish(&options)
}

#[derive(Default)]
struct Options {
    debs: Vec<PathBuf>,
    repo: Option<PathBuf>,
    allow_prerelease: bool,
    dry_run: bool,
}

fn print_usage() {
    println!(
        "Publishes a Debian package to the signed apt repository on Cloudflare R2.\n\n\
         Usage: cargo xtask apt-publish --deb <package.deb> [options]\n\n\
         Options:\n\
         \x20 --deb <path>          Debian package to publish (repeatable, required)\n\
         \x20 --repo <path>         Local reprepro repository (default: {DEFAULT_REPOSITORY})\n\
         \x20 --allow-prerelease    Publish a version that contains a prerelease suffix\n\
         \x20 --dry-run             Build and sign the repository without uploading\n\
         \x20 --help                Show this message\n\n\
         Required environment:\n\
         \x20 LP_APT_BASE_URL            Public https URL of the repository\n\
         \x20 LP_APT_GPG_PRIVATE_KEY     Armored private signing key (or LP_APT_GPG_PRIVATE_KEY_FILE)\n\
         \x20 LP_APT_R2_BUCKET           R2 bucket that serves the repository\n\
         \x20 CLOUDFLARE_API_TOKEN       API token with R2 write access\n\
         \x20 CLOUDFLARE_ACCOUNT_ID      Account that owns the bucket\n\n\
         Optional environment:\n\
         \x20 LP_APT_GPG_PASSPHRASE      Passphrase for a protected signing key\n\
         \x20 LP_APT_GPG_FINGERPRINT     Require a specific signing key fingerprint\n\
         \x20 LP_APT_R2_JURISDICTION     R2 jurisdiction: default, eu, fedramp, or us"
    );
}

fn publish(options: &Options) -> Result<()> {
    let base_url = normalize_apt_base_url(&require_environment(
        "LP_APT_BASE_URL",
        "the public apt repository URL",
    )?)?;
    let bucket = if options.dry_run {
        environment("LP_APT_R2_BUCKET").unwrap_or_else(|| "dry-run".to_string())
    } else {
        require_environment("LP_APT_R2_BUCKET", "the R2 bucket name")?
    };
    if !options.dry_run {
        require_environment(
            "CLOUDFLARE_API_TOKEN",
            "the Cloudflare API token with R2 write access",
        )?;
        require_environment(
            "CLOUDFLARE_ACCOUNT_ID",
            "the Cloudflare account that owns the bucket",
        )?;
    }

    let repo = options
        .repo
        .clone()
        .unwrap_or_else(|| root_path(PathBuf::from(DEFAULT_REPOSITORY)));
    // GnuPG needs a short home path for its Unix-domain agent socket.
    let work = tempfile::Builder::new()
        .prefix("lp-apt-")
        .tempdir()
        .context("could not create private apt signing workspace")?;
    let packages = options
        .debs
        .iter()
        .enumerate()
        .map(|(index, deb)| inspect_deb(deb, work.path(), index))
        .collect::<Result<Vec<_>>>()?;
    validate_packages(&packages, &base_url, options.allow_prerelease)?;

    let identity = prepare_signing_key(work.path())?;
    println!("Signing key: {}", identity.fingerprint);
    let keyring = validate_embedded_keyring(&packages, &identity, work.path())?;

    build_repository(&repo, &packages, &identity)?;
    verify_repository(&repo, &keyring, &identity)?;
    validate_index(&repo, &packages)?;

    let keys = publication_order(&repository_keys(&repo)?)?;
    println!("Publishing {} objects to {bucket}:", keys.len());
    for key in &keys {
        upload_object(&repo, key, &bucket, options.dry_run)?;
    }

    println!(
        "{}",
        if options.dry_run {
            format!("Dry run complete. Local repository: {}", repo.display())
        } else {
            format!("Published {base_url} from {}", repo.display())
        }
    );
    drop(identity);
    Ok(())
}

// ---------------------------------------------------------------------------
// Environment and process helpers
// ---------------------------------------------------------------------------

fn environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn require_environment(name: &str, describe: &str) -> Result<String> {
    environment(name).with_context(|| format!("{name} is not set. It provides {describe}."))
}

fn root_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        crate::root().join(path)
    }
}

fn os(value: impl AsRef<OsStr>) -> OsString {
    value.as_ref().to_os_string()
}

fn tool_hint(tool: &str) -> Option<&'static str> {
    match tool {
        "gpg" | "gpgv" | "gpgconf" => Some("Install GnuPG (package: gnupg)."),
        "gpg-preset-passphrase" => Some("It ships with GnuPG; install the gnupg package."),
        "reprepro" => Some("Install reprepro on the publishing host."),
        "dpkg-deb" => Some("Install dpkg."),
        "cf" => Some("Install the Cloudflare CLI: npm install --global cf."),
        _ => None,
    }
}

fn start_error(tool: &str, error: &std::io::Error) -> anyhow::Error {
    match tool_hint(tool) {
        Some(hint) => anyhow!("{tool} could not be started: {error}. {hint}"),
        None => anyhow!("{tool} could not be started: {error}"),
    }
}

fn describe_args(args: &[OsString]) -> String {
    args.iter()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

fn prepared_command(tool: &str, home: Option<&Path>) -> Command {
    let mut command = Command::new(tool);
    command.current_dir(crate::root());
    if let Some(home) = home {
        command.env("GNUPGHOME", home).env("LC_ALL", "C");
    }
    command
}

fn check_output(tool: &str, args: &[OsString], output: std::process::Output) -> Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            match output.status.code() {
                Some(code) => format!("exit code {code}"),
                None => "terminated by a signal".to_string(),
            }
        } else {
            stderr.trim().to_string()
        };
        bail!("{tool} {} failed: {detail}", describe_args(args));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_tool(tool: &str, args: &[OsString], home: Option<&Path>) -> Result<String> {
    let mut command = prepared_command(tool, home);
    command.args(args);
    let output = command
        .output()
        .map_err(|error| start_error(tool, &error))?;
    check_output(tool, args, output)
}

fn run_tool_with_input(
    tool: &str,
    args: &[OsString],
    home: Option<&Path>,
    input: &[u8],
) -> Result<String> {
    let mut command = prepared_command(tool, home);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| start_error(tool, &error))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .context("The child process did not expose standard input.")?;
        stdin
            .write_all(input)
            .with_context(|| format!("Could not write input to {tool}."))?;
    }
    let output = child.wait_with_output()?;
    check_output(tool, args, output)
}

// ---------------------------------------------------------------------------
// Temporary workspace and signing key
// ---------------------------------------------------------------------------

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

struct SigningIdentity {
    fingerprint: String,
    home: PathBuf,
}

impl Drop for SigningIdentity {
    fn drop(&mut self) {
        kill_gpg_agent(&self.home);
    }
}

fn kill_gpg_agent(home: &Path) {
    let _ = prepared_command("gpgconf", Some(home))
        .args(["--kill", "gpg-agent"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Kills the temporary gpg-agent if any step before `SigningIdentity` exists
/// fails, so a partially prepared key never lingers in an agent.
struct AgentCleanup<'a> {
    home: &'a Path,
    armed: bool,
}

impl Drop for AgentCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            kill_gpg_agent(self.home);
        }
    }
}

fn read_private_key() -> Result<Vec<u8>> {
    let file = environment("LP_APT_GPG_PRIVATE_KEY_FILE");
    let inline = environment("LP_APT_GPG_PRIVATE_KEY");
    if file.is_some() && inline.is_some() {
        bail!("Set only one of LP_APT_GPG_PRIVATE_KEY_FILE and LP_APT_GPG_PRIVATE_KEY.");
    }
    if let Some(file) = file {
        let path = root_path(PathBuf::from(file));
        require_file(&path, "Apt signing key")?;
        return fs::read(&path)
            .with_context(|| format!("Could not read the apt signing key {}", path.display()));
    }
    if let Some(inline) = inline {
        let mut key = inline.into_bytes();
        if !key.ends_with(b"\n") {
            key.push(b'\n');
        }
        return Ok(key);
    }
    bail!(
        "The apt signing key is missing. Set LP_APT_GPG_PRIVATE_KEY or \
         LP_APT_GPG_PRIVATE_KEY_FILE. The publisher never creates or generates keys."
    )
}

fn primary_fingerprints(colon: &str, record: &str) -> Vec<String> {
    let mut fingerprints = Vec::new();
    let mut expects_fingerprint = false;
    for line in colon.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let kind = fields.first().copied().unwrap_or("");
        if kind == record {
            expects_fingerprint = true;
        } else if kind == "fpr" && expects_fingerprint {
            if let Some(fingerprint) = fields.get(9).filter(|value| !value.is_empty()) {
                fingerprints.push((*fingerprint).to_string());
            }
            expects_fingerprint = false;
        } else if matches!(kind, "sub" | "ssb" | "uid") {
            expects_fingerprint = false;
        }
    }
    fingerprints
}

fn prepare_signing_key(work_root: &Path) -> Result<SigningIdentity> {
    let home = work_root.join("gnupg");
    create_private_dir(&home)
        .with_context(|| format!("Could not create signing key home {}", home.display()))?;
    let mut cleanup = AgentCleanup {
        home: &home,
        armed: true,
    };
    let key = read_private_key()?;
    run_tool_with_input(
        "gpg",
        &[os("--batch"), os("--yes"), os("--import")],
        Some(&home),
        &key,
    )
    .context("gpg could not import the signing key")?;

    let listed = run_tool(
        "gpg",
        &[os("--batch"), os("--with-colons"), os("--list-secret-keys")],
        Some(&home),
    )?;
    let fingerprints = primary_fingerprints(&listed, "sec");
    if fingerprints.is_empty() {
        bail!("The configured key contains no secret key; the publisher never generates keys.");
    }
    let fingerprint = match environment("LP_APT_GPG_FINGERPRINT") {
        Some(expected) => fingerprints
            .iter()
            .find(|candidate| same_fingerprint(candidate.as_str(), expected.as_str()))
            .cloned()
            .with_context(|| {
                format!("The signing key {expected} is not present in the configured private key.")
            })?,
        None => fingerprints[0].clone(),
    };

    if let Some(passphrase) = environment("LP_APT_GPG_PASSPHRASE") {
        preset_passphrase(&home, &passphrase)?;
    }
    cleanup.armed = false;
    drop(cleanup);
    Ok(SigningIdentity { fingerprint, home })
}

fn preset_passphrase(home: &Path, passphrase: &str) -> Result<()> {
    fs::write(home.join("gpg-agent.conf"), "allow-preset-passphrase\n")
        .context("Could not configure the temporary gpg-agent.")?;
    let _ = prepared_command("gpgconf", Some(home))
        .args(["--kill", "gpg-agent"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    run_tool("gpgconf", &[os("--launch"), os("gpg-agent")], Some(home))?;

    let keygrips = run_tool(
        "gpg",
        &[
            os("--batch"),
            os("--with-colons"),
            os("--with-keygrip"),
            os("--list-secret-keys"),
        ],
        Some(home),
    )?
    .lines()
    .filter(|line| line.starts_with("grp:"))
    .filter_map(|line| line.split(':').nth(9))
    .filter(|keygrip| !keygrip.is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();

    let preset = locate_preset_command(home)?;
    for keygrip in keygrips {
        let mut command = Command::new(&preset);
        command
            .current_dir(crate::root())
            .args(["--preset", keygrip.as_str()])
            .env("GNUPGHOME", home)
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| start_error("gpg-preset-passphrase", &error))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(passphrase.as_bytes())
                .and_then(|()| stdin.write_all(b"\n"))
                .context("Could not write the passphrase to gpg-preset-passphrase.")?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "gpg-preset-passphrase failed for the signing key: {}",
                stderr.trim()
            );
        }
    }
    Ok(())
}

fn locate_preset_command(home: &Path) -> Result<PathBuf> {
    let which = prepared_command("which", None)
        .arg("gpg-preset-passphrase")
        .output()
        .ok()
        .filter(|output| output.status.success());
    if let Some(output) = which {
        let found = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !found.is_empty() {
            return Ok(PathBuf::from(found));
        }
    }
    let directories = run_tool("gpgconf", &[os("--list-dirs")], Some(home))?;
    for line in directories.lines() {
        if let Some(directory) = line.strip_prefix("libexecdir:") {
            let candidate = Path::new(directory.trim()).join("gpg-preset-passphrase");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!("gpg-preset-passphrase is required for LP_APT_GPG_PASSPHRASE but was not found.")
}

// ---------------------------------------------------------------------------
// Package inspection and validation
// ---------------------------------------------------------------------------

struct DebPackage {
    path: PathBuf,
    name: String,
    version: String,
    architecture: String,
    source: String,
    keyring: Vec<u8>,
}

fn deb_field(deb: &Path, field: &str) -> Result<String> {
    Ok(run_tool("dpkg-deb", &[os("-f"), os(deb), os(field)], None)?
        .trim()
        .to_string())
}

fn inspect_deb(deb: &Path, work_root: &Path, index: usize) -> Result<DebPackage> {
    require_file(deb, "Debian package")?;
    let name = deb_field(deb, "Package")?;
    let version = deb_field(deb, "Version")?;
    let architecture = deb_field(deb, "Architecture")?;
    if name.is_empty() || version.is_empty() || architecture.is_empty() {
        bail!(
            "Could not read the Debian control fields from {}.",
            deb.display()
        );
    }

    let extract_root = work_root.join(format!("deb-{index}"));
    create_private_dir(&extract_root)
        .with_context(|| format!("Could not create {}", extract_root.display()))?;
    run_tool("dpkg-deb", &[os("-x"), os(deb), os(&extract_root)], None)?;

    let source_path = embedded_path(&extract_root, APT_SOURCE_PATH);
    let source = fs::read_to_string(&source_path).with_context(|| {
        format!(
            "The Debian package does not embed {APT_SOURCE_PATH}. \
             Build it with the apt-enabled Linux package task."
        )
    })?;
    let keyring_path = embedded_path(&extract_root, APT_KEYRING_PATH);
    let keyring = fs::read(&keyring_path).with_context(|| {
        format!(
            "The Debian package does not embed {APT_KEYRING_PATH}. \
             Build it with the apt-enabled Linux package task."
        )
    })?;

    Ok(DebPackage {
        path: deb.to_path_buf(),
        name,
        version,
        architecture,
        source,
        keyring,
    })
}

fn validate_packages(
    packages: &[DebPackage],
    base_url: &str,
    allow_prerelease: bool,
) -> Result<()> {
    let expected_source = render_apt_source(base_url)?;
    let manifest = crate::version().context("Could not read the application version.")?;
    for deb in packages {
        let file = deb
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| deb.path.display().to_string());
        if deb.name != APT_PACKAGE {
            bail!(
                "{file} is package {}; the apt repository publishes {APT_PACKAGE}.",
                deb.name
            );
        }
        if deb.architecture != APT_ARCHITECTURE {
            bail!(
                "{file} targets {}; the apt repository serves {APT_ARCHITECTURE}.",
                deb.architecture
            );
        }
        let version = semver::Version::parse(&deb.version).with_context(|| {
            format!(
                "{file} has version {}; a semantic version is required.",
                deb.version
            )
        })?;
        if !version.pre.is_empty() && !allow_prerelease {
            bail!(
                "{} {} is a prerelease. Pass --allow-prerelease to publish it to apt.",
                deb.name,
                deb.version
            );
        }
        if version != manifest {
            bail!(
                "{file} has version {}; the application manifest declares {manifest}. \
                 Publish the Debian package built from this checkout.",
                deb.version
            );
        }
        if deb.source != expected_source {
            bail!(
                "{file} does not embed {} Rebuild it with the apt-enabled Linux package task \
                 and the matching LP_APT_BASE_URL.",
                expected_source.trim()
            );
        }
    }
    Ok(())
}

fn validate_embedded_keyring(
    packages: &[DebPackage],
    identity: &SigningIdentity,
    work_root: &Path,
) -> Result<PathBuf> {
    let reference = &packages[0].keyring;
    for deb in packages {
        if deb.keyring != *reference {
            bail!("The Debian packages embed different apt repository keyrings.");
        }
    }

    let keyring_path = work_root.join("embedded-keyring.gpg");
    fs::write(&keyring_path, reference)
        .with_context(|| format!("Could not write {}", keyring_path.display()))?;
    let colon = run_tool(
        "gpg",
        &[
            os("--batch"),
            os("--with-colons"),
            os("--show-keys"),
            os(&keyring_path),
        ],
        Some(&identity.home),
    )?;
    for line in colon.lines() {
        let kind = line.split(':').next().unwrap_or("");
        if kind == "sec" || kind == "ssb" {
            bail!("The Debian package embeds private key material. Rebuild it with a public key.");
        }
    }
    let fingerprints = primary_fingerprints(&colon, "pub");
    if !fingerprints
        .iter()
        .any(|fingerprint| same_fingerprint(fingerprint, &identity.fingerprint))
    {
        let embedded = if fingerprints.is_empty() {
            "no key".to_string()
        } else {
            fingerprints.join(", ")
        };
        bail!(
            "The Debian package embeds {embedded}, which does not match signing key {}.",
            identity.fingerprint
        );
    }
    Ok(keyring_path)
}

// ---------------------------------------------------------------------------
// Repository generation
// ---------------------------------------------------------------------------

fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate::root().join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn create_repository(repo_root: &Path, fingerprint: &str) -> Result<()> {
    let project_root = fs::canonicalize(crate::root())
        .with_context(|| format!("Could not resolve {}.", crate::root().display()))?;
    let repository = if repo_root.exists() {
        fs::canonicalize(repo_root)
            .with_context(|| format!("Could not resolve {}.", repo_root.display()))?
    } else {
        lexical_absolute(repo_root)
    };
    let contains_project = project_root
        .strip_prefix(&repository)
        .map(|rest| !rest.as_os_str().is_empty())
        .unwrap_or(false);
    if repository.parent().is_none() || repository == project_root || contains_project {
        bail!(
            "Refusing to replace repository directory {}.",
            repo_root.display()
        );
    }

    if repo_root.exists() {
        let marker = repo_root.join(REPOSITORY_MARKER);
        if !marker.is_file() {
            bail!(
                "Refusing to replace an unrecognized directory: {}.",
                repo_root.display()
            );
        }
        fs::remove_dir_all(repo_root).with_context(|| {
            format!(
                "Could not remove the previous local repository {}.",
                repo_root.display()
            )
        })?;
    }

    fs::create_dir_all(repo_root.join("conf"))
        .with_context(|| format!("Could not create {}", repo_root.display()))?;
    fs::write(repo_root.join(REPOSITORY_MARKER), b"")
        .context("Could not mark the local apt repository.")?;
    fs::write(
        repo_root.join("conf").join("distributions"),
        format!(
            "Origin: Litematica Preview\n\
             Label: Litematica Preview\n\
             Suite: {APT_DISTRIBUTION}\n\
             Codename: {APT_DISTRIBUTION}\n\
             Architectures: {APT_ARCHITECTURE}\n\
             Components: {APT_COMPONENT}\n\
             Description: Litematica Preview Debian repository\n\
             SignWith: {fingerprint}\n\
             DebIndices: Packages Release . .gz .xz\n"
        ),
    )
    .context("Could not write the reprepro distributions file.")?;
    fs::write(repo_root.join("conf").join("options"), "verbose\n")
        .context("Could not write the reprepro options file.")?;
    Ok(())
}

fn reprepro(repo_root: &Path, extra: &[OsString], home: &Path) -> Result<String> {
    let mut arguments = vec![os("--basedir"), os(repo_root)];
    arguments.extend_from_slice(extra);
    run_tool("reprepro", &arguments, Some(home))
}

fn build_repository(
    repo_root: &Path,
    packages: &[DebPackage],
    identity: &SigningIdentity,
) -> Result<()> {
    create_repository(repo_root, &identity.fingerprint)?;
    for deb in packages {
        reprepro(
            repo_root,
            &[
                os("-C"),
                os(APT_COMPONENT),
                os("-S"),
                os("utils"),
                os("-P"),
                os("optional"),
                os("includedeb"),
                os(APT_DISTRIBUTION),
                os(&deb.path),
            ],
            &identity.home,
        )?;
        println!("Included {} {}", deb.name, deb.version);
    }
    reprepro(
        repo_root,
        &[os("export"), os(APT_DISTRIBUTION)],
        &identity.home,
    )?;

    let distribution = repo_root.join("dists").join(APT_DISTRIBUTION);
    let index_dir = distribution
        .join(APT_COMPONENT)
        .join(format!("binary-{APT_ARCHITECTURE}"));
    let by_hash = index_dir.join("by-hash").join("SHA256");
    fs::create_dir_all(&by_hash)
        .with_context(|| format!("Could not create {}", by_hash.display()))?;

    let release_path = distribution.join("Release");
    let release = fs::read_to_string(&release_path)
        .with_context(|| format!("Could not read {}", release_path.display()))?;
    let digests = release_sha256(&release);
    for name in ["Packages", "Packages.gz", "Packages.xz"] {
        let index = index_dir.join(name);
        require_file(&index, &format!("Generated repository index {name}"))?;
        let key = format!("{APT_COMPONENT}/binary-{APT_ARCHITECTURE}/{name}");
        let digest = digests.get(&key).with_context(|| {
            format!("The generated Release file has no SHA256 entry for {key}.")
        })?;
        if digest.len() != 64
            || !digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            bail!("The generated Release file has an invalid SHA256 entry for {key}.");
        }
        fs::copy(&index, by_hash.join(digest))
            .with_context(|| format!("Could not create immutable by-hash copy of {name}."))?;
    }

    fs::OpenOptions::new()
        .append(true)
        .open(&release_path)
        .with_context(|| format!("Could not open {}", release_path.display()))?
        .write_all(b"Acquire-By-Hash: yes\n")
        .context("Could not enable apt by-hash downloads in Release.")?;

    let common = [
        os("--batch"),
        os("--yes"),
        os("--local-user"),
        os(identity.fingerprint.as_str()),
    ];
    let mut arguments = common.to_vec();
    arguments.extend([
        os("--armor"),
        os("--detach-sign"),
        os("--output"),
        os(distribution.join("Release.gpg")),
        os(&release_path),
    ]);
    run_tool("gpg", &arguments, Some(&identity.home))?;

    arguments = common.to_vec();
    arguments.extend([
        os("--clearsign"),
        os("--output"),
        os(distribution.join("InRelease")),
        os(&release_path),
    ]);
    run_tool("gpg", &arguments, Some(&identity.home))?;
    Ok(())
}

/// Reads the `SHA256:` section of a Release file, mapping each file path to its
/// digest. The publisher never signs an index before reprepro has listed it.
fn release_sha256(release: &str) -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();
    let mut in_sha256 = false;
    for line in release.lines() {
        if let Some(rest) = line.strip_prefix(' ') {
            if in_sha256 {
                let mut fields = rest.split_whitespace();
                if let (Some(digest), Some(_size), Some(path)) =
                    (fields.next(), fields.next(), fields.next())
                {
                    digests.insert(path.to_string(), digest.to_string());
                }
            }
        } else if !line.is_empty() {
            in_sha256 = line == "SHA256:";
        }
    }
    digests
}

fn verify_repository(
    repo_root: &Path,
    keyring_path: &Path,
    identity: &SigningIdentity,
) -> Result<()> {
    let distribution = repo_root.join("dists").join(APT_DISTRIBUTION);
    for required in [
        PathBuf::from("InRelease"),
        PathBuf::from("Release"),
        PathBuf::from("Release.gpg"),
        Path::new(APT_COMPONENT)
            .join(format!("binary-{APT_ARCHITECTURE}"))
            .join("Packages"),
    ] {
        require_file(
            &distribution.join(&required),
            &format!("Generated repository file {}", required.display()),
        )?;
    }
    run_tool(
        "gpgv",
        &[
            os("--keyring"),
            os(keyring_path),
            os(distribution.join("InRelease")),
        ],
        Some(&identity.home),
    )?;
    run_tool(
        "gpgv",
        &[
            os("--keyring"),
            os(keyring_path),
            os(distribution.join("Release.gpg")),
            os(distribution.join("Release")),
        ],
        Some(&identity.home),
    )?;
    Ok(())
}

fn parse_control(stanza: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for line in stanza.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            fields.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    fields
}

fn validate_index(repo_root: &Path, packages: &[DebPackage]) -> Result<()> {
    let packages_path = repo_root
        .join("dists")
        .join(APT_DISTRIBUTION)
        .join(APT_COMPONENT)
        .join(format!("binary-{APT_ARCHITECTURE}"))
        .join("Packages");
    require_file(&packages_path, "Generated Packages index")?;

    let expected = packages
        .iter()
        .map(|deb| (deb.name.clone(), deb.version.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeMap::new();
    let contents = fs::read_to_string(&packages_path)
        .with_context(|| format!("Could not read {}", packages_path.display()))?;
    for stanza in contents.split("\n\n") {
        let stanza = stanza.trim();
        if stanza.is_empty() {
            continue;
        }
        let fields = parse_control(stanza);
        let Some(name) = fields.get("Package") else {
            continue;
        };
        if !expected.contains_key(name) {
            bail!("The repository index contains unexpected package {name}.");
        }
        if seen.contains_key(name) {
            bail!("The repository index contains more than one version of {name}.");
        }
        let version = fields.get("Version").map(String::as_str).unwrap_or("");
        if Some(version) != expected.get(name).map(String::as_str) {
            bail!(
                "The repository index lists {name} {version}; expected {}.",
                expected.get(name).map(String::as_str).unwrap_or("")
            );
        }
        if fields.get("Architecture").map(String::as_str) != Some(APT_ARCHITECTURE) {
            bail!("The repository index lists {name} for an unexpected architecture.");
        }
        let Some(filename) = fields.get("Filename") else {
            bail!("The repository index does not list a pool file for {name}.");
        };
        let pool_path = Path::new(filename);
        if pool_path.is_absolute()
            || pool_path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            bail!("The repository index points to an invalid pool file for {name}: {filename}");
        }
        require_file(
            &repo_root.join(pool_path),
            &format!("Repository pool file for {name}"),
        )?;
        seen.insert(name.clone(), version.to_string());
    }
    for name in expected.keys() {
        if !seen.contains_key(name) {
            bail!("The repository index does not list {name}.");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Publication
// ---------------------------------------------------------------------------

fn walk(directory: &Path, repo_root: &Path, keys: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("Could not list {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let absolute = entry.path();
        if file_type.is_dir() {
            walk(&absolute, repo_root, keys)?;
        } else if file_type.is_file() {
            let relative = absolute
                .strip_prefix(repo_root)
                .with_context(|| format!("{} is outside the repository.", absolute.display()))?;
            let key = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            keys.push(key);
        }
    }
    Ok(())
}

fn repository_keys(repo_root: &Path) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    for root in ["pool", "dists"] {
        let directory = repo_root.join(root);
        if directory.exists() {
            walk(&directory, repo_root, &mut keys)?;
        }
    }
    keys.sort();
    Ok(keys)
}

fn publication_order(keys: &[String]) -> Result<Vec<String>> {
    let in_release = format!("dists/{APT_DISTRIBUTION}/InRelease");
    if !keys.iter().any(|key| key == &in_release) {
        bail!("The repository does not contain {in_release}.");
    }
    let release = [
        format!("dists/{APT_DISTRIBUTION}/Release"),
        format!("dists/{APT_DISTRIBUTION}/Release.gpg"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let component_prefix = format!("dists/{APT_DISTRIBUTION}/{APT_COMPONENT}/");

    let mut pool = Vec::new();
    let mut metadata = Vec::new();
    let mut release_keys = Vec::new();
    let mut remaining = Vec::new();
    for key in keys {
        if key.starts_with("pool/") {
            pool.push(key.clone());
        } else if key.starts_with(&component_prefix) {
            metadata.push(key.clone());
        } else if release.contains(key) {
            release_keys.push(key.clone());
        } else if key != &in_release {
            remaining.push(key.clone());
        }
    }
    for group in [&mut pool, &mut metadata, &mut release_keys, &mut remaining] {
        group.sort();
    }

    let mut ordered = Vec::with_capacity(keys.len());
    ordered.extend(pool);
    ordered.extend(metadata);
    ordered.extend(release_keys);
    ordered.extend(remaining);
    ordered.push(in_release);
    Ok(ordered)
}

fn content_type(key: &str) -> &'static str {
    if key.ends_with(".deb") {
        "application/vnd.debian.binary-package"
    } else if key.ends_with(".gz") {
        "application/gzip"
    } else if key.ends_with(".xz") {
        "application/x-xz"
    } else if key.ends_with(".gpg") {
        "application/pgp-signature"
    } else {
        "text/plain; charset=utf-8"
    }
}

fn upload_object(repo_root: &Path, key: &str, bucket: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("PUT {key}");
        return Ok(());
    }
    let mut arguments = vec![
        os("r2"),
        os("objects"),
        os("put"),
        os(key),
        os("--bucket-name"),
        os(bucket),
        os("--file"),
        os(repo_root.join(key)),
        os("--content-type"),
        os(content_type(key)),
        os("--quiet"),
    ];
    if let Some(jurisdiction) =
        environment("LP_APT_R2_JURISDICTION").filter(|value| value != "default")
    {
        arguments.push(os("--cf-r2-jurisdiction"));
        arguments.push(os(jurisdiction.as_str()));
    }
    let mut command = prepared_command("cf", None);
    command.args(&arguments).env("CF_SEND_TELEMETRY", "false");
    let output = command
        .output()
        .map_err(|error| start_error("cf", &error))?;
    check_output("cf", &arguments, output)?;
    println!("PUT {key}");
    Ok(())
}
