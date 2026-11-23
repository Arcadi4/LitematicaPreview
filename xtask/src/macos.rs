use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use semver::Version;
use serde_json::Value as JsonValue;

const APP_NAME: &str = "Litematica Preview";
const EXECUTABLE: &str = "LitematicaPreview";
const SIGN_IDENTITY_ENV: &str = "LP_MACOS_SIGN_IDENTITY";

const USAGE: &str = "\
usage: cargo xtask package-macos <aarch64|x86_64> [--allow-unsigned] [--binary PATH]

Builds LitematicaPreview for the requested target and packages:
  artifacts/LitematicaPreview-<version>-<arch>.dmg

The DMG contains 'Litematica Preview.app' at the volume root and an
/Applications link. Release packaging signs the app and DMG with a Developer ID
Application identity, notarizes and staples both the app and the DMG. Use
--binary to package an already-built executable without building again, and
--allow-unsigned for one-off local verification. Unsigned packaging is refused
when CI or GITHUB_ACTIONS is set.

release environment:
  LP_MACOS_SIGN_IDENTITY   Developer ID Application identity in the keychain
  MACOS_CERTIFICATE_P12    optional base64 PKCS#12 imported into a temporary keychain
  MACOS_CERTIFICATE_PASSWORD  password for MACOS_CERTIFICATE_P12
  LP_NOTARY_KEYCHAIN_PROFILE
    or LP_NOTARY_KEY_PATH + LP_NOTARY_KEY_ID + LP_NOTARY_ISSUER
    or LP_NOTARY_APPLE_ID + LP_NOTARY_TEAM_ID + LP_NOTARY_PASSWORD
";

pub fn run(args: &[OsString]) -> Result<()> {
    if args
        .iter()
        .any(|argument| matches!(argument.to_str(), Some("--help" | "-h")))
    {
        print!("{USAGE}");
        return Ok(());
    }

    let options = Options::parse(args)?;
    if env::consts::OS != "macos" {
        bail!("package-macos needs a macOS host (hdiutil, codesign, sips, and iconutil)");
    }
    if is_ci() && options.allow_unsigned {
        bail!(
            "--allow-unsigned is refused in CI; a release package must be Developer ID signed, \
             notarized, and stapled"
        );
    }

    let signing = if options.allow_unsigned {
        None
    } else {
        Some(Signing::from_env()?)
    };
    let root = crate::root();
    let version = crate::version()?;
    let binary = match options.binary {
        Some(binary) => {
            let binary = if binary.is_absolute() {
                binary
            } else {
                root.join(binary)
            };
            if !binary.is_file() {
                bail!("--binary {} does not exist", binary.display());
            }
            binary
        }
        None => build_binary(&root, options.arch)?,
    };
    validate_binary(&binary, options.arch)?;
    package(&root, &version, options.arch, &binary, signing.as_ref())
}

struct Options {
    arch: Arch,
    allow_unsigned: bool,
    binary: Option<PathBuf>,
}

impl Options {
    fn parse(args: &[OsString]) -> Result<Self> {
        let mut arch = None;
        let mut allow_unsigned = false;
        let mut binary = None;
        let mut arguments = args.iter();

        while let Some(argument) = arguments.next() {
            let Some(text) = argument.to_str() else {
                bail!("package-macos arguments must be valid UTF-8");
            };
            match text {
                "--allow-unsigned" => allow_unsigned = true,
                "--binary" => {
                    let value = arguments.next().context("--binary requires a path")?;
                    set_binary(&mut binary, PathBuf::from(value.clone()))?;
                }
                _ if text.starts_with("--binary=") => {
                    let value = &text["--binary=".len()..];
                    if value.is_empty() {
                        bail!("--binary requires a path");
                    }
                    set_binary(&mut binary, PathBuf::from(value))?;
                }
                _ if text.starts_with('-') => {
                    bail!("unknown package-macos option '{text}'\n\n{USAGE}");
                }
                _ => {
                    if arch.is_some() {
                        bail!("expected exactly one architecture (aarch64 or x86_64)");
                    }
                    arch = Some(Arch::parse(text)?);
                }
            }
        }

        Ok(Self {
            arch: arch.context("missing architecture (aarch64 or x86_64)")?,
            allow_unsigned,
            binary,
        })
    }
}

fn set_binary(slot: &mut Option<PathBuf>, value: PathBuf) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("--binary was given more than once");
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Arch {
    Aarch64,
    X86_64,
}

impl Arch {
    fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "aarch64" | "arm64" => Ok(Self::Aarch64),
            "x86_64" | "amd64" => Ok(Self::X86_64),
            _ => bail!("unknown macOS architecture '{value}'; expected aarch64 or x86_64"),
        }
    }

    fn triple(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64-apple-darwin",
            Self::X86_64 => "x86_64-apple-darwin",
        }
    }

    fn macho(self) -> &'static str {
        match self {
            Self::Aarch64 => "arm64",
            Self::X86_64 => "x86_64",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        }
    }
}

fn is_ci() -> bool {
    ["CI", "GITHUB_ACTIONS"].iter().copied().any(env_truthy)
}

fn env_truthy(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

struct Signing {
    identity: String,
    credentials: NotaryCredentials,
    keychain: Option<CertificateKeychain>,
}

impl Signing {
    fn from_env() -> Result<Self> {
        let identity = env_nonempty(SIGN_IDENTITY_ENV).with_context(|| {
            format!(
                "{SIGN_IDENTITY_ENV} is required for release packaging; pass --allow-unsigned \
                 only for explicit local verification"
            )
        })?;
        let identity = identity.trim_matches('"').to_owned();
        if !identity.contains("Developer ID Application") {
            bail!(
                "{SIGN_IDENTITY_ENV} must name a Developer ID Application identity, got \
                 '{identity}'"
            );
        }

        let credentials = NotaryCredentials::from_env()?;
        let keychain = CertificateKeychain::from_env()?;
        let listing = signing_identities(keychain.as_ref().map(CertificateKeychain::path))?;
        if !listing.contains(identity.as_str()) {
            bail!(
                "signing identity '{identity}' is not present in the {}; available \
                 identities:\n{listing}",
                if keychain.is_some() {
                    "temporary keychain"
                } else {
                    "keychain"
                }
            );
        }

        Ok(Self {
            identity,
            credentials,
            keychain,
        })
    }

    fn keychain(&self) -> Option<&Path> {
        self.keychain.as_ref().map(CertificateKeychain::path)
    }
}

struct CertificateKeychain {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl CertificateKeychain {
    fn from_env() -> Result<Option<Self>> {
        let Some(encoded) = env_nonempty("MACOS_CERTIFICATE_P12") else {
            if env_var("MACOS_CERTIFICATE_PASSWORD").is_some() {
                bail!("MACOS_CERTIFICATE_PASSWORD is set but MACOS_CERTIFICATE_P12 is missing");
            }
            return Ok(None);
        };
        let password = env_var("MACOS_CERTIFICATE_PASSWORD").unwrap_or_default();

        let encoded: String = encoded
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect();
        let certificate = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .context("MACOS_CERTIFICATE_P12 is not valid base64")?;

        let directory =
            tempfile::tempdir().context("could not create the temporary certificate directory")?;
        let certificate_path = directory.path().join("certificate.p12");
        fs::write(&certificate_path, &certificate).with_context(|| {
            format!(
                "could not write the certificate to {}",
                certificate_path.display()
            )
        })?;
        set_mode(&certificate_path, 0o600)?;

        let keychain_path = directory.path().join("litematica-preview.keychain-db");
        let keychain_password = random_password()?;

        let mut create = Command::new("security");
        create
            .args(["create-keychain", "-p"])
            .arg(&keychain_password)
            .arg(&keychain_path);
        command_status(&mut create, "security create-keychain")?;

        let keychain = Self {
            _directory: directory,
            path: keychain_path,
        };

        let mut settings = Command::new("security");
        settings
            .args(["set-keychain-settings", "-lut", "21600"])
            .arg(keychain.path());
        command_status(&mut settings, "security set-keychain-settings")?;

        let mut unlock = Command::new("security");
        unlock
            .args(["unlock-keychain", "-p"])
            .arg(&keychain_password)
            .arg(keychain.path());
        command_status(&mut unlock, "security unlock-keychain")?;

        let mut import = Command::new("security");
        import
            .arg("import")
            .arg(&certificate_path)
            .args(["-k"])
            .arg(keychain.path())
            .args(["-P"])
            .arg(&password)
            .args([
                "-f",
                "pkcs12",
                "-T",
                "/usr/bin/codesign",
                "-T",
                "/usr/bin/security",
            ]);
        command_status(&mut import, "security import")?;

        let mut partition = Command::new("security");
        partition
            .args([
                "set-key-partition-list",
                "-S",
                "apple-tool:,apple:,codesign:",
                "-s",
                "-k",
            ])
            .arg(&keychain_password)
            .arg(keychain.path());
        command_status(&mut partition, "security set-key-partition-list")?;

        fs::remove_file(&certificate_path)
            .with_context(|| format!("could not remove {}", certificate_path.display()))?;
        Ok(Some(keychain))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CertificateKeychain {
    fn drop(&mut self) {
        let _ = Command::new("security")
            .args(["delete-keychain"])
            .arg(&self.path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn signing_identities(keychain: Option<&Path>) -> Result<String> {
    let mut command = Command::new("security");
    command.args(["find-identity", "-v", "-p", "codesigning"]);
    if let Some(keychain) = keychain {
        command.arg(keychain);
    }
    let output = command_output(&mut command, "security find-identity")?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

enum NotaryCredentials {
    KeychainProfile(String),
    ApiKey {
        key: PathBuf,
        key_id: String,
        issuer: String,
    },
    AppleId {
        apple_id: String,
        team_id: String,
        password: String,
    },
}

impl NotaryCredentials {
    fn from_env() -> Result<Self> {
        let profile = env_nonempty("LP_NOTARY_KEYCHAIN_PROFILE");
        let key = env_nonempty("LP_NOTARY_KEY_PATH");
        let key_id = env_nonempty("LP_NOTARY_KEY_ID");
        let issuer = env_nonempty("LP_NOTARY_ISSUER");
        let apple_id = env_nonempty("LP_NOTARY_APPLE_ID");
        let team_id = env_nonempty("LP_NOTARY_TEAM_ID");
        let password = env_nonempty("LP_NOTARY_PASSWORD");

        let api = key.is_some() || key_id.is_some() || issuer.is_some();
        let apple = apple_id.is_some() || team_id.is_some() || password.is_some();
        let selected = u8::from(profile.is_some()) + u8::from(api) + u8::from(apple);
        if selected != 1 {
            bail!(
                "configure exactly one notarization credential set: LP_NOTARY_KEYCHAIN_PROFILE, \
                 LP_NOTARY_KEY_PATH+LP_NOTARY_KEY_ID+LP_NOTARY_ISSUER, or \
                 LP_NOTARY_APPLE_ID+LP_NOTARY_TEAM_ID+LP_NOTARY_PASSWORD"
            );
        }

        if let Some(profile) = profile {
            return Ok(Self::KeychainProfile(profile));
        }

        if api {
            let key = key.context(
                "LP_NOTARY_KEY_PATH is required with LP_NOTARY_KEY_ID and LP_NOTARY_ISSUER",
            )?;
            let key_id = key_id.context(
                "LP_NOTARY_KEY_ID is required with LP_NOTARY_KEY_PATH and LP_NOTARY_ISSUER",
            )?;
            let issuer = issuer.context(
                "LP_NOTARY_ISSUER is required with LP_NOTARY_KEY_PATH and LP_NOTARY_KEY_ID",
            )?;
            let key = PathBuf::from(key);
            if !key.is_file() {
                bail!("LP_NOTARY_KEY_PATH {} is not a file", key.display());
            }
            return Ok(Self::ApiKey {
                key,
                key_id,
                issuer,
            });
        }

        Ok(Self::AppleId {
            apple_id: apple_id.context(
                "LP_NOTARY_APPLE_ID is required with LP_NOTARY_TEAM_ID and LP_NOTARY_PASSWORD",
            )?,
            team_id: team_id.context(
                "LP_NOTARY_TEAM_ID is required with LP_NOTARY_APPLE_ID and LP_NOTARY_PASSWORD",
            )?,
            password: password.context(
                "LP_NOTARY_PASSWORD is required with LP_NOTARY_APPLE_ID and LP_NOTARY_TEAM_ID",
            )?,
        })
    }

    fn arguments(&self) -> Vec<OsString> {
        match self {
            Self::KeychainProfile(profile) => vec![
                OsString::from("--keychain-profile"),
                OsString::from(profile.as_str()),
            ],
            Self::ApiKey {
                key,
                key_id,
                issuer,
            } => vec![
                OsString::from("--key"),
                key.as_os_str().to_os_string(),
                OsString::from("--key-id"),
                OsString::from(key_id.as_str()),
                OsString::from("--issuer"),
                OsString::from(issuer.as_str()),
            ],
            Self::AppleId {
                apple_id,
                team_id,
                password,
            } => vec![
                OsString::from("--apple-id"),
                OsString::from(apple_id.as_str()),
                OsString::from("--team-id"),
                OsString::from(team_id.as_str()),
                OsString::from("--password"),
                OsString::from(password.as_str()),
            ],
        }
    }
}

fn build_binary(root: &Path, arch: Arch) -> Result<PathBuf> {
    println!("building {EXECUTABLE} for {}", arch.triple());
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command.current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "--target",
        arch.triple(),
        "--bin",
        EXECUTABLE,
    ]);
    command_status(&mut command, "cargo build")?;

    let binary = target_dir(root)
        .join(arch.triple())
        .join("release")
        .join(EXECUTABLE);
    if !binary.is_file() {
        bail!(
            "cargo build completed but {} does not exist",
            binary.display()
        );
    }
    Ok(binary)
}

fn target_dir(root: &Path) -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR").filter(|value| !value.is_empty()) {
        Some(directory) => {
            let directory = PathBuf::from(directory);
            if directory.is_absolute() {
                directory
            } else {
                root.join(directory)
            }
        }
        None => root.join("target"),
    }
}

fn validate_binary(binary: &Path, arch: Arch) -> Result<()> {
    let mut file = Command::new("file");
    file.arg("-b").arg(binary);
    let output = command_output(&mut file, "file")?;
    let kind = String::from_utf8_lossy(&output.stdout);
    if !kind.contains("Mach-O") {
        bail!(
            "{} is not a Mach-O executable: {}",
            binary.display(),
            kind.trim()
        );
    }

    let mut lipo = Command::new("lipo");
    lipo.arg("-archs").arg(binary);
    let output = command_output(&mut lipo, "lipo -archs")?;
    let listing = String::from_utf8_lossy(&output.stdout);
    let architectures: Vec<&str> = listing.split_whitespace().collect();
    if architectures.len() != 1 || architectures[0] != arch.macho() {
        bail!(
            "expected a thin {} Mach-O binary, got {:?} from {}",
            arch.macho(),
            architectures,
            binary.display()
        );
    }
    Ok(())
}

fn package(
    root: &Path,
    version: &Version,
    arch: Arch,
    binary: &Path,
    signing: Option<&Signing>,
) -> Result<()> {
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts)
        .with_context(|| format!("could not create {}", artifacts.display()))?;

    let version_text = version.to_string();
    let dmg_path = artifacts.join(format!(
        "LitematicaPreview-{version_text}-{}.dmg",
        arch.suffix()
    ));
    if dmg_path.exists() {
        fs::remove_file(&dmg_path)
            .with_context(|| format!("could not replace {}", dmg_path.display()))?;
    }

    let work = tempfile::tempdir().context("could not create the macOS packaging directory")?;
    let dmg_root = work.path().join("dmg");
    let app = dmg_root.join(format!("{APP_NAME}.app"));
    create_bundle(root, version, binary, &app)?;

    if let Some(signing) = signing {
        sign_app(&app, &signing.identity, signing.keychain())?;
        notarize_app(&app, &signing.credentials, work.path())?;
    }

    create_applications_link(&dmg_root.join("Applications"))?;
    create_dmg(&dmg_root, &dmg_path)?;

    if let Some(signing) = signing {
        sign_dmg(&dmg_path, &signing.identity, signing.keychain())?;
        notarize_and_staple(&dmg_path, &signing.credentials)?;
    }

    let mode = if signing.is_some() {
        "Developer ID signed, notarized, and stapled"
    } else {
        "unsigned local verification build"
    };
    println!("packaged {mode}");
    println!("  {}", dmg_path.display());
    Ok(())
}

fn create_bundle(root: &Path, version: &Version, binary: &Path, app: &Path) -> Result<()> {
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos).with_context(|| format!("could not create {}", macos.display()))?;
    fs::create_dir_all(&resources)
        .with_context(|| format!("could not create {}", resources.display()))?;

    let executable = macos.join(EXECUTABLE);
    fs::copy(binary, &executable).with_context(|| {
        format!(
            "could not copy {} into {}",
            binary.display(),
            executable.display()
        )
    })?;
    set_executable(&executable)?;
    write_info_plist(root, version, &contents.join("Info.plist"))?;
    write_icon(root, &resources.join("AppIcon.icns"))?;
    fs::copy(
        root.join("app/icons/LICENSE"),
        resources.join("Fluent-UI-System-Icons-LICENSE.txt"),
    )
    .context("could not copy the Fluent UI System Icons license")?;
    Ok(())
}

fn write_info_plist(root: &Path, version: &Version, destination: &Path) -> Result<()> {
    let template = root.join("packaging/macos/Info.plist");
    let mut document = plist::Value::from_file(&template)
        .with_context(|| format!("could not read {}", template.display()))?;
    let dictionary = document
        .as_dictionary_mut()
        .context("packaging/macos/Info.plist must have a dictionary root")?;

    let bundle_version = format!("{}.{}.{}", version.major, version.minor, version.patch);
    dictionary.insert(
        "CFBundleShortVersionString".to_owned(),
        plist::Value::String(bundle_version.clone()),
    );
    dictionary.insert(
        "CFBundleVersion".to_owned(),
        plist::Value::String(bundle_version),
    );

    document
        .to_file_xml(destination)
        .with_context(|| format!("could not write {}", destination.display()))?;
    Ok(())
}

fn write_icon(root: &Path, destination: &Path) -> Result<()> {
    let source = root.join("Assets/icon.png");
    if !source.is_file() {
        bail!("missing app icon {}", source.display());
    }

    let work = tempfile::tempdir().context("could not create the icon staging directory")?;
    let iconset = work.path().join("AppIcon.iconset");
    fs::create_dir_all(&iconset)
        .with_context(|| format!("could not create {}", iconset.display()))?;

    for size in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let pixels = size * scale;
            let pixels = pixels.to_string();
            let name = if scale == 1 {
                format!("icon_{size}x{size}.png")
            } else {
                format!("icon_{size}x{size}@2x.png")
            };
            let output = iconset.join(name);
            let mut sips = Command::new("sips");
            sips.arg("-s")
                .arg("format")
                .arg("png")
                .arg("-z")
                .arg(&pixels)
                .arg(&pixels)
                .arg(&source)
                .arg("--out")
                .arg(&output)
                .stdout(Stdio::null());
            command_status(&mut sips, "sips")?;
        }
    }

    let mut iconutil = Command::new("iconutil");
    iconutil
        .args(["-c", "icns"])
        .arg(&iconset)
        .arg("-o")
        .arg(destination);
    command_status(&mut iconutil, "iconutil")?;
    Ok(())
}

fn sign_app(app: &Path, identity: &str, keychain: Option<&Path>) -> Result<()> {
    let mut sign = Command::new("codesign");
    sign.args([
        "--force",
        "--timestamp",
        "--options",
        "runtime",
        "--sign",
        identity,
    ]);
    if let Some(keychain) = keychain {
        sign.args(["--keychain"]).arg(keychain);
    }
    sign.arg(app);
    command_status(&mut sign, "codesign")?;
    verify_app(app)
}

fn verify_app(app: &Path) -> Result<()> {
    let mut verify = Command::new("codesign");
    verify
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(app);
    command_status(&mut verify, "codesign --verify")
}

fn sign_dmg(dmg: &Path, identity: &str, keychain: Option<&Path>) -> Result<()> {
    let mut sign = Command::new("codesign");
    sign.args(["--force", "--timestamp", "--sign", identity]);
    if let Some(keychain) = keychain {
        sign.args(["--keychain"]).arg(keychain);
    }
    sign.arg(dmg);
    command_status(&mut sign, "codesign")?;

    let mut verify = Command::new("codesign");
    verify.args(["--verify", "--verbose=2"]).arg(dmg);
    command_status(&mut verify, "codesign --verify")?;
    Ok(())
}

fn create_dmg(source: &Path, destination: &Path) -> Result<()> {
    let mut hdiutil = Command::new("hdiutil");
    hdiutil
        .args(["create", "-volname", APP_NAME, "-srcfolder"])
        .arg(source)
        .args(["-ov", "-format", "UDZO", "-fs", "HFS+"])
        .arg(destination);
    command_status(&mut hdiutil, "hdiutil create")
}

fn notarize_app(app: &Path, credentials: &NotaryCredentials, work: &Path) -> Result<()> {
    let archive = work.join(format!("{APP_NAME}.zip"));
    let mut ditto = Command::new("ditto");
    ditto
        .args(["-c", "-k", "--keepParent"])
        .arg(app)
        .arg(&archive);
    command_status(&mut ditto, "ditto")?;

    submit_notarization(&archive, credentials, "app")?;
    staple_and_validate(app, "app")?;
    verify_app(app)
}

fn notarize_and_staple(dmg: &Path, credentials: &NotaryCredentials) -> Result<()> {
    submit_notarization(dmg, credentials, "DMG")?;
    staple_and_validate(dmg, "DMG")
}

fn submit_notarization(
    artifact: &Path,
    credentials: &NotaryCredentials,
    label: &str,
) -> Result<()> {
    let mut submit = Command::new("xcrun");
    submit
        .args(["notarytool", "submit"])
        .arg(artifact)
        .args(["--wait", "--timeout", "60m", "--output-format", "json"])
        .args(credentials.arguments());
    let output = command_output(&mut submit, "xcrun notarytool submit")?;
    let response: JsonValue = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "notarytool submit did not return JSON: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        )
    })?;
    let status = response
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let id = response
        .get("id")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();

    if status != "Accepted" {
        let log = notary_log(id, credentials)
            .unwrap_or_else(|error| format!("could not retrieve the notarization log: {error:#}"));
        bail!(
            "notarization failed for {label} {}: status {status:?} (submission {id})\n{log}",
            artifact.display()
        );
    }
    println!("notarization accepted for {label}: {id}");
    Ok(())
}

fn staple_and_validate(artifact: &Path, label: &str) -> Result<()> {
    let mut staple = Command::new("xcrun");
    staple.args(["stapler", "staple"]).arg(artifact);
    command_status(&mut staple, "stapler staple")?;

    let mut validate = Command::new("xcrun");
    validate.args(["stapler", "validate"]).arg(artifact);
    command_status(&mut validate, "stapler validate")?;
    println!("stapled {label}");
    Ok(())
}

fn notary_log(id: &str, credentials: &NotaryCredentials) -> Result<String> {
    if id.is_empty() {
        bail!("notarytool did not return a submission id");
    }
    let mut command = Command::new("xcrun");
    command
        .args(["notarytool", "log", id])
        .args(credentials.arguments());
    let output = command_output(&mut command, "xcrun notarytool log")?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(unix)]
fn create_applications_link(link: &Path) -> Result<()> {
    std::os::unix::fs::symlink("/Applications", link)
        .with_context(|| format!("could not create {}", link.display()))
}

#[cfg(not(unix))]
fn create_applications_link(_: &Path) -> Result<()> {
    bail!("the /Applications link requires a macOS host")
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .with_context(|| format!("could not inspect {}", path.display()))?
        .permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("could not set the mode on {}", path.display()))
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<()> {
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    set_mode(path, 0o755)
}

#[cfg(unix)]
fn random_password() -> Result<String> {
    use std::io::Read;

    let mut bytes = [0u8; 32];
    fs::File::open("/dev/urandom")
        .context("could not open /dev/urandom")?
        .read_exact(&mut bytes)
        .context("could not read /dev/urandom")?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[cfg(not(unix))]
fn random_password() -> Result<String> {
    bail!("creating a temporary keychain requires a macOS host")
}

fn command_status(command: &mut Command, what: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not run {what}"))?;
    if !status.success() {
        bail!("{what} failed with {status}");
    }
    Ok(())
}

fn command_output(command: &mut Command, what: &str) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("could not run {what}"))?;
    if !output.status.success() {
        bail!(
            "{what} failed with {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout).trim_end(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    Ok(output)
}
