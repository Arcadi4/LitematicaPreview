//! Persistent square thumbnails for the schematic browser.
//!
//! Thumbnails are keyed by the SHA-256 of the source file bytes, so the same
//! content reached through different paths shares one cached image. The cache
//! lives under the app cache directory and holds only validated 128 px PNGs;
//! lookup never reads an image path supplied by the caller.
//!
//! A hit is PNG bytes or the validated cache path, ready for the app's own
//! image cache: `ImageCache::load` for [`ThumbnailCache::lookup`] bytes or
//! `ImageCache::file` for [`ThumbnailCache::cached_path`].

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Square edge of every browser thumbnail.
pub const THUMBNAIL_SIDE: u32 = 128;

/// Directory name the cache uses below the app cache directory.
pub const CACHE_DIRECTORY: &str = "browse-thumbnails";

const HASH_CHUNK_BYTES: usize = 64 * 1024;
/// A 128 × 128 PNG is far smaller; this bound keeps a corrupt or replaced
/// cache file from forcing a large decode.
const MAX_THUMBNAIL_BYTES: usize = 1024 * 1024;
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

/// Content-addressed PNG cache of validated square thumbnails.
#[derive(Clone, Debug)]
pub struct ThumbnailCache {
    root: PathBuf,
}

impl ThumbnailCache {
    /// Opens (and creates) the browser thumbnail cache below `cache_dir`,
    /// normally the app's cache directory.
    pub fn from_path(cache_dir: impl Into<PathBuf>) -> Result<Self, String> {
        Self::at(cache_dir.into().join(CACHE_DIRECTORY))
    }

    /// Opens (and creates) the cache at exactly `root`.
    pub fn at(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| format!("Unable to create {}: {error}", root.display()))?;
        Ok(Self { root })
    }

    fn file(&self, key: &str) -> Option<PathBuf> {
        valid_key(key).then(|| self.root.join(format!("{key}.png")))
    }

    /// The validated cache file and its bytes. A corrupt or mistyped file is
    /// removed and treated as a miss.
    fn validated(&self, key: &str) -> Option<(PathBuf, Vec<u8>)> {
        let path = self.file(key)?;
        let bytes = fs::read(&path).ok()?;
        if png_is_thumbnail(&bytes) {
            Some((path, bytes))
        } else {
            let _ = fs::remove_file(&path);
            None
        }
    }

    /// Whether a valid cached PNG exists for `key`. Corrupt files are removed
    /// and reported as absent, so a hit here is safe to use as a cache hit.
    pub fn contains(&self, key: &str) -> bool {
        self.validated(key).is_some()
    }

    /// Validated PNG bytes for `key`, ready for `ImageCache::load`.
    pub fn lookup(&self, key: &str) -> Option<Vec<u8>> {
        self.validated(key).map(|(_, bytes)| bytes)
    }

    /// The validated cache path for `key`, ready for `ImageCache::file`.
    pub fn cached_path(&self, key: &str) -> Option<PathBuf> {
        self.validated(key).map(|(path, _)| path)
    }

    /// Validates and atomically persists `png`; an already cached valid image
    /// for `key` is kept.
    pub fn store(&self, key: &str, png: &[u8]) -> Result<(), String> {
        let path = self
            .file(key)
            .ok_or_else(|| String::from("The thumbnail key is invalid."))?;
        if !png_is_thumbnail(png) {
            return Err(format!(
                "The thumbnail image is not a valid {THUMBNAIL_SIDE} × {THUMBNAIL_SIDE} PNG."
            ));
        }
        if fs::read(&path)
            .ok()
            .is_some_and(|bytes| png_is_thumbnail(&bytes))
        {
            return Ok(());
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .root
            .join(format!(".{key}.{}-{sequence}.tmp", std::process::id()));
        if let Err(error) = write_atomic(&temp, &path, png) {
            let _ = fs::remove_file(&temp);
            // Another producer may have completed the same content-keyed write.
            return match fs::read(&path).ok().filter(|bytes| png_is_thumbnail(bytes)) {
                Some(_) => Ok(()),
                None => Err(error),
            };
        }
        Ok(())
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

/// Persists a rendered thumbnail only while `source` still matches the key it
/// was rendered for, checking `current` before every streamed chunk. `false`
/// means the source changed and the image was dropped.
pub fn store_source_thumbnail_checked(
    cache: &ThumbnailCache,
    source: &ThumbnailSource,
    png: &[u8],
    current: impl Fn() -> Result<(), String>,
) -> Result<bool, String> {
    if !source.is_current_checked(current)? {
        return Ok(false);
    }
    cache.store(source.key(), png).map(|()| true)
}

#[cfg(test)]
mod tests {
    use super::{
        CACHE_DIRECTORY, PNG_SIGNATURE, THUMBNAIL_SIDE, ThumbnailCache, ThumbnailSource,
        content_hash, store_source_thumbnail_checked,
    };
    use std::path::{Path, PathBuf};

    fn scratch_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("litematica-thumbnail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn thumbnail_png(tint: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(
            THUMBNAIL_SIDE,
            THUMBNAIL_SIDE,
            image::Rgba([tint, 40, 40, 255]),
        );
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
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
        cache.store(first.key(), &png).unwrap();
        assert_eq!(cache.lookup(second.key()), Some(png.clone()));
        assert_eq!(
            std::fs::read(cache.cached_path(second.key()).unwrap()).unwrap(),
            png
        );
        assert!(cache.contains(second.key()));

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
        assert!(cache.lookup(original.key()).is_some());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn persisted_png_is_atomic_valid_and_corruption_is_a_miss() {
        let directory = scratch_dir();
        let cache = ThumbnailCache::at(directory.join("cache")).unwrap();
        let key = "a".repeat(64);
        let png = thumbnail_png(180);
        cache.store(&key, &png).unwrap();
        let stored = cache.root.join(format!("{key}.png"));
        assert_eq!(std::fs::read(&stored).unwrap(), png);
        assert_eq!(cache.lookup(&key), Some(png.clone()));
        assert_eq!(cache.cached_path(&key), Some(stored.clone()));
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
        assert_eq!(cache.cached_path(&key), None);
        assert!(!stored.exists());

        assert!(cache.store(&key, b"not a png").is_err());
        assert!(cache.store("../escape", &png).is_err());
        assert_eq!(cache.lookup("../escape"), None);
        assert!(!cache.contains("../escape"));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn from_path_nests_the_cache_below_the_app_cache_directory() {
        let directory = scratch_dir();
        let cache = ThumbnailCache::from_path(directory.clone()).unwrap();
        assert_eq!(cache.root, directory.join(CACHE_DIRECTORY));
        assert!(cache.root.is_dir());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_changed_source_is_never_stored() {
        let directory = scratch_dir();
        let cache = ThumbnailCache::at(directory.join("cache")).unwrap();
        let path = write_source(&directory, "model.schem", b"stable content");
        let source = ThumbnailSource::open(&path).unwrap().unwrap();
        assert_eq!(cache.lookup(source.key()), None);

        let png = thumbnail_png(90);
        assert!(store_source_thumbnail_checked(&cache, &source, &png, || Ok(())).unwrap());
        assert_eq!(cache.lookup(source.key()), Some(png));

        let changed = ThumbnailSource::open(&path).unwrap().unwrap();
        std::fs::write(&path, b"new content").unwrap();
        assert!(
            !store_source_thumbnail_checked(&cache, &changed, &thumbnail_png(30), || Ok(()))
                .unwrap()
        );
        assert_eq!(
            ThumbnailSource::open(&path).unwrap().unwrap().key(),
            content_hash(b"new content")
        );
        assert_eq!(cache.lookup(content_hash(b"new content").as_str()), None);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
