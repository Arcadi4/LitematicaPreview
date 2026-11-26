pub mod browse;
pub mod browse_thumbnail;
pub mod file_dialog;
pub mod formats;
pub mod loader;
pub mod preferences;
pub mod preview;
pub mod preview_image;
pub mod render;
pub mod resources;
pub mod texture_packs;
pub mod update;

mod app;
mod compositor;
#[cfg(target_os = "macos")]
mod document_open;
mod icons;
mod preferences_ui;
mod view;

use std::path::PathBuf;
use std::sync::OnceLock;

use app::App;
use compositor::ViewerCompositor;

static NON_UNICODE_PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();
const APP_ID: &str = "dev.litematica.preview";

fn path_argument(arg: &str) -> Option<PathBuf> {
    if arg.starts_with("file://") {
        url::Url::parse(arg).ok()?.to_file_path().ok()
    } else if arg.starts_with('-') {
        None
    } else {
        Some(PathBuf::from(arg))
    }
}

fn initial_paths() -> Vec<PathBuf> {
    rune::extra_args()
        .iter()
        .filter_map(|arg| path_argument(arg))
        .chain(NON_UNICODE_PATHS.get().into_iter().flatten().cloned())
        .collect()
}

#[cfg(not(target_os = "macos"))]
fn forward_documents(paths: impl Iterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let urls = paths
        .map(|path| {
            if let Some(path) = path.to_str()
                && path.starts_with("file://")
            {
                return Ok(path.to_owned());
            }
            let absolute = cwd.join(path);
            url::Url::from_file_path(&absolute)
                .map(|url| url.to_string())
                .map_err(|()| format!("Could not open {}", absolute.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let program = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)
        .map_err(|error| error.to_string())?;
    std::process::Command::new(program)
        .args(urls)
        .spawn()
        .map_err(|error| format!("Could not open Litematica Preview: {error}"))?;
    Ok(())
}

pub fn run() -> Result<(), String> {
    let mut argv = std::env::args_os().skip(1).peekable();
    if argv.peek().is_some_and(|arg| arg == "--licenses") {
        println!(
            "Fluent UI System Icons\n\n{}",
            include_str!("../icons/LICENSE")
        );
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    if argv.peek().is_some_and(|arg| arg == "--open") {
        argv.next();
        return forward_documents(argv);
    }
    let mut args = Vec::new();
    let mut non_unicode = Vec::new();
    for arg in argv {
        match arg.into_string() {
            Ok(arg) => args.push(arg),
            Err(path) => non_unicode.push(PathBuf::from(path)),
        }
    }
    let _ = NON_UNICODE_PATHS.set(non_unicode);
    rune::set_app_id(APP_ID);
    #[cfg(not(target_os = "macos"))]
    if !cfg!(debug_assertions)
        && NON_UNICODE_PATHS.get().is_some_and(Vec::is_empty)
        && !args.iter().any(|arg| arg.starts_with('-'))
        && !rune::single_instance(APP_ID)
    {
        return Ok(());
    }
    let spec = rune::AppSpec {
        min_size: Some((680.0, 480.0)),
        setup: Some(App::setup),
        poll: Some(App::poll),
        on_key: Some(App::key),
        ..rune::AppSpec::new("Litematica Preview", (1120.0, 740.0), App::new, view::view)
    };
    let app = rune::App::new(spec)
        .args(args)
        .chrome(rune::Chrome {
            traffic_lights: Some((16.0, 17.0)),
            drag_regions: true,
            ..Default::default()
        })
        .modifier_keys()
        .srgb_blend(false)
        .on_focus(|s, focused, _| {
            if !focused {
                s.stop_flying();
            }
        })
        .on_open_url(|s, url, _| {
            if url.starts_with("file://")
                && let Some(path) = path_argument(url)
            {
                s.queue_open(path);
            }
        })
        .compositor(ViewerCompositor::default());
    #[cfg(target_os = "macos")]
    let _documents = document_open::OpenDocuments::install(app.handle());
    app.try_run()
}
