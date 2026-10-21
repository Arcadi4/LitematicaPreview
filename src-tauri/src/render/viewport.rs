//! Provides native window handles and size for the cross-platform wgpu surface.

use std::fmt;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use tauri::webview::{Color, WebviewWindow};
use tauri::{Manager, Runtime};

pub const MAIN_WINDOW_LABEL: &str = "main";

#[derive(Debug)]
pub enum ViewportError {
    MainWebviewMissing,
    WindowHandleUnavailable,
    ZeroSurfaceSize,
    WebviewBackground(tauri::Error),
}

impl fmt::Display for ViewportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MainWebviewMissing => write!(f, "The main WebView is no longer available."),
            Self::WindowHandleUnavailable => write!(f, "The main window handle is unavailable or invalid."),
            Self::ZeroSurfaceSize => write!(f, "The main window client area is empty; defer surface creation until it is shown."),
            Self::WebviewBackground(error) => write!(f, "Unable to make the WebView background transparent: {error}"),
        }
    }
}

impl std::error::Error for ViewportError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportSurface {
    handle: RawWindowHandle,
    display: RawDisplayHandle,
    width: u32,
    height: u32,
}

impl ViewportSurface {
    pub fn handle(&self) -> RawWindowHandle { self.handle }
    pub fn display(&self) -> RawDisplayHandle { self.display }
    pub fn size(&self) -> (u32, u32) { (self.width, self.height) }
}

pub fn install<R: Runtime>(app: &impl Manager<R>) -> Result<ViewportSurface, ViewportError> {
    let window: WebviewWindow<R> = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or(ViewportError::MainWebviewMissing)?;
    window
        .set_background_color(Some(Color(0, 0, 0, 0)))
        .map_err(ViewportError::WebviewBackground)?;
    let size = window
        .inner_size()
        .map_err(|_| ViewportError::WindowHandleUnavailable)?;
    if size.width == 0 || size.height == 0 {
        return Err(ViewportError::ZeroSurfaceSize);
    }
    let display = window
        .display_handle()
        .map_err(|_| ViewportError::WindowHandleUnavailable)?
        .as_raw();
    let handle = window
        .window_handle()
        .map_err(|_| ViewportError::WindowHandleUnavailable)?
        .as_raw();
    Ok(ViewportSurface { handle, display, width: size.width, height: size.height })
}
