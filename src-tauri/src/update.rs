use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

const FEED_URL: &str = "https://update.tanh.moe/litematica-preview/windows/";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRelease {
    pub version: String,
    pub notes: String,
    pub size: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    percent: i16,
}

#[derive(Clone)]
pub struct UpdateState {
    inner: platform::UpdateState,
}

impl UpdateState {
    pub fn new() -> Self {
        Self {
            inner: platform::UpdateState::new(),
        }
    }

    pub fn supported(&self) -> bool {
        self.inner.supported()
    }

    pub fn pending_version(&self) -> Option<String> {
        self.inner.pending_version()
    }

    fn check(&self) -> Result<Option<UpdateRelease>, String> {
        self.inner.check()
    }

    fn download(&self, progress: Option<std::sync::mpsc::Sender<i16>>) -> Result<(), String> {
        self.inner.download(progress)
    }

    fn install(&self) -> Result<(), String> {
        self.inner.install()
    }
}

/// Runs Velopack's installer and updater hooks before any other startup work.
pub fn run_startup_hooks() {
    #[cfg(windows)]
    velopack::VelopackApp::build()
        .on_before_uninstall_fast_callback(|_| {
            let _ = crate::associations::unregister();
        })
        .run();
}

/// Returns the path of a pre-Velopack per-user installation that is still
/// registered on this system, so the UI can tell the user to remove it.
pub fn legacy_install_path() -> Option<String> {
    platform::legacy_install_path()
}

#[tauri::command]
pub async fn check_for_update(
    state: State<'_, UpdateState>,
) -> Result<Option<UpdateRelease>, String> {
    let update = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || update.check())
        .await
        .map_err(|error| format!("The update check did not finish: {error}"))?
}

#[tauri::command]
pub async fn download_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    let update = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (sender, receiver) = std::sync::mpsc::channel();
        let progress_app = app.clone();
        let progress = std::thread::spawn(move || {
            for percent in receiver {
                let _ = progress_app.emit("update-progress", UpdateProgress { percent });
            }
        });
        let result = update.download(Some(sender));
        let _ = progress.join();
        result
    })
    .await
    .map_err(|error| format!("The update download did not finish: {error}"))?
}

#[tauri::command]
pub async fn install_update(state: State<'_, UpdateState>) -> Result<(), String> {
    let update = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || update.install())
        .await
        .map_err(|error| format!("The update could not be applied: {error}"))?
}

#[cfg(windows)]
mod platform {
    use std::sync::{Arc, Mutex};

    use velopack::{
        sources::HttpSource, HttpOptions, UpdateCheck, UpdateInfo, UpdateManager, VelopackAsset,
    };

    use super::{UpdateRelease, FEED_URL};

    #[derive(Clone)]
    struct Candidate {
        info: Option<Box<UpdateInfo>>,
        asset: VelopackAsset,
    }

    #[derive(Clone)]
    pub struct UpdateState {
        manager: Option<UpdateManager>,
        candidate: Arc<Mutex<Option<Candidate>>>,
    }

    impl UpdateState {
        pub fn new() -> Self {
            let source = HttpSource::new_with_options(
                FEED_URL,
                HttpOptions {
                    TimeoutMilliseconds: 120_000,
                    ..Default::default()
                },
            );
            let manager = UpdateManager::new(source, None, None).ok();
            let candidate = manager
                .as_ref()
                .and_then(|manager| manager.get_update_pending_restart())
                .map(|asset| Candidate { info: None, asset });
            Self {
                manager,
                candidate: Arc::new(Mutex::new(candidate)),
            }
        }

        pub fn supported(&self) -> bool {
            self.manager.is_some()
        }

        pub fn pending_version(&self) -> Option<String> {
            self.candidate.lock().ok().and_then(|candidate| {
                candidate
                    .as_ref()
                    .map(|candidate| candidate.asset.Version.clone())
            })
        }

        pub fn check(&self) -> Result<Option<UpdateRelease>, String> {
            let Some(manager) = &self.manager else {
                return Ok(None);
            };
            let UpdateCheck::UpdateAvailable(info) = manager
                .check_for_updates()
                .map_err(|error| error.to_string())?
            else {
                return Ok(None);
            };
            let asset = info.TargetFullRelease.clone();
            let release = UpdateRelease {
                version: asset.Version.clone(),
                notes: asset.NotesMarkdown.clone(),
                size: asset.Size,
            };
            *self
                .candidate
                .lock()
                .map_err(|_| "The updater is unavailable.".to_string())? = Some(Candidate {
                info: Some(info),
                asset,
            });
            Ok(Some(release))
        }

        pub fn download(
            &self,
            progress: Option<std::sync::mpsc::Sender<i16>>,
        ) -> Result<(), String> {
            let Some(manager) = &self.manager else {
                return Err("This copy does not support in-app updates.".to_string());
            };
            let candidate = self
                .candidate
                .lock()
                .map_err(|_| "The updater is unavailable.".to_string())?
                .clone()
                .ok_or_else(|| "Check for updates before downloading.".to_string())?;
            let Some(info) = candidate.info else {
                return Ok(());
            };
            manager
                .download_updates(&info, progress)
                .map_err(|error| error.to_string())?;
            if let Ok(mut candidate) = self.candidate.lock() {
                if let Some(candidate) = candidate.as_mut() {
                    candidate.info = None;
                }
            }
            Ok(())
        }

        pub fn install(&self) -> Result<(), String> {
            let Some(manager) = &self.manager else {
                return Err("This copy does not support in-app updates.".to_string());
            };
            let asset = self
                .candidate
                .lock()
                .map_err(|_| "The updater is unavailable.".to_string())?
                .as_ref()
                .map(|candidate| candidate.asset.clone())
                .or_else(|| manager.get_update_pending_restart())
                .ok_or_else(|| "No downloaded update is ready to install.".to_string())?;
            manager
                .apply_updates_and_restart(asset)
                .map_err(|error| error.to_string())
        }
    }

    pub fn legacy_install_path() -> Option<String> {
        use winreg::enums::HKEY_CURRENT_USER;

        let current = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let key = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Litematica Preview")
            .ok()?;
        let location: String = key.get_value("InstallLocation").ok()?;
        let location = location.trim_matches('"');
        let path = std::path::Path::new(location);
        if current.as_path() == path || !path.join("uninstall.exe").is_file() {
            return None;
        }
        Some(location.to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::UpdateRelease;

    #[derive(Clone)]
    pub struct UpdateState;

    impl UpdateState {
        pub fn new() -> Self {
            Self
        }

        pub fn supported(&self) -> bool {
            false
        }

        pub fn pending_version(&self) -> Option<String> {
            None
        }

        pub fn check(&self) -> Result<Option<UpdateRelease>, String> {
            Ok(None)
        }

        pub fn download(&self, _: Option<std::sync::mpsc::Sender<i16>>) -> Result<(), String> {
            Err("In-app updates are only available in the Windows package.".to_string())
        }

        pub fn install(&self) -> Result<(), String> {
            Err("In-app updates are only available in the Windows package.".to_string())
        }
    }

    pub fn legacy_install_path() -> Option<String> {
        None
    }
}
