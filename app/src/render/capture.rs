//! Map the current preview stage from the GPU into a CPU image.
//!
//! Encoding and copying a frame is a one-shot command, not a presentation
//! path: the copy is submitted under the renderer lock and waited on by the
//! command's blocking worker, never by the UI or render threads.

use crate::preview_image::PreviewImage;
use crate::render::context::RenderError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WAIT_SLICE: Duration = Duration::from_millis(50);
const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// A submitted texture-to-buffer copy whose pixels have not been mapped yet.
pub struct CaptureJob {
    device: wgpu::Device,
    buffer: wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
    width: u32,
    height: u32,
    stride: u32,
    format: wgpu::TextureFormat,
}

impl CaptureJob {
    pub(crate) fn new(
        device: wgpu::Device,
        buffer: wgpu::Buffer,
        submission: wgpu::SubmissionIndex,
        width: u32,
        height: u32,
        stride: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            device,
            buffer,
            submission,
            width,
            height,
            stride,
            format,
        }
    }

    /// Waits for and reads back the copied rows, returning tightly packed RGBA.
    /// The wait is bounded so a stalled device cannot hang the command.
    pub fn finish(self) -> Result<PreviewImage, RenderError> {
        let map_result: Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>> =
            Arc::new(Mutex::new(None));
        {
            let slot = Arc::clone(&map_result);
            self.buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
                });
        }
        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            let mapped = map_result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(result) = mapped {
                result.map_err(|error| {
                    RenderError::Device(format!("The preview image could not be mapped: {error}"))
                })?;
                break;
            }
            if Instant::now() >= deadline {
                self.buffer.unmap();
                return Err(RenderError::Capture(String::from(
                    "The preview image could not be read back in time.",
                )));
            }
            match self.device.poll(wgpu::PollType::Wait {
                submission_index: Some(self.submission.clone()),
                timeout: Some(WAIT_SLICE),
            }) {
                Ok(_) | Err(wgpu::PollError::Timeout) => {}
                Err(error) => {
                    self.buffer.unmap();
                    return Err(RenderError::Device(format!(
                        "The preview image could not be read back: {error}"
                    )));
                }
            }
        }

        let row_bytes = usize::try_from(self.width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or_else(|| {
                RenderError::Capture(String::from("The preview image dimensions overflowed."))
            })?;
        let stride = usize::try_from(self.stride).map_err(|_| {
            RenderError::Capture(String::from("The preview image row is too wide."))
        })?;
        let height = usize::try_from(self.height)
            .map_err(|_| RenderError::Capture(String::from("The preview image is too tall.")))?;
        let allocation = row_bytes
            .checked_mul(height)
            .ok_or_else(|| RenderError::Capture(String::from("The preview image is too large.")))?;
        let expected = stride
            .checked_mul(height)
            .ok_or_else(|| RenderError::Capture(String::from("The preview image is too large.")))?;
        let mut rgba = Vec::with_capacity(allocation);
        let mut unsupported = false;
        {
            let mapped = self.buffer.slice(..).get_mapped_range().map_err(|error| {
                RenderError::Device(format!(
                    "The preview image's mapped bytes are unavailable: {error}"
                ))
            })?;
            if mapped.len() < expected {
                drop(mapped);
                self.buffer.unmap();
                return Err(RenderError::Capture(String::from(
                    "The preview image's mapped bytes were shorter than expected.",
                )));
            }
            for row in mapped.chunks(stride).take(height) {
                let row = &row[..row_bytes];
                match self.format {
                    wgpu::TextureFormat::Rgba8UnormSrgb => rgba.extend_from_slice(row),
                    wgpu::TextureFormat::Bgra8UnormSrgb => {
                        for pixel in row.as_chunks::<4>().0 {
                            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                        }
                    }
                    _ => {
                        unsupported = true;
                        break;
                    }
                }
            }
        }
        self.buffer.unmap();
        if unsupported {
            return Err(RenderError::Capture(String::from(
                "The preview image uses an unsupported target format.",
            )));
        }
        unpremultiply(&mut rgba);
        PreviewImage::new(self.width, self.height, rgba).map_err(RenderError::Capture)
    }
}

/// Converts premultiplied stage pixels to the straight alpha PNG expects.
/// Opaque thumbnails pass through unchanged.
fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                let value = u16::from(*channel) * 255 + alpha / 2;
                *channel = (value / alpha).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::unpremultiply;

    #[test]
    fn readback_recovers_straight_alpha_and_keeps_opaque_pixels() {
        let mut pixels = vec![
            64, 0, 0, 128, // half-covered red
            0, 0, 0, 0, // uncovered
            200, 40, 40, 255, // opaque
        ];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [128, 0, 0, 128, 0, 0, 0, 0, 200, 40, 40, 255]);
    }
}
