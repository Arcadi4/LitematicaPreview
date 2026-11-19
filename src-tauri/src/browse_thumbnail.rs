//! Persistent square thumbnails for the schematic browser.
//!
//! Thumbnails are keyed by the SHA-256 of the source file bytes, so the same
//! content reached through different paths shares one cached image. The cache
//! lives under the app cache directory and holds only validated 128 px PNGs;
//! lookup never reads an image path supplied by the caller.
//!
//! Rendering is separate: [`crate::render::context::ThumbnailRenderer`] streams
//! decoded batches into a scratch scene, while a live vanilla scene can be
//! captured after `finish_load` without a second decode or upload.

use crate::render::capture::CaptureJob;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager};

/// Square edge of every browser thumbnail.
pub const THUMBNAIL_SIDE: u32 = 128;

const CACHE_DIRECTORY: &str = "browse-thumbnails";
const HASH_CHUNK_BYTES: usize = 64 * 1024;
/// A 128 × 128 PNG is far smaller; this bound keeps a corrupt or replaced
/// cache file from forcing a large decode.
const MAX_THUMBNAIL_BYTES: usize = 1024 * 1024;
const DATA_URL_PREFIX: &str = "data:image/png;base64,";
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// SHA-256 of `bytes` as lowercase hexadecimal.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    finish_hash(hasher)
}

fn finish_hash(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for &byte in digest.iter() {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}

/// Streams the file through SHA-256 without holding it in memory. `current`
/// runs before every chunk so a superseded browse job can stop promptly.
fn hash_file_checked(
    path: &Path,
    current: &impl Fn() -> Result<(), String>,
) -> Result<String, String> {
    let file =
        File::open(path).map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
    let mut reader = BufReader::with_capacity(HASH_CHUNK_BYTES, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        current()?;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(finish_hash(hasher))
}

/// Cheap identity used to detect a source changing underneath a job.
#[derive(Clone, Copy, PartialEq, Eq)]
struct FileVersion {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

fn file_version(path: &Path) -> io::Result<Option<FileVersion>> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(FileVersion {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// A source file identified by its exact content.
///
/// `open` hashes the streamed bytes and returns `None` when the file changes
/// while it is being read, so an unstable source never yields a key. Callers
/// that decode from the same path later re-validate with
/// [`ThumbnailSource::is_current`] before persisting.
#[derive(Clone, Debug)]
pub struct ThumbnailSource {
    path: PathBuf,
    key: String,
}

impl ThumbnailSource {
    /// Hashes `path`, retrying once if it changes during the read.
    pub fn open(path: impl Into<PathBuf>) -> Result<Option<Self>, String> {
        Self::open_checked(path, || Ok(()))
    }

    /// Like [`Self::open`], but checks `current` before every streamed chunk so
    /// a superseded background job yields promptly on a large file.
    pub fn open_checked(
        path: impl Into<PathBuf>,
        current: impl Fn() -> Result<(), String>,
    ) -> Result<Option<Self>, String> {
        let path = path.into();
        for _ in 0..2 {
            current()?;
            let Some(before) = file_version(&path)
                .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?
            else {
                return Ok(None);
            };
            let key = hash_file_checked(&path, &current)?;
            let after = file_version(&path)
                .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
            if after == Some(before) {
                return Ok(Some(Self { path, key }));
            }
        }
        Ok(None)
    }

    /// Lowercase 64-character SHA-256 of the file bytes.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Re-hashes the source and confirms its version and content still match.
    pub fn is_current(&self) -> Result<bool, String> {
        self.is_current_checked(|| Ok(()))
    }

    /// Like [`Self::is_current`], but checks `current` before every streamed
    /// chunk so validation of a large source can be superseded.
    pub fn is_current_checked(
        &self,
        current: impl Fn() -> Result<(), String>,
    ) -> Result<bool, String> {
        current()?;
        let Some(before) = file_version(&self.path)
            .map_err(|error| format!("Unable to inspect {}: {error}", self.path.display()))?
        else {
            return Ok(false);
        };
        let key = hash_file_checked(&self.path, &current)?;
        let after = file_version(&self.path)
            .map_err(|error| format!("Unable to inspect {}: {error}", self.path.display()))?;
        Ok(after == Some(before) && key == self.key)
    }
}

/// Content-addressed PNG cache under the app cache directory.
#[derive(Clone, Debug)]
pub struct ThumbnailCache {
    root: PathBuf,
}

impl ThumbnailCache {
    /// Opens (and creates) the browser thumbnail cache in the app cache dir.
    pub fn from_app(app: &AppHandle) -> Result<Self, String> {
        let root = app
            .path()
            .app_cache_dir()
            .map_err(|error| format!("The thumbnail cache directory is unavailable: {error}"))?
            .join(CACHE_DIRECTORY);
        Self::at(root)
    }

    /// Opens a cache rooted at an explicit directory. Used by the app cache
    /// directory and by tests.
    pub fn at(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| format!("Unable to create {}: {error}", root.display()))?;
        Ok(Self { root })
    }

    fn file(&self, key: &str) -> Option<PathBuf> {
        valid_key(key).then(|| self.root.join(format!("{key}.png")))
    }

    fn valid_png(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.file(key)?;
        let bytes = fs::read(&path).ok()?;
        if png_is_thumbnail(&bytes) {
            Some(bytes)
        } else {
            let _ = fs::remove_file(&path);
            None
        }
    }

    /// Whether a valid cached PNG exists for `key`. Corrupt files are removed
    /// and reported as absent, so a hit here is safe to use as a cache hit.
    pub fn contains(&self, key: &str) -> bool {
        self.valid_png(key).is_some()
    }

    /// Returns the cached PNG as a data URL. A corrupt or mistyped file is
    /// removed and treated as a miss.
    pub fn lookup(&self, key: &str) -> Option<String> {
        self.valid_png(key).map(|bytes| data_url(&bytes))
    }

    /// Validates and atomically persists `png`, returning its data URL.
    pub fn store(&self, key: &str, png: &[u8]) -> Result<String, String> {
        let path = self
            .file(key)
            .ok_or_else(|| String::from("The thumbnail key is invalid."))?;
        if !png_is_thumbnail(png) {
            return Err(format!(
                "The thumbnail image is not a valid {THUMBNAIL_SIDE} × {THUMBNAIL_SIDE} PNG."
            ));
        }
        if let Some(existing) = fs::read(&path).ok().filter(|bytes| png_is_thumbnail(bytes)) {
            return Ok(data_url(&existing));
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .root
            .join(format!(".{key}.{}-{sequence}.tmp", std::process::id()));
        if let Err(error) = write_atomic(&temp, &path, png) {
            let _ = fs::remove_file(&temp);
            // Another producer may have completed the same content-keyed write.
            return match fs::read(&path).ok().filter(|bytes| png_is_thumbnail(bytes)) {
                Some(existing) => Ok(data_url(&existing)),
                None => Err(error),
            };
        }
        Ok(data_url(png))
    }
}

fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn png_is_thumbnail(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_THUMBNAIL_BYTES
        && bytes.starts_with(&PNG_SIGNATURE)
        && image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .is_ok_and(|image| (image.width(), image.height()) == (THUMBNAIL_SIDE, THUMBNAIL_SIDE))
}

fn data_url(png: &[u8]) -> String {
    let mut url = String::with_capacity(DATA_URL_PREFIX.len() + png.len() * 4 / 3 + 4);
    url.push_str(DATA_URL_PREFIX);
    url.push_str(&STANDARD.encode(png));
    url
}

fn write_atomic(temp: &Path, path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(temp)
        .map_err(|error| format!("Unable to create {}: {error}", temp.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("Unable to write {}: {error}", temp.display()))?;
    file.sync_all()
        .map_err(|error| format!("Unable to finish {}: {error}", temp.display()))?;
    drop(file);
    fs::rename(temp, path).map_err(|error| format!("Unable to replace {}: {error}", path.display()))
}

/// Turns a submitted capture into PNG bytes. The caller uses this outside the
/// renderer lock, on whichever thread owns the job.
pub fn finish_capture_png(job: CaptureJob) -> Result<Vec<u8>, String> {
    job.finish().map_err(|error| error.to_string())?.png_bytes()
}

/// Looks up a thumbnail by hashing `path`; it never renders or writes.
pub fn cached_thumbnail(app: &AppHandle, path: &Path) -> Result<Option<String>, String> {
    let Some(source) = ThumbnailSource::open(path)? else {
        return Ok(None);
    };
    Ok(ThumbnailCache::from_app(app)?.lookup(source.key()))
}

/// Persists a rendered thumbnail only while `source` still matches the key it
/// was rendered for, checking `current` before every streamed chunk. `None`
/// means the source changed and the image was dropped.
pub fn store_source_thumbnail_checked(
    app: &AppHandle,
    source: &ThumbnailSource,
    png: &[u8],
    current: impl Fn() -> Result<(), String>,
) -> Result<Option<String>, String> {
    if !source.is_current_checked(current)? {
        return Ok(None);
    }
    ThumbnailCache::from_app(app)?
        .store(source.key(), png)
        .map(Some)
}

/// Lookup-only command: returns a cached data URL or `None`.
#[tauri::command]
pub async fn browse_thumbnail(app: AppHandle, path: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || cached_thumbnail(&app, Path::new(&path)))
        .await
        .map_err(|error| format!("The thumbnail lookup stopped unexpectedly: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{
        DATA_URL_PREFIX, PNG_SIGNATURE, THUMBNAIL_SIDE, ThumbnailCache, ThumbnailSource,
        content_hash, finish_capture_png,
    };
    use crate::preview_image::PreviewImage;
    use crate::render::context::{RenderError, ThumbnailRenderer};
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use litematica_preview_core::{MeshBatch, MeshPart, PreviewInfo, Texture};
    use std::path::{Path, PathBuf};

    fn scratch_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("litematica-thumbnail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn thumbnail_png(tint: u8) -> Vec<u8> {
        let pixels = [tint, 40, 40, 255].repeat((THUMBNAIL_SIDE * THUMBNAIL_SIDE) as usize);
        PreviewImage::new(THUMBNAIL_SIDE, THUMBNAIL_SIDE, pixels)
            .unwrap()
            .png_bytes()
            .unwrap()
    }

    fn write_source(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn identical_bytes_at_distinct_paths_share_one_cache_entry() {
        let directory = scratch_dir();
        let cache = ThumbnailCache::at(directory.join("cache")).unwrap();
        let first = write_source(&directory, "first.schem", b"the same bytes");
        let second = write_source(&directory, "second.schem", b"the same bytes");
        let first = ThumbnailSource::open(&first).unwrap().unwrap();
        let second = ThumbnailSource::open(&second).unwrap().unwrap();
        assert_eq!(first.key(), second.key());
        assert_eq!(first.key().len(), 64);

        let png = thumbnail_png(200);
        let url = cache.store(first.key(), &png).unwrap();
        assert!(url.starts_with(DATA_URL_PREFIX));
        assert_eq!(
            STANDARD
                .decode(url.strip_prefix(DATA_URL_PREFIX).unwrap())
                .unwrap(),
            png
        );
        assert!(cache.contains(second.key()));
        assert_eq!(cache.lookup(second.key()).as_deref(), Some(url.as_str()));

        let png_entries = std::fs::read_dir(&cache.root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.to_str() == Some("png"))
            })
            .count();
        assert_eq!(png_entries, 1);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn changed_bytes_at_the_same_path_produce_a_new_key_and_a_miss() {
        let directory = scratch_dir();
        let cache = ThumbnailCache::at(directory.join("cache")).unwrap();
        let path = write_source(&directory, "model.schem", b"first content");
        let original = ThumbnailSource::open(&path).unwrap().unwrap();
        cache.store(original.key(), &thumbnail_png(120)).unwrap();
        assert!(original.is_current().unwrap());

        std::fs::write(&path, b"second content").unwrap();
        assert!(!original.is_current().unwrap());
        let changed = ThumbnailSource::open(&path).unwrap().unwrap();
        assert_ne!(original.key(), changed.key());
        assert_eq!(changed.key(), content_hash(b"second content"));
        assert!(!cache.contains(changed.key()));
        assert_eq!(cache.lookup(changed.key()), None);
        assert_eq!(cache.lookup(original.key()).is_some(), true);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn persisted_png_is_atomic_valid_and_corruption_is_a_miss() {
        let directory = scratch_dir();
        let cache = ThumbnailCache::at(directory.join("cache")).unwrap();
        let key = "a".repeat(64);
        let png = thumbnail_png(180);
        let url = cache.store(&key, &png).unwrap();
        assert!(url.starts_with(DATA_URL_PREFIX));
        let stored = cache.root.join(format!("{key}.png"));
        assert_eq!(std::fs::read(&stored).unwrap(), png);
        assert!(png.starts_with(&PNG_SIGNATURE));
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (THUMBNAIL_SIDE, THUMBNAIL_SIDE)
        );

        // No temporary file survives a completed write.
        let temporaries = std::fs::read_dir(&cache.root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.to_str() == Some("tmp"))
            })
            .count();
        assert_eq!(temporaries, 0);

        std::fs::write(&stored, b"not a png").unwrap();
        assert_eq!(cache.lookup(&key), None);
        assert!(!stored.exists());

        assert!(cache.store(&key, b"not a png").is_err());
        assert!(cache.store("../escape", &png).is_err());
        assert_eq!(cache.lookup("../escape"), None);
        assert!(!cache.contains("../escape"));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    fn test_batch() -> MeshBatch {
        let mut geometry = Vec::new();
        for (position, uv) in [
            ([-1.0f32, -1.0, 0.0], [0.0f32, 0.0]),
            ([1.0, -1.0, 0.0], [1.0, 0.0]),
            ([0.0, 1.0, 0.0], [0.5, 1.0]),
        ] {
            for value in position {
                geometry.extend_from_slice(&value.to_le_bytes());
            }
            for value in uv {
                geometry.extend_from_slice(&value.to_le_bytes());
            }
            geometry.extend_from_slice(&[0, 0, 127, 0]);
            geometry.extend_from_slice(&[255, 255, 255, 255]);
        }
        assert_eq!(geometry.len(), 84);
        // u16 indices 0, 1, 2 in little-endian order, then two padding bytes to
        // keep the packed geometry 4-byte aligned.
        geometry.extend_from_slice(&[0, 0, 1, 0, 2, 0, 0, 0]);
        let info = PreviewInfo {
            block_count: 1,
            block_entity_count: 0,
            triangle_count: 1,
            part_count: 1,
            texture_count: 1,
            min: [-1.0, -1.0, 0.0],
            max: [1.0, 1.0, 0.0],
        };
        MeshBatch {
            geometry,
            vertex_stride: 28,
            parts: vec![MeshPart {
                vertex_offset: 0,
                index_offset: 84,
                vertex_count: 3,
                index_count: 3,
                index_size: 2,
                alpha_mode: 1,
                texture_index: 0,
                min: [-1.0, -1.0, 0.0],
                max: [1.0, 1.0, 0.0],
            }],
            textures: vec![Texture {
                index: 0,
                pixels: vec![255, 255, 255, 255],
                width: 1,
                height: 1,
                repeat: false,
            }],
            origin: [0.0; 3],
            extent: [1.0; 3],
            source_min: [-1.0; 3],
            source_max: [1.0; 3],
            chunk_coord: None,
            vertex_count: 3,
            index_count: 3,
            info,
        }
    }

    #[test]
    fn scratch_renderer_frames_the_model_inside_the_thumbnail() {
        let batch = test_batch();
        let mut renderer = match ThumbnailRenderer::new() {
            Ok(renderer) => renderer,
            // Hosts without a usable Metal/Vulkan/DX12 adapter skip the GPU case.
            Err(RenderError::Initialization(_)) => return,
            Err(error) => panic!("the thumbnail renderer failed: {error}"),
        };
        renderer.prepare(batch.info.min, batch.info.max);
        renderer.attach_checked(&batch, || Ok(())).unwrap();
        renderer.finish(&batch.info);
        let job = renderer
            .begin_capture_checked(THUMBNAIL_SIDE, || Ok(()))
            .unwrap();
        let png = finish_capture_png(job).unwrap();
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(image.dimensions(), (THUMBNAIL_SIDE, THUMBNAIL_SIDE));
        let center = image.get_pixel(THUMBNAIL_SIDE / 2, THUMBNAIL_SIDE / 2).0;
        let corner = image.get_pixel(1, 1).0;
        assert!(
            center[0] > 60 && center[1] > 60 && center[2] > 60,
            "the fitted model should cover the centre: {center:?}"
        );
        assert!(
            corner[0] < 40 && corner[1] < 40 && corner[2] < 40,
            "the fitted model must leave the frame corner clear: {corner:?}"
        );
    }
}
