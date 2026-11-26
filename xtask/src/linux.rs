// Builds the Linux x86_64 release packages from the native Rune application:
// a Debian package and an AppImage with adjacent zsync metadata.
// `--apt` embeds the signed apt repository source and public key in the deb.

use std::fs::{self, File};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{
    env,
    ffi::{OsStr, OsString},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::apt::metadata::{
    APT_ARCHITECTURE, APT_KEYRING_PATH, APT_PACKAGE, APT_SOURCE_PATH, normalize_apt_base_url,
    render_apt_source,
};

const TARGET: &str = "x86_64-unknown-linux-gnu";
const BINARY: &str = "LitematicaPreview";
const PACKAGE: &str = "litematica-preview";
const ICON_NAME: &str = "dev.litematica.preview";
const DESKTOP_NAME: &str = "dev.litematica.preview.desktop";
const REPOSITORY_OWNER: &str = "Arcadi4";
const REPOSITORY_NAME: &str = "LitematicaPreview";
const REPOSITORY: &str = "Arcadi4/LitematicaPreview";

// AppImageCommunity/AppImageUpdate, release 2.0.0-alpha-1-20251018.
const APPIMAGE_UPDATE_TAG: &str = "2.0.0-alpha-1-20251018";
const APPIMAGE_UPDATE_URL: &str = "https://github.com/AppImageCommunity/AppImageUpdate/releases/download/2.0.0-alpha-1-20251018/AppImageUpdate-x86_64.AppImage";
const APPIMAGE_UPDATE_SHA256: &str =
    "52a58d7edf49eecf62b9d04e574406509cf58f204f0616b72724df03b1f3d37e";
const APPIMAGE_UPDATE_SIZE: u64 = 48_638_456;

// AppImage/appimagetool 1.9.1, downloaded only when packaging an AppImage.
const APPIMAGETOOL_VERSION: &str = "1.9.1";
const APPIMAGETOOL_URL: &str =
    "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage";
const APPIMAGETOOL_SHA256: &str =
    "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0";
const APPIMAGETOOL_SIZE: u64 = 15_092_216;

// Loaded by the native backend at runtime. winit/wgpu/fontique open the X11,
// Wayland, xkbcommon and Vulkan libraries dynamically, so they are not ELF
// dependencies and must be declared explicitly.
const RUNTIME_DEPENDS: &str = "libc6, libgcc-s1, libfontconfig1, libvulkan1, libwayland-client0, libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxkbcommon0, libxkbcommon-x11-0";

struct AptMaterials {
    source: String,
    keyring: Vec<u8>,
    fingerprints: Vec<String>,
}

const USAGE: &str = "\
usage: cargo xtask package-linux [--apt]

Builds the Linux x86_64 release packages from the native app:
  artifacts/LitematicaPreview-<version>-x86_64.deb
  artifacts/LitematicaPreview-<version>-x86_64.AppImage
  artifacts/LitematicaPreview-<version>-x86_64.AppImage.zsync

  --apt   Build the Debian package with the signed apt repository source and
          public key embedded (stable versions only).

environment:
  LP_APT_BASE_URL         Public https URL of the apt repository bucket root
  LP_APT_PUBLIC_KEY_FILE  Armored apt public key file
  LP_APT_PUBLIC_KEY       Armored apt public key text (instead of the file)
  LP_APPIMAGETOOL         Use this appimagetool instead of the pinned download

Without LP_APPIMAGETOOL, appimagetool 1.9.1 is fetched from its GitHub release,
verified with a pinned sha256, and cached under target/packaging/tools/.
";

pub fn run(args: &[OsString]) -> Result<()> {
    if args
        .iter()
        .any(|argument| argument == OsStr::new("-h") || argument == OsStr::new("--help"))
    {
        print!("{USAGE}");
        return Ok(());
    }

    let mut apt_enabled = false;
    for argument in args {
        match argument.to_string_lossy().as_ref() {
            "--" => {}
            "--apt" => apt_enabled = true,
            other => {
                bail!("Unsupported argument: {other}. Usage: cargo xtask package-linux [--apt]")
            }
        }
    }

    if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
        bail!(
            "Linux x86_64 packages must be built on Linux x86_64; this host is {} {}.",
            env::consts::OS,
            env::consts::ARCH
        );
    }

    let root = crate::root();
    let version = crate::version()?;
    if apt_enabled && !version.pre.is_empty() {
        bail!("--apt requires a stable version, but the app crate version is {version}.");
    }

    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts).context("Creating the artifacts directory")?;
    let target_dir = target_dir(&root);
    let package_version = version.to_string();
    let stem = format!("LitematicaPreview-{package_version}-x86_64");
    let deb_path = artifacts.join(format!("{stem}.deb"));
    let appimage_path = artifacts.join(format!("{stem}.AppImage"));
    let zsync_path = adjacent_zsync(&appimage_path);

    for stale in [&deb_path, &appimage_path, &zsync_path] {
        remove_if_present(stale)?;
    }

    println!("Litematica Preview {package_version} - Linux x86_64");
    println!("Target: {TARGET}");
    println!(
        "Apt repository: {}",
        if apt_enabled {
            "embedded"
        } else {
            "not embedded"
        }
    );

    let deb_binary = build(&root, &target_dir, true)?;
    stage_and_build_deb(
        &root,
        &target_dir,
        &deb_binary,
        &version,
        &deb_path,
        apt_enabled,
    )?;

    let appimage_binary = build(&root, &target_dir, false)?;
    build_appimage(
        &root,
        &target_dir,
        &appimage_binary,
        &version,
        &appimage_path,
        &zsync_path,
    )?;

    println!("\nArtifacts:");
    for artifact in [&deb_path, &appimage_path, &zsync_path] {
        report(artifact)?;
    }
    println!(
        "\nInstall the Debian package: sudo apt install ./{}",
        deb_path.file_name().unwrap().to_string_lossy()
    );
    println!(
        "Run the AppImage: chmod +x {} && ./{}",
        appimage_path.file_name().unwrap().to_string_lossy(),
        appimage_path.file_name().unwrap().to_string_lossy()
    );
    Ok(())
}

fn target_dir(root: &Path) -> PathBuf {
    match env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        Some(path) => root.join(path),
        None => root.join("target"),
    }
}

fn build(root: &Path, target_dir: &Path, managed_install: bool) -> Result<PathBuf> {
    println!(
        "\nBuilding {} ({}):",
        BINARY,
        if managed_install {
            "managed-install"
        } else {
            "AppImage"
        }
    );
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "build",
        "--release",
        "--locked",
        "-p",
        PACKAGE,
        "--bin",
        BINARY,
        "--target",
        TARGET,
    ]);
    if managed_install {
        command.args(["--features", "managed-install"]);
    }
    let status = command
        .status()
        .context("Running cargo build; install Rust from https://rustup.rs")?;
    if !status.success() {
        bail!("cargo build failed for {BINARY} (managed-install: {managed_install}).");
    }
    let binary = target_dir.join(TARGET).join("release").join(BINARY);
    require_file(&binary, "Release executable")?;
    set_mode(&binary, 0o755)?;
    Ok(binary)
}

fn stage_and_build_deb(
    root: &Path,
    target_dir: &Path,
    binary: &Path,
    version: &semver::Version,
    deb_path: &Path,
    apt_enabled: bool,
) -> Result<()> {
    println!("\nStaging the Debian package:");
    let staging = target_dir.join("linux").join("deb");
    remove_dir_if_present(&staging)?;

    install_file(binary, &staging.join("usr/bin").join(BINARY), 0o755)?;
    install_file(
        &root.join("app/linux/litematica-preview.desktop"),
        &staging.join("usr/share/applications").join(DESKTOP_NAME),
        0o644,
    )?;
    install_file(
        &root.join("assets/icon.png"),
        &staging
            .join("usr/share/icons/hicolor/1024x1024/apps")
            .join(format!("{ICON_NAME}.png")),
        0o644,
    )?;
    install_file(
        &root.join("app/linux/dev.litematica.preview.xml"),
        &staging
            .join("usr/share/mime/packages")
            .join(format!("{ICON_NAME}.xml")),
        0o644,
    )?;
    let doc_dir = staging.join("usr/share/doc").join(PACKAGE);
    install_file(&root.join("LICENSE"), &doc_dir.join("copyright"), 0o644)?;
    install_file(
        &root.join("app/linux/third-party-notices.txt"),
        &doc_dir.join("third-party-notices.txt"),
        0o644,
    )?;
    install_file(
        &root.join("app/icons/LICENSE"),
        &doc_dir.join("Fluent-UI-System-Icons-LICENSE"),
        0o644,
    )?;

    verify_desktop_entry(&root.join("app/linux/litematica-preview.desktop"))?;

    let apt = if apt_enabled {
        Some(prepare_apt(root, &staging)?)
    } else {
        None
    };

    let control = format!(
        "Package: {APT_PACKAGE}\n\
         Version: {version}\n\
         Architecture: {APT_ARCHITECTURE}\n\
         Maintainer: Arcadi4 <Arcadi4@users.noreply.github.com>\n\
         Section: graphics\n\
         Priority: optional\n\
         Depends: {RUNTIME_DEPENDS}\n\
         Recommends: mesa-vulkan-drivers, xdg-desktop-portal\n\
         Homepage: https://github.com/{REPOSITORY}\n\
         Description: Offline Minecraft schematic and structure viewer\n\
         \x20Litematica Preview renders Litematica schematics and Minecraft structure\n\
         \x20files on the GPU without uploading them.\n"
    );
    let control_dir = staging.join("DEBIAN");
    fs::create_dir_all(&control_dir).context("Creating DEBIAN metadata")?;
    fs::write(control_dir.join("control"), control).context("Writing the deb control file")?;

    let status = Command::new("dpkg-deb")
        .arg("--build")
        .arg("--root-owner-group")
        .arg(&staging)
        .arg(deb_path)
        .status()
        .context("Running dpkg-deb; install dpkg (package: dpkg)")?;
    if !status.success() {
        bail!("dpkg-deb failed to build {}.", deb_path.display());
    }

    verify_deb(deb_path, version, apt.as_ref())?;
    if let Some(apt) = &apt {
        println!(
            "Embedded apt source for {} (key {})",
            apt.source.trim(),
            apt.fingerprints.join(", ")
        );
    }
    Ok(())
}

fn verify_desktop_entry(path: &Path) -> Result<()> {
    let desktop =
        fs::read_to_string(path).with_context(|| format!("Reading {}", path.display()))?;
    if !desktop.contains("Exec=LitematicaPreview --open %F") {
        bail!(
            "{} must launch `LitematicaPreview --open %F`.",
            path.display()
        );
    }
    if !desktop.contains("Icon=dev.litematica.preview") {
        bail!("{} must use Icon=dev.litematica.preview.", path.display());
    }
    for mime in [
        "application/x-litematic",
        "application/x-schematic",
        "application/x-nbt",
        "application/x-snbt",
        "application/x-mcstructure",
        "application/x-mca",
        "application/x-nusn",
    ] {
        if !desktop.contains(mime) {
            bail!("{} does not associate {mime}.", path.display());
        }
    }
    Ok(())
}

fn prepare_apt(root: &Path, staging: &Path) -> Result<AptMaterials> {
    let base_url = env::var("LP_APT_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("LP_APT_BASE_URL is not set; it provides the public apt repository URL.")?;
    let base_url = normalize_apt_base_url(base_url.trim())?;
    let source = render_apt_source(&base_url)?;

    let key = read_apt_public_key(root)?;
    let temporary = tempfile::tempdir().context("Creating an apt keyring workspace")?;
    let keyring_path = temporary.path().join("archive-keyring.gpg");
    write_keyring(&key, &keyring_path)?;
    let fingerprints = inspect_public_key(&keyring_path)?;

    let embedded_keyring = fs::read(&keyring_path).context("Reading the generated apt keyring")?;
    let source_path = staging.join(strip_leading_slash(APT_SOURCE_PATH));
    let keyring_target = staging.join(strip_leading_slash(APT_KEYRING_PATH));
    write_file(&source_path, source.as_bytes(), 0o644)?;
    write_file(&keyring_target, &embedded_keyring, 0o644)?;

    Ok(AptMaterials {
        source,
        keyring: embedded_keyring,
        fingerprints,
    })
}

fn read_apt_public_key(root: &Path) -> Result<Vec<u8>> {
    let file = env::var_os("LP_APT_PUBLIC_KEY_FILE").filter(|value| !value.is_empty());
    let inline = env::var_os("LP_APT_PUBLIC_KEY").filter(|value| !value.is_empty());
    match (file, inline) {
        (Some(_), Some(_)) => {
            bail!("Set only one of LP_APT_PUBLIC_KEY_FILE and LP_APT_PUBLIC_KEY.")
        }
        (Some(path), None) => {
            let path = {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                }
            };
            fs::read(&path)
                .with_context(|| format!("Reading the apt public key from {}", path.display()))
        }
        (None, Some(text)) => {
            let text = text.to_string_lossy().into_owned();
            Ok(text.into_bytes())
        }
        (None, None) => bail!(
            "The apt repository public key is missing. Set LP_APT_PUBLIC_KEY_FILE or LP_APT_PUBLIC_KEY."
        ),
    }
}

fn write_keyring(key: &[u8], target: &Path) -> Result<()> {
    if key.is_empty() {
        bail!("The apt repository public key is empty.");
    }
    let text = String::from_utf8_lossy(key);
    if text.contains("-----BEGIN PGP PRIVATE KEY BLOCK-----") {
        bail!("The apt repository key must be a public key, not a private key.");
    }
    if text.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----") {
        let mut armored = key.to_vec();
        if !armored.ends_with(b"\n") {
            armored.push(b'\n');
        }
        let mut child = Command::new("gpg")
            .args(["--batch", "--yes", "--dearmor", "--output"])
            .arg(target)
            .stdin(Stdio::piped())
            .spawn()
            .context("Running gpg; install GnuPG (package: gnupg)")?;
        child
            .stdin
            .take()
            .context("Opening gpg standard input")?
            .write_all(&armored)
            .context("Passing the apt public key to gpg")?;
        let status = child.wait().context("Waiting for gpg")?;
        if !status.success() {
            bail!("gpg could not convert the apt public key into a keyring.");
        }
        set_mode(target, 0o644)?;
        return Ok(());
    }
    fs::write(target, key).context("Writing the apt keyring")?;
    set_mode(target, 0o644)?;
    Ok(())
}

fn inspect_public_key(keyring: &Path) -> Result<Vec<String>> {
    let output = Command::new("gpg")
        .args(["--batch", "--with-colons", "--show-keys"])
        .arg(keyring)
        .output()
        .context("Running gpg to inspect the apt public key")?;
    if !output.status.success() {
        bail!(
            "gpg could not inspect the apt public key: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let colon = String::from_utf8_lossy(&output.stdout);
    let mut fingerprints = Vec::new();
    let mut expects_fingerprint = false;
    for line in colon.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        match fields.first().copied() {
            Some("sec") | Some("ssb") => {
                bail!("The apt repository key must be a public key, not a private key.")
            }
            Some("pub") => {
                if let Some(validity) = fields.get(1)
                    && matches!(*validity, "e" | "r" | "i" | "d")
                {
                    bail!(
                        "The apt repository public key is not usable (GnuPG status: {validity})."
                    );
                }
                expects_fingerprint = true;
            }
            Some("fpr") if expects_fingerprint => {
                let fingerprint = fields
                    .get(9)
                    .filter(|value| !value.is_empty())
                    .context("GnuPG reported no fingerprint for the apt public key.")?;
                fingerprints.push((*fingerprint).to_string());
                expects_fingerprint = false;
            }
            Some("sub") | Some("uid") => expects_fingerprint = false,
            _ => {}
        }
    }
    if fingerprints.is_empty() {
        bail!("The apt repository public key contains no key.");
    }
    Ok(fingerprints)
}

fn verify_deb(deb: &Path, version: &semver::Version, apt: Option<&AptMaterials>) -> Result<()> {
    for (field, expected) in [
        ("Package", APT_PACKAGE.to_string()),
        ("Architecture", APT_ARCHITECTURE.to_string()),
        ("Version", version.to_string()),
    ] {
        let actual = capture_field(deb, field)?;
        if actual != expected {
            bail!(
                "{} reports {field} '{actual}', expected '{expected}'.",
                deb.display()
            );
        }
    }

    let extracted = tempfile::tempdir().context("Creating a Debian verification workspace")?;
    run_tool(
        "dpkg-deb",
        &[
            OsString::from("-x"),
            deb.as_os_str().to_owned(),
            extracted.path().as_os_str().to_owned(),
        ],
        None,
    )?;
    for required in [
        format!("usr/bin/{BINARY}"),
        format!("usr/share/applications/{DESKTOP_NAME}"),
        format!("usr/share/icons/hicolor/1024x1024/apps/{ICON_NAME}.png"),
        format!("usr/share/mime/packages/{ICON_NAME}.xml"),
    ] {
        let path = extracted.path().join(&required);
        require_file(&path, "Debian package content")?;
    }
    let binary = extracted.path().join("usr/bin").join(BINARY);
    if !is_executable(&binary)? {
        bail!("The Debian package does not mark {BINARY} executable.");
    }
    if extracted
        .path()
        .join("usr/lib/litematica-preview/AppImageUpdate.AppImage")
        .exists()
    {
        bail!("The Debian package must not bundle the AppImage updater.");
    }

    if let Some(apt) = apt {
        let source = extracted.path().join(strip_leading_slash(APT_SOURCE_PATH));
        let keyring = extracted.path().join(strip_leading_slash(APT_KEYRING_PATH));
        let embedded_source =
            fs::read_to_string(&source).with_context(|| format!("Reading {}", source.display()))?;
        if embedded_source != apt.source {
            bail!(
                "The Debian package contains a different apt source than {}.",
                apt.source.trim()
            );
        }
        let embedded_keyring =
            fs::read(&keyring).with_context(|| format!("Reading {}", keyring.display()))?;
        if embedded_keyring != apt.keyring {
            bail!("The Debian package contains a different apt repository keyring.");
        }
        for path in [&source, &keyring] {
            if !is_readable(path)? {
                bail!(
                    "The Debian package makes {} unreadable to apt.",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn build_appimage(
    root: &Path,
    target_dir: &Path,
    binary: &Path,
    version: &semver::Version,
    appimage_path: &Path,
    zsync_path: &Path,
) -> Result<()> {
    println!("\nStaging the AppImage:");
    let appdir = target_dir
        .join("linux")
        .join("appimage")
        .join("LitematicaPreview.AppDir");
    remove_dir_if_present(&appdir)?;

    install_file(binary, &appdir.join("usr/bin").join(BINARY), 0o755)?;
    for destination in [
        appdir.join(DESKTOP_NAME),
        appdir.join("usr/share/applications").join(DESKTOP_NAME),
    ] {
        install_file(
            &root.join("app/linux/litematica-preview.desktop"),
            &destination,
            0o644,
        )?;
    }
    for destination in [
        appdir.join(format!("{ICON_NAME}.png")),
        appdir
            .join("usr/share/icons/hicolor/1024x1024/apps")
            .join(format!("{ICON_NAME}.png")),
    ] {
        install_file(&root.join("assets/icon.png"), &destination, 0o644)?;
    }
    install_file(
        &root.join("app/linux/dev.litematica.preview.xml"),
        &appdir
            .join("usr/share/mime/packages")
            .join(format!("{ICON_NAME}.xml")),
        0o644,
    )?;
    install_file(
        &root.join("app/linux/third-party-notices.txt"),
        &appdir
            .join("usr/share/doc")
            .join(PACKAGE)
            .join("third-party-notices.txt"),
        0o644,
    )?;
    install_file(
        &root.join("app/icons/LICENSE"),
        &appdir
            .join("usr/share/doc")
            .join(PACKAGE)
            .join("Fluent-UI-System-Icons-LICENSE"),
        0o644,
    )?;

    let update_binary = fetch_appimage_update(target_dir)?;
    install_file(
        &update_binary,
        &appdir
            .join("usr/lib/litematica-preview")
            .join("AppImageUpdate.AppImage"),
        0o755,
    )?;

    // AppRun only needs to hand control to the bundled executable; the AppImage
    // runtime exports APPDIR before exec, so a relative symlink is sufficient.
    create_symlink("usr/bin/LitematicaPreview", &appdir.join("AppRun"))?;
    let icon_link = format!("{ICON_NAME}.png");
    create_symlink(&icon_link, &appdir.join(".DirIcon"))?;

    let update_information = update_information(version)?;
    let tool = appimagetool(root, target_dir)?;
    check_appimagetool(&tool)?;

    // zsyncmake writes <AppImage>.zsync into the current directory.
    let artifacts_dir = appimage_path
        .parent()
        .with_context(|| format!("{} has no parent directory", appimage_path.display()))?;
    let mut command = Command::new(&tool);
    command
        .current_dir(artifacts_dir)
        .arg("-n")
        .arg("-u")
        .arg(&update_information)
        .arg(&appdir)
        .arg(appimage_path)
        .env("ARCH", "x86_64")
        .env("VERSION", version.to_string())
        .env("APPIMAGE_EXTRACT_AND_RUN", "1");
    let status = command.status().with_context(|| {
        format!(
            "Running {}; set LP_APPIMAGETOOL to its path",
            tool.display()
        )
    })?;
    if !status.success() {
        bail!(
            "appimagetool failed. Install the zsync package (for zsyncmake) and keep network \
             access for the pinned appimagetool download, or set LP_APPIMAGETOOL."
        );
    }
    set_mode(appimage_path, 0o755)?;
    require_file(zsync_path, "AppImage zsync metadata").with_context(
        || "appimagetool did not write zsync metadata; install the zsync package (zsyncmake).",
    )?;

    verify_appimage(appimage_path, &update_information)?;
    Ok(())
}

fn update_information(version: &semver::Version) -> Result<String> {
    let channel = if version.pre.is_empty() {
        "latest".to_string()
    } else {
        format!("v{version}")
    };
    Ok(format!(
        "gh-releases-zsync|{REPOSITORY_OWNER}|{REPOSITORY_NAME}|{channel}|LitematicaPreview-*-x86_64.AppImage.zsync"
    ))
}

fn appimagetool(root: &Path, target_dir: &Path) -> Result<PathBuf> {
    if let Some(path) = env::var_os("LP_APPIMAGETOOL") {
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        require_file(&path, "appimagetool (LP_APPIMAGETOOL)")?;
        if !is_executable(&path)? {
            bail!(
                "LP_APPIMAGETOOL must point to an executable file: {}",
                path.display()
            );
        }
        return Ok(path);
    }
    let tool = target_dir
        .join("packaging")
        .join("tools")
        .join("appimagetool-x86_64.AppImage");
    fetch_pinned(
        APPIMAGETOOL_URL,
        &tool,
        APPIMAGETOOL_SHA256,
        APPIMAGETOOL_SIZE,
        &format!("appimagetool {APPIMAGETOOL_VERSION}"),
    )?;
    set_mode(&tool, 0o755)?;
    Ok(tool)
}

fn check_appimagetool(tool: &Path) -> Result<()> {
    let output = Command::new(tool)
        .arg("--version")
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => bail!(
            "{} is not usable: {}",
            tool.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => bail!(
            "appimagetool could not run at {}: {error}. Set LP_APPIMAGETOOL to a working copy.",
            tool.display()
        ),
    }
}

fn fetch_appimage_update(target_dir: &Path) -> Result<PathBuf> {
    let cache = target_dir.join("linux-cache").join(format!(
        "AppImageUpdate-{APPIMAGE_UPDATE_TAG}-x86_64.AppImage"
    ));
    fetch_pinned(
        APPIMAGE_UPDATE_URL,
        &cache,
        APPIMAGE_UPDATE_SHA256,
        APPIMAGE_UPDATE_SIZE,
        &format!("AppImageUpdate {APPIMAGE_UPDATE_TAG}"),
    )?;
    Ok(cache)
}

fn fetch_pinned(url: &str, cache: &Path, sha256: &str, size: u64, describe: &str) -> Result<()> {
    if cache.exists() {
        let metadata =
            fs::metadata(cache).with_context(|| format!("Reading {}", cache.display()))?;
        if metadata.len() == size && sha256_file(cache)? == sha256 {
            return Ok(());
        }
        remove_if_present(cache)?;
    }

    let parent = cache
        .parent()
        .context("Resolving the download cache directory")?;
    fs::create_dir_all(parent).context("Creating the download cache directory")?;
    let mut partial_name = cache
        .file_name()
        .context("Resolving the download cache file name")?
        .to_owned();
    partial_name.push(".part");
    let partial = cache.with_file_name(partial_name);
    remove_if_present(&partial)?;

    println!("Downloading {describe}:");
    println!("  {url}");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "3",
            "--retry-all-errors",
            "--connect-timeout",
            "30",
            "--output",
        ])
        .arg(&partial)
        .arg(url)
        .status()
        .context("Downloading a pinned tool requires curl (package: curl)")?;
    if !status.success() {
        remove_if_present(&partial)?;
        bail!("curl could not download {url}.");
    }

    let actual_size = fs::metadata(&partial)
        .with_context(|| format!("Reading {}", partial.display()))?
        .len();
    if actual_size != size {
        remove_if_present(&partial)?;
        bail!("{describe} download is {actual_size} bytes, expected {size}.");
    }
    let digest = sha256_file(&partial)?;
    if digest != sha256 {
        remove_if_present(&partial)?;
        bail!("{describe} download has sha256 {digest}, expected {sha256}.");
    }
    // Rename only after the digest matches so a failed build never caches a
    // partial or tampered tool for the next run.
    fs::rename(&partial, cache).context("Caching the verified download")?;
    Ok(())
}

fn verify_appimage(appimage: &Path, update_information: &str) -> Result<()> {
    let extracted = tempfile::tempdir().context("Creating an AppImage verification workspace")?;
    let status = Command::new(appimage)
        .arg("--appimage-extract")
        .current_dir(extracted.path())
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .status()
        .with_context(|| format!("Extracting {}", appimage.display()))?;
    if !status.success() {
        bail!("Could not extract the generated AppImage for verification.");
    }

    let root = extracted.path().join("squashfs-root");
    for required in [
        "AppRun".to_string(),
        format!("usr/bin/{BINARY}"),
        format!("usr/share/applications/{DESKTOP_NAME}"),
        format!("usr/share/mime/packages/{ICON_NAME}.xml"),
        "usr/lib/litematica-preview/AppImageUpdate.AppImage".to_string(),
    ] {
        let path = root.join(&required);
        if required.starts_with("usr/lib/") {
            require_file(&path, "Bundled AppImageUpdate")?;
        } else if required.as_str() == "AppRun" {
            if !path.exists() {
                bail!("The AppImage does not contain AppRun.");
            }
        } else {
            require_file(&path, "AppImage content")?;
        }
    }
    let apprun = fs::read_link(root.join("AppRun")).context("Reading the AppRun symlink")?;
    if apprun.as_path() != Path::new("usr/bin/LitematicaPreview") {
        bail!(
            "The AppImage AppRun points to {}, expected usr/bin/LitematicaPreview.",
            apprun.display()
        );
    }
    let updater = root.join("usr/lib/litematica-preview/AppImageUpdate.AppImage");
    let digest = sha256_file(&updater)?;
    if digest != APPIMAGE_UPDATE_SHA256 {
        bail!("The bundled AppImageUpdate has sha256 {digest}, expected {APPIMAGE_UPDATE_SHA256}.");
    }
    if !is_executable(&updater)? {
        bail!("The bundled AppImageUpdate is not executable.");
    }

    let output = Command::new("readelf")
        .args(["-p", ".upd_info"])
        .arg(appimage)
        .output()
        .context("Reading the AppImage update information with readelf (package: binutils)")?;
    if !output.status.success() {
        bail!(
            "readelf could not read the AppImage update information: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let section = String::from_utf8_lossy(&output.stdout);
    if !section.contains(update_information) {
        bail!("The AppImage does not embed the expected update information: {update_information}");
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("Opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Hashing {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest.iter().copied() {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(hex)
}

fn strip_leading_slash(path: &str) -> &str {
    path.trim_start_matches('/')
}

fn capture_field(deb: &Path, field: &str) -> Result<String> {
    let output = Command::new("dpkg-deb")
        .arg("-f")
        .arg(deb)
        .arg(field)
        .output()
        .context("Running dpkg-deb; install dpkg (package: dpkg)")?;
    if !output.status.success() {
        bail!(
            "dpkg-deb -f {} {field} failed: {}",
            deb.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_tool(tool: &str, args: &[OsString], cwd: Option<&Path>) -> Result<()> {
    let mut command = Command::new(tool);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let status = command
        .status()
        .with_context(|| format!("Running {tool}"))?;
    if !status.success() {
        bail!("{tool} failed with {status}.");
    }
    Ok(())
}

fn install_file(source: &Path, destination: &Path, mode: u32) -> Result<()> {
    require_file(source, "Build input")?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Creating {}", parent.display()))?;
    }
    fs::copy(source, destination)
        .with_context(|| format!("Copying {} to {}", source.display(), destination.display()))?;
    set_mode(destination, mode)
}

fn write_file(destination: &Path, contents: &[u8], mode: u32) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Creating {}", parent.display()))?;
    }
    fs::write(destination, contents)
        .with_context(|| format!("Writing {}", destination.display()))?;
    set_mode(destination, mode)
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("Setting permissions on {}", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

fn create_symlink(target: &str, link: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("Creating the symlink {}", link.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (target, link);
        bail!("symbolic links are required to build Linux packages")
    }
}

fn is_executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        Ok(fs::metadata(path)
            .with_context(|| format!("Reading {}", path.display()))?
            .permissions()
            .mode()
            & 0o111
            != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(true)
    }
}

fn is_readable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        Ok(fs::metadata(path)
            .with_context(|| format!("Reading {}", path.display()))?
            .permissions()
            .mode()
            & 0o444
            == 0o444)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(true)
    }
}

fn require_file(path: &Path, describe: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("{describe} is missing: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{describe} is not a file: {}", path.display());
    }
    if metadata.len() == 0 {
        bail!("{describe} is empty: {}", path.display());
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Removing {}", path.display())),
    }
}

fn remove_dir_if_present(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).with_context(|| format!("Removing {}", path.display()))?;
    }
    Ok(())
}

fn adjacent_zsync(appimage: &Path) -> PathBuf {
    let mut name = appimage.as_os_str().to_owned();
    name.push(".zsync");
    PathBuf::from(name)
}

fn report(path: &Path) -> Result<()> {
    let size = fs::metadata(path)
        .with_context(|| format!("Reading {}", path.display()))?
        .len();
    println!(
        "  {}\n    {:.2} MiB ({} bytes)",
        path.display(),
        size as f64 / (1024.0 * 1024.0),
        size
    );
    Ok(())
}
