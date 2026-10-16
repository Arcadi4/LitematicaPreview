#![cfg(target_os = "windows")]

//! Owns the composition of the wgpu surface and WebView2 DOM layer in the
//! single `main` window; it owns no geometry upload, streaming, or camera state.
//!
//! The host must omit `WS_CLIPCHILDREN`, which clips the full-window WebView
//! child out of the parent's update region and blanks the swapchain. The WebView
//! must be a child of the host so DWM composites the DOM above the parent's
//! redirected surface. Alpha 0 for the WebView's `DefaultBackgroundColor` makes
//! the parent's swapchain its background.

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

/// Errors from preparing the native viewport.
#[derive(Debug)]
pub enum ViewportError {
    /// The main WebView is unavailable; do not create the surface.
    MainWebviewMissing,
    /// Invalid handle or failed handle/size query; skip surface creation.
    WindowHandleUnavailable,
    /// The client size has a zero dimension; wait until it is nonzero.
    ZeroSurfaceSize,
    /// The host carries `WS_CLIPCHILDREN`, which would clip the WebView layer
    /// out of the swapchain's region.
    HostWindowClipped,
    /// The WebView is not a child of the host; restore that relationship before setup.
    WebviewLayerMissing,
    /// Transparent background setup failed; skip surface creation.
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

/// Host HWND and nonzero physical client size used to configure a wgpu surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportSurface {
    hwnd: isize,
    width: u32,
    height: u32,
}

impl ViewportSurface {
    /// Reads the host HWND and physical client size in pixels; errors on failure or zero size.
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

    /// Host window handle used to create the wgpu surface.
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// Physical client size in pixels for the wgpu surface.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Sets the main WebView background transparent, verifies the composition
/// invariants, and returns the host HWND and physical client size in pixels.
pub fn install<R: Runtime>(app: &impl Manager<R>) -> Result<ViewportSurface, ViewportError> {
    // `WebviewWindow` provides the stable Manager API for a window and its WebView.
    let window = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or(ViewportError::MainWebviewMissing)?;

    // Alpha 0 exposes the parent's swapchain as the WebView background.
    window
        .set_background_color(Some(Color(0, 0, 0, 0)))
        .map_err(ViewportError::WebviewBackground)?;

    let surface = ViewportSurface::from_window(&window)?;
    verify_surface_host(surface.hwnd())?;
    Ok(surface)
}

/// Rejects an invalid host or one that cannot layer the DOM above its surface.
fn verify_surface_host(hwnd: isize) -> Result<(), ViewportError> {
    let host: HWND = hwnd as *mut std::ffi::c_void;
    if host.is_null() || unsafe { IsWindow(host) } == 0 {
        return Err(ViewportError::WindowHandleUnavailable);
    }

    // `WS_CLIPCHILDREN` clips the full-window child from the host's update region,
    // blanking the swapchain.
    let host_style = unsafe { GetWindowLongPtrW(host, GWL_STYLE) } as u32;
    if host_style & WS_CLIPCHILDREN != 0 {
        return Err(ViewportError::HostWindowClipped);
    }

    // Without the WebView child, DWM composites the DOM below the parent's redirected surface.
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
