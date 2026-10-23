#![cfg(target_os = "linux")]

//! GTK3 composition for the offscreen native renderer.
//!
//! Tauri packs the WebKitGTK view directly into the window's default vertical
//! box. This module replaces that child with a [`gtk::Overlay`] whose main child
//! is a [`DrawingArea`] and whose overlay child is the unchanged WebView. The
//! DrawingArea displays completed CPU frames; GTK owns the only native window.
//!
//! Preconditions: the install runs on the GTK main thread, and the top-level
//! window has an RGBA visual with application painting (Tauri's `transparent`
//! window setting). WebKitGTK renders through its own GdkWindow, which would
//! obscure the canvas with an opaque visual.
//!
//! [`FrameSink`] may be called from the renderer's readback worker.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use gtk::cairo::{Context, Filter, Format, ImageSurface, Operator};
use gtk::glib;
use gtk::prelude::*;
use gtk::{gdk, DrawingArea, Overlay};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use webkit2gtk::WebViewExt;

use super::frame::{Frame, FramePixels, FrameSink};

const VIEWPORT_FAILED: &str = "viewport-failed";

#[derive(Clone, Serialize)]
struct ViewportFailed {
    message: String,
}

/// A frame whose geometry Cairo has already accepted.
struct QueuedFrame {
    width: i32,
    height: i32,
    stride: i32,
    physical_size: (u32, u32),
    pixels: FramePixels,
}

/// A single-slot mailbox bounds retained frame memory and coalesces updates.
/// `wake_scheduled` admits at most one outstanding main-context idle.
struct FrameMailbox {
    latest: Option<QueuedFrame>,
    wake_scheduled: bool,
    closed: bool,
}

struct Shared {
    mailbox: Mutex<FrameMailbox>,
    app: AppHandle,
    area: glib::SendWeakRef<DrawingArea>,
    background: [f64; 3],
}

struct PaintedFrame {
    surface: ImageSurface,
    /// Host physical size the surface was requested for. Backing dimensions may
    /// be smaller when device limits clamped the texture.
    physical_size: (u32, u32),
}

#[derive(Default)]
struct PaintState {
    frame: Option<PaintedFrame>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, FrameMailbox> {
        self.mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn emit_error(&self, message: String) {
        if self.lock().closed {
            return;
        }
        // Errors must reach the UI; stderr is invisible in a bundled app.
        let _ = self.app.emit(VIEWPORT_FAILED, ViewportFailed { message });
    }

    fn deliver(self: &Arc<Self>, result: Result<Frame, String>) {
        match result {
            Ok(frame) => {
                // A minimized window can report no client area; that is not an error.
                if frame.physical_size.0 == 0 || frame.physical_size.1 == 0 {
                    return;
                }
                let (width, height, stride) = match frame_geometry(&frame) {
                    Ok(geometry) => geometry,
                    Err(reason) => {
                        self.emit_error(format!(
                            "The native renderer returned an unusable graphics frame: {reason}."
                        ));
                        return;
                    }
                };
                let queued = QueuedFrame {
                    width,
                    height,
                    stride,
                    physical_size: frame.physical_size,
                    pixels: frame.pixels,
                };
                let schedule = {
                    let mut mailbox = self.lock();
                    if mailbox.closed {
                        return;
                    }
                    // Replacing the frame returns the old buffer to the pool.
                    mailbox.latest = Some(queued);
                    if mailbox.wake_scheduled {
                        false
                    } else {
                        mailbox.wake_scheduled = true;
                        true
                    }
                };
                if schedule {
                    self.schedule_wake();
                }
            }
            Err(message) => self.emit_error(message),
        }
    }

    fn schedule_wake(self: &Arc<Self>) {
        let shared = Arc::clone(self);
        // This is created on the worker but runs on the default main context;
        // the sendable weak reference may only be dereferenced there.
        let _ = glib::idle_add(move || {
            let Some(area) = shared.area.upgrade() else {
                shared.close();
                return glib::ControlFlow::Break;
            };
            let draw = {
                let mut mailbox = shared.lock();
                if mailbox.closed {
                    false
                } else {
                    mailbox.wake_scheduled = false;
                    mailbox.latest.is_some()
                }
            };
            if draw {
                area.queue_draw();
            }
            glib::ControlFlow::Break
        });
    }

    fn take_latest(&self) -> Option<QueuedFrame> {
        self.lock().latest.take()
    }

    fn close(&self) {
        let mut mailbox = self.lock();
        mailbox.closed = true;
        mailbox.wake_scheduled = false;
        mailbox.latest = None;
    }
}

/// Installs the canvas below the existing WebView and returns its frame sink.
///
/// Must run on the GTK main thread after Tauri has built the main window, with
/// a transparent top-level visual. An unsupported window layout is an error,
/// never a silent fallback.
/// `on_resize` may run before return and later on GTK allocation/scale changes.
pub fn install(
    window: &WebviewWindow,
    on_resize: impl Fn((u32, u32), f64) + 'static,
) -> Result<FrameSink, String> {
    if cfg!(target_endian = "big") {
        return Err(String::from(
            "Native Linux composition requires a little-endian target because Cairo RGB24 is \
             filled with the renderer's BGRA bytes.",
        ));
    }
    if !gtk::is_initialized_main_thread() {
        return Err(String::from(
            "The Linux viewport must be installed on the GTK main thread during application setup.",
        ));
    }

    let gtk_window = window
        .gtk_window()
        .map_err(|error| format!("The main window has no GTK window: {error}"))?;
    let vbox = window
        .default_vbox()
        .map_err(|error| format!("The main window has no Tauri content box: {error}"))?;

    let Some(parent) = vbox.parent() else {
        return Err(String::from(
            "The Tauri content box has no GTK parent; the standard Tauri window layout is \
             required for native Linux composition.",
        ));
    };
    if parent != gtk_window.clone().upcast::<gtk::Widget>() {
        return Err(String::from(
            "The Tauri content box is not a child of the main GTK window; native Linux \
             composition supports only the standard Tauri window layout.",
        ));
    }

    // WebKitGTK creates its own GdkWindow with the inherited visual, so an
    // opaque top-level visual permanently hides the canvas behind it.
    let rgba_visual = gtk::prelude::WidgetExt::screen(&gtk_window)
        .and_then(|screen| screen.rgba_visual())
        .ok_or_else(|| {
            String::from(
                "The GTK display has no RGBA visual; native Linux viewport composition needs a \
                 compositing-friendly display.",
            )
        })?;
    if gtk_window.visual().as_ref() != Some(&rgba_visual) || !gtk_window.is_app_paintable() {
        return Err(String::from(
            "The main GTK window was not created for transparent composition. Enable Linux \
             window transparency (`transparent: true`) so the WebKit layer can reveal the \
             native canvas.",
        ));
    }

    let mut views = vbox
        .children()
        .into_iter()
        .filter_map(|child| child.downcast::<webkit2gtk::WebView>().ok());
    let Some(webview) = views.next() else {
        return Err(String::from(
            "The main window has no WebKitGTK child directly in Tauri's content box; native \
             Linux composition supports one standard window-content WebView.",
        ));
    };
    if views.next().is_some() {
        return Err(String::from(
            "The main window has more than one WebKitGTK child in Tauri's content box; native \
             Linux composition supports a single window-content WebView.",
        ));
    }

    // Preserve Tauri's packing, index (menu bars stay first), and focus.
    let (expand, fill, padding, pack_type) = vbox.query_child_packing(&webview);
    let packed_from_start = match pack_type {
        gtk::PackType::Start => true,
        gtk::PackType::End => false,
        _ => {
            return Err(String::from(
                "The Tauri content box reported an unknown child packing mode; the GTK window \
                 layout is unsupported.",
            ));
        }
    };
    let position = vbox
        .children()
        .iter()
        .position(|child| child == webview.upcast_ref::<gtk::Widget>())
        .and_then(|position| i32::try_from(position).ok())
        .ok_or_else(|| {
            String::from(
                "The WebKitGTK child disappeared from Tauri's content box during viewport setup.",
            )
        })?;
    let had_focus = webview.has_focus();

    webview.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));

    let canvas = DrawingArea::new();
    // No GdkWindow, events, or focus: the WebView above keeps all input.
    canvas.set_has_window(false);
    canvas.set_can_focus(false);
    canvas.set_focus_on_click(false);
    canvas.set_events(gdk::EventMask::empty());
    canvas.set_app_paintable(true);
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);

    let overlay = Overlay::new();
    overlay.set_can_focus(false);
    overlay.set_focus_on_click(false);

    let paint = Rc::new(RefCell::new(PaintState::default()));
    let shared = Arc::new(Shared {
        mailbox: Mutex::new(FrameMailbox {
            latest: None,
            wake_scheduled: false,
            closed: false,
        }),
        app: window.app_handle().clone(),
        area: glib::SendWeakRef::from(canvas.downgrade()),
        background: super::context::CLEAR_LINEAR.map(|linear| {
            if linear <= 0.003_130_8 {
                12.92 * linear
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            }
        }),
    });

    let on_resize: Rc<dyn Fn((u32, u32), f64)> = Rc::new(on_resize);
    install_signals(&canvas, &shared, &paint, Rc::clone(&on_resize));

    vbox.remove(&webview);
    overlay.add(&canvas);
    overlay.add_overlay(&webview);
    if packed_from_start {
        vbox.pack_start(&overlay, expand, fill, padding);
    } else {
        vbox.pack_end(&overlay, expand, fill, padding);
    }
    vbox.reorder_child(&overlay, position);

    // GTK widget moves can silently fail; restore instead of leaving a blank window.
    let overlay_widget: gtk::Widget = overlay.clone().upcast();
    let vbox_widget: gtk::Widget = vbox.clone().upcast();
    let hierarchy_ok = webview.parent().as_ref() == Some(&overlay_widget)
        && canvas.parent().as_ref() == Some(&overlay_widget)
        && overlay.parent().as_ref() == Some(&vbox_widget);
    if !hierarchy_ok {
        restore_webview(
            &vbox,
            &overlay,
            &canvas,
            &webview,
            packed_from_start,
            expand,
            fill,
            padding,
            position,
        );
        return Err(String::from(
            "GTK rejected the main window's overlay hierarchy; the WebView was restored and \
             native Linux composition was not installed.",
        ));
    }

    overlay.show_all();
    if had_focus {
        webview.grab_focus();
    }
    notify_size(&canvas, on_resize.as_ref());

    Ok(Arc::new(move |result| shared.deliver(result)))
}

fn install_signals(
    canvas: &DrawingArea,
    shared: &Arc<Shared>,
    paint: &Rc<RefCell<PaintState>>,
    on_resize: Rc<dyn Fn((u32, u32), f64)>,
) {
    let paint_for_draw = Rc::clone(paint);
    let shared_for_draw = Arc::clone(shared);
    canvas.connect_draw(move |canvas, context| {
        draw(canvas, context, &paint_for_draw, &shared_for_draw);
        glib::Propagation::Proceed
    });

    let paint_for_destroy = Rc::clone(paint);
    let shared_for_destroy = Arc::clone(shared);
    canvas.connect_destroy(move |_| {
        shared_for_destroy.close();
        // Release the Cairo surface on main so its pixels return to the pool.
        paint_for_destroy.borrow_mut().frame = None;
    });

    let allocation_resize = Rc::clone(&on_resize);
    canvas.connect_size_allocate(move |canvas, _| {
        notify_size(canvas, allocation_resize.as_ref());
    });
    canvas.connect_scale_factor_notify(move |canvas| {
        notify_size(canvas, on_resize.as_ref());
    });
}

/// Drawable pixels exclude GTK client-side decorations and shadows.
fn notify_size(canvas: &DrawingArea, on_resize: &dyn Fn((u32, u32), f64)) {
    canvas.queue_draw();
    if let Some(size) = expected_physical_size(canvas) {
        on_resize(size, f64::from(canvas.scale_factor()));
    }
}

fn draw(canvas: &DrawingArea, context: &Context, paint: &RefCell<PaintState>, shared: &Shared) {
    let allocation = canvas.allocation();
    let width = allocation.width();
    let height = allocation.height();
    paint_background(context, width, height, shared.background);
    if width <= 0 || height <= 0 {
        return;
    }
    let Some(expected) = expected_physical_size(canvas) else {
        return;
    };

    if let Some(frame) = shared.take_latest() {
        if frame.physical_size == expected {
            let QueuedFrame {
                width,
                height,
                stride,
                pixels,
                ..
            } = frame;
            match ImageSurface::create_for_data(pixels, Format::Rgb24, width, height, stride) {
                Ok(surface) => {
                    paint.borrow_mut().frame = Some(PaintedFrame {
                        surface,
                        physical_size: expected,
                    });
                }
                Err(error) => {
                    paint.borrow_mut().frame = None;
                    shared.emit_error(format!(
                        "The native graphics frame could not be prepared for display: {error}."
                    ));
                }
            }
        }
        // A frame requested for another physical size is dropped, never stretched.
    }

    let matches = paint
        .borrow()
        .frame
        .as_ref()
        .is_some_and(|painted| painted.physical_size == expected);
    if !matches {
        paint.borrow_mut().frame = None;
        return;
    }

    let paint_error = {
        let state = paint.borrow();
        state
            .frame
            .as_ref()
            .and_then(|painted| paint_frame(context, &painted.surface, width, height).err())
    };
    if let Some(error) = paint_error {
        paint.borrow_mut().frame = None;
        shared.emit_error(format!(
            "The native graphics frame could not be painted: {error}."
        ));
    }
}

fn paint_background(context: &Context, width: i32, height: i32, background: [f64; 3]) {
    if width <= 0 || height <= 0 {
        return;
    }
    context.set_operator(Operator::Source);
    context.set_source_rgb(background[0], background[1], background[2]);
    context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
    let _ = context.fill();
    context.set_operator(Operator::Over);
}

/// Maps the entire backing texture across the widget's logical allocation.
/// GTK's draw context carries the device scale, so a matching backing size is a
/// 1:1 pixel mapping and a clamped backing size is downscaled.
fn paint_frame(
    context: &Context,
    surface: &ImageSurface,
    width: i32,
    height: i32,
) -> Result<(), gtk::cairo::Error> {
    let surface_width = surface.width();
    let surface_height = surface.height();
    if surface_width <= 0 || surface_height <= 0 {
        return Ok(());
    }
    context.save()?;
    context.set_operator(Operator::Over);
    context.scale(
        f64::from(width) / f64::from(surface_width),
        f64::from(height) / f64::from(surface_height),
    );
    context.set_source_surface(surface, 0.0, 0.0)?;
    context.source().set_filter(Filter::Good);
    context.paint()?;
    context.restore()
}

/// Physical drawable extent before the renderer applies device limits.
fn expected_physical_size(canvas: &DrawingArea) -> Option<(u32, u32)> {
    let allocation = canvas.allocation();
    let width = u32::try_from(allocation.width()).ok()?;
    let height = u32::try_from(allocation.height()).ok()?;
    let scale = canvas.scale_factor();
    if width == 0 || height == 0 || scale <= 0 {
        return None;
    }
    let scale = scale as u32;
    Some((width.checked_mul(scale)?, height.checked_mul(scale)?))
}

/// Cairo RGB24 stores little-endian BGRA bytes, matching the renderer's
/// readback. Padded strides are preserved.
fn frame_geometry(frame: &Frame) -> Result<(i32, i32, i32), &'static str> {
    let width = i32::try_from(frame.width).map_err(|_| "its width exceeds the Cairo limit")?;
    let height = i32::try_from(frame.height).map_err(|_| "its height exceeds the Cairo limit")?;
    let stride =
        i32::try_from(frame.stride).map_err(|_| "its row stride exceeds the Cairo limit")?;
    if width <= 0 || height <= 0 {
        return Err("it has no pixels");
    }
    let minimum_stride = width
        .checked_mul(4)
        .ok_or("its row stride overflows the Cairo limit")?;
    if stride < minimum_stride || stride % 4 != 0 {
        return Err("its row stride cannot describe opaque BGRA pixels");
    }
    height
        .checked_mul(stride)
        .ok_or("its pixel buffer size exceeds the Cairo limit")?;
    let required = (stride as usize)
        .checked_mul(height as usize)
        .ok_or("its pixel buffer size overflows")?;
    let pixels: &[u8] = frame.pixels.as_ref();
    if pixels.len() < required {
        return Err("its pixel buffer is shorter than its stride and height");
    }
    Ok((width, height, stride))
}

fn restore_webview(
    vbox: &gtk::Box,
    overlay: &Overlay,
    canvas: &DrawingArea,
    webview: &webkit2gtk::WebView,
    packed_from_start: bool,
    expand: bool,
    fill: bool,
    padding: u32,
    position: i32,
) {
    overlay.remove(webview);
    overlay.remove(canvas);
    vbox.remove(overlay);
    if packed_from_start {
        vbox.pack_start(webview, expand, fill, padding);
    } else {
        vbox.pack_end(webview, expand, fill, padding);
    }
    vbox.reorder_child(webview, position);
    webview.show();
}
