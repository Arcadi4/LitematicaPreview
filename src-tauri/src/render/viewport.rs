#![cfg(target_os = "windows")]

//! Native viewport composition for the Windows host.
//!
//! This module owns exactly one decision: how the wgpu surface (owned by
//! [`crate::render::context`]) and the WebView2 DOM layer (owned by Tauri/wry)
//! are stacked inside the single `"main"` window declared in
//! `tauri.conf.json`. It does not own geometry upload, streaming, or camera
//! state, and it never touches the preview process pipeline.
//!
//! # Decision 1: the wgpu surface targets the MAIN window
//!
//! The surface is created for the main window itself; the WebView2 layer is
//! made transparent and composites above it.
//!
//! Evidence:
//!
//! - This is the verified upstream technique: <https://github.com/tauri-apps/wry/blob/dev/examples/wgpu.rs>
//!   creates the wgpu surface for the top-level window and builds the webview
//!   as a transparent child (`with_transparent(true)`, window
//!   `attributes.transparent = true`).
//! - wry 0.57.0 hosts the webview as a child window (`WS_CHILD`) of the parent
//!   HWND (src/webview2/mod.rs `create_container_hwnd`, the `WRY_WEBVIEW`
//!   container class) and, through a window subclass installed on the parent,
//!   keeps the webview bounds glued to the parent client area on `WM_SIZE` /
//!   `WM_MOVE` (src/webview2/mod.rs `parent_subclass_proc`). The integrator
//!   therefore never has to position or reorder the DOM layer on resize.
//! - DWM composites a redirected parent surface below all of its child
//!   windows, so the WebView2 child paints above the swapchain with no
//!   sibling z-order to manage. Because the webview is the *only* child of the
//!   main window, there is no z-order race to lose.
//! - Per the WebView2 contract, a transparent `DefaultBackgroundColor` makes
//!   the webview render the hosting app's content as its background
//!   (<https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller2?view=webview2-1.0.2849.39>).
//!   "Hosting app content" is the parent window's redirection surface, i.e.
//!   the swapchain.
//!
//! The rejected alternative — a dedicated child HWND hosting the swapchain,
//! ordered below the webview — was dropped because (a) Tauri's public API
//! exposes no handle to the wry webview container (`Webview::with_webview`
//! hands out the wry webview object, not the container HWND), so the required
//! "insert the GPU child below the webview child" ordering could not be done
//! through supported APIs, and (b) it would add a window class, a WndProc and
//! DPI/z-order maintenance that the upstream technique makes unnecessary.
//!
//! # Decision 2: no Win32 fallback ordering is needed
//!
//! - wgpu surface target: `tauri::Window::hwnd()` (windows-only accessor);
//!   context.rs may equally use `HasWindowHandle` on the same window.
//! - WebView2 transparency: `Webview::set_background_color(Some(Color(0, 0, 0,
//!   0)))` — the runtime call path maps to
//!   `ICoreWebView2Controller2::SetDefaultBackgroundColor` with alpha 0
//!   (wry 0.57.0 `set_background_color`; MS documents alpha 0 = transparent,
//!   anything but 0/255 is rejected with `E_INVALIDARG`).
//! - Window-level transparency (`WindowBuilder::transparent(true)`,
//!   `tauri.conf.json > app > windows > transparent`) is deliberately NOT
//!   used: it maps to tao's blur-behind trick
//!   (`DwmEnableBlurBehindWindow` with an empty region, tao 0.37.1
//!   src/platform_impl/windows/window.rs `on_create`), which makes the whole
//!   window a sheet of glass. The viewport renders an opaque clear color, so
//!   glass gains nothing and risks desktop bleed-through before the first
//!   frame.
//! - `WS_CLIPCHILDREN` on the surface host: tao 0.37.1 never sets it
//!   (`to_window_styles` in
//!   <https://github.com/tauri-apps/tao/blob/tauri-v0.37.1/src/platform_impl/windows/window_state.rs>
//!   composes `WS_CAPTION | WS_CLIPSIBLINGS | WS_SYSMENU` plus decoration
//!   bits), wry never sets it on the parent, and the only upstream user that
//!   mentions it (the wry wgpu example) *clears* it (`with_clip_children(false)`;
//!   winit's default is already `false`). `WS_CLIPCHILDREN` excludes child
//!   regions from the parent's update/paint region
//!   (<https://learn.microsoft.com/en-us/windows/win32/winmsg/window-styles>),
//!   which over a full-window webview child would blank the swapchain. Since
//!   no dependency sets it, no style surgery is performed here;
//!   [`install`] instead *verifies* the invariant and fails loudly if a future
//!   toolkit change introduces it.
//!
//! The read-only calls used for that verification
//! (`IsWindow`, `GetWindowLongPtrW`, `GetWindow`, `GetParent`) all come from
//! the `Win32_UI_WindowsAndMessaging` feature already enabled for
//! `windows-sys` 0.61 in `src-tauri/Cargo.toml`; no feature or dependency
//! changes are required.
//!
//! # DOM overlay layering
//!
//! The transparent webview is the single child of the surface host, so every
//! DOM region — Fluent UI dialogs, dropdown menus, the drag overlay, the
//! header/footer/toolbar — is composited above the GPU surface by construction
//! and keeps receiving input: the webview still spans the full client area and
//! still owns pointer/keyboard hit-testing over the viewport region. The
//! frontend therefore keeps the existing overlay markup unchanged; the only
//! frontend requirement is a transparent page background so the swapchain
//! shows through where the DOM does not paint.
//!
//! # Integrator usage
//!
//! ```rust,ignore
//! tauri::Builder::default()
//!     .setup(|app| {
//!         let surface = render::viewport::install(app.handle())?;
//!         let renderer = render::context::init(surface.hwnd(), surface.size(), surface.scale_factor())?;
//!         app.manage(renderer);
//!         Ok(())
//!     })
//! ```
//!
//! On `WindowEvent::Resized` / `ScaleFactorChanged` the integrator re-reads
//! the same facts (`ViewportSurface::from_window`) and reconfigures the wgpu
//! surface; the webview follows the window on its own.
//!
//! # Windows-only verification checklist (real hardware)
//!
//! Runtime verification requires Windows GPU hardware and is out of scope for
//! this wave. Confirm on a real Windows machine:
//!
//! 1. **Layering** — the swapchain fills the whole client area under the
//!    transparent webview; DOM overlays (header, footer, toolbar, settings
//!    panel, dialogs, drag overlay) float above the GPU content.
//! 2. **Startup flash** — no persistent white or black window before the first
//!    presented frame. If a pre-navigation white flicker is observed, set
//!    `WEBVIEW2_DEFAULT_BACKGROUND_COLOR=00000000` in `main()` before the
//!    Tauri runtime starts (documented in
//!    <https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller2?view=webview2-1.0.2849.39>),
//!    or add `"transparent": true` to the window entry of
//!    `tauri.conf.json` (webview layer only benefit; weigh the blur-behind
//!    side effect documented above).
//! 3. **DPI** — moving the window between displays with different scale
//!    factors and dynamic DPI changes reconfigures the surface to the new
//!    physical size with no blur or double scaling (WebView2 handles its own
//!    DPI).
//! 4. **Resize / snap** — drag-resize, Win11 snap layouts, maximize and
//!    restore re-present with no stretch, letterbox, or ghosting.
//! 5. **Fullscreen** — entering and leaving fullscreen reconfigures the
//!    surface to the new presentation size.
//! 6. **Focus** — clicking the viewport moves keyboard focus into the DOM
//!    (wry's parent subclass calls `MoveFocus` on `WM_SETFOCUS`); Tab and
//!    arrow keys reach the canvas element.
//! 7. **Input routing** — orbit/pan/zoom pointer sequences and the context
//!    menu still reach the DOM canvas; DOM overlays remain interactive and
//!    non-overlay regions stay click-through per the existing CSS.
//! 8. **Dialogs** — native dialogs (open/save/licenses) render above the main
//!    window with correct owner, shadow, and modality; they are separate
//!    top-level windows and are unaffected by the surface.
//! 9. **Minimize / restore / occlusion / alt-tab** — presenting suspends on
//!    minimize and resumes on restore with no device-lost state.
//! 10. **WebView2 runtime** — verification uses the Evergreen WebView2
//!     runtime the installer bootstraps; note the exact version tested.
//! 11. **Long-run** — after sustained rendering, semi-transparent DOM overlay
//!     regions still composite over live GPU content (no stale surface
//!     showing through alpha regions).

use std::fmt;

use tauri::webview::Color;
use tauri::webview::WebviewWindow;
use tauri::{Manager, Runtime};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetParent, GetWindow, GetWindowLongPtrW, IsWindow, GWL_STYLE, GW_CHILD, WS_CHILD,
    WS_CLIPCHILDREN,
};

/// Label of the main window declared in `src-tauri/tauri.conf.json`.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// Failures raised while composing the native viewport.
#[derive(Debug)]
pub enum ViewportError {
    /// The `main` WebView window is gone; the DOM layer cannot be placed above
    /// the surface without it.
    MainWebviewMissing,
    /// The window handle could not be read (or is no longer a live window).
    WindowHandleUnavailable,
    /// The host window reported a zero-sized client area; a wgpu surface
    /// cannot be created for it yet.
    ZeroSurfaceSize,
    /// The main window carries `WS_CLIPCHILDREN`. It would clip the webview
    /// child's region out of the parent's update region, blanking the
    /// swapchain underneath the full-window DOM layer. tao 0.37.1 never sets
    /// this style; treat the error as a toolkit regression and clear the bit
    /// upstream rather than papering over it here.
    HostWindowClipped,
    /// The webview is not present as a child window of the surface host, so
    /// the DOM layer would composite below the rendered surface.
    WebviewLayerMissing,
    /// Tauri failed to apply the transparent webview background color.
    WebviewBackground(tauri::Error),
}

impl fmt::Display for ViewportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MainWebviewMissing => {
                write!(f, "The main WebView is no longer available.")
            }
            Self::WindowHandleUnavailable => {
                write!(f, "The main window handle is unavailable or invalid.")
            }
            Self::ZeroSurfaceSize => write!(
                f,
                "The main window client area is empty; defer surface creation until it is shown."
            ),
            Self::HostWindowClipped => write!(
                f,
                "The main window carries WS_CLIPCHILDREN, which would clip the \
                 transparent WebView layer out of the swapchain's region."
            ),
            Self::WebviewLayerMissing => write!(
                f,
                "The WebView layer is not a child of the main window; the DOM \
                 would render below the native surface."
            ),
            Self::WebviewBackground(error) => {
                write!(
                    f,
                    "Unable to make the WebView background transparent: {error}"
                )
            }
        }
    }
}

impl std::error::Error for ViewportError {}

/// Everything the render engine needs to create the native surface for a
/// window: the host HWND, the physical client size, and the current scale
/// factor.
///
/// Re-created from the live window after every resize or scale-factor change;
/// the webview layer needs no equivalent refresh (wry glues it to the parent
/// client area itself).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportSurface {
    hwnd: isize,
    width: u32,
    height: u32,
}

impl ViewportSurface {
    /// Reads the surface facts from a live webview window.
    ///
    /// `WebviewWindow::hwnd()` is Windows-only; this whole module is compiled
    /// only for Windows targets.
    pub fn from_window<R: Runtime>(window: &WebviewWindow<R>) -> Result<Self, ViewportError> {
        let raw = window
            .hwnd()
            .map_err(|_| ViewportError::WindowHandleUnavailable)?;
        let hwnd = raw.0 as isize;
        let size = window
            .inner_size()
            .map_err(|_| ViewportError::WindowHandleUnavailable)?;
        if size.width == 0 || size.height == 0 {
            return Err(ViewportError::ZeroSurfaceSize);
        }
        Ok(Self {
            hwnd,
            width: size.width,
            height: size.height,
        })
    }

    /// Host window handle for the wgpu surface (an `HWND` value).
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// Physical client size in pixels, the wgpu surface configuration size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Composes the native viewport and returns the surface plan.
///
/// Called once from the Tauri `setup` hook, after the main window and webview
/// exist and before the first frame is produced. It:
///
/// 1. makes the main WebView background fully transparent so the swapchain
///    shows through the DOM layer (WebView2 `DefaultBackgroundColor` alpha 0);
/// 2. verifies the composition invariants the layering decision rests on
///    (host window alive and not `WS_CLIPCHILDREN`, webview present as its
///    child window);
/// 3. returns the host HWND, physical client size, and scale factor for the
///    wgpu surface.
///
/// Idempotent: safe to call again (e.g. from tests or a re-setup path).
pub fn install<R: Runtime>(app: &impl Manager<R>) -> Result<ViewportSurface, ViewportError> {
    // The stable Manager access to a window+webview pair is the
    // `WebviewWindow`; `get_window`/`get_webview` are the `unstable` feature.
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or(ViewportError::MainWebviewMissing)?;

    // WebView renders this color underneath all web content; alpha 0 makes it
    // transparent so the hosting app content (the wgpu surface on the parent
    // window) becomes the webview's background. The window layer ignores
    // alpha on Windows, so setting it there is harmless.
    window
        .set_background_color(Some(Color(0, 0, 0, 0)))
        .map_err(ViewportError::WebviewBackground)?;

    let surface = ViewportSurface::from_window(&window)?;
    verify_surface_host(surface.hwnd())?;
    Ok(surface)
}

/// Verifies the host window can present a surface under the webview child.
///
/// All checks are read-only Win32 queries through `windows-sys`
/// (`Win32_UI_WindowsAndMessaging`, already an enabled feature); they raise
/// typed errors instead of mutating toolkit-owned state or panicking.
fn verify_surface_host(hwnd: isize) -> Result<(), ViewportError> {
    let host: HWND = hwnd as *mut std::ffi::c_void;
    if host.is_null() || unsafe { IsWindow(host) } == 0 {
        return Err(ViewportError::WindowHandleUnavailable);
    }

    // The webview child must not clip its region out of the host's update
    // region, or the swapchain disappears under the full-window DOM layer.
    let host_style = unsafe { GetWindowLongPtrW(host, GWL_STYLE) } as u32;
    if host_style & WS_CLIPCHILDREN != 0 {
        return Err(ViewportError::HostWindowClipped);
    }

    // The wry webview container must exist as a child of the host: DWM
    // composites children above the parent's redirected surface, so a missing
    // child means the DOM layer would sit below the rendered surface.
    let child = unsafe { GetWindow(host, GW_CHILD) };
    if child.is_null() || unsafe { GetParent(child) } != host {
        return Err(ViewportError::WebviewLayerMissing);
    }
    let child_style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) } as u32;
    if child_style & WS_CHILD == 0 {
        return Err(ViewportError::WebviewLayerMissing);
    }
    Ok(())
}
