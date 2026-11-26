mod apt;
mod homebrew;
mod linux;
mod macos;
mod release;
mod windows;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use semver::Version;

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  package-windows [--allow-unsigned] [--binary PATH]
  package-linux [--apt]
  package-macos <aarch64|x86_64> [--allow-unsigned] [--binary PATH]
  publish-homebrew vX.Y.Z [--repair]
  apt-publish --deb FILE [--dry-run]
  release-check vX.Y.Z[-pre]
  release-publish vX.Y.Z[-pre]
  release-apt vX.Y.Z
  checksums [--win] [--mac] [--linux]
  version [major|minor|patch|X.Y.Z]

Run `cargo xtask <command> --help` for command-specific help.";

fn main() {
    if let Err(error) = run() {
        eprintln!("xtask: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(command) = args.next() else {
        println!("{USAGE}");
        return Ok(());
    };
    let command = command
        .to_str()
        .context("the command name must be valid UTF-8")?;
    let rest: Vec<OsString> = args.collect();

    match command {
        "package-windows" => windows::run(&rest),
        "package-linux" => linux::run(&rest),
        "package-macos" => macos::run(&rest),
        "publish-homebrew" => homebrew::run(&rest),
        "apt-publish" => apt::run(&rest),
        "release-check" => release::check(&rest),
        "release-publish" => release::publish(&rest),
        "release-apt" => release::publish_apt(&rest),
        "checksums" => release::checksums(&rest),
        "version" => version_command(&rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

/// Workspace root, independent of the directory the command was invoked from.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives directly under the workspace root")
        .to_path_buf()
}

/// The app crate's `[package] version`, the single source of release versions.
pub fn version() -> Result<Version> {
    Ok(read_package(&manifest_path())?.version)
}

fn manifest_path() -> PathBuf {
    root().join("app").join("Cargo.toml")
}

struct Package {
    name: String,
    version: Version,
}

fn read_package(path: &Path) -> Result<Package> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let document: toml::Value =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let package = document
        .get("package")
        .and_then(toml::Value::as_table)
        .with_context(|| format!("{} has no [package] table", path.display()))?;
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} has no package.name", path.display()))?
        .to_owned();
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .with_context(|| format!("{} has no package.version", path.display()))?;
    let version = Version::parse(version).with_context(|| {
        format!(
            "{} package.version is not valid semver: {version}",
            path.display()
        )
    })?;
    Ok(Package { name, version })
}

fn version_command(args: &[OsString]) -> Result<()> {
    match args {
        [] => {
            println!("{}", version()?);
            Ok(())
        }
        [argument] => {
            let argument = argument
                .to_str()
                .context("the version must be valid UTF-8")?;
            if matches!(argument, "-h" | "--help") {
                println!(
                    "usage: cargo xtask version [major|minor|patch|X.Y.Z]\n\n\
                     With no argument, prints the app version. With an argument, writes it to\n\
                     app/Cargo.toml and regenerates the app lockfile entry."
                );
                return Ok(());
            }
            let manifest = manifest_path();
            let package = read_package(&manifest)?;
            let next = match argument {
                "major" => Version::new(package.version.major + 1, 0, 0),
                "minor" => Version::new(package.version.major, package.version.minor + 1, 0),
                "patch" => Version::new(
                    package.version.major,
                    package.version.minor,
                    package.version.patch + 1,
                ),
                other => Version::parse(other.strip_prefix('v').unwrap_or(other))
                    .with_context(|| format!("invalid semantic version: {other}"))?,
            };
            write_version(&manifest, &package.version, &next)?;
            regenerate_lock(&manifest, &package.name)?;
            println!("{next}");
            Ok(())
        }
        _ => bail!("usage: cargo xtask version [major|minor|patch|X.Y.Z]"),
    }
}

/// Replaces only the `version` value in `[package]`; the rest of the manifest,
/// including comments and formatting, is preserved.
fn write_version(manifest: &Path, current: &Version, next: &Version) -> Result<()> {
    let current_text = current.to_string();
    let next_text = next.to_string();
    let literal = format!("\"{current_text}\"");
    let replacement = format!("\"{next_text}\"");
    let text =
        fs::read_to_string(manifest).with_context(|| format!("reading {}", manifest.display()))?;

    let mut offset = 0;
    let mut in_package = false;
    let mut range = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let section = trimmed.split('#').next().unwrap_or_default().trim_end();
        if section.starts_with('[') {
            in_package = section == "[package]";
        }
        if in_package && range.is_none() {
            let key = trimmed.split('#').next().unwrap_or_default();
            let is_version = key
                .strip_prefix("version")
                .is_some_and(|rest| rest.trim_start().starts_with('='));
            if is_version {
                let position = line.find(&literal).with_context(|| {
                    format!(
                        "{} does not declare package.version as {literal}",
                        manifest.display()
                    )
                })?;
                range = Some((offset + position, offset + position + literal.len()));
            }
        }
        offset += line.len();
    }

    let (start, end) =
        range.with_context(|| format!("{} has no version in [package]", manifest.display()))?;
    let mut updated = String::with_capacity(text.len() + replacement.len());
    updated.push_str(&text[..start]);
    updated.push_str(&replacement);
    updated.push_str(&text[end..]);

    let document: toml::Value = toml::from_str(&updated)
        .with_context(|| format!("parsing the updated {}", manifest.display()))?;
    let written = document
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str);
    ensure!(
        written == Some(next_text.as_str()),
        "{} still declares a different package.version after the rewrite",
        manifest.display()
    );

    fs::write(manifest, updated).with_context(|| format!("writing {}", manifest.display()))
}

/// `cargo update` rewrites the workspace lockfile for the app package only.
/// A local core checkout must not leak into the committed lockfile, so the
/// override variable is removed from the child environment.
fn regenerate_lock(manifest: &Path, package: &str) -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let status = Command::new(&cargo)
        .arg("update")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--package")
        .arg(package)
        .env_remove("LP_CORE_PATH")
        .current_dir(root())
        .status()
        .with_context(|| format!("running {}", cargo.to_string_lossy()))?;
    ensure!(status.success(), "cargo update failed ({status})");
    Ok(())
}
