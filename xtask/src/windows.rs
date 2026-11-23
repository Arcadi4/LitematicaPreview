//! Windows packaging: one native release build, Authenticode signing, the
//! portable ZIP, and the Inno Setup installer declared in
//! `packaging/windows/setup.iss`.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use anyhow::{Context, Result, bail, ensure};
use tempfile::Builder as TempBuilder;

const TARGET: &str = "x86_64-pc-windows-msvc";
const BINARY_NAME: &str = "LitematicaPreview.exe";
const PACKAGE_NAME: &str = "LitematicaPreview";
const TIMESTAMP_URL: &str = "http://timestamp.acs.microsoft.com";
const AZURE_DLIB: &str = "Azure.CodeSigning.Dlib.dll";

const USAGE: &str = "\
usage: cargo xtask package-windows [--allow-unsigned] [--binary PATH]

Builds LitematicaPreview.exe for x86_64-pc-windows-msvc exactly once, signs
the executable, packages artifacts/LitematicaPreview-windows-x86_64.exe,
artifacts/LitematicaPreview-<version>-x86_64-portable.zip, and
artifacts/LitematicaPreview-<version>-x86_64-setup.exe.

  --binary PATH      Package an existing executable instead of building it.
  --allow-unsigned   Local verification only: skip Authenticode signing.
                     Refused when CI or GITHUB_ACTIONS is set.";

/// Entry point used by `xtask` and the `package-windows` command.
pub fn run(args: &[OsString]) -> Result<()> {
    if args
        .iter()
        .any(|argument| argument == OsStr::new("--help") || argument == OsStr::new("-h"))
    {
        println!("{USAGE}");
        return Ok(());
    }

    let options = Options::parse(args)?;
    ensure!(
        env::consts::OS == "windows",
        "Windows packages can only be produced on Windows."
    );
    if is_ci() && options.allow_unsigned {
        bail!("--allow-unsigned is refused in CI; a release package must be Authenticode signed");
    }

    let root: PathBuf = crate::root();
    let version = crate::version()?;
    let version_text = version.to_string();
    let numeric_version = format!("{}.{}.{}.0", version.major, version.minor, version.patch);

    let signing = if options.allow_unsigned {
        Signing::Unsigned
    } else {
        Signing::from_env()?
    };
    let signtool = match &signing {
        Signing::Unsigned => None,
        _ => Some(find_signtool().context(
            "signtool.exe was not found. Install the Windows 10/11 SDK \
             (Windows Kits\\10\\bin\\<version>\\x64\\signtool.exe).",
        )?),
    };

    step("Preparing the output directories");
    let artifacts = root.join("artifacts");
    let stage = artifacts.join("x86_64");
    fs::create_dir_all(&artifacts).with_context(|| format!("create {}", artifacts.display()))?;
    if stage.exists() {
        fs::remove_dir_all(&stage).with_context(|| format!("clear {}", stage.display()))?;
    }
    fs::create_dir_all(&stage).with_context(|| format!("create {}", stage.display()))?;

    let setup = artifacts.join(format!("{PACKAGE_NAME}-{version_text}-x86_64-setup.exe"));
    let portable = artifacts.join(format!("{PACKAGE_NAME}-{version_text}-x86_64-portable.zip"));
    let raw = artifacts.join("LitematicaPreview-windows-x86_64.exe");
    for stale in [&setup, &portable] {
        if stale.exists() {
            fs::remove_file(stale).with_context(|| format!("remove {}", stale.display()))?;
        }
    }

    step("Locating the release executable");
    let input = match options.binary {
        Some(path) => {
            let path = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            ensure!(
                path.is_file(),
                "the --binary path is not a file: {}",
                path.display()
            );
            path
        }
        None => build_binary(&root)?,
    };
    let input = fs::canonicalize(&input)
        .with_context(|| format!("resolve the executable {}", input.display()))?;

    // The raw asset and the executable embedded in the installer are the same
    // signed bytes, so sign once before either copy is made.
    if !raw.exists() || fs::canonicalize(&raw).ok().as_deref() != Some(input.as_path()) {
        fs::copy(&input, &raw)
            .with_context(|| format!("copy {} to {}", input.display(), raw.display()))?;
    }

    if let (Some(signtool), signing @ (Signing::Params(_) | Signing::Azure { .. })) =
        (signtool.as_deref(), &signing)
    {
        step("Signing the executable");
        sign_file(signtool, signing, &raw)?;
        verify_signature(signtool, &raw)?;
    } else {
        println!("WARNING: packaging an unsigned executable; do not publish these artifacts.");
    }

    let staged_exe = stage.join(BINARY_NAME);
    fs::copy(&raw, &staged_exe)
        .with_context(|| format!("stage the executable at {}", staged_exe.display()))?;

    step("Staging the license and notices");
    let license = root.join("LICENSE");
    ensure!(
        license.is_file(),
        "the project license is missing: {}",
        license.display()
    );
    fs::copy(&license, stage.join("LICENSE"))
        .with_context(|| format!("stage {}", license.display()))?;
    fs::copy(
        root.join("app/icons/LICENSE"),
        stage.join("Fluent-UI-System-Icons-LICENSE.txt"),
    )
    .context("stage the Fluent UI System Icons license")?;

    step("Writing the portable ZIP");
    write_zip(&stage, &portable)?;

    step("Compiling the Inno Setup installer");
    let iscc = find_iscc().context(
        "ISCC.exe was not found. Install Inno Setup 6 or newer on the runner \
         (for example: choco install innosetup --yes --no-progress).",
    )?;
    let script = root.join("packaging/windows/setup.iss");
    ensure!(
        script.is_file(),
        "the installer script is missing: {}",
        script.display()
    );
    // Inno requires a .txt or .rtf license file; the packaged copy keeps the
    // project's LICENSE name.
    let mut license_file = TempBuilder::new()
        .prefix("litematica-license-")
        .suffix(".txt")
        .tempfile()
        .context("create the installer license file")?;
    license_file.as_file_mut().write_all(&fs::read(&license)?)?;
    license_file.as_file_mut().flush()?;
    let status = Command::new(&iscc)
        .current_dir(&root)
        .arg(format!("/DAppVersion={version_text}"))
        .arg(format!("/DNumericVersion={numeric_version}"))
        .arg(format!("/DStageDir={}", stage.display()))
        .arg(format!("/DOutputDir={}", artifacts.display()))
        .arg(format!("/DLicenseFile={}", license_file.path().display()))
        .arg(&script)
        .status()
        .with_context(|| format!("run {}", iscc.display()))?;
    ensure!(
        status.success(),
        "Inno Setup failed with {}",
        status_text(status)
    );
    ensure!(
        setup.is_file(),
        "Inno Setup did not write the expected installer: {}",
        setup.display()
    );

    if let (Some(signtool), signing @ (Signing::Params(_) | Signing::Azure { .. })) =
        (signtool.as_deref(), &signing)
    {
        step("Signing the installer");
        sign_file(signtool, signing, &setup)?;
        verify_signature(signtool, &setup)?;
    }

    step("Packaged");
    for artifact in [&setup, &portable, &raw] {
        let size = fs::metadata(artifact)
            .with_context(|| format!("read {}", artifact.display()))?
            .len();
        println!(
            "{}\n  {:.2} MiB ({} bytes)",
            artifact.display(),
            size as f64 / (1024.0 * 1024.0),
            size
        );
    }
    if matches!(&signing, Signing::Unsigned) {
        println!("WARNING: these packages are unsigned. Do not publish them.");
    }
    Ok(())
}

struct Options {
    binary: Option<PathBuf>,
    allow_unsigned: bool,
}

impl Options {
    fn parse(args: &[OsString]) -> Result<Self> {
        let mut binary = None;
        let mut allow_unsigned = false;
        let mut args = args.iter();
        while let Some(argument) = args.next() {
            let Some(value) = argument.to_str() else {
                bail!("package-windows arguments must be valid UTF-8");
            };
            match value {
                "--allow-unsigned" => allow_unsigned = true,
                "--" => {}
                "--binary" => {
                    let path = args
                        .next()
                        .with_context(|| "--binary requires a path after it")?;
                    set_binary(&mut binary, PathBuf::from(path.clone()))?;
                }
                _ if value.starts_with("--binary=") => {
                    let path = &value["--binary=".len()..];
                    ensure!(!path.is_empty(), "--binary requires a path");
                    set_binary(&mut binary, PathBuf::from(path))?;
                }
                other => bail!("unexpected argument {other:?}\n\n{USAGE}"),
            }
        }
        Ok(Self {
            binary,
            allow_unsigned,
        })
    }
}

fn set_binary(slot: &mut Option<PathBuf>, value: PathBuf) -> Result<()> {
    ensure!(
        slot.replace(value).is_none(),
        "--binary was given more than once"
    );
    Ok(())
}

fn build_binary(root: &Path) -> Result<PathBuf> {
    step("Building the release executable");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(&cargo);
    command
        .current_dir(root)
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            TARGET,
            "--bin",
        ])
        .arg("LitematicaPreview");
    let rustflags = env::var("RUSTFLAGS").unwrap_or_default();
    if !rustflags.contains("target-feature=+crt-static") {
        let combined = format!("{rustflags} -C target-feature=+crt-static");
        command.env("RUSTFLAGS", combined.trim());
    }
    let status = command
        .status()
        .with_context(|| format!("run {}", cargo.to_string_lossy()))?;
    ensure!(
        status.success(),
        "cargo build failed with {}. Install the MSVC build tools, the Windows SDK, \
         and run: rustup target add {TARGET}",
        status_text(status)
    );

    let executable = target_dir(root)
        .join(TARGET)
        .join("release")
        .join(BINARY_NAME);
    ensure!(
        executable.is_file(),
        "the release executable is missing: {}",
        executable.display()
    );
    Ok(executable)
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

fn write_zip(directory: &Path, destination: &Path) -> Result<()> {
    let file =
        File::create(destination).with_context(|| format!("create {}", destination.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("read {}", directory.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().replace('\\', "/");
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file(name, options)?;
        let mut source =
            File::open(entry.path()).with_context(|| format!("read {}", entry.path().display()))?;
        io::copy(&mut source, &mut writer)
            .with_context(|| format!("archive {}", entry.path().display()))?;
    }
    writer
        .finish()
        .with_context(|| format!("finish {}", destination.display()))?;
    ensure!(
        fs::metadata(destination)?.len() > 0,
        "the portable ZIP is empty: {}",
        destination.display()
    );
    Ok(())
}

enum Signing {
    Params(Vec<String>),
    Azure { metadata: String, dlib: PathBuf },
    Unsigned,
}

impl Signing {
    fn from_env() -> Result<Self> {
        let params = env_nonempty("LITEMATICA_WINDOWS_SIGN_PARAMS");
        let metadata = env_nonempty("LITEMATICA_WINDOWS_AZURE_SIGN_METADATA");
        if params.is_some() && metadata.is_some() {
            bail!(
                "set only one of LITEMATICA_WINDOWS_SIGN_PARAMS and \
                 LITEMATICA_WINDOWS_AZURE_SIGN_METADATA"
            );
        }
        if let Some(params) = params {
            let arguments = split_windows_arguments(&params);
            ensure!(
                !arguments.is_empty(),
                "LITEMATICA_WINDOWS_SIGN_PARAMS contains no signtool arguments"
            );
            return Ok(Self::Params(arguments));
        }
        if let Some(metadata) = metadata {
            for name in ["AZURE_CLIENT_ID", "AZURE_TENANT_ID", "AZURE_CLIENT_SECRET"] {
                ensure!(
                    env_nonempty(name).is_some(),
                    "{name} is required by LITEMATICA_WINDOWS_AZURE_SIGN_METADATA"
                );
            }
            let metadata = read_azure_metadata(&metadata)?;
            let dlib = find_azure_dlib()?;
            return Ok(Self::Azure { metadata, dlib });
        }
        bail!(
            "Windows packages must be code-signed. Set LITEMATICA_WINDOWS_SIGN_PARAMS, \
             or LITEMATICA_WINDOWS_AZURE_SIGN_METADATA with AZURE_CLIENT_ID, \
             AZURE_TENANT_ID, and AZURE_CLIENT_SECRET. For a local unsigned test \
             build only, pass --allow-unsigned. Unsigned packaging is refused in CI."
        );
    }
}

fn read_azure_metadata(value: &str) -> Result<String> {
    let candidate = Path::new(value);
    let contents = if candidate.is_file() {
        fs::read_to_string(candidate)
            .with_context(|| format!("read the Azure signing metadata {}", candidate.display()))?
    } else if value.trim_start().starts_with('{') {
        value.to_string()
    } else {
        bail!(
            "LITEMATICA_WINDOWS_AZURE_SIGN_METADATA is neither an existing file nor \
             a JSON document: {value}"
        );
    };
    let document: serde_json::Value = serde_json::from_str(&contents)
        .context("LITEMATICA_WINDOWS_AZURE_SIGN_METADATA is not valid JSON")?;
    for key in [
        "Endpoint",
        "CodeSigningAccountName",
        "CertificateProfileName",
    ] {
        let present = document
            .as_object()
            .map(|object| {
                object.iter().any(|(name, value)| {
                    name.eq_ignore_ascii_case(key)
                        && value.as_str().is_some_and(|text| !text.is_empty())
                })
            })
            .unwrap_or(false);
        ensure!(present, "the Azure signing metadata is missing {key}");
    }
    Ok(contents)
}

fn find_azure_dlib() -> Result<PathBuf> {
    if let Some(value) = env_nonempty("LITEMATICA_WINDOWS_AZURE_SIGN_DLIB") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        if path.is_dir() {
            let candidate = path.join(AZURE_DLIB);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        bail!(
            "LITEMATICA_WINDOWS_AZURE_SIGN_DLIB does not name {}: {}",
            AZURE_DLIB,
            path.display()
        );
    }
    if let Some(home) = env::var_os("USERPROFILE").map(PathBuf::from) {
        let packages = home.join(".nuget/packages/microsoft.artifactsigning.client");
        if let Ok(versions) = fs::read_dir(packages)
            && let Some(dlib) = versions
                .flatten()
                .map(|entry| entry.path().join("bin/x64").join(AZURE_DLIB))
                .find(|path| path.is_file())
        {
            return Ok(dlib);
        }
    }
    bail!(
        "could not find x64 {AZURE_DLIB}. Install Microsoft.ArtifactSigning.Client via \
         NuGet, or set LITEMATICA_WINDOWS_AZURE_SIGN_DLIB to its x64 DLL."
    )
}

fn sign_file(signtool: &Path, signing: &Signing, target: &Path) -> Result<()> {
    let mut command = Command::new(signtool);
    command.arg("sign");
    // Holding the temporary metadata file until signtool exits keeps its path
    // valid without a separate cleanup branch.
    let _metadata = match signing {
        Signing::Params(arguments) => {
            command.args(arguments.iter().map(String::as_str));
            None
        }
        Signing::Azure {
            metadata: document,
            dlib,
        } => {
            // Write the metadata without a BOM; signtool reads it directly.
            let mut file = TempBuilder::new()
                .prefix("litematica-signing-")
                .suffix(".json")
                .tempfile()
                .context("create the Azure signing metadata file")?;
            file.as_file_mut().write_all(document.as_bytes())?;
            file.as_file_mut().flush()?;
            command
                .args([
                    "/fd",
                    "SHA256",
                    "/td",
                    "SHA256",
                    "/tr",
                    TIMESTAMP_URL,
                    "/dlib",
                ])
                .arg(dlib.as_os_str())
                .arg("/dmdf")
                .arg(file.path());
            Some(file)
        }
        Signing::Unsigned => return Ok(()),
    };
    command.arg(target);
    let output = command
        .output()
        .with_context(|| format!("run {}", signtool.display()))?;
    if !output.status.success() {
        bail!(
            "signtool sign failed with {}\n{}{}",
            status_text(output.status),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn verify_signature(signtool: &Path, target: &Path) -> Result<()> {
    let output = Command::new(signtool)
        .arg("verify")
        .arg("/pa")
        .arg("/all")
        .arg(target)
        .output()
        .with_context(|| format!("run {} verify", signtool.display()))?;
    ensure!(
        output.status.success(),
        "the Authenticode signature on {} is not valid:\n{}{}",
        target.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn find_signtool() -> Option<PathBuf> {
    if let Some(found) = find_on_path("signtool.exe") {
        return Some(found);
    }
    for base in [
        env::var_os("ProgramFiles(x86)"),
        env::var_os("ProgramFiles"),
    ]
    .into_iter()
    .flatten()
    {
        let kits = PathBuf::from(base).join("Windows Kits/10/bin");
        let mut versions = match fs::read_dir(&kits) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>(),
            Err(_) => continue,
        };
        versions.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
        for version in versions {
            let candidate = version.join("x64/signtool.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn find_iscc() -> Option<PathBuf> {
    if let Some(found) = find_on_path("ISCC.exe") {
        return Some(found);
    }
    let mut bases = [
        env::var_os("ProgramFiles(x86)"),
        env::var_os("ProgramFiles"),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::from)
    .collect::<Vec<_>>();
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        bases.push(PathBuf::from(local).join("Programs"));
    }
    for base in bases {
        for version in ["Inno Setup 8", "Inno Setup 7", "Inno Setup 6"] {
            let candidate = base.join(version).join("ISCC.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_ci() -> bool {
    ["CI", "GITHUB_ACTIONS", "TF_BUILD"]
        .into_iter()
        .any(env_truthy)
}

fn env_truthy(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn status_text(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "an unknown signal".to_string())
}

/// Splits arguments with the Windows command-line rules, preserving quoted
/// paths and passwords in `LITEMATICA_WINDOWS_SIGN_PARAMS`.
fn split_windows_arguments(input: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut backslashes = 0usize;
    let mut present = false;

    for character in input.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            for _ in 0..backslashes / 2 {
                current.push('\\');
            }
            if backslashes.is_multiple_of(2) {
                quoted = !quoted;
            } else {
                current.push('"');
            }
            backslashes = 0;
            present = true;
            continue;
        }
        for _ in 0..backslashes {
            current.push('\\');
        }
        backslashes = 0;
        if !quoted && character.is_whitespace() {
            if present {
                arguments.push(std::mem::take(&mut current));
                present = false;
            }
        } else {
            current.push(character);
            present = true;
        }
    }
    for _ in 0..backslashes {
        current.push('\\');
    }
    if present {
        arguments.push(current);
    }
    arguments
}

fn step(message: &str) {
    println!("\n[{message}]");
}

#[cfg(test)]
mod tests {
    use super::split_windows_arguments;

    #[test]
    fn keeps_quoted_paths_and_passwords_together() {
        assert_eq!(
            split_windows_arguments(r#"/f "C:\Program Files\cert.pfx" /p "a b""#),
            vec!["/f", r"C:\Program Files\cert.pfx", "/p", "a b"]
        );
    }

    #[test]
    fn keeps_literal_quotes_and_trailing_backslashes() {
        assert_eq!(
            split_windows_arguments(r#"/d "C:\path\with\\" tail"#),
            vec!["/d", r#"C:\path\with\"#, "tail"]
        );
    }
}
