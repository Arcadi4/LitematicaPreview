//! CPU-readable frames for offscreen presentation.
//!
//! Rows are padded to the graphics API's 256-byte texel-copy alignment and the
//! bytes are BGRA (little-endian) with an unused alpha byte, which matches
//! Cairo's native-endian `RGB24` image layout. The renderer owns the pixels;
//! [`FramePixels`] returns its allocation to a shared pool when the consumer
//! drops the frame.

use std::mem;
use std::sync::{Arc, Mutex};

/// Alignment required by `wgpu` when copying texture rows into a buffer.
pub(crate) const COPY_ROW_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// The most pooled pixel allocations retained for future frames.
const MAX_POOLED_BUFFERS: usize = 2;

/// One complete offscreen frame.
pub struct Frame {
    /// Bounded backing texture width in pixels.
    pub width: u32,
    /// Bounded backing texture height in pixels.
    pub height: u32,
    /// Bytes per row, including alignment padding.
    pub stride: u32,
    /// Requested drawable size before device-limit clamping.
    /// Consumers scale the backing image to this extent.
    pub physical_size: (u32, u32),
    /// BGRA rows in Cairo's native-endian `RGB24` layout.
    pub pixels: FramePixels,
}

/// An owned pixel allocation that returns to the renderer's shared pool when
/// the display surface releases it.
pub struct FramePixels {
    bytes: Vec<u8>,
    pool: PixelPool,
}

impl FramePixels {
    pub(crate) fn new(pool: PixelPool, bytes: Vec<u8>) -> Self {
        Self { bytes, pool }
    }
}

impl AsRef<[u8]> for FramePixels {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsMut<[u8]> for FramePixels {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl Drop for FramePixels {
    fn drop(&mut self) {
        self.pool.recycle(mem::take(&mut self.bytes));
    }
}

/// A bounded, shared cache of pixel allocations. Frames can be dropped from the
/// UI thread, so recycling must be thread safe.
#[derive(Clone)]
pub(crate) struct PixelPool {
    inner: Arc<Mutex<PooledPixels>>,
}

struct PooledPixels {
    buffers: Vec<Vec<u8>>,
    frame_bytes: usize,
}

impl PixelPool {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PooledPixels {
                buffers: Vec::new(),
                frame_bytes: 0,
            })),
        }
    }

    /// Returns a cleared allocation for `length` bytes and retires other sizes.
    pub(crate) fn take(&self, length: usize) -> Vec<u8> {
        let recycled = {
            let mut pooled = self.lock();
            if pooled.frame_bytes != length {
                pooled.buffers.clear();
                pooled.frame_bytes = length;
            }
            pooled.buffers.pop()
        };
        recycled.unwrap_or_else(|| Vec::with_capacity(length))
    }

    fn recycle(&self, mut buffer: Vec<u8>) {
        let mut pooled = self.lock();
        if buffer.is_empty()
            || buffer.len() != pooled.frame_bytes
            || pooled.buffers.len() >= MAX_POOLED_BUFFERS
        {
            return;
        }
        buffer.clear();
        pooled.buffers.push(buffer);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PooledPixels> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Rounds a row of RGBA pixels up to the copy alignment.
pub(crate) fn padded_stride(width: u32) -> Option<u32> {
    let bytes = u64::from(width) * 4;
    let aligned =
        bytes.checked_add(u64::from(COPY_ROW_ALIGNMENT) - 1)? & !u64::from(COPY_ROW_ALIGNMENT - 1);
    u32::try_from(aligned).ok()
}

/// A sink for completed frames or asynchronous readback failures. It is called
/// from the renderer's readback worker, never from the main thread.
pub type FrameSink = Arc<dyn Fn(Result<Frame, String>) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::{padded_stride, FramePixels, PixelPool};

    #[test]
    fn stride_is_aligned_and_rejects_widths_that_exceed_u32() {
        assert_eq!(padded_stride(1), Some(256));
        assert_eq!(padded_stride(64), Some(256));
        assert_eq!(padded_stride(65), Some(512));
        assert_eq!(padded_stride(16384), Some(65536));
        assert_eq!(padded_stride(u32::MAX), None);
        assert_eq!(padded_stride(u32::MAX / 4), None);
    }

    #[test]
    fn dropping_pixels_recycles_the_allocation_through_the_shared_pool() {
        let pool = PixelPool::new();
        let mut bytes = pool.take(512);
        bytes.resize(512, 7);
        let mut pixels = FramePixels::new(pool.clone(), bytes);
        let allocation = pixels.as_mut().as_ptr();
        drop(pixels);
        let recycled = pool.take(512);
        assert!(recycled.capacity() >= 512);
        assert_eq!(recycled.as_ptr(), allocation);
        assert!(recycled.is_empty());
    }

    #[test]
    fn dropping_retired_frames_does_not_retain_their_large_allocations() {
        let pool = PixelPool::new();
        let mut previous = pool.take(4096);
        previous.resize(4096, 1);
        let previous = FramePixels::new(pool.clone(), previous);
        let _current = pool.take(1024);
        drop(previous);
        let next = pool.take(1024);
        assert!(next.capacity() < 4096);
    }
}
