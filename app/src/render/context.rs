//! Reusable GPU scene rendering for the Rune-composited preview.
//!
//! A [`LiveScene`] owns the uploaded geometry, material pipelines and a
//! reusable sRGB presentation texture. The host encodes one frame into its own
//! command encoder and composites the returned texture view into Rune's
//! compositor, so presentation never reads pixels back. [`LiveScene::capture`]
//! renders the same scene into a one-shot capture target for PNG, JPEG and
//! clipboard export on a background task.
//!
//! Every target uses an `*Srgb` format, so blends and stores encode in linear
//! space. The camera is owned by the host (one per preview tab); this module
//! only consumes it. Scene state lives in [`crate::render::scene::Scene`];
//! camera math lives in [`crate::render::camera::Camera`].

use crate::render::camera::{Camera, FrameCamera};
use crate::render::capture::CaptureJob;
use crate::render::lighting::FrameLighting;
use crate::render::pipelines::{CAMERA_UNIFORM_BYTES, Pipelines};
use crate::render::scene::Scene;
use crate::render::upload::{DeviceStatus, UploadPermit, UploadWindow};
use litematica_preview_core::{MeshBatch, PreviewInfo};
use std::sync::Arc;

/// Opaque backdrop for browser thumbnails, which never expose transparency.
const THUMBNAIL_CLEAR: [f64; 3] = [0.00335, 0.00518, 0.00802];

/// Format of every presentation and capture target. The Rune compositor
/// samples the same sRGB view, so its blit pipeline must declare this format.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

/// Depth attachment format shared by every scene pass.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

/// Errors the renderer can hand to the host.
#[derive(Debug)]
pub enum RenderError {
    /// Adapter or device creation failed.
    Initialization(String),
    /// A Core batch did not match the supported geometry layout.
    MalformedBatch(String),
    /// A device error or loss interrupted GPU work.
    Device(String),
    /// The current scene cannot produce a preview image.
    Capture(String),
}

impl RenderError {
    pub(crate) fn malformed_batch(message: &'static str) -> Self {
        Self::MalformedBatch(String::from(message))
    }
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Initialization(message)
            | Self::MalformedBatch(message)
            | Self::Device(message)
            | Self::Capture(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// One device, one progressively uploaded scene and one reusable presentation
/// texture.
///
/// Uploads only reserve staging credits and write GPU buffers; nothing is
/// drawn or presented here. Each preview tab attaches its stream to its own
/// `LiveScene` and keeps its own [`Camera`]. The host calls [`Self::encode`]
/// whenever it needs a fresh frame for the Rune compositor.
pub struct LiveScene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Arc<Pipelines>,
    format: wgpu::TextureFormat,
    samples: u32,
    scene: Option<Scene>,
    device_status: Arc<DeviceStatus>,
    /// Source or final model bounds; used to fit canonical thumbnails.
    model_bounds: Option<([f32; 3], [f32; 3])>,
    has_visible_batch: bool,
    lighting: FrameLighting,
    /// Reused presentation target, replaced only when the effect size changes.
    presentation: Option<PresentationTarget>,
    depth_view: Option<wgpu::TextureView>,
    /// MSAA attachment view (`samples` > 1 only).
    color_view: Option<wgpu::TextureView>,
    /// Bumped whenever the encoded image would change.
    revision: u64,
    /// Size, camera and revision of the last encoded frame.
    last_encoded: Option<EncodedFrame>,
}

/// Snapshot of the last encoded frame, used to skip redundant GPU work.
#[derive(Clone, Copy)]
struct EncodedFrame {
    size: (u32, u32),
    grid: FrameCamera,
    scene: FrameCamera,
    grid_visible: bool,
    lighting: FrameLighting,
    revision: u64,
}

/// A size-stable sRGB texture the Rune compositor samples.
struct PresentationTarget {
    view: wgpu::TextureView,
    size: (u32, u32),
}

impl LiveScene {
    /// Builds the pipelines and an empty scene on the device Rune renders with.
    ///
    /// The scene never installs device callbacks: the device belongs to Rune,
    /// which keeps its own error and device-lost handlers. Upload waits still
    /// poll the device and report their own failures.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let samples = preferred_samples(device);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines: Arc::new(Pipelines::new(
                device,
                queue,
                samples,
                TARGET_FORMAT,
                DEPTH_FORMAT,
            )),
            format: TARGET_FORMAT,
            samples,
            scene: None,
            device_status: DeviceStatus::passive(device.clone()),
            model_bounds: None,
            has_visible_batch: false,
            lighting: FrameLighting::default(),
            presentation: None,
            depth_view: None,
            color_view: None,
            revision: 0,
            last_encoded: None,
        }
    }

    /// Starts an empty scene on the same device, pipelines and error status.
    ///
    /// Forking is cheap and installs no new callbacks, so a tab switch can
    /// hand a fresh scene to the new load while a cancelled job still drains
    /// into the old one. The fork owns no geometry or presentation texture
    /// until its own [`Self::prepare`] and [`Self::encode`] run.
    pub fn fork(&self) -> Self {
        Self {
            device: self.device.clone(),
            queue: self.queue.clone(),
            pipelines: Arc::clone(&self.pipelines),
            format: self.format,
            samples: self.samples,
            scene: None,
            device_status: Arc::clone(&self.device_status),
            model_bounds: None,
            has_visible_batch: false,
            lighting: self.lighting,
            presentation: None,
            depth_view: None,
            color_view: None,
            revision: 0,
            last_encoded: None,
        }
    }

    /// Monotonic content revision, bumped by every call that can change the
    /// encoded image. The compositor may skip [`Self::encode`] while this and
    /// its own camera state are unchanged.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the scene lighting and invalidates the cached frame.
    pub fn set_lighting(&mut self, lighting: FrameLighting) {
        if self.lighting != lighting {
            self.lighting = lighting;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// The most recently encoded presentation view, if one exists. Its pixels
    /// match [`Self::revision`] only until the next mutating call.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.presentation.as_ref().map(|target| &target.view)
    }

    /// Largest texture extent this device accepts.
    pub fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// A staging-credit window sized for `workers` concurrent uploaders.
    pub fn upload_window(&self, workers: usize) -> Arc<UploadWindow> {
        Arc::new(UploadWindow::with_device(
            Arc::clone(&self.device_status),
            workers.max(1),
        ))
    }

    /// Starts a new load and records the source bounds used to frame it.
    ///
    /// Any previously displayed geometry is dropped. Call once before the
    /// first [`Self::attach`] of a stream; the host fits its own camera from
    /// these bounds (or from [`Self::model_bounds`]).
    pub fn prepare(&mut self, min: [f32; 3], max: [f32; 3]) {
        self.scene = Some(Scene::new(self.device.clone(), self.queue.clone()));
        self.model_bounds = Some((min, max));
        self.has_visible_batch = false;
        self.release_attachments();
        self.revision = self.revision.wrapping_add(1);
        self.last_encoded = None;
    }

    /// Appends one live batch through its upload permit and flushes staging.
    ///
    /// This never draws; the host encodes the next frame itself. A malformed
    /// batch clears the scene so a broken stream cannot leave partial geometry.
    pub fn attach(&mut self, batch: &MeshBatch, permit: UploadPermit) -> Result<(), RenderError> {
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| RenderError::malformed_batch("the preview load has not started"))?;
        if let Err(error) = scene.attach_batch(&self.pipelines, batch, permit) {
            self.clear();
            return Err(error);
        }
        if !batch.parts.is_empty() {
            self.has_visible_batch = true;
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    /// Finishes the stream: builds the ground grid and adopts exact geometry
    /// bounds. The host decides whether to refit or preserve its camera.
    pub fn finish(&mut self, metadata: &PreviewInfo) {
        let Some(scene) = self.scene.as_mut() else {
            return;
        };
        scene.finish(metadata);
        self.model_bounds = Some((metadata.min, metadata.max));
        self.revision = self.revision.wrapping_add(1);
    }

    /// Drops all displayed geometry and its depth/MSAA attachments. The
    /// size-stable presentation texture is kept for reuse; the next
    /// [`Self::encode`] clears it.
    pub fn clear(&mut self) {
        self.scene = None;
        self.model_bounds = None;
        self.has_visible_batch = false;
        self.release_attachments();
        self.revision = self.revision.wrapping_add(1);
        self.last_encoded = None;
    }

    /// Source or final model bounds, once [`Self::prepare`] or [`Self::finish`]
    /// has recorded them.
    pub fn model_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        self.model_bounds
    }

    /// Encodes the scene at `size` and returns the premultiplied sRGB view to
    /// blit. Empty and loading scenes produce a transparent view, so the UI
    /// backdrop shows through and readback can preserve coverage.
    ///
    /// The target has `TEXTURE_BINDING | COPY_SRC` usage, so the Rune
    /// compositor samples it directly and exports can still copy it. The
    /// texture is reused while the size is unchanged.
    ///
    /// If the size, both camera frames, grid visibility and [`Self::revision`]
    /// all match the last encoded frame, no commands are recorded and the
    /// previous view is returned unchanged.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
        camera: &Camera,
    ) -> Result<&wgpu::TextureView, RenderError> {
        self.device_status.check().map_err(RenderError::Device)?;
        let size = backing_size(size, self.max_texture_side());
        let grid = camera.frame_camera(true);
        let scene_frame = camera.frame_camera(false);
        let grid_visible = camera.grid_visible();
        let unchanged = self
            .presentation
            .as_ref()
            .is_some_and(|target| target.size == size)
            && self.last_encoded.is_some_and(|last| {
                last.size == size
                    && last.revision == self.revision
                    && last.grid == grid
                    && last.scene == scene_frame
                    && last.grid_visible == grid_visible
                    && last.lighting == self.lighting
            });
        if unchanged {
            return Ok(&self
                .presentation
                .as_ref()
                .expect("an unchanged frame has a presentation target")
                .view);
        }
        self.ensure_presentation(size);
        if self.has_visible_batch && self.depth_view.is_none() {
            self.rebuild_attachments(size);
        }
        self.pipelines.write_cameras(&self.queue, grid, scene_frame);
        self.pipelines.write_lighting(&self.queue, self.lighting);
        let target = &self
            .presentation
            .as_ref()
            .expect("the presentation target was just ensured")
            .view;
        self.encode_pass(
            encoder,
            target,
            self.depth_view.as_ref(),
            self.color_view.as_ref(),
            camera,
        )?;
        self.last_encoded = Some(EncodedFrame {
            size,
            grid,
            scene: scene_frame,
            grid_visible,
            lighting: self.lighting,
            revision: self.revision,
        });
        Ok(&self
            .presentation
            .as_ref()
            .expect("the presentation target was just ensured")
            .view)
    }

    /// Renders the scene into a one-shot capture target and submits the copy.
    ///
    /// The caller waits for [`CaptureJob::finish`] on a background task. The
    /// presentation texture, its MSAA/depth attachments and the host camera
    /// are untouched; only the shared camera uniforms are rewritten, and the
    /// next scene pass rewrites them again before it draws.
    pub fn capture(&self, size: (u32, u32), camera: &Camera) -> Result<CaptureJob, RenderError> {
        self.device_status.check().map_err(RenderError::Device)?;
        if !self.has_visible_batch || self.scene.is_none() {
            return Err(RenderError::Capture(String::from(
                "There is no preview image to capture yet.",
            )));
        }
        let (width, height) = backing_size(size, self.max_texture_side());
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview capture"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let (depth, color) = attachments(&self.device, (width, height), self.format, self.samples);
        let stride = padded_stride(width).ok_or_else(|| {
            RenderError::Capture(String::from("The preview image row is too wide."))
        })?;
        let bytes = u64::from(stride)
            .checked_mul(u64::from(height))
            .ok_or_else(|| RenderError::Capture(String::from("The preview image is too large.")))?;
        if bytes > self.device.limits().max_buffer_size {
            return Err(RenderError::Capture(String::from(
                "The preview area is too large to capture on this graphics device.",
            )));
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview capture readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.pipelines.write_cameras(
            &self.queue,
            camera.frame_camera(true),
            camera.frame_camera(false),
        );
        self.pipelines.write_lighting(&self.queue, self.lighting);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview capture"),
            });
        self.encode_pass(&mut encoder, &view, Some(&depth), color.as_ref(), camera)?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        let submission = self.queue.submit(Some(encoder.finish()));
        Ok(CaptureJob::new(
            self.device.clone(),
            buffer,
            submission,
            width,
            height,
            stride,
            self.format,
        ))
    }

    /// Encodes the clear and scene into `target`, resolving MSAA when enabled.
    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
        color: Option<&wgpu::TextureView>,
        camera: &Camera,
    ) -> Result<(), RenderError> {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("schematic render"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color.unwrap_or(target),
                depth_slice: None,
                resolve_target: color.map(|_| target),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: if color.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(scene) = self.scene.as_ref().filter(|_| self.has_visible_batch) {
            scene.draw(&self.pipelines, &mut pass, camera.grid_visible())?;
        }
        Ok(())
    }

    /// Reuses the presentation texture at `size`, replacing it (and dropping
    /// stale attachments) only when the effect size changes.
    fn ensure_presentation(&mut self, size: (u32, u32)) {
        if self
            .presentation
            .as_ref()
            .is_some_and(|target| target.size == size)
        {
            return;
        }
        self.release_attachments();
        self.last_encoded = None;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview presentation"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.presentation = Some(PresentationTarget {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
        });
    }

    fn rebuild_attachments(&mut self, size: (u32, u32)) {
        let (depth, color) = attachments(&self.device, size, self.format, self.samples);
        self.depth_view = Some(depth);
        self.color_view = color;
    }

    fn release_attachments(&mut self) {
        self.depth_view = None;
        self.color_view = None;
    }
}

/// Builds depth and MSAA attachments for one target extent.
fn attachments(
    device: &wgpu::Device,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    samples: u32,
) -> (wgpu::TextureView, Option<wgpu::TextureView>) {
    let (width, height) = (width.max(1), height.max(1));
    let depth = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth view"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    let color = (samples > 1).then(|| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("multisample color view"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    });
    (depth, color)
}

/// Uses 4x MSAA when the WebGPU guarantees it for the target format; 1x
/// otherwise.
fn preferred_samples(device: &wgpu::Device) -> u32 {
    let features = TARGET_FORMAT.guaranteed_format_features(device.features());
    if features.flags.sample_count_supported(4) {
        4
    } else {
        1
    }
}

/// Renders a scene into an opaque square sRGB texture with the default fitted
/// orientation and submits a copy for readback. The grid is omitted: browser
/// thumbnails show only the model. Camera uniforms are left holding this
/// canonical frame; callers sharing them with a live view must restore.
#[allow(clippy::too_many_arguments)]
fn capture_scene_thumbnail(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &Pipelines,
    format: wgpu::TextureFormat,
    samples: u32,
    side: u32,
    (min, max): ([f32; 3], [f32; 3]),
    scene: &Scene,
) -> Result<CaptureJob, RenderError> {
    let side = side.clamp(1, device.limits().max_texture_dimension_2d);
    let mut camera = Camera::new();
    camera.set_model_bounds(min, max);
    camera.resize(side as f32, side as f32);
    let frame = camera.frame_camera(false);
    let extent = wgpu::Extent3d {
        width: side,
        height: side,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("thumbnail color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let (depth, multisample) = attachments(device, (side, side), format, samples);
    let stride = padded_stride(side)
        .ok_or_else(|| RenderError::Capture(String::from("The thumbnail row is too wide.")))?;
    let bytes = u64::from(stride)
        .checked_mul(u64::from(side))
        .ok_or_else(|| RenderError::Capture(String::from("The thumbnail is too large.")))?;
    if bytes > device.limits().max_buffer_size {
        return Err(RenderError::Capture(String::from(
            "The thumbnail is too large to read back on this graphics device.",
        )));
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("thumbnail readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    pipelines.write_cameras(queue, frame, frame);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("thumbnail capture"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("thumbnail render"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: multisample.as_ref().unwrap_or(&view),
                depth_slice: None,
                resolve_target: multisample.as_ref().map(|_| &view),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: THUMBNAIL_CLEAR[0],
                        g: THUMBNAIL_CLEAR[1],
                        b: THUMBNAIL_CLEAR[2],
                        a: 1.0,
                    }),
                    store: if multisample.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, side as f32, side as f32, 0.0, 1.0);
        scene.draw(pipelines, &mut pass, false)?;
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(side),
            },
        },
        extent,
    );
    let submission = queue.submit(Some(encoder.finish()));
    Ok(CaptureJob::new(
        device.clone(),
        buffer,
        submission,
        side,
        side,
        stride,
        format,
    ))
}

/// Reusable square thumbnail renderer for the browse grid.
///
/// It owns the same pipeline family and streaming scene as a live renderer,
/// accepts decoded batches, and captures one canonical square image with no
/// presentation state or host camera. [`Self::from_device`] builds one on a
/// shared device; [`Self::new`] requests a headless device of its own.
pub struct ThumbnailRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    format: wgpu::TextureFormat,
    samples: u32,
    scene: Scene,
    uploads: Arc<UploadWindow>,
    status: Arc<DeviceStatus>,
    bounds: Option<([f32; 3], [f32; 3])>,
    has_geometry: bool,
}

impl ThumbnailRenderer {
    /// Requests a dedicated headless device (Metal on macOS, Vulkan on Linux,
    /// DX12 on Windows) and starts an empty scene. This crate owns that device,
    /// so it also installs the device error and device-lost handlers.
    pub fn new() -> Result<Self, RenderError> {
        let (device, queue, _adapter) = request_headless_device()?;
        Ok(Self::with_status(
            &device,
            &queue,
            DeviceStatus::new(device.clone()),
        ))
    }

    /// Builds pipelines and a scene on an existing device and queue without
    /// touching its device callbacks, for a device shared with Rune or a live
    /// scene.
    pub fn from_device(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::with_status(device, queue, DeviceStatus::passive(device.clone()))
    }

    fn with_status(device: &wgpu::Device, queue: &wgpu::Queue, status: Arc<DeviceStatus>) -> Self {
        let samples = preferred_samples(device);
        let pipelines = Pipelines::new(device, queue, samples, TARGET_FORMAT, DEPTH_FORMAT);
        let scene = Scene::new(device.clone(), queue.clone());
        let uploads = Arc::new(UploadWindow::with_device(Arc::clone(&status), 1));
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipelines,
            format: TARGET_FORMAT,
            samples,
            scene,
            uploads,
            status,
            bounds: None,
            has_geometry: false,
        }
    }

    /// Largest texture extent this device accepts; the stream uses it to size
    /// decoded batches before upload.
    pub fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Selects the bounds that fit the canonical camera, once source or final
    /// geometry bounds are known.
    pub fn prepare(&mut self, min: [f32; 3], max: [f32; 3]) {
        self.bounds = Some((min, max));
    }

    /// Clears the scene so the same device, pipelines and credit window can
    /// render another neighbor. Call only after the previous capture finished.
    pub fn reset(&mut self) {
        self.scene = Scene::new(self.device.clone(), self.queue.clone());
        self.bounds = None;
        self.has_geometry = false;
    }

    /// Uploads one streamed batch through this renderer's one-credit window,
    /// waiting through the same bounded credit and staging-byte admission as
    /// the live scene while checking `current` before every wait. No GPU
    /// allocation happens before admission.
    pub fn attach_checked(
        &mut self,
        batch: &MeshBatch,
        current: impl Fn() -> Result<(), String>,
    ) -> Result<(), RenderError> {
        let bytes = batch
            .textures
            .iter()
            .try_fold(batch.geometry.len(), |bytes, texture| {
                bytes.checked_add(texture.pixels.len())
            })
            .ok_or_else(|| {
                RenderError::malformed_batch(
                    "the thumbnail batch overflowed the upload size accounting",
                )
            })?;
        let permit = self
            .uploads
            .acquire(bytes, &current)
            .map_err(RenderError::Device)?;
        self.scene.attach_batch(&self.pipelines, batch, permit)?;
        if !batch.parts.is_empty() {
            self.has_geometry = true;
        }
        Ok(())
    }

    /// Waits for every submitted upload to complete, checking `current` while
    /// waiting. Call before capturing after a large stream.
    pub fn drain_checked(
        &mut self,
        current: impl Fn() -> Result<(), String>,
    ) -> Result<(), RenderError> {
        self.uploads.drain(&current).map_err(RenderError::Device)
    }

    /// Finishes the stream with final geometry bounds. The thumbnail omits the
    /// grid, so no grid geometry is built.
    pub fn finish(&mut self, metadata: &PreviewInfo) {
        self.bounds = Some((metadata.min, metadata.max));
    }

    /// Submits one square canonical capture after draining pending uploads,
    /// checking `current` while waiting for a large stream's uploads. Wait for
    /// [`CaptureJob::finish`] outside any caller lock.
    pub fn begin_capture_checked(
        &mut self,
        side: u32,
        current: impl Fn() -> Result<(), String>,
    ) -> Result<CaptureJob, RenderError> {
        self.status.check().map_err(RenderError::Device)?;
        self.drain_checked(current)?;
        let bounds = self.bounds.ok_or_else(|| {
            RenderError::Capture(String::from(
                "The thumbnail bounds are not ready to capture yet.",
            ))
        })?;
        if !self.has_geometry {
            return Err(RenderError::Capture(String::from(
                "The thumbnail has no visible geometry to capture.",
            )));
        }
        capture_scene_thumbnail(
            &self.device,
            &self.queue,
            &self.pipelines,
            self.format,
            self.samples,
            side,
            bounds,
            &self.scene,
        )
    }
}

/// Requests the platform adapter and device used by a headless thumbnail
/// renderer.
fn request_headless_device() -> Result<(wgpu::Device, wgpu::Queue, wgpu::Adapter), RenderError> {
    let (backends, backend_label) = headless_backend();
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|_| {
        RenderError::Initialization(format!(
            "{} is unavailable. Update your graphics driver to preview schematics.",
            backend_label
        ))
    })?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("schematic offscreen renderer"),
        required_features: wgpu::Features::empty(),
        required_limits: headless_limits(&adapter),
        memory_hints: wgpu::MemoryHints::Performance,
        ..Default::default()
    }))
    .map_err(|error| {
        RenderError::Initialization(format!("The graphics device could not initialize: {error}"))
    })?;
    Ok((device, queue, adapter))
}

fn headless_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let adapter_limits = adapter.limits();
    let max_buffer_size = adapter_limits.max_buffer_size;
    let mut limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter_limits);
    limits.max_buffer_size = max_buffer_size;
    limits
}

fn headless_backend() -> (wgpu::Backends, &'static str) {
    cfg_select! {
        windows => { (wgpu::Backends::DX12, "Direct3D 12") }
        target_os = "linux" => { (wgpu::Backends::VULKAN, "Vulkan") }
        target_os = "macos" => { (wgpu::Backends::METAL, "Metal") }
        _ => { (wgpu::Backends::PRIMARY, "The graphics backend") }
    }
}

fn backing_size((width, height): (u32, u32), max_dimension: u32) -> (u32, u32) {
    (
        width.clamp(1, max_dimension),
        height.clamp(1, max_dimension),
    )
}

/// Rounds a row of RGBA pixels up to the copy alignment.
fn padded_stride(width: u32) -> Option<u32> {
    let bytes = u64::from(width) * 4;
    let aligned = bytes.checked_add(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) - 1)?
        & !u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT - 1);
    u32::try_from(aligned).ok()
}

/// Matches the camera buffer size to `FrameCamera::encode`.
const _: () = {
    assert!(CAMERA_UNIFORM_BYTES == 80, "camera uniform layout drifted");
};

#[cfg(test)]
mod tests {
    use super::{backing_size, padded_stride};

    #[test]
    fn backing_size_preserves_supported_extents_and_bounds_oversized_targets() {
        assert_eq!(backing_size((1200, 800), 16384), (1200, 800));
        assert_eq!(backing_size((12000, 9000), 16384), (12000, 9000));
        assert_eq!(backing_size((24000, 9000), 16384), (16384, 9000));
        assert_eq!(backing_size((9000, 24000), 16384), (9000, 16384));
        assert_eq!(backing_size((24000, 24000), 16384), (16384, 16384));
        assert_eq!(backing_size((0, 0), 16384), (1, 1));
    }

    #[test]
    fn stride_is_aligned_and_rejects_widths_that_exceed_u32() {
        assert_eq!(padded_stride(1), Some(256));
        assert_eq!(padded_stride(64), Some(256));
        assert_eq!(padded_stride(65), Some(512));
        assert_eq!(padded_stride(16384), Some(65536));
        assert_eq!(padded_stride(u32::MAX), None);
    }
}
