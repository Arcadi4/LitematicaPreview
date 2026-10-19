//! Device, surface and frame orchestration for the native renderer.
//!
//! Presentation goes directly to the main window's swapchain, with no
//! offscreen target or output pass. An `*Srgb` surface format encodes on store.
//! Windows uses DX12 only; there is no software fallback.
//! The camera uses CSS pixels; the swapchain is bounded by device limits.
//! Drawn geometry clips to the DOM-reported stage rectangle, while the clear
//! spans the window.
//!
//! Scene state lives in [`crate::render::scene::Scene`]; camera math lives in
//! [`crate::render::camera::Camera`].

use crate::render::camera::Camera;
use crate::render::pipelines::{Pipelines, CAMERA_UNIFORM_BYTES};
use crate::render::scene::Scene;
use crate::render::upload::{DeviceStatus, UploadPermit, UploadWindow};
use litematica_preview_core::{MeshBatch, PreviewInfo};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CLEAR_SRGB: [f64; 3] = [0.00335, 0.00518, 0.00802];

/// Errors the renderer can hand to the Tauri layer.
#[derive(Debug)]
pub enum RenderError {
    /// A frame could not be acquired or presented.
    SurfaceLost,
    /// Adapter or device creation failed.
    Initialization(String),
    /// A Core batch did not match the supported geometry layout.
    MalformedBatch(String),
    /// A device error or loss interrupted GPU work.
    Device(String),
}

impl RenderError {
    pub(crate) fn malformed_batch(message: &'static str) -> Self {
        Self::MalformedBatch(String::from(message))
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::SurfaceLost => write!(
                formatter,
                "The graphics surface is unavailable. Reopen the schematic after the window recovers."
            ),
            Self::Initialization(message) | Self::MalformedBatch(message) | Self::Device(message) => {
                write!(formatter, "{message}")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// One device, surface and progressively displayed scene. Only upload-window
/// waits may block for GPU completion, and those run outside this renderer.
pub struct NativeRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    pipelines: Pipelines,
    camera: Camera,
    config: wgpu::SurfaceConfiguration,
    physical_size: (u32, u32),
    /// Physical-pixel to CSS-pixel scale.
    scale: f64,
    /// DOM-reported stage rectangle in CSS pixels.
    stage_css: Option<(f32, f32, f32, f32)>,
    /// MSAA attachment view (`samples` > 1 only).
    color_view: Option<wgpu::TextureView>,
    depth_view: Option<wgpu::TextureView>,
    samples: u32,
    scene: Option<Scene>,
    device_status: Arc<DeviceStatus>,
    load_prepared: bool,
    user_interacted: bool,
    has_visible_batch: bool,
    last_render: Option<Instant>,
}

impl NativeRenderer {
    /// Creates a renderer from raw window handles and a physical pixel size.
    /// On Windows, initialization requires a Direct3D 12 adapter; there is no
    /// software fallback.
    #[allow(dead_code)]
    pub fn new_from_raw(
        handle: wgpu::rwh::RawWindowHandle,
        display: Option<wgpu::rwh::RawDisplayHandle>,
        (width, height): (u32, u32),
    ) -> Result<Self, RenderError> {
        #[cfg(windows)]
        let backends = wgpu::Backends::DX12;
        #[cfg(not(windows))]
        let backends = wgpu::Backends::METAL;
        let _ = backends;
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
        let surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: display,
                    raw_window_handle: handle,
                })
                .map_err(|error| {
                    RenderError::Initialization(format!(
                        "The application could not attach a graphics surface to the window: {error}"
                    ))
                })?
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|_| {
            RenderError::Initialization(String::from(
                "Direct3D 12 is unavailable. Update your graphics driver to preview schematics.",
            ))
        })?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("schematic renderer"),
            required_features: wgpu::Features::empty(),
            required_limits: Self::limits(&adapter),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        }))
        .map_err(|error| {
            RenderError::Initialization(format!(
                "The graphics device could not initialize: {error}"
            ))
        })?;
        let format = Self::surface_format(&surface, &adapter)?;
        let alpha_mode = {
            let capabilities = surface.get_capabilities(&adapter);
            capabilities.alpha_modes.first().copied().ok_or_else(|| {
                RenderError::Initialization(String::from(
                    "The graphics surface has no presentation modes.",
                ))
            })?
        };
        let depth_format = wgpu::TextureFormat::Depth24Plus;
        let samples = Self::samples(&adapter, format);
        let pipelines = Pipelines::new(&device, samples, format, depth_format);
        let physical_size = (width.max(1), height.max(1));
        let (width, height) = backing_size(physical_size, device.limits().max_texture_dimension_2d);
        let mut config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);
        config.view_formats = Vec::new();
        let scale = 1.0;
        let camera = Camera::new();
        let (depth_view, color_view) = (None, None);
        let device_status = DeviceStatus::new(device.clone());
        let mut renderer = Self {
            device,
            queue,
            surface,
            pipelines,
            camera,
            config,
            physical_size,
            scale,
            stage_css: None,
            color_view,
            depth_view,
            samples,
            scene: None,
            device_status,
            load_prepared: false,
            user_interacted: false,
            has_visible_batch: false,
            last_render: None,
        };
        renderer.rebuild_targets();
        renderer.refresh_camera();
        Ok(renderer)
    }

    /// Convenience for the Tauri layer: builds the raw handle pair from an
    /// `HWND` (`ViewportSurface::hwnd()`). Compiles for all hosts; on a
    /// non-Windows host the adapter request itself fails with the
    /// Initialization message.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn new_from_hwnd(hwnd: isize, size: (u32, u32)) -> Result<Self, RenderError> {
        let pointer = std::num::NonZeroIsize::new(hwnd).ok_or_else(|| {
            RenderError::Initialization(String::from(
                "The application window has no native handle.",
            ))
        })?;
        let mut window = wgpu::rwh::Win32WindowHandle::new(pointer);
        window.hinstance = None;
        let handle = wgpu::rwh::RawWindowHandle::Win32(window);
        let display = wgpu::rwh::RawDisplayHandle::Windows(wgpu::rwh::WindowsDisplayHandle::new());
        Self::new_from_raw(handle, Some(display), size)
    }

    fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        let adapter_limits = adapter.limits();
        let max_buffer_size = adapter_limits.max_buffer_size;
        let mut limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter_limits);
        limits.max_buffer_size = max_buffer_size;
        limits
    }

    /// Prefers an sRGB format so blending occurs in linear space.
    fn surface_format(
        surface: &wgpu::Surface,
        adapter: &wgpu::Adapter,
    ) -> Result<wgpu::TextureFormat, RenderError> {
        let capabilities = surface.get_capabilities(adapter);
        let requested = [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        requested
            .iter()
            .find(|format| capabilities.formats.contains(format))
            .copied()
            .ok_or_else(|| {
                RenderError::Initialization(String::from(
                    "The graphics surface does not offer an sRGB presentation format.",
                ))
            })
    }

    /// Uses 4x MSAA when supported by the surface format; otherwise 1x.
    fn samples(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> u32 {
        let features = adapter.get_texture_format_features(format);
        if features.flags.sample_count_supported(4) {
            4
        } else {
            1
        }
    }

    /// Builds depth and MSAA attachments for the current size.
    fn depth_and_color(&self) -> (wgpu::TextureView, Option<wgpu::TextureView>) {
        let (width, height) = (self.config.width.max(1), self.config.height.max(1));
        let depth = self
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth view"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: self.samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24Plus,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let color = (self.samples > 1).then(|| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("multisample color view"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: self.samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.config.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        (depth, color)
    }


    fn rebuild_targets(&mut self) {
        let (depth, color) = self.depth_and_color();
        self.depth_view = Some(depth);
        self.color_view = color;
    }

    /// Reconfigures for a new physical size. Zero extents are ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.physical_size = (width, height);
        let (width, height) = backing_size(
            self.physical_size,
            self.device.limits().max_texture_dimension_2d,
        );
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.rebuild_targets();
        }
        self.refresh_camera();
    }

    /// Updates the DPI scale used to convert physical dimensions to CSS pixels.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = scale.max(0.25);
        self.refresh_camera();
    }

    /// Records the visible stage rectangle in CSS pixels and re-fits the
    /// camera when its size changes. Invalid or unchanged values are ignored.
    pub fn set_stage(&mut self, left: f32, top: f32, width: f32, height: f32) {
        let next = Some((left, top, width, height));
        if !left.is_finite()
            || !top.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || self.stage_css == next
        {
            return;
        }
        self.stage_css = next;
        self.refresh_camera();
    }

    /// Sizes the camera to the reported stage rectangle, or the whole window
    /// until a stage rectangle is available.
    fn refresh_camera(&mut self) {
        let (css_width, css_height) = self.stage_css.map_or_else(
            || {
                (
                    (self.physical_size.0 as f64 / self.scale) as f32,
                    (self.physical_size.1 as f64 / self.scale) as f32,
                )
            },
            |(_, _, width, height)| (width, height),
        );
        self.camera.resize(css_width, css_height);
        self.camera.mark_dirty();
    }

    pub fn upload_window(&self, capacity: usize) -> Arc<UploadWindow> {
        Arc::new(UploadWindow::with_device(
            Arc::clone(&self.device_status),
            capacity,
        ))
    }

    /// Drops the previous model and resets framing before a new stream starts.
    pub fn begin_load(&mut self) {
        self.cancel_load();
        self.scene = Some(Scene::new(self.device.clone(), self.queue.clone()));
    }

    /// Frames occupied source bounds once, before the first batch arrives.
    pub fn prepare_load(&mut self, min: [f32; 3], max: [f32; 3]) {
        if self.load_prepared {
            return;
        }
        self.camera.set_model_bounds(min, max);
        self.load_prepared = true;
    }

    /// Appends a live batch and flushes staging even when no frame is due.
    pub fn attach_batch(
        &mut self,
        batch: &MeshBatch,
        permit: UploadPermit,
    ) -> Result<(), RenderError> {
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| RenderError::malformed_batch("the preview load has not started"))?;
        if let Err(error) = scene.attach_batch(&self.pipelines, batch, permit) {
            self.cancel_load();
            return Err(error);
        }
        if !batch.parts.is_empty() {
            self.camera.mark_dirty();
            let render_now = !self.has_visible_batch
                || self
                    .last_render
                    .is_none_or(|time| time.elapsed() >= Duration::from_millis(50));
            self.has_visible_batch = true;
            if render_now {
                self.render()?;
            }
        }
        Ok(())
    }

    /// Replaces source bounds with exact geometry bounds; input always wins over auto-fit.
    pub fn finish_load(&mut self, metadata: &PreviewInfo) {
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        scene.finish(metadata);
        if self.user_interacted && self.load_prepared {
            self.camera.update_model_bounds(metadata.min, metadata.max);
        } else {
            self.camera.set_model_bounds(metadata.min, metadata.max);
        }
        self.load_prepared = true;
    }

    /// Clears all displayed geometry; the caller renders the empty surface immediately.
    pub fn cancel_load(&mut self) {
        self.scene = None;
        self.load_prepared = false;
        self.user_interacted = false;
        self.has_visible_batch = false;
        self.last_render = None;
        self.camera.clear_model();
    }

    pub fn mark_user_interaction(&mut self) {
        self.user_interacted = true;
    }

    pub fn camera(&mut self) -> &mut Camera {
        &mut self.camera
    }

    /// Presents one frame; a clean camera makes this a no-op.
    pub fn render(&mut self) -> Result<(), RenderError> {
        self.device_status.check().map_err(RenderError::Device)?;
        if !self.camera.dirty() {
            return Ok(());
        }
        self.pipelines.write_cameras(
            &self.queue,
            self.camera.frame_camera(true),
            self.camera.frame_camera(false),
        );
        let frame = self.acquire()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = self.depth_view.as_ref().ok_or_else(|| {
            RenderError::Initialization(String::from(
                "The depth attachment is unavailable. Reopen the schematic to retry.",
            ))
        })?;
        let clear = clear_color_linear();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("schematic frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("schematic render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.color_view.as_ref().unwrap_or(&view),
                    depth_slice: None,
                    resolve_target: self.color_view.as_ref().map(|_| &view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let viewport = map_stage_viewport(
                self.stage_css,
                self.scale,
                self.physical_size,
                (self.config.width, self.config.height),
                self.device.limits().max_texture_dimension_2d,
            );
            if let Some((left, top, width, height)) = viewport {
                pass.set_viewport(left, top, width, height, 0.0, 1.0);
                if let Some(scene) = self.scene.as_ref() {
                    let grid_visible = self.camera.grid_visible();
                    scene.draw(&self.pipelines, &mut pass, grid_visible)?;
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        self.camera.clear_dirty();
        self.last_render = Some(Instant::now());
        Ok(())
    }

    /// Acquires the next swapchain frame, reconfiguring once on an expected
    /// resize race and failing with `SurfaceLost` for everything else.
    fn acquire(&mut self) -> Result<wgpu::SurfaceTexture, RenderError> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(texture),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.rebuild_targets();
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(texture),
                    _ => Err(RenderError::SurfaceLost),
                }
            }
            _ => Err(RenderError::SurfaceLost),
        }
    }
}

fn backing_size((width, height): (u32, u32), max_dimension: u32) -> (u32, u32) {
    (
        width.clamp(1, max_dimension),
        height.clamp(1, max_dimension),
    )
}

/// A stale DOM stage can exceed viewport limits while a resize is in flight.
fn map_stage_viewport(
    stage_css: Option<(f32, f32, f32, f32)>,
    scale: f64,
    physical_size: (u32, u32),
    backing_size: (u32, u32),
    max_dimension: u32,
) -> Option<(f32, f32, f32, f32)> {
    let Some((left, top, width, height)) = stage_css else {
        return Some((0.0, 0.0, backing_size.0 as f32, backing_size.1 as f32));
    };
    let scale_x = scale * backing_size.0 as f64 / physical_size.0 as f64;
    let scale_y = scale * backing_size.1 as f64 / physical_size.1 as f64;
    let [left, top, width, height] = [
        (left as f64 * scale_x) as f32,
        (top as f64 * scale_y) as f32,
        (width as f64 * scale_x) as f32,
        (height as f64 * scale_y) as f32,
    ];
    ([left, top, width, height]
        .iter()
        .all(|value| value.is_finite())
        && left >= 0.0
        && top >= 0.0
        && width > 0.0
        && height > 0.0
        && left as f64 + width as f64 <= max_dimension as f64
        && top as f64 + height as f64 <= max_dimension as f64)
        .then_some((left, top, width, height))
}

/// Matches the camera buffer size to `FrameCamera::encode`.
const _: () = {
    assert!(CAMERA_UNIFORM_BYTES == 80, "camera uniform layout drifted");
};

fn clear_color_linear() -> [f64; 3] {
    CLEAR_SRGB.map(|channel| {
        // The inverse sRGB transfer function with the standard cutoff.
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::NativeRenderer;
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};

    #[test]
    #[ignore = "Requires a Windows Direct3D 12 hardware adapter"]
    fn renderer_preserves_pixel_aspect_from_initialization_through_resize() {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                800,
                400,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        struct Window(windows_sys::Win32::Foundation::HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let _window = Window(hwnd);
        let mut renderer = NativeRenderer::new_from_hwnd(hwnd as isize, (800, 400)).unwrap();
        renderer.camera().set_model_bounds([-1.0; 3], [1.0; 3]);
        let check =
            |renderer: &NativeRenderer| {
                let matrix = renderer.camera.frame_camera(false).view_projection;
                let row_length = |row: usize| {
                    (0..3)
                        .map(|column| matrix[column * 4 + row].powi(2))
                        .sum::<f32>()
                        .sqrt()
                };
                let horizontal = row_length(0) * renderer.config.width as f32;
                let vertical = row_length(1) * renderer.config.height as f32;
                assert!((horizontal / vertical - 1.0).abs() < 0.002,
                "World-unit pixel scales must match: horizontal={horizontal}, vertical={vertical}");
            };
        check(&renderer);
        for (width, height, scale) in [(400, 800, 1.25), (1200, 400, 1.5), (800, 800, 2.0)] {
            renderer.resize(width, height);
            renderer.set_scale(scale);
            check(&renderer);
        }
    }
}
