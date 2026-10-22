//! Offscreen BGRA target and single-threaded readback worker.
//!
//! The renderer draws into [`OffscreenTarget`]'s texture and posts a
//! [`ReadbackRequest`] to a one-slot mailbox. A single worker copies the
//! completed texture into a mapped buffer, copies those bytes once into a
//! pooled CPU allocation, and publishes the frame through the sink. Waiting for
//! the GPU or a mapping always happens on that worker, never under the
//! renderer lock. A newer request replaces an older pending one and marks any
//! in-flight copy obsolete, so stale work is discarded before its CPU pixel
//! copy or publication.

use crate::render::context::RenderError;
use crate::render::frame::{padded_stride, Frame, FramePixels, FrameSink, PixelPool};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How long the worker waits for the GPU before checking for shutdown.
const POLL_SLICE: Duration = Duration::from_millis(50);

/// The newest generation requested by the renderer. Any older in-flight
/// texture copy is obsolete even if it finishes first.
struct LatestRequest {
    generation: AtomicU64,
}

impl LatestRequest {
    fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
        }
    }

    fn post(&self, generation: u64) {
        self.generation.fetch_max(generation, Ordering::AcqRel);
    }

    fn is_current(&self, generation: u64) -> bool {
        generation >= self.generation.load(Ordering::Acquire)
    }
}

/// Texture, view, and bounded extent for the offscreen frame.
pub(crate) struct OffscreenTarget {
    texture: Arc<wgpu::Texture>,
    view: wgpu::TextureView,
    size: (u32, u32),
    stride: u32,
    mailbox: Arc<Mailbox<ReadbackRequest>>,
    latest: Arc<LatestRequest>,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
}

impl OffscreenTarget {
    /// Builds the sRGB offscreen texture and starts its sole readback worker.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        physical_size: (u32, u32),
        sink: FrameSink,
    ) -> Result<Self, RenderError> {
        let size = bounded_size(physical_size, &device.limits());
        let (texture, view, stride) = create_target(device, size);
        let mailbox = Arc::new(Mailbox::new());
        let latest = Arc::new(LatestRequest::new());
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let worker = spawn_reader(
            device.clone(),
            queue.clone(),
            Arc::clone(&mailbox),
            Arc::clone(&latest),
            Arc::clone(&stop),
            Arc::clone(&failure),
            sink,
        )?;
        Ok(Self {
            texture,
            view,
            size,
            stride,
            mailbox,
            latest,
            stop,
            failure,
            worker: Some(worker),
        })
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Rebuilds the texture when the bounded extent changes and returns the
    /// extent in use.
    pub(crate) fn resize(
        &mut self,
        device: &wgpu::Device,
        physical_size: (u32, u32),
    ) -> (u32, u32) {
        let size = bounded_size(physical_size, &device.limits());
        if size == self.size {
            return size;
        }
        let (texture, view, stride) = create_target(device, size);
        self.texture = texture;
        self.view = view;
        self.size = size;
        self.stride = stride;
        size
    }

    /// Replaces any pending request; no queue of stale frames can accumulate.
    /// The shared newest generation invalidates in-flight work immediately.
    pub(crate) fn request(&self, generation: u64, physical_size: (u32, u32)) {
        self.latest.post(generation);
        self.mailbox.post(ReadbackRequest {
            generation,
            physical_size,
            texture: Arc::clone(&self.texture),
            width: self.size.0,
            height: self.size.1,
            stride: self.stride,
        });
    }

    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for OffscreenTarget {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.mailbox.clear();
        self.mailbox.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The largest frame the device can both create and copy into a buffer.
fn bounded_size((width, height): (u32, u32), limits: &wgpu::Limits) -> (u32, u32) {
    let maximum = limits.max_texture_dimension_2d.max(1);
    // Keep a padded row addressable even on a device reporting impossible
    // texture limits.
    let maximum_width = maximum.min((u32::MAX - 255) / 4);
    let width = width.clamp(1, maximum_width);
    let height = height.clamp(1, maximum);
    let stride = padded_stride(width).expect("the bounded width always has a stride");
    let rows = (limits.max_buffer_size / u64::from(stride)).clamp(1, u64::from(maximum)) as u32;
    (width, height.min(rows))
}

fn create_target(
    device: &wgpu::Device,
    size: (u32, u32),
) -> (Arc<wgpu::Texture>, wgpu::TextureView, u32) {
    let stride = padded_stride(size.0).expect("a texture-sized width always has an aligned stride");
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen frame"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (Arc::new(texture), view, stride)
}

struct ReadbackRequest {
    generation: u64,
    physical_size: (u32, u32),
    texture: Arc<wgpu::Texture>,
    width: u32,
    height: u32,
    stride: u32,
}

/// A one-slot mailbox: posting replaces any request not yet taken.
struct Mailbox<T> {
    value: Mutex<Option<T>>,
    ready: Condvar,
}

impl<T> Mailbox<T> {
    fn new() -> Self {
        Self {
            value: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn post(&self, value: T) {
        *self.lock() = Some(value);
        self.ready.notify_one();
    }

    /// Waits for the latest request, returning `None` once shutdown is set.
    /// The caller only observes poll slices while GPU work is active; an idle
    /// worker sleeps until a post or shutdown wake.
    fn take(&self, stop: &AtomicBool) -> Option<T> {
        let mut value = self.lock();
        loop {
            if stop.load(Ordering::Acquire) {
                *value = None;
                return None;
            }
            if let Some(request) = value.take() {
                return Some(request);
            }
            value = self
                .ready
                .wait(value)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn clear(&self) {
        *self.lock() = None;
    }

    fn wake(&self) {
        self.ready.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<T>> {
        self.value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn spawn_reader(
    device: wgpu::Device,
    queue: wgpu::Queue,
    mailbox: Arc<Mailbox<ReadbackRequest>>,
    latest: Arc<LatestRequest>,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    sink: FrameSink,
) -> Result<JoinHandle<()>, RenderError> {
    std::thread::Builder::new()
        .name(String::from("offscreen-frame-reader"))
        .spawn(move || {
            let panic_failure = Arc::clone(&failure);
            let panic_sink = Arc::clone(&sink);
            let panic_stop = Arc::clone(&stop);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                ReadbackWorker {
                    device,
                    queue,
                    mailbox,
                    latest,
                    stop,
                    failure,
                    sink,
                    pool: PixelPool::new(),
                    map_result: Arc::new(Mutex::new(None)),
                    readback: None,
                }
                .run();
            }));
            if let Err(payload) = result {
                if panic_stop.load(Ordering::Acquire) {
                    return;
                }
                let detail = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                record_failure(
                    &panic_failure,
                    &panic_sink,
                    format!("The offscreen frame reader stopped unexpectedly: {detail}"),
                );
            }
        })
        .map_err(|error| {
            RenderError::Initialization(format!(
                "The offscreen frame reader could not start: {error}"
            ))
        })
}

struct ReadbackBuffer {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    stride: u32,
    bytes: u64,
}

struct ReadbackWorker {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mailbox: Arc<Mailbox<ReadbackRequest>>,
    latest: Arc<LatestRequest>,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    sink: FrameSink,
    pool: PixelPool,
    /// Reused mapping result; only one mapping is outstanding per worker.
    map_result: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>,
    readback: Option<ReadbackBuffer>,
}

impl ReadbackWorker {
    fn run(mut self) {
        if let Err(message) = self.read_loop() {
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            record_failure(&self.failure, &self.sink, message);
        }
    }

    fn read_loop(&mut self) -> Result<(), String> {
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some(request) = self.mailbox.take(&self.stop) else {
                return Ok(());
            };
            if !self.latest.is_current(request.generation) {
                continue;
            }
            if self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            self.capture(request)?;
        }
    }

    /// Copies the texture into a reused staging buffer, maps it on this worker,
    /// and copies the mapped bytes once into a pooled frame allocation. Work
    /// that a newer request has already obsoleted is discarded before the copy,
    /// before the CPU memcpy, and before publication.
    fn capture(&mut self, request: ReadbackRequest) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) || !self.latest.is_current(request.generation) {
            return Ok(());
        }
        let width = request.width;
        let height = request.height;
        let stride = request.stride;
        let bytes = u64::from(stride)
            .checked_mul(u64::from(height))
            .ok_or_else(|| String::from("The offscreen frame dimensions overflowed."))?;
        if bytes > self.device.limits().max_buffer_size {
            return Err(String::from(
                "The offscreen frame exceeds the graphics device's buffer limit.",
            ));
        }
        let reusable = self.readback.as_ref().is_some_and(|readback| {
            readback.width == width
                && readback.height == height
                && readback.stride == stride
                && readback.bytes == bytes
        });
        if !reusable {
            self.readback = None;
            self.readback = Some(ReadbackBuffer {
                buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("offscreen frame readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                width,
                height,
                stride,
                bytes,
            });
        }
        if self.stop.load(Ordering::Acquire) || !self.latest.is_current(request.generation) {
            return Ok(());
        }
        let readback = self
            .readback
            .as_ref()
            .expect("the readback buffer was just prepared");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("offscreen frame copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: request.texture.as_ref(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit(Some(encoder.finish()));

        *self
            .map_result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        {
            let map_result = Arc::clone(&self.map_result);
            readback
                .buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    *map_result
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
                });
        }
        loop {
            if self.stop.load(Ordering::Acquire) {
                readback.buffer.unmap();
                return Ok(());
            }
            match self.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: Some(POLL_SLICE),
            }) {
                Ok(_) => {}
                Err(wgpu::PollError::Timeout) => continue,
                Err(error) => {
                    readback.buffer.unmap();
                    return Err(format!(
                        "The rendered frame could not be read back: {error}"
                    ));
                }
            }
            let result = self
                .map_result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(result) = result {
                if let Err(error) = result {
                    readback.buffer.unmap();
                    return Err(format!("The rendered frame could not be mapped: {error}"));
                }
                break;
            }
        }
        if self.stop.load(Ordering::Acquire) || !self.latest.is_current(request.generation) {
            readback.buffer.unmap();
            return Ok(());
        }

        let length = match usize::try_from(bytes) {
            Ok(length) => length,
            Err(_) => {
                readback.buffer.unmap();
                return Err(String::from(
                    "The offscreen frame is too large for this host.",
                ));
            }
        };
        let mut pixels = self.pool.take(length);
        {
            let mapped = match readback.buffer.slice(..).get_mapped_range() {
                Ok(mapped) => mapped,
                Err(error) => {
                    readback.buffer.unmap();
                    return Err(format!(
                        "The rendered frame's mapped bytes are unavailable: {error}"
                    ));
                }
            };
            if mapped.len() < length {
                drop(mapped);
                readback.buffer.unmap();
                return Err(String::from(
                    "The rendered frame's mapped bytes were shorter than expected.",
                ));
            }
            pixels.extend_from_slice(&mapped[..length]);
        }
        readback.buffer.unmap();

        let frame = Frame {
            width,
            height,
            stride,
            physical_size: request.physical_size,
            pixels: FramePixels::new(self.pool.clone(), pixels),
        };
        if self.stop.load(Ordering::Acquire) || !self.latest.is_current(request.generation) {
            return Ok(());
        }
        (self.sink)(Ok(frame));
        Ok(())
    }
}

fn record_failure(failure: &Mutex<Option<String>>, sink: &FrameSink, message: String) {
    let mut slot = failure
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.is_some() {
        return;
    }
    *slot = Some(message.clone());
    drop(slot);
    (sink)(Err(message));
}

#[cfg(test)]
mod tests {
    use super::{LatestRequest, Mailbox};
    use crate::render::frame::padded_stride;

    #[test]
    fn mailbox_keeps_only_the_latest_unclaimed_request() {
        let mailbox = Mailbox::new();
        for generation in 1..=3 {
            mailbox.post(generation);
        }
        let stop = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(mailbox.take(&stop), Some(3));
        stop.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(mailbox.take(&stop), None);
    }

    #[test]
    fn a_newer_request_obsoletes_an_in_flight_frame() {
        let latest = LatestRequest::new();
        latest.post(1);
        assert!(latest.is_current(1));

        // A scene readback in flight at generation 1 becomes obsolete as soon
        // as Home or a resize posts generation 3.
        latest.post(3);
        assert!(!latest.is_current(1));
        assert!(!latest.is_current(2));

        // A delayed post cannot lower the newest requested generation.
        latest.post(2);
        assert!(!latest.is_current(2));
        assert!(latest.is_current(3));

        latest.post(5);
        assert!(!latest.is_current(3));
        assert!(latest.is_current(5));
    }

    #[test]
    fn readback_requests_are_bounded_by_the_device_limits() {
        let limits = wgpu::Limits {
            max_texture_dimension_2d: 16384,
            max_buffer_size: 65536,
            ..wgpu::Limits::downlevel_webgl2_defaults()
        };
        let size = super::bounded_size((20000, 20000), &limits);
        assert_eq!(size.0, 16384);
        assert_eq!(size.1, 1);
        assert_eq!(
            u64::from(size.1) * u64::from(padded_stride(size.0).unwrap()),
            65536
        );
    }
}
