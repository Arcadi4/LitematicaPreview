#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod associations;
mod formats;
mod preview;
mod preview_process;
mod preview_protocol;
mod render;
mod resources;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use preview::PreviewWorker;
use render::context::NativeRenderer;
use resources::{Demo, Resources};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

pub use formats::EXTENSIONS;

const VIEWPORT_UNAVAILABLE: &str = "The native viewport is unavailable. \
     Restart the application to initialize the graphics device.";

struct HostState {
    worker: Arc<PreviewWorker>,
    resources: Resources,
    initial_path: Option<String>,
}

/// A failed viewport remains unavailable; its error is retained for bootstrap.
struct ViewerState {
    renderer: Arc<Mutex<Option<NativeRenderer>>>,
    init_error: Arc<Mutex<Option<String>>>,
}

impl ViewerState {
    fn unavailable(initial_error: Option<String>) -> Self {
        Self {
            renderer: Arc::new(Mutex::new(None)),
            init_error: Arc::new(Mutex::new(initial_error)),
        }
    }
}

/// A poisoned lock makes the viewport unavailable; callers must not hold it
/// across blocking waits.
fn lock_renderer(
    renderer: &Mutex<Option<NativeRenderer>>,
) -> Result<MutexGuard<'_, Option<NativeRenderer>>, String> {
    renderer
        .lock()
        .map_err(|_| VIEWPORT_UNAVAILABLE.to_string())
}

/// Renderer actions hold the viewport lock only for their duration.
fn with_renderer<T>(
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    action: impl FnOnce(&mut NativeRenderer) -> T,
) -> Result<T, String> {
    let mut locked = lock_renderer(renderer)?;
    let renderer = locked
        .as_mut()
        .ok_or_else(|| VIEWPORT_UNAVAILABLE.to_string())?;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(renderer))).map_err(|panic| {
        let detail = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("Unknown graphics panic");
        format!("The graphics operation failed: {detail}")
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    extensions: &'static [&'static str],
    demos: Vec<Demo>,
    initial_path: Option<String>,
    version: &'static str,
    request_id: u64,
    max_worker_threads: u8,
    viewport_error: Option<String>,
}

#[tauri::command]
async fn bootstrap(
    state: State<'_, HostState>,
    viewer: State<'_, ViewerState>,
) -> Result<Bootstrap, String> {
    let resources = state.resources.clone();
    let initial_path = state.initial_path.clone();
    let request_id = state.worker.begin_session();
    let viewport_error = viewer
        .init_error
        .lock()
        .ok()
        .and_then(|error| error.clone());
    tauri::async_runtime::spawn_blocking(move || {
        Ok(Bootstrap {
            extensions: EXTENSIONS,
            demos: resources.demos()?,
            initial_path,
            version: env!("CARGO_PKG_VERSION"),
            request_id,
            max_worker_threads: litematica_preview_core::max_worker_threads(),
            viewport_error,
        })
    })
    .await
    .map_err(|e| format!("Unable to load the welcome screen: {e}"))?
}

#[tauri::command]
async fn choose_file(app: AppHandle, window: WebviewWindow) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let extensions: Vec<_> = EXTENSIONS.iter().map(|extension| &extension[1..]).collect();
        app.dialog()
            .file()
            .set_parent(&window)
            .set_title("Open Minecraft schematic")
            .add_filter("Minecraft schematics", &extensions)
            .add_filter("All files", &["*"])
            .blocking_pick_file()
            .map(|file| {
                let path = file
                    .into_path()
                    .map_err(|e| format!("Unable to open the selected file: {e}"))?;
                resources::path_string(&path)
            })
            .transpose()
    })
    .await
    .map_err(|e| format!("Unable to show the file picker: {e}"))?
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewProgress {
    request_id: u64,
    phase: &'static str,
    completed: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewFailed {
    request_id: u64,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewLoaded {
    request_id: u64,
    metadata: preview::Metadata,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewportFailed {
    message: String,
}

fn preview_path(path: String) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    let supported = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            EXTENSIONS
                .iter()
                .any(|candidate| candidate[1..].eq_ignore_ascii_case(extension))
        });
    if !supported {
        return Err(format!(
            "Choose a supported schematic: {}.",
            EXTENSIONS.join(", ")
        ));
    }
    Ok(path)
}

fn report_progress(
    app: &AppHandle,
    worker: &PreviewWorker,
    request_id: u64,
    phase: &'static str,
    completed: u64,
    total: u64,
) {
    let _ = worker.with_current(request_id, || {
        app.emit(
            "preview-progress",
            PreviewProgress {
                request_id,
                phase,
                completed,
                total,
            },
        )
        .map_err(|error| error.to_string())
    });
}

#[tauri::command]
fn start_preview(
    path: String,
    request_id: u64,
    app: AppHandle,
    options: preview::LoadOptions,
    state: State<'_, HostState>,
    viewer: State<'_, ViewerState>,
) -> Result<(), String> {
    let options = options.validate()?;
    let path = preview_path(path)?;
    let pack_path = state.resources.pack()?;
    let worker = Arc::clone(&state.worker);
    let renderer = Arc::clone(&viewer.renderer);
    let job_worker = Arc::clone(&worker);
    let prepare_renderer = Arc::clone(&renderer);
    worker.submit(
        request_id,
        move || {
            with_renderer(&prepare_renderer, |renderer| {
                renderer.begin_load();
                renderer.render().map_err(|error| error.to_string())
            })?
        },
        Box::new(move |decoder| {
            let current = || job_worker.ensure_current(request_id);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                current()?;
                let uploads = job_worker.with_current(request_id, || {
                    with_renderer(&renderer, |renderer| {
                        renderer.upload_window(usize::from(options.thread_count.unwrap_or(1)))
                    })
                })?;
                let metadata = preview_process::load(
                    decoder,
                    &job_worker,
                    request_id,
                    &path,
                    &pack_path,
                    options,
                    |min, max| {
                        job_worker.with_current(request_id, || {
                            with_renderer(&renderer, |renderer| renderer.prepare_load(min, max))
                        })
                    },
                    |batch| {
                        let bytes = batch
                            .textures
                            .iter()
                            .try_fold(batch.geometry.len(), |bytes, texture| {
                                bytes.checked_add(texture.pixels.len())
                            })
                            .ok_or_else(|| {
                                String::from(
                                    "The preview mesh batch overflowed the upload size accounting.",
                                )
                            })?;
                        let permit = uploads.acquire(bytes, &current)?;
                        job_worker.with_current(request_id, || {
                            with_renderer(&renderer, |renderer| {
                                renderer
                                    .attach_batch(&batch, permit)
                                    .map_err(|error| error.to_string())
                            })?
                        })
                    },
                    |phase, completed, total| {
                        report_progress(&app, &job_worker, request_id, phase, completed, total)
                    },
                );
                let drained = uploads.drain(&|| Ok(()));
                let metadata = metadata?;
                drained?;
                current()?;
                job_worker.with_current(request_id, || {
                    with_renderer(&renderer, |renderer| {
                        renderer.finish_load(&metadata.info());
                        renderer.render().map_err(|error| error.to_string())
                    })??;
                    job_worker.release(request_id);
                    app.emit(
                        "preview-loaded",
                        PreviewLoaded {
                            request_id,
                            metadata,
                        },
                    )
                    .map_err(|error| error.to_string())
                })
            }))
            .unwrap_or_else(|panic| {
                *decoder = None;
                let detail = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("Unknown native panic");
                Err(format!("The preview worker stopped unexpectedly: {detail}"))
            });
            if let Err(message) = result {
                let _ = job_worker.with_current(request_id, || {
                    let cleanup = with_renderer(&renderer, |renderer| {
                        renderer.cancel_load();
                        renderer.render().map_err(|error| error.to_string())
                    })
                    .and_then(|result| result);
                    let message = match cleanup {
                        Ok(()) => message,
                        Err(error) => format!("{message}\n\nRecovery: {error}"),
                    };
                    job_worker.release(request_id);
                    app.emit(
                        "preview-failed",
                        PreviewFailed {
                            request_id,
                            message,
                        },
                    )
                    .map_err(|error| error.to_string())
                });
            }
        }),
    )
}

#[tauri::command]
fn preview_memory(request_id: u64, state: State<'_, HostState>) -> Option<u64> {
    state.worker.preview_memory(request_id)
}

#[tauri::command]
fn cancel_load(
    request_id: u64,
    state: State<'_, HostState>,
    viewer: State<'_, ViewerState>,
) -> Result<(), String> {
    state.worker.cancel(request_id, || {
        with_renderer(&viewer.renderer, |renderer| {
            renderer.cancel_load();
            renderer.render().map_err(|error| error.to_string())
        })?
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum ViewportInput {
    Orbit {
        dx: f32,
        dy: f32,
    },
    Pan {
        dx: f32,
        dy: f32,
    },
    #[serde(rename_all = "camelCase")]
    Wheel {
        delta_y: f32,
        delta_mode: u8,
    },
    Zoom {
        factor: f32,
    },
    Arrow {
        dx: f32,
        dy: f32,
        panning: bool,
    },
    Fit {},
    Grid {
        visible: bool,
    },
    /// The visible preview rectangle in CSS pixels, re-reported when DOM chrome changes.
    /// It rides the ordered per-frame stream; see [`NativeRenderer::set_stage`].
    Stage {
        left: f32,
        top: f32,
        width: f32,
        height: f32,
    },
}

/// Applies ordered input accumulated for one display frame, then presents once.
#[tauri::command]
fn viewport_input(
    commands: Vec<ViewportInput>,
    state: State<'_, ViewerState>,
) -> Result<(), String> {
    let mut locked = lock_renderer(&state.renderer)?;
    let renderer = locked
        .as_mut()
        .ok_or_else(|| VIEWPORT_UNAVAILABLE.to_string())?;
    for command in commands {
        if !matches!(
            command,
            ViewportInput::Grid { .. } | ViewportInput::Stage { .. }
        ) {
            renderer.mark_user_interaction();
        }
        match command {
            ViewportInput::Orbit { dx, dy } => renderer.camera().orbit(dx, dy),
            ViewportInput::Pan { dx, dy } => renderer.camera().pan(dx, dy),
            ViewportInput::Wheel {
                delta_y,
                delta_mode,
            } => renderer.camera().zoom_wheel(delta_y, delta_mode),
            ViewportInput::Zoom { factor } => renderer.camera().zoom(factor),
            ViewportInput::Arrow { dx, dy, panning } => renderer.camera().arrow(dx, dy, panning),
            ViewportInput::Fit {} => renderer.camera().fit(),
            ViewportInput::Grid { visible } => renderer.camera().set_grid(visible),
            ViewportInput::Stage {
                left,
                top,
                width,
                height,
            } => {
                renderer.set_stage(left, top, width, height);
            }
        }
    }
    renderer.render().map_err(|error| error.to_string())
}

#[tauri::command]
async fn register_associations() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        associations::register()?;
        associations::open_settings()
    })
    .await
    .map_err(|e| format!("Unable to register file associations: {e}"))?
}

#[tauri::command]
async fn unregister_associations() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(associations::unregister)
        .await
        .map_err(|e| format!("Unable to remove file associations: {e}"))?
}

#[tauri::command]
async fn show_licenses(state: State<'_, HostState>) -> Result<(), String> {
    let resources = state.resources.clone();
    tauri::async_runtime::spawn_blocking(move || open_folder(&resources.licenses()?))
        .await
        .map_err(|e| format!("Unable to open the license folder: {e}"))?
}

#[cfg(windows)]
fn open_folder(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let folder: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let open: Vec<u16> = "open\0".encode_utf16().collect();
    // Separate UTF-16 arguments avoid command interpolation and preserve paths with spaces.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            open.as_ptr(),
            folder.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(format!(
            "Windows could not open the license folder (code {}).",
            result as isize
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn open_folder(_: &std::path::Path) -> Result<(), String> {
    Err("Litematica Preview's desktop integration requires Windows.".into())
}

fn initial_path() -> Result<Option<String>, String> {
    let mut args = std::env::args_os().skip(1);
    let Some(argument) = args.next() else {
        return Ok(None);
    };
    if args.next().is_some() {
        return Err("Open one schematic per window.".into());
    }
    let path = PathBuf::from(argument);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Unable to resolve the schematic path: {e}"))?
            .join(path)
    };
    resources::path_string(&absolute).map(Some)
}

/// Composes the native viewport and builds the renderer for the main window's
/// surface. A failure emits `viewport-failed` and leaves the renderer absent,
/// so the app keeps running without a viewport.
fn create_viewer(handle: &AppHandle) -> ViewerState {
    let surface = match render::viewport::install(handle) {
        Ok(surface) => surface,
        Err(error) => {
            let message = error.to_string();
            let _ = handle.emit("viewport-failed", ViewportFailed { message: message.clone() });
            return ViewerState::unavailable(Some(message));
        }
    };
    match render::context::NativeRenderer::new_from_raw(
        surface.handle(),
        Some(surface.display()),
        surface.size(),
    ) {
        Ok(mut renderer) => {
            if let Some(window) = handle.get_webview_window("main") {
                if let Ok(scale) = window.scale_factor() {
                    renderer.set_scale(scale);
                }
            }
            ViewerState {
                renderer: Arc::new(Mutex::new(Some(renderer))),
                init_error: Arc::new(Mutex::new(None)),
            }
        }
        Err(error) => {
            let message = error.to_string();
            let _ = handle.emit("viewport-failed", ViewportFailed { message: message.clone() });
            ViewerState::unavailable(Some(message))
        }
    }
}

fn run() -> Result<(), String> {
    let initial_path = initial_path()?;
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let resources = Resources::new(app.path().resource_dir()?);
            app.manage(HostState {
                worker: Arc::new(PreviewWorker::default()),
                resources,
                initial_path,
            });
            app.manage(create_viewer(app.handle()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let _ = window
                    .state::<HostState>()
                    .worker
                    .cancel(u64::MAX, || Ok(()));
            }
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
            ) {
                if let Some(viewer) = window.try_state::<ViewerState>() {
                    if let Ok(mut renderer) = viewer.renderer.lock() {
                        if let Some(renderer) = renderer.as_mut() {
                            if let Ok(size) = window.inner_size() {
                                renderer.resize(size.width, size.height);
                            }
                            if let Ok(scale) = window.scale_factor() {
                                renderer.set_scale(scale);
                            }
                            // A failed frame here is retried by the next input; only load
                            // failures reach the DOM as `preview-failed`.
                            let _ = renderer.render();
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            choose_file,
            start_preview,
            preview_memory,
            cancel_load,
            viewport_input,
            register_associations,
            unregister_associations,
            show_licenses,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| format!("Unable to start Litematica Preview: {e}"))
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(std::ffi::OsStr::new("--preview-worker")) {
        let result = match (args.next(), args.next()) {
            (Some(port), None) => preview_process::run(&port),
            _ => Err("Pass exactly one decoder connection port.".into()),
        };
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    let remaining: Vec<_> = args.collect();
    let registration = match first.as_deref().and_then(|arg| arg.to_str()) {
        Some("--register") if remaining.is_empty() => Some(associations::register()),
        Some("--unregister") if remaining.is_empty() => Some(associations::unregister()),
        Some("--default-apps") if remaining.is_empty() => Some(associations::open_settings()),
        Some("--register-extensions") => Some(match remaining.as_slice() {
            [selection] => selection
                .to_str()
                .ok_or_else(|| "Invalid extension selection.".to_string())
                .and_then(associations::register_extensions),
            _ => Err("Pass a comma-separated list of supported extensions.".into()),
        }),
        Some("--register" | "--unregister" | "--default-apps") => {
            Some(Err("Unexpected registration arguments.".into()))
        }
        _ => None,
    };
    // Installer hooks exit before Tauri, WebView2, or Settings starts; NSIS reads the status.
    if let Some(result) = registration {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = run() {
        eprintln!("{error}");
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
            let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Litematica Preview\0".encode_utf16().collect();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
}
