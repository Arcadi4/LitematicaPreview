//! Platform image clipboards for preview snapshots.
//!
//! Linux uses the GTK clipboard already owned by the presentation stack, so
//! X11 and Wayland both publish the image through the display server's
//! selection. Windows registers `PNG` and `CF_DIBV5` so browsers and
//! Office-style consumers each find a native format.

use crate::preview_image::PreviewImage;

#[cfg(target_os = "linux")]
pub fn copy_image(app: &tauri::AppHandle, image: PreviewImage) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let result: Result<(), String> = (|| {
            let width = i32::try_from(image.width())
                .ok()
                .filter(|width| *width > 0)
                .ok_or("The preview image width is too large for the clipboard.")?;
            let height = i32::try_from(image.height())
                .ok()
                .filter(|height| *height > 0)
                .ok_or("The preview image height is too large for the clipboard.")?;
            let stride = width
                .checked_mul(4)
                .ok_or("The preview image row is too wide for the clipboard.")?;
            let bytes = gtk::glib::Bytes::from_owned(image.into_rgba());
            let pixels = gtk::gdk_pixbuf::Pixbuf::from_bytes(
                &bytes,
                gtk::gdk_pixbuf::Colorspace::Rgb,
                true,
                8,
                width,
                height,
                stride,
            );
            let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
            clipboard.set_image(&pixels);
            // Hand the pixels to a clipboard manager so they can outlive the app.
            clipboard.store();
            Ok(())
        })();
        let _ = sender.send(result);
    })
    .map_err(|error| format!("Unable to reach the window system clipboard: {error}"))?;
    receiver
        .recv()
        .map_err(|error| format!("The window system clipboard did not respond: {error}"))?
}

#[cfg(windows)]
pub fn copy_image(app: &tauri::AppHandle, image: PreviewImage) -> Result<(), String> {
    use std::ptr;
    use std::time::Duration;
    use tauri::Manager;
    use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, HGLOBAL, HWND};
    use windows_sys::Win32::Graphics::Gdi::{BITMAPV5HEADER, BI_BITFIELDS, LCS_GM_IMAGES};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
    use windows_sys::Win32::System::Ole::CF_DIBV5;

    const CLIPBOARD_ATTEMPTS: usize = 8;
    const CLIPBOARD_RETRY: Duration = Duration::from_millis(15);
    const LCS_SRGB: u32 = 0x7352_4742;

    struct GlobalBuffer(HGLOBAL);

    // SAFETY: HGLOBAL is process-wide memory with no thread affinity; the
    // buffer moves to the main-thread clipboard task and is not shared.
    unsafe impl Send for GlobalBuffer {}

    impl GlobalBuffer {
        fn new(bytes: &[u8]) -> Result<Self, String> {
            let handle = unsafe { GlobalAlloc(GHND, bytes.len()) };
            if handle.is_null() {
                return Err(String::from(
                    "Windows could not allocate memory for the clipboard image.",
                ));
            }
            let pointer = unsafe { GlobalLock(handle) };
            if pointer.is_null() {
                unsafe { GlobalFree(handle) };
                return Err(String::from(
                    "Windows could not lock the clipboard image memory.",
                ));
            }
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
                GlobalUnlock(handle);
            }
            Ok(Self(handle))
        }

        fn into_handle(mut self) -> HGLOBAL {
            std::mem::replace(&mut self.0, ptr::null_mut())
        }
    }

    impl Drop for GlobalBuffer {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { GlobalFree(self.0) };
            }
        }
    }

    fn open_clipboard(owner: HWND) -> Result<(), String> {
        let mut last_error = 0;
        for attempt in 0..CLIPBOARD_ATTEMPTS {
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(());
            }
            last_error = unsafe { GetLastError() };
            if attempt + 1 < CLIPBOARD_ATTEMPTS {
                std::thread::sleep(CLIPBOARD_RETRY);
            }
        }
        Err(format!(
            "Windows could not open the clipboard (error {last_error}). Close the application holding it and try again."
        ))
    }

    fn copy_to_clipboard(
        app: &tauri::AppHandle,
        formats: [(u32, GlobalBuffer); 2],
    ) -> Result<(), String> {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| String::from("The main window is unavailable for the clipboard."))?;
        let owner: HWND = window
            .hwnd()
            .map_err(|error| format!("The main window has no native handle: {error}"))?
            .0;
        if owner.is_null() {
            return Err(String::from("The main window has no native handle."));
        }

        // EmptyClipboard makes the window passed to OpenClipboard the owner; a
        // NULL owner makes SetClipboardData fail.
        open_clipboard(owner)?;
        let result = (|| {
            if unsafe { EmptyClipboard() } == 0 {
                return Err(format!(
                    "Windows could not clear the clipboard (error {}).",
                    unsafe { GetLastError() }
                ));
            }
            // PNG preserves the pixels exactly; DIBV5 covers applications that
            // ignore the registered PNG format.
            let mut stored = false;
            let mut first_error = None;
            for (format, buffer) in formats {
                let handle = buffer.into_handle();
                if unsafe { SetClipboardData(format, handle) }.is_null() {
                    let error = unsafe { GetLastError() };
                    unsafe { GlobalFree(handle) };
                    first_error.get_or_insert_with(|| {
                        format!("Windows could not store the clipboard format (error {error}).")
                    });
                } else {
                    stored = true;
                }
            }
            if stored {
                Ok(())
            } else {
                Err(first_error.unwrap_or_else(|| {
                    String::from("Windows did not accept the preview image on the clipboard.")
                }))
            }
        })();
        unsafe { CloseClipboard() };
        result
    }

    fn png_format() -> Result<u32, String> {
        let name: Vec<u16> = "PNG\0".encode_utf16().collect();
        let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        if format == 0 {
            return Err(String::from(
                "Windows could not register the PNG clipboard format.",
            ));
        }
        Ok(format)
    }

    /// A `BITMAPV5HEADER` followed by bottom-up BGRA rows, the usual
    /// `CF_DIBV5` layout Windows applications expect.
    fn dibv5(image: &PreviewImage) -> Result<Vec<u8>, String> {
        let width = i32::try_from(image.width())
            .ok()
            .filter(|width| *width > 0)
            .ok_or("The preview image width is too large for the clipboard.")?;
        let height = i32::try_from(image.height())
            .ok()
            .filter(|height| *height > 0)
            .ok_or("The preview image height is too large for the clipboard.")?;
        let row_bytes = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or("The preview image row is too wide for the clipboard.")?;
        let pixels = row_bytes
            .checked_mul(usize::try_from(height).map_err(|_| "The preview image is too tall.")?)
            .ok_or("The preview image is too large for the clipboard.")?;
        let header_size = std::mem::size_of::<BITMAPV5HEADER>();
        let header = BITMAPV5HEADER {
            bV5Size: header_size as u32,
            bV5Width: width,
            bV5Height: height,
            bV5Planes: 1,
            bV5BitCount: 32,
            bV5Compression: BI_BITFIELDS,
            bV5SizeImage: u32::try_from(pixels)
                .map_err(|_| "The preview image is too large for the clipboard.")?,
            bV5RedMask: 0x00ff_0000,
            bV5GreenMask: 0x0000_ff00,
            bV5BlueMask: 0x0000_00ff,
            bV5AlphaMask: 0xff00_0000,
            bV5CSType: LCS_SRGB,
            bV5Intent: LCS_GM_IMAGES as u32,
            ..Default::default()
        };
        let mut bytes = Vec::with_capacity(header_size + pixels);
        bytes.extend_from_slice(unsafe {
            std::slice::from_raw_parts((&header as *const BITMAPV5HEADER).cast::<u8>(), header_size)
        });
        let rgba = image.rgba();
        for row in (0..image.height()).rev() {
            let start = row as usize * row_bytes;
            for pixel in rgba[start..start + row_bytes].chunks_exact(4) {
                bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        Ok(bytes)
    }

    let png = GlobalBuffer::new(&image.png_bytes()?)?;
    let dib = GlobalBuffer::new(&dibv5(&image)?)?;
    let formats = [(png_format()?, png), (CF_DIBV5 as u32, dib)];
    // EmptyClipboard associates the clipboard with a window, so write from the
    // thread that owns the main window.
    let (sender, receiver) = std::sync::mpsc::channel();
    let main_thread_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(copy_to_clipboard(&main_thread_app, formats));
    })
    .map_err(|error| format!("The main thread is unavailable for the clipboard: {error}"))?;
    receiver
        .recv()
        .map_err(|error| format!("The main thread did not finish the clipboard copy: {error}"))?
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn copy_image(_app: &tauri::AppHandle, _image: PreviewImage) -> Result<(), String> {
    Err(String::from(
        "Copying preview images requires Windows or Linux.",
    ))
}
