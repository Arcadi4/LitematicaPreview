//! Platform-specific pieces of the updater.
//!
//! Windows replaces the signed standalone executable through `self_update`.
//! macOS installs the whole signed/notarized `.app` from the release DMG.
//! Linux only hands the manual update off to the AppImageUpdate GUI bundled in
//! the AppImage.

#[cfg(not(target_os = "macos"))]
use std::path::PathBuf;

/// The release asset published for this build and version.
pub(super) fn asset_name(_version: &str) -> Result<String, String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok("LitematicaPreview-windows-x86_64.exe".to_string())
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok(format!("LitematicaPreview-{_version}-aarch64.dmg"))
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Ok(format!("LitematicaPreview-{_version}-x86_64.dmg"))
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        Err(format!(
            "No update asset is published for {}-{}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    }
}

/// The executable `self_update` replaces on Windows (and any non-macOS
/// self-updating build).
#[cfg(not(target_os = "macos"))]
pub(super) fn install_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("Could not locate the running executable: {error}"))
}

/// Replace the whole macOS app bundle from the signed, notarized release DMG.
#[cfg(target_os = "macos")]
pub(super) fn install_macos(release: &super::Release) -> Result<(), String> {
    macos::install(release)
}

/// Run the AppImageUpdate GUI bundled inside the running AppImage.
///
/// Only the AppImage runtime's own `$APPIMAGE` and `$APPDIR` are accepted. The
/// nested AppImage is started with `APPIMAGE_EXTRACT_AND_RUN=1` so it does not
/// need FUSE inside the outer mount, and the outer runtime's variables are
/// removed so the nested runtime does not mistake itself for the outer app.
#[cfg(target_os = "linux")]
pub(super) fn appimage_update() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let appimage = runtime_appimage()?;
    let appdir = runtime_appdir()?;
    let tool = appdir.join("usr/lib/litematica-preview/AppImageUpdate.AppImage");
    let metadata = std::fs::metadata(&tool).map_err(|error| {
        format!(
            "The bundled AppImageUpdate is missing at {}: {error}. Reinstall this AppImage or \
             update it manually.",
            tool.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "The bundled AppImageUpdate at {} is not a file.",
            tool.display()
        ));
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!(
            "The bundled AppImageUpdate at {} is not executable.",
            tool.display()
        ));
    }

    let status = std::process::Command::new(&tool)
        .arg(&appimage)
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .env_remove("APPIMAGE")
        .env_remove("APPDIR")
        .env_remove("ARGV0")
        .env_remove("OWD")
        .status()
        .map_err(|error| {
            format!(
                "Could not start the bundled AppImageUpdate at {}: {error}",
                tool.display()
            )
        })?;
    if !status.success() {
        return Err(format!("AppImageUpdate exited with {status}."));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn appimage_update() -> Result<(), String> {
    Err("AppImage updates are only available on Linux.".to_string())
}

#[cfg(target_os = "linux")]
fn runtime_appimage() -> Result<PathBuf, String> {
    let value = environment_path("APPIMAGE", "This copy is not running from an AppImage")?;
    if !value.is_file() {
        return Err(format!(
            "The APPIMAGE path {} is not a file.",
            value.display()
        ));
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn runtime_appdir() -> Result<PathBuf, String> {
    let value = environment_path("APPDIR", "This copy is not running from a mounted AppImage")?;
    if !value.is_dir() {
        return Err(format!(
            "The APPDIR path {} is not a directory.",
            value.display()
        ));
    }
    Ok(value)
}

#[cfg(target_os = "linux")]
fn environment_path(name: &str, context: &str) -> Result<PathBuf, String> {
    let value = std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{context} ({name} is not set)."))?;
    std::fs::canonicalize(&value).map_err(|error| {
        format!(
            "The {name} path {} is not accessible: {error}",
            value.display()
        )
    })
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CString, OsStr};
    use std::io::Read;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use sha2::{Digest, Sha256};

    use crate::update::{DOWNLOAD_TIMEOUT, Release, current_version};

    const CODESIGN: &str = "/usr/bin/codesign";
    const DITTO: &str = "/usr/bin/ditto";
    const HDIUTIL: &str = "/usr/bin/hdiutil";
    const SPCTL: &str = "/usr/sbin/spctl";
    const BUNDLE_IDENTIFIER: &str = "dev.litematica.preview";
    const MAX_DMG_BYTES: u64 = 1024 * 1024 * 1024;

    pub(super) fn install(release: &Release) -> Result<(), String> {
        let running = running_bundle()?;

        let dmg_dir = tempfile::Builder::new()
            .prefix("litematica-preview-dmg-")
            .tempdir()
            .map_err(|error| {
                format!("Could not create a temporary directory for the macOS update: {error}")
            })?;
        let dmg = dmg_dir.path().join("LitematicaPreview.dmg");
        download_dmg(release, &dmg)?;
        verify_sha256(&dmg, &release.sha256)?;

        let mount_dir = tempfile::Builder::new()
            .prefix("litematica-preview-mount-")
            .tempdir()
            .map_err(|error| format!("Could not create a temporary mount point: {error}"))?;
        let mount = Mount::attach(&dmg, mount_dir.path())?;
        let running_name = running
            .file_name()
            .ok_or_else(|| "The running app bundle has no file name.".to_string())?;
        let mounted_app = find_mounted_app(mount.path(), running_name)?;
        verify_signed_app(&running, &mounted_app)?;
        copy_and_swap(&running, &mounted_app)?;
        drop(mount);
        Ok(())
    }

    fn download_dmg(release: &Release, destination: &Path) -> Result<(), String> {
        let mut file = std::fs::File::create(destination)
            .map_err(|error| format!("Could not create the temporary DMG: {error}"))?;
        self_update::Download::from_url(release.url.as_str())
            .request_header("Accept", "application/octet-stream")
            .request_header("Cache-Control", "no-cache")
            .request_header(
                "User-Agent",
                format!("LitematicaPreview/{}", current_version()),
            )
            .timeout(DOWNLOAD_TIMEOUT)
            .max_download_size(MAX_DMG_BYTES)
            .download_to(&mut file)
            .map_err(|error| format!("Could not download the macOS update DMG: {error}"))?;
        Ok(())
    }

    fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
        let mut file = std::fs::File::open(path)
            .map_err(|error| format!("Could not open the downloaded DMG: {error}"))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("Could not read the downloaded DMG: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let actual = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{:02x}", *byte))
            .collect::<String>();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(
                "The downloaded macOS update DMG does not match its published SHA-256.".to_string(),
            );
        }
        Ok(())
    }

    fn running_bundle() -> Result<PathBuf, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("Could not locate the running executable: {error}"))?;
        let executable = std::fs::canonicalize(&executable)
            .map_err(|error| format!("Could not resolve the running executable: {error}"))?;
        executable
            .ancestors()
            .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("app"))
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                "This copy is not inside a macOS .app bundle, so the signed DMG update cannot be \
                 applied."
                    .to_string()
            })
    }

    fn find_mounted_app(mount_root: &Path, running_name: &OsStr) -> Result<PathBuf, String> {
        let candidate = mount_root.join(Path::new(running_name));
        if candidate.is_dir() {
            return Ok(candidate);
        }

        let mut apps = Vec::new();
        let entries = std::fs::read_dir(mount_root)
            .map_err(|error| format!("Could not read the mounted update DMG: {error}"))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Could not read the mounted update DMG: {error}"))?;
            let path = entry.path();
            if path.is_dir()
                && path.extension().and_then(|extension| extension.to_str()) == Some("app")
            {
                apps.push(path);
            }
        }
        if apps.len() == 1 {
            Ok(apps.remove(0))
        } else {
            Err(
                "The macOS update DMG does not contain the expected Litematica Preview.app."
                    .to_string(),
            )
        }
    }

    fn verify_signed_app(running: &Path, mounted: &Path) -> Result<(), String> {
        run_codesign_verify(mounted)?;
        run_spctl(mounted)?;

        let running_identifier = signature_value(running, "Identifier=")?;
        let mounted_identifier = signature_value(mounted, "Identifier=")?;
        if running_identifier != BUNDLE_IDENTIFIER || mounted_identifier != BUNDLE_IDENTIFIER {
            return Err(format!(
                "The app bundle identifier must be `{BUNDLE_IDENTIFIER}`; this app is \
                 `{running_identifier}` and the downloaded app is `{mounted_identifier}`."
            ));
        }

        let running_team = signature_value(running, "TeamIdentifier=")?;
        let mounted_team = signature_value(mounted, "TeamIdentifier=")?;
        if running_team != mounted_team {
            return Err(
                "The downloaded app's Apple Developer Team Identifier does not match this app's."
                    .to_string(),
            );
        }
        if running_team == "not set" {
            return Err(
                "This app is not signed with an Apple Developer Team Identifier, so the signed \
                 DMG update cannot be verified."
                    .to_string(),
            );
        }

        let requirement = designated_requirement(running)?;
        let output = Command::new(CODESIGN)
            .arg("--verify")
            .arg("--deep")
            .arg("--strict")
            .arg("-R")
            .arg(&requirement)
            .arg(mounted)
            .output()
            .map_err(|error| {
                format!(
                    "Could not verify the downloaded app against this app's designated \
                     requirement: {error}"
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "The downloaded app does not match this app's designated requirement: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn run_codesign_verify(app: &Path) -> Result<(), String> {
        let output = Command::new(CODESIGN)
            .arg("--verify")
            .arg("--deep")
            .arg("--strict")
            .arg(app)
            .output()
            .map_err(|error| format!("Could not verify the app's code signature: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "The app is not validly signed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn run_spctl(app: &Path) -> Result<(), String> {
        let output = Command::new(SPCTL)
            .arg("--assess")
            .arg("--type")
            .arg("execute")
            .arg("--verbose=4")
            .arg(app)
            .output()
            .map_err(|error| format!("Could not assess the app with Gatekeeper: {error}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() || !text.contains("Notarized Developer ID") {
            return Err(format!(
                "Gatekeeper did not accept the downloaded app as a notarized Developer ID build: {}",
                text.trim()
            ));
        }
        Ok(())
    }

    fn signature_value(app: &Path, prefix: &str) -> Result<String, String> {
        let output = Command::new(CODESIGN)
            .arg("-dv")
            .arg("--verbose=4")
            .arg(app)
            .output()
            .map_err(|error| format!("Could not read the app's code signature: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Could not read the app's code signature: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&output.stderr);
        text.lines()
            .find_map(|line| line.trim().strip_prefix(prefix))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("The app's code signature has no {prefix} value."))
    }

    fn designated_requirement(app: &Path) -> Result<String, String> {
        let output = Command::new(CODESIGN)
            .arg("-d")
            .arg("-r-")
            .arg(app)
            .output()
            .map_err(|error| {
                format!("Could not read this app's designated requirement: {error}")
            })?;
        if !output.status.success() {
            return Err(format!(
                "Could not read this app's designated requirement: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&output.stderr);
        text.lines()
            .find_map(|line| line.trim().strip_prefix("designated => "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "This app's code signature has no designated requirement.".to_string())
    }

    fn copy_and_swap(running: &Path, mounted: &Path) -> Result<(), String> {
        let parent = running
            .parent()
            .ok_or_else(|| "The running app bundle has no parent directory.".to_string())?;
        let running_name = running
            .file_name()
            .ok_or_else(|| "The running app bundle has no file name.".to_string())?;
        let staging = tempfile::Builder::new()
            .prefix(".litematica-preview-update-")
            .tempdir_in(parent)
            .map_err(|error| {
                format!(
                    "Could not create a staging directory next to {}: {error}. The app's parent \
                     directory must be writable.",
                    running.display()
                )
            })?;
        let staged_app = staging.path().join(Path::new(running_name));

        let status = Command::new(DITTO)
            .arg(mounted)
            .arg(&staged_app)
            .status()
            .map_err(|error| format!("Could not copy the downloaded app: {error}"))?;
        if !status.success() {
            return Err(
                "Could not copy the downloaded app into the staging directory.".to_string(),
            );
        }
        run_codesign_verify(&staged_app)?;

        let running_c = CString::new(running.as_os_str().as_bytes())
            .map_err(|_| "The app path contains a NUL byte.".to_string())?;
        let staged_c = CString::new(staged_app.as_os_str().as_bytes())
            .map_err(|_| "The staging path contains a NUL byte.".to_string())?;
        let result =
            unsafe { libc::renamex_np(running_c.as_ptr(), staged_c.as_ptr(), libc::RENAME_SWAP) };
        if result != 0 {
            return Err(format!(
                "Could not atomically replace the app bundle: {}",
                std::io::Error::last_os_error()
            ));
        }

        // The old bundle is now at `staged_app`; dropping the guard removes it.
        drop(staging);
        Ok(())
    }

    struct Mount {
        mountpoint: PathBuf,
    }

    impl Mount {
        fn attach(dmg: &Path, mountpoint: &Path) -> Result<Self, String> {
            let output = Command::new(HDIUTIL)
                .arg("attach")
                .arg("-nobrowse")
                .arg("-readonly")
                .arg("-mountpoint")
                .arg(mountpoint)
                .arg(dmg)
                .output()
                .map_err(|error| format!("Could not attach the macOS update DMG: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "Could not attach the macOS update DMG: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Ok(Self {
                mountpoint: mountpoint.to_path_buf(),
            })
        }

        fn path(&self) -> &Path {
            &self.mountpoint
        }
    }

    impl Drop for Mount {
        fn drop(&mut self) {
            let detached = Command::new(HDIUTIL)
                .arg("detach")
                .arg(&self.mountpoint)
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            if !detached {
                let _ = Command::new(HDIUTIL)
                    .arg("detach")
                    .arg("-force")
                    .arg(&self.mountpoint)
                    .status();
            }
        }
    }
}
