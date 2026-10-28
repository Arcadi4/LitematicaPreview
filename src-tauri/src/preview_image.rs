//! CPU preview snapshots, encoded as PNG or baseline JPEG.
//!
//! Pixels are tightly packed RGBA8 in sRGB. Snapshots are produced by
//! [`crate::render::capture`]; this module owns file encoding and the
//! clipboard-format bytes.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

/// JPEG has no alpha channel; the preview stage is opaque, so quality alone
/// controls loss.
const JPEG_QUALITY: u8 = 90;

/// One opaque preview stage image with four bytes per pixel.
pub struct PreviewImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl PreviewImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        let expected = usize::try_from(width)
            .ok()
            .zip(usize::try_from(height).ok())
            .and_then(|(width, height)| width.checked_mul(height))
            .and_then(|pixels| pixels.checked_mul(4));
        if width == 0 || height == 0 || expected != Some(rgba.len()) {
            return Err(String::from(
                "The captured preview image has inconsistent dimensions.",
            ));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }

    /// PNG bytes for clipboards that accept an `image/png`-style format.
    pub fn png_bytes(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                &self.rgba,
                self.width,
                self.height,
                ExtendedColorType::Rgba8,
            )
            .map_err(|error| format!("The preview image could not be encoded as PNG: {error}"))?;
        Ok(bytes)
    }
}

/// Saves `image` next to a name chosen in a save dialog. A missing extension
/// selects PNG; other extensions must name a supported format. The file is
/// written beside the target and renamed, so a failed encode cannot destroy an
/// existing image.
pub fn save(image: &PreviewImage, path: PathBuf) -> Result<(), String> {
    let (path, format) = image_format(path)?;
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = path.as_os_str().to_owned();
    temp_name.push(format!(".{}-{sequence}.tmp", std::process::id()));
    let temp = PathBuf::from(temp_name);
    let result = write_file(image, &temp, format).and_then(|()| {
        std::fs::rename(&temp, &path)
            .map_err(|error| format!("Unable to replace {}: {error}", path.display()))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn write_file(
    image: &PreviewImage,
    path: &std::path::Path,
    format: ImageFormat,
) -> Result<(), String> {
    let file = File::create(path)
        .map_err(|error| format!("Unable to create {}: {error}", path.display()))?;
    let mut writer = BufWriter::new(file);
    match format {
        ImageFormat::Png => PngEncoder::new(&mut writer)
            .write_image(
                &image.rgba,
                image.width,
                image.height,
                ExtendedColorType::Rgba8,
            )
            .map_err(|error| format!("Unable to encode the preview image as PNG: {error}"))?,
        ImageFormat::Jpeg => {
            let rgb: Vec<u8> = image
                .rgba
                .chunks_exact(4)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                .collect();
            JpegEncoder::new_with_quality(&mut writer, JPEG_QUALITY)
                .write_image(&rgb, image.width, image.height, ExtendedColorType::Rgb8)
                .map_err(|error| format!("Unable to encode the preview image as JPEG: {error}"))?;
        }
    }
    writer
        .flush()
        .map_err(|error| format!("Unable to finish writing {}: {error}", path.display()))
}

#[derive(Clone, Copy)]
enum ImageFormat {
    Png,
    Jpeg,
}

fn image_format(mut path: PathBuf) -> Result<(PathBuf, ImageFormat), String> {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .filter(|extension| !extension.is_empty());
    let format = match extension {
        None => {
            path.set_extension("png");
            ImageFormat::Png
        }
        Some(extension) if extension.eq_ignore_ascii_case("png") => ImageFormat::Png,
        Some(extension)
            if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") =>
        {
            ImageFormat::Jpeg
        }
        Some(extension) => {
            return Err(format!(
                "Choose a .png, .jpg, or .jpeg file name, not .{extension}."
            ))
        }
    };
    Ok((path, format))
}

#[cfg(test)]
mod tests {
    use super::{save, PreviewImage};
    use std::path::PathBuf;

    fn test_image() -> PreviewImage {
        PreviewImage::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )
        .unwrap()
    }

    fn scratch_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("litematica-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn saves_png_and_jpeg_and_defaults_a_missing_extension_to_png() {
        let directory = scratch_dir();
        let image = test_image();

        let png = directory.join("shot.png");
        save(&image, png.clone()).unwrap();
        let decoded = image::open(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.as_raw(), image.rgba());

        let jpeg = directory.join("shot.jpeg");
        let solid = PreviewImage::new(8, 8, [200, 40, 40, 255].repeat(64)).unwrap();
        save(&solid, jpeg.clone()).unwrap();
        let decoded = image::open(&jpeg).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (8, 8));
        for pixel in decoded.pixels() {
            for (actual, expected) in pixel.0.into_iter().zip([200u8, 40, 40]) {
                assert!(actual.abs_diff(expected) <= 10);
            }
        }

        save(&image, directory.join("untitled")).unwrap();
        assert_eq!(
            image::open(directory.join("untitled.png"))
                .unwrap()
                .to_rgba8()
                .as_raw(),
            image.rgba()
        );

        let unsupported = directory.join("existing.bmp");
        std::fs::write(&unsupported, b"original").unwrap();
        assert!(save(&image, unsupported.clone()).is_err());
        assert_eq!(std::fs::read(unsupported).unwrap(), b"original");

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
