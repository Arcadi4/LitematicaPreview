#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod associations;
mod formats;
mod preview;
mod preview_process;
mod protocol;
mod render;
mod resources;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use preview::PreviewWorker;
use render::context::NativeRenderer;
use render::scene::{BatchSource, TextureBytes};
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

/// The native viewport. The renderer exists once the wgpu surface was created
/// for the main window and stays `None` after a creation failure, so the DOM
/// can show the actionable `viewport-failed` error without the app exiting.
struct ViewerState {
    renderer: Arc<Mutex<Option<NativeRenderer>>>,
}

impl ViewerState {
    fn unavailable() -> Self {
        Self {
            renderer: Arc::new(Mutex::new(None)),
        }
    }
}

/// Locks the renderer, treating a poisoned lock as an unavailable viewport
/// instead of unwinding. Callers never hold the lock across blocking waits.
fn lock_renderer(
    renderer: &Mutex<Option<NativeRenderer>>,
) -> Result<MutexGuard<'_, Option<NativeRenderer>>, String> {
    renderer
        .lock()
        .map_err(|_| VIEWPORT_UNAVAILABLE.to_string())
}

/// Runs one renderer action while holding the lock.
fn with_renderer<T>(
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    action: impl FnOnce(&mut NativeRenderer) -> T,
) -> Result<T, String> {
    lock_renderer(renderer)?
        .as_mut()
        .map(action)
        .ok_or_else(|| VIEWPORT_UNAVAILABLE.to_string())
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
}

#[tauri::command]
async fn bootstrap(state: State<'_, HostState>) -> Result<Bootstrap, String> {
    let resources = state.resources.clone();
    let initial_path = state.initial_path.clone();
    let request_id = state.worker.begin_session();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(Bootstrap {
            extensions: EXTENSIONS,
            demos: resources.demos()?,
            initial_path,
            version: env!("CARGO_PKG_VERSION"),
            request_id,
            max_worker_threads: litematica_preview_core::max_worker_threads(),
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
    metadata: protocol::Metadata,
}

#[cfg(target_os = "windows")]
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
    completed: u64,
    total: u64,
) {
    if worker.ensure_current(request_id).is_ok() {
        let _ = app.emit(
            "preview-progress",
            PreviewProgress {
                request_id,
                phase: "mesh",
                completed,
                total,
            },
        );
    }
}

/// Uploads one decoded batch's joined buffers to the staged renderer scene.
fn attach_batch(
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    batch: &protocol::Batch,
    payload: &protocol::Payload,
) -> Result<(), String> {
    let geometry = payload.joined_buffers(&[0])?;
    let texture_ids: Vec<usize> = batch
        .metadata
        .textures
        .iter()
        .map(|texture| texture.buffer_id)
        .collect();
    let textures = payload.joined_buffers(&texture_ids)?;
    let texture_pixels: Vec<TextureBytes> = textures
        .iter()
        .map(|bytes| TextureBytes { pixels: bytes })
        .collect();
    let source = BatchSource {
        texture_offset: batch.texture_offset,
        geometry: geometry
            .first()
            .map(Vec::as_slice)
            .ok_or("Invalid preview buffer.")?,
        texture_pixels: &texture_pixels,
        metadata: &batch.metadata,
    };
    lock_renderer(renderer)?
        .as_mut()
        .ok_or_else(|| VIEWPORT_UNAVAILABLE.to_string())?
        .attach_batch(&source)
        .map_err(|error| error.to_string())
}

/// Drops a staged load, keeping the model already on screen.
fn cancel_staged(renderer: &Arc<Mutex<Option<NativeRenderer>>>) {
    if let Ok(mut renderer) = renderer.lock() {
        if let Some(renderer) = renderer.as_mut() {
            renderer.cancel_load();
        }
    }
}

/// Ends a failed load: the staged model is dropped, the producer is unblocked,
/// the request's bookkeeping is released and the DOM sees the actionable
/// `preview-failed` message.
fn fail_load(
    app: &AppHandle,
    worker: &PreviewWorker,
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    request_id: u64,
    message: String,
) -> String {
    let _ = app.emit(
        "preview-failed",
        PreviewFailed {
            request_id,
            message: message.clone(),
        },
    );
    cancel_staged(renderer);
    worker.release(request_id);
    message
}

/// Drains one request's preview batches into the native renderer, attaching
/// each leased payload and reporting the outcome as `preview-loaded` or
/// `preview-failed`. The lock is only held around renderer calls, so window
/// events keep flowing while the decoder produces. A superseded request stops
/// silently, matching the session's generation rules.
fn consume_batches(
    app: &AppHandle,
    worker: &PreviewWorker,
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    request_id: u64,
) -> Result<protocol::Metadata, String> {
    // Every load starts from a clean staged scene: an interrupted earlier
    // request may have left one behind.
    if let Err(error) = with_renderer(renderer, |renderer| renderer.begin_load()) {
        return Err(fail_load(app, worker, renderer, request_id, error));
    }
    let mut previous: Option<u64> = None;
    loop {
        match worker.next(request_id, previous) {
            Ok(protocol::StreamEvent::Batch { batch }) => {
                let payload = match worker.batch_payload(request_id, batch.batch_id) {
                    Ok(payload) => payload,
                    Err(error) => {
                        return Err(fail_load(app, worker, renderer, request_id, error));
                    }
                };
                if let Err(error) = attach_batch(renderer, &batch, &payload) {
                    return Err(fail_load(app, worker, renderer, request_id, error));
                }
                if worker.ensure_current(request_id).is_err() {
                    cancel_staged(renderer);
                    worker.release(request_id);
                    return Err("Cancelled".into());
                }
                previous = Some(batch.batch_id);
            }
            Ok(protocol::StreamEvent::Complete { metadata }) => {
                let frame = with_renderer(renderer, |renderer| {
                    renderer.finish_load(&metadata);
                    renderer.render()
                });
                match frame {
                    Ok(Ok(())) => {
                        worker.release(request_id);
                        let _ = app.emit(
                            "preview-loaded",
                            PreviewLoaded {
                                request_id,
                                metadata: metadata.clone(),
                            },
                        );
                        return Ok(metadata);
                    }
                    // The staged model is already displayed; only the frame
                    // failure is reported.
                    Ok(Err(error)) => {
                        return Err(fail_load(
                            app,
                            worker,
                            renderer,
                            request_id,
                            error.to_string(),
                        ));
                    }
                    Err(error) => return Err(fail_load(app, worker, renderer, request_id, error)),
                }
            }
            Err(error) => {
                if worker.ensure_current(request_id).is_err() {
                    cancel_staged(renderer);
                    worker.release(request_id);
                    return Err("Cancelled".into());
                }
                return Err(fail_load(app, worker, renderer, request_id, error));
            }
        }
    }
}

#[tauri::command]
async fn load_preview(
    path: String,
    request_id: u64,
    app: AppHandle,
    options: preview::LoadOptions,
    state: State<'_, HostState>,
    viewer: State<'_, ViewerState>,
) -> Result<protocol::Metadata, String> {
    let options = options.validate()?;
    let worker = Arc::clone(&state.worker);
    let renderer = Arc::clone(&viewer.renderer);
    worker.advance(request_id);
    worker.ensure_current(request_id)?;
    let path = preview_path(path)?;
    let resources = state.resources.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker.ensure_current(request_id)?;
        let pack_path = resources.pack()?;
        // The blocking preview keeps its command synchronous: the producer
        // runs on its own thread inside the bounded queue while this task
        // drains it, so the answer still means the whole model is attached.
        worker.begin_stream(request_id, options)?;
        let producer = Arc::clone(&worker);
        let producer_app = app.clone();
        let producing = std::thread::spawn(move || {
            // A panicking producer must still end the stream, or this task's
            // consumer would wait for batches that never arrive.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                producer.load_stream(
                    &path,
                    &pack_path,
                    request_id,
                    options,
                    |completed, total| {
                        report_progress(&producer_app, &producer, request_id, completed, total)
                    },
                )
            }))
            .unwrap_or_else(|panic| {
                let detail = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("Unknown native panic");
                let error = format!("The preview worker stopped unexpectedly: {detail}");
                producer.fail_stream(request_id, error.clone());
                Err(error)
            })
        });
        let consumed = consume_batches(&app, &worker, &renderer, request_id);
        // The producer stops once the consumer ends the stream; its own error
        // was already surfaced through the consumer.
        let produced = producing.join();
        match consumed {
            Err(error) => Err(error),
            Ok(metadata) => {
                produced.map_err(|_| "The preview worker stopped unexpectedly.".to_string())??;
                worker.ensure_current(request_id)?;
                Ok(metadata)
            }
        }
    })
    .await
    .map_err(|e| format!("The preview worker stopped unexpectedly: {e}"))?
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
    if options.thread_count.is_none() {
        return Err("Streaming preview requires multithreading.".into());
    }
    let path = preview_path(path)?;
    let worker = Arc::clone(&state.worker);
    let renderer = Arc::clone(&viewer.renderer);
    worker.advance(request_id);
    worker.begin_stream(request_id, options)?;
    let resources = state.resources.clone();
    tauri::async_runtime::spawn(async move {
        let producer = Arc::clone(&worker);
        let producer_app = app.clone();
        let producing = tauri::async_runtime::spawn_blocking(move || {
            let pack = resources.pack()?;
            producer.load_stream(&path, &pack, request_id, options, |completed, total| {
                report_progress(&producer_app, &producer, request_id, completed, total)
            })
        });
        // The consumer drains the same bounded queue, so it runs beside the
        // producer instead of waiting for it: the producer stops at the
        // queue's window until uploads release leases.
        let consumer_worker = Arc::clone(&worker);
        let consumer_app = app.clone();
        let consuming = tauri::async_runtime::spawn_blocking(move || {
            consume_batches(&consumer_app, &consumer_worker, &renderer, request_id)
        });
        match producing.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => worker.fail_stream(request_id, error),
            Err(error) => worker.fail_stream(
                request_id,
                format!("The preview worker stopped unexpectedly: {error}"),
            ),
        }
        // Every outcome already reached the DOM as `preview-progress`,
        // `preview-loaded` or `preview-failed`, and the consumer released the
        // request's CPU bookkeeping.
        let _ = consuming.await;
    });
    Ok(())
}

#[tauri::command]
fn preview_memory(request_id: u64, state: State<'_, HostState>) -> Option<u64> {
    state.worker.preview_memory(request_id)
}

#[tauri::command]
fn cancel_load(request_id: u64, state: State<'_, HostState>) {
    state.worker.advance(request_id);
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
    // Pass the directory as a separate UTF-16 argument. ShellExecute does not
    // execute an interpolated command, and resource paths may contain spaces.
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

/// Composes the native viewport under the transparent DOM layer and creates
/// the renderer for the main window's surface. A failure leaves the renderer
/// absent and emits `viewport-failed`, so the DOM shows the actionable error
/// while the app keeps running.
#[cfg(target_os = "windows")]
fn create_viewer(handle: &AppHandle) -> ViewerState {
    let surface = match render::viewport::install(handle) {
        Ok(surface) => surface,
        Err(error) => {
            let _ = handle.emit(
                "viewport-failed",
                ViewportFailed {
                    message: error.to_string(),
                },
            );
            return ViewerState::unavailable();
        }
    };
    match render::context::NativeRenderer::new_from_hwnd(surface.hwnd(), surface.size()) {
        Ok(mut renderer) => {
            if let Some(window) = handle.get_webview_window("main") {
                if let Ok(scale) = window.scale_factor() {
                    renderer.set_scale(scale);
                }
            }
            ViewerState {
                renderer: Arc::new(Mutex::new(Some(renderer))),
            }
        }
        Err(error) => {
            let _ = handle.emit(
                "viewport-failed",
                ViewportFailed {
                    message: error.to_string(),
                },
            );
            ViewerState::unavailable()
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn create_viewer(_handle: &AppHandle) -> ViewerState {
    ViewerState::unavailable()
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
            // Synchronous init, matching the WebGL renderer it replaces: the
            // surface and renderer exist before the first frame.
            app.manage(create_viewer(app.handle()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window.state::<HostState>().worker.advance(u64::MAX);
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
                            // A failed frame here is retried by the next
                            // input; only load failures reach the DOM as
                            // `preview-failed`.
                            let _ = renderer.render();
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            choose_file,
            load_preview,
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
    // Decoder mode bypasses Tauri and WebView2. It receives authentication
    // through the inherited stdin pipe, not process arguments or a Tauri command.
    if first
        .as_deref()
        .is_some_and(|argument| argument == "--preview-worker")
    {
        let result = match (args.next(), args.next()) {
            (Some(port), None) => preview_process::run(&port),
            _ => Err("Invalid decoder process arguments.".into()),
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
    // Installer hooks must exit before Tauri, WebView2, or the Settings dialog
    // starts; NSIS reports the nonzero process status.
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
