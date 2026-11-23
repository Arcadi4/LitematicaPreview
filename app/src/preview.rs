use std::fs::File;
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use litematica_preview_core::{MeshBatch, MeshStream, PreviewInfo, PreviewOptions};
use nucleation::formats::limits::DecodeLimits;
use nucleation::meshing::ResourcePackSource;
use serde::{Deserialize, Serialize};

use crate::resources;

pub const MAX_ATLAS_SIDE: u32 = 8192;
const MAX_TEXTURE_PACK_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadOptions {
    #[serde(deserialize_with = "required_nullable")]
    chunk_size: Option<u16>,
    #[serde(deserialize_with = "required_nullable")]
    thread_count: Option<u8>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer)
}

impl LoadOptions {
    pub fn validate(self) -> Result<PreviewOptions, String> {
        let options = PreviewOptions {
            memory_limit_mb: None,
            chunk_size: self.chunk_size,
            thread_count: self.thread_count,
            speed_first: self.thread_count.is_some(),
        };
        options.validate()?;
        Ok(options)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub block_count: i64,
    pub block_entity_count: i64,
    pub triangle_count: u64,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub geometry_byte_length: usize,
    pub byte_length: usize,
    pub texture_count: u32,
    pub part_count: u32,
}

impl Metadata {
    pub fn info(&self) -> PreviewInfo {
        PreviewInfo {
            block_count: self.block_count,
            block_entity_count: self.block_entity_count,
            triangle_count: self.triangle_count,
            min: self.min,
            max: self.max,
            texture_count: self.texture_count,
            part_count: self.part_count,
        }
    }
}

/// One resource pack in priority order, highest first. `name` identifies the
/// pack in errors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackLayer {
    pub name: String,
    pub path: PathBuf,
}

/// A preview request always uses the embedded vanilla pack. Only the ordered
/// user texture-pack layers and the schematic path are supplied per load.
#[derive(Clone, Copy)]
pub struct PreviewRequest<'a> {
    pub path: &'a Path,
    pub layers: &'a [PackLayer],
    pub options: PreviewOptions,
    pub max_texture_side: u32,
}

/// Parses the vanilla pack embedded in the executable. No adjacent asset is
/// read, so previews work from a relocatable or self-updated install.
pub fn vanilla_pack() -> Result<ResourcePackSource, String> {
    ResourcePackSource::from_bytes(resources::VANILLA_PACK)
        .map_err(|error| format!("The bundled block resources are invalid: {error}"))
}

/// Overlays `layers` from lowest to highest on one clone of the parsed base.
/// The bundled vanilla pack stays the final per-key fallback.
pub fn compose_resource_packs(
    base: &ResourcePackSource,
    layers: &[PackLayer],
    mut read: impl FnMut(&PackLayer) -> Result<Vec<u8>, String>,
) -> Result<ResourcePackSource, String> {
    let empty = ResourcePackSource::from_resource_pack(Default::default());
    let mut merged = base.pack().clone();
    for layer in layers.iter().rev() {
        let bytes = read(layer).map_err(|error| {
            format!(
                "The texture pack “{}” could not be read: {error}",
                layer.name
            )
        })?;
        let mut parsed =
            litematica_preview_core::resource_pack_overlay(&empty, &bytes).map_err(|error| {
                format!(
                    "The texture pack “{}” could not be used: {error}",
                    layer.name
                )
            })?;
        merged.overlay(std::mem::take(parsed.pack_mut()));
    }
    Ok(ResourcePackSource::from_resource_pack(merged))
}

/// Composes the enabled texture-pack stack over the embedded vanilla pack.
/// This is the validation path for a saved texture library.
pub fn compose_texture_library(
    layers: &[PackLayer],
    read: impl FnMut(&PackLayer) -> Result<Vec<u8>, String>,
) -> Result<ResourcePackSource, String> {
    compose_resource_packs(&vanilla_pack()?, layers, read)
}

/// Validates that every enabled layer parses and overlays cleanly.
pub fn validate_texture_library(
    layers: &[PackLayer],
    read: impl FnMut(&PackLayer) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    compose_texture_library(layers, read).map(|_| ())
}

/// Validates one imported texture pack before it enters the library.
pub fn validate_texture_pack(name: &str, bytes: &[u8]) -> Result<(), String> {
    let empty = ResourcePackSource::from_resource_pack(Default::default());
    litematica_preview_core::resource_pack_overlay(&empty, bytes)
        .map(|_| ())
        .map_err(|error| format!("The texture pack “{name}” could not be used: {error}"))
}

/// Parsed packs reused across loader jobs. The embedded base parses once. The
/// overlay rebuilds only when the ordered layer paths change.
#[derive(Default)]
pub struct PackCache {
    base: Option<ResourcePackSource>,
    overlay: Option<(Vec<PathBuf>, ResourcePackSource)>,
}

impl PackCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seeds the cache with an already parsed base pack so a caller can keep
    /// that parse outside a measured window.
    pub fn with_base(base: ResourcePackSource) -> Self {
        Self {
            base: Some(base),
            overlay: None,
        }
    }

    fn base(&mut self) -> Result<&ResourcePackSource, String> {
        if self.base.is_none() {
            self.base = Some(vanilla_pack()?);
        }
        self.base
            .as_ref()
            .ok_or_else(|| "The bundled block resources are unavailable.".to_string())
    }

    fn resolve(
        &mut self,
        layers: &[PackLayer],
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<&ResourcePackSource, String> {
        current()?;
        self.base()?;
        if layers.is_empty() {
            self.overlay = None;
            return self
                .base
                .as_ref()
                .ok_or_else(|| "The bundled block resources are unavailable.".to_string());
        }
        let key: Vec<PathBuf> = layers.iter().map(|layer| layer.path.clone()).collect();
        if self
            .overlay
            .as_ref()
            .is_none_or(|(cached, _)| cached != &key)
        {
            self.overlay = None;
            let merged = compose_resource_packs(
                self.base
                    .as_ref()
                    .ok_or("The bundled block resources are unavailable.")?,
                layers,
                |layer| read_resource_pack(&layer.path, current),
            )?;
            self.overlay = Some((key, merged));
        }
        Ok(&self
            .overlay
            .as_ref()
            .ok_or("The texture pack stack is unavailable.")?
            .1)
    }
}

/// Decodes and streams one preview. `consume` receives each upload-ready
/// batch by value, so geometry and first-use atlas pixels are never copied.
/// `current` is polled throughout so a superseded load stops promptly.
pub fn decode(
    request: PreviewRequest<'_>,
    packs: &mut PackCache,
    current: impl Fn() -> Result<(), String>,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    mut consume: impl FnMut(MeshBatch) -> Result<(), String>,
    mut progress: impl FnMut(&'static str, u64, u64),
) -> Result<Metadata, String> {
    let PreviewRequest {
        path,
        layers,
        options,
        max_texture_side,
    } = request;
    let result = catch_unwind(AssertUnwindSafe(|| {
        options.validate()?;
        progress("decode", 0, 0);
        let data = read_file(path, "schematic", u64::MAX, &current)?;
        let pack = packs.resolve(layers, &current)?;
        let source = litematica_preview_core::decode_with_options(
            &data,
            &decode_limits(),
            options,
            &current,
        )
        .map_err(|error| error.to_string())?;
        drop(data);
        if source.block_count() == 0 {
            return Err("The schematic must contain at least one block.".into());
        }
        let origin = source.content_origin();
        let dimensions = source.content_dimensions();
        let min = [
            origin.0 as f32 - 0.5,
            origin.1 as f32 - 0.5,
            origin.2 as f32 - 0.5,
        ];
        let max = [
            min[0] + dimensions.0 as f32,
            min[1] + dimensions.1 as f32,
            min[2] + dimensions.2 as f32,
        ];
        prepared(min, max)?;
        let atlas_side = max_texture_side.min(MAX_ATLAS_SIDE);
        let mut stream = MeshStream::new(
            &source,
            pack,
            &litematica_preview_core::mesh_config().with_atlas_max_size(atlas_side),
            usize::from(options.thread_count.unwrap_or(1)),
            &current,
        )
        .map_err(|error| {
            if error.contains("Atlas building error: Failed to pack ") {
                format!("{error}. The texture atlas limit is {atlas_side} × {atlas_side}; try a lower-resolution texture pack.")
            } else {
                error
            }
        })?;
        let total = stream.batch_count() as u64;
        progress("mesh", 0, total);
        let mut atlas = Some(stream.take_atlas());
        let mut metadata = Metadata {
            block_count: source.block_count(),
            block_entity_count: source.block_entity_count(),
            triangle_count: 0,
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
            geometry_byte_length: 0,
            byte_length: 0,
            texture_count: 0,
            part_count: 0,
        };
        let mut completed = 0;
        let mut last_progress = Instant::now();
        while let Some(mut batch) = stream.next(&current)? {
            if let Some(texture) = atlas.take() {
                batch.textures.insert(0, texture);
            }
            metadata.triangle_count += batch.info.triangle_count;
            metadata.part_count += batch.info.part_count;
            metadata.texture_count = batch.info.texture_count;
            metadata.geometry_byte_length += batch.geometry.len();
            metadata.byte_length += batch.geometry.len()
                + batch
                    .textures
                    .iter()
                    .map(|texture| texture.pixels.len())
                    .sum::<usize>();
            if !batch.parts.is_empty() {
                for axis in 0..3 {
                    metadata.min[axis] = metadata.min[axis].min(batch.info.min[axis]);
                    metadata.max[axis] = metadata.max[axis].max(batch.info.max[axis]);
                }
            }
            consume(batch)?;
            completed += 1;
            if completed == total || last_progress.elapsed() >= Duration::from_millis(150) {
                progress("mesh", completed, total);
                last_progress = Instant::now();
            }
        }
        current()?;
        if metadata.part_count == 0 {
            return Err("The schematic contains no visible geometry.".into());
        }
        progress("upload", completed, total);
        Ok(metadata)
    }));
    result.unwrap_or_else(|panic| {
        *packs = PackCache::default();
        Err(panic_message(panic))
    })
}

/// Foreground previews keep the historical representation limits.
fn decode_limits() -> DecodeLimits {
    DecodeLimits {
        max_input_bytes: isize::MAX as usize,
        max_decompressed_bytes: isize::MAX as usize,
        max_dimension: i32::MAX as usize,
        max_volume: isize::MAX as usize / std::mem::size_of::<usize>(),
        max_regions: usize::MAX,
        max_palette_entries: usize::MAX,
        max_entities: usize::MAX,
        max_block_entities: usize::MAX,
        max_nbt_depth: 64,
        max_nbt_string_bytes: isize::MAX as usize,
        max_nbt_collection_items: usize::MAX,
        max_nbt_nodes: usize::MAX,
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    let detail = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("Unknown native panic");
    format!("The preview worker encountered an internal error: {detail}")
}

fn read_resource_pack(
    path: &Path,
    current: impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    read_file(path, "resource pack", MAX_TEXTURE_PACK_BYTES, current)
}

fn read_file(
    path: &Path,
    kind: &str,
    max_bytes: u64,
    current: impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let mut file =
        File::open(path).map_err(|e| format!("Unable to open {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Unable to inspect {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("Choose a {kind} file, not a folder or device."));
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "Choose a {kind} smaller than {} MB.",
            max_bytes / (1024 * 1024)
        ));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        current()?;
        let count = match file.read(&mut chunk) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Unable to read the file: {error}")),
        };
        if count == 0 {
            break;
        }
        bytes
            .try_reserve(count)
            .map_err(|_| "There is not enough memory to read this file.".to_string())?;
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() as u64 > max_bytes {
            return Err(format!(
                "Choose a {kind} smaller than {} MB.",
                max_bytes / (1024 * 1024)
            ));
        }
    }
    if bytes.is_empty() {
        return Err(format!("Choose a nonempty {kind}."));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_vanilla_pack_parses_without_adjacent_assets() {
        assert!(vanilla_pack().is_ok());
    }

    #[test]
    fn stack_decodes_and_meshes_a_fixture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let schematic = root.join("../fixtures/formats/Sponge.schem");
        let layer = PackLayer {
            name: "Vanilla copy".into(),
            path: root.join("../assets/pack.zip"),
        };
        let mut packs = PackCache::default();
        let mut batches = 0;
        let metadata = decode(
            PreviewRequest {
                path: &schematic,
                layers: std::slice::from_ref(&layer),
                options: PreviewOptions {
                    chunk_size: Some(32),
                    thread_count: Some(2),
                    ..PreviewOptions::default()
                },
                max_texture_side: MAX_ATLAS_SIDE,
            },
            &mut packs,
            || Ok(()),
            |_, _| Ok(()),
            |_| {
                batches += 1;
                Ok(())
            },
            |_, _, _| {},
        )
        .expect("preview with the texture pack stack");
        assert!(batches > 0);
        assert!(metadata.part_count > 0);
    }

    #[test]
    fn malformed_stack_layer_is_rejected_with_its_name() {
        let custom = std::env::temp_dir().join(format!(
            "litematica-preview-stack-layer-{}.zip",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&custom, b"not a zip").unwrap();
        let layers = [PackLayer {
            name: "Broken".into(),
            path: custom.clone(),
        }];
        let mut packs = PackCache::default();
        let current = || Ok(());
        assert!(packs.resolve(&[], &current).is_ok());
        let error = match packs.resolve(&layers, &current) {
            Err(error) => error,
            Ok(_) => panic!("a malformed stack layer must not be accepted"),
        };
        assert!(error.contains("Broken"), "{error}");
        std::fs::remove_file(&custom).unwrap();
    }

    #[test]
    fn higher_priority_pack_wins_and_lower_pack_fills_missing_resources() {
        let high = resource_pack(&[("shared", [0, 0, 255, 255])]);
        let low = resource_pack(&[("shared", [255, 0, 0, 255]), ("low_only", [0, 255, 0, 255])]);
        let layers = [
            PackLayer {
                name: "High".into(),
                path: PathBuf::from("high.zip"),
            },
            PackLayer {
                name: "Low".into(),
                path: PathBuf::from("low.zip"),
            },
        ];
        let base =
            ResourcePackSource::from_bytes(&resource_pack(&[("base_only", [10, 20, 30, 255])]))
                .unwrap();
        let composed = compose_resource_packs(&base, &layers, |layer| {
            Ok(if layer.name == "High" {
                high.clone()
            } else {
                low.clone()
            })
        })
        .unwrap();

        assert_eq!(
            composed.get_texture_pixels("minecraft:block/shared"),
            Some([0_u8, 0, 255, 255].as_slice())
        );
        assert_eq!(
            composed.get_texture_pixels("minecraft:block/low_only"),
            Some([0_u8, 255, 0, 255].as_slice())
        );
        assert_eq!(
            composed.get_texture_pixels("minecraft:block/base_only"),
            Some([10_u8, 20, 30, 255].as_slice())
        );
    }

    #[test]
    fn invalid_layer_errors_name_the_pack() {
        let base = ResourcePackSource::from_resource_pack(Default::default());
        let layers = [PackLayer {
            name: "Broken".into(),
            path: PathBuf::from("broken.zip"),
        }];
        let error = compose_resource_packs(&base, &layers, |_| Ok(b"not a zip".to_vec()))
            .err()
            .expect("the invalid layer must fail");
        assert!(error.contains("Broken"), "{error}");
    }

    fn resource_pack(textures: &[(&str, [u8; 4])]) -> Vec<u8> {
        let mut files = vec![(
            "pack.mcmeta".to_owned(),
            br#"{"pack":{"pack_format":15,"description":"test"}}"#.to_vec(),
        )];
        for (name, color) in textures {
            files.push((
                format!("assets/minecraft/textures/block/{name}.png"),
                texture_png(*color),
            ));
        }
        zip_bytes(&files)
    }

    fn texture_png(color: [u8; 4]) -> Vec<u8> {
        let raw = [0, color[0], color[1], color[2], color[3]];
        let mut zlib = vec![0x78, 0x01, 0x01, 0x05, 0x00, 0xfa, 0xff];
        zlib.extend_from_slice(&raw);
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png_chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]);
        png_chunk(&mut png, b"IDAT", &zlib);
        png_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let mut crc = Vec::with_capacity(4 + data.len());
        crc.extend_from_slice(kind);
        crc.extend_from_slice(data);
        png.extend_from_slice(&crc32(&crc).to_be_bytes());
    }

    fn adler32(data: &[u8]) -> u32 {
        let mut a = 1_u32;
        let mut b = 0_u32;
        for byte in data {
            a = (a + u32::from(*byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        (b << 16) | a
    }

    fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut archive = Vec::new();
        let mut directory = Vec::new();
        for (name, data) in entries {
            let offset = archive.len() as u32;
            let crc = crc32(data);
            archive.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            archive.extend_from_slice(&20_u16.to_le_bytes());
            archive.extend_from_slice(&[0; 2]);
            archive.extend_from_slice(&[0; 2]);
            archive.extend_from_slice(&[0; 4]);
            archive.extend_from_slice(&crc.to_le_bytes());
            archive.extend_from_slice(&(data.len() as u32).to_le_bytes());
            archive.extend_from_slice(&(data.len() as u32).to_le_bytes());
            archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
            archive.extend_from_slice(&0_u16.to_le_bytes());
            archive.extend_from_slice(name.as_bytes());
            archive.extend_from_slice(data);

            directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            directory.extend_from_slice(&20_u16.to_le_bytes());
            directory.extend_from_slice(&20_u16.to_le_bytes());
            directory.extend_from_slice(&[0; 2]);
            directory.extend_from_slice(&[0; 2]);
            directory.extend_from_slice(&[0; 4]);
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 2]);
            directory.extend_from_slice(&[0; 2]);
            directory.extend_from_slice(&[0; 2]);
            directory.extend_from_slice(&[0; 2]);
            directory.extend_from_slice(&[0; 4]);
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_offset = archive.len() as u32;
        archive.extend_from_slice(&directory);
        archive.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        archive.extend_from_slice(&[0; 4]);
        archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        archive.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        archive.extend_from_slice(&directory_offset.to_le_bytes());
        archive.extend_from_slice(&[0; 2]);
        archive
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }

    #[test]
    fn invalid_worker_and_chunk_combinations_are_rejected() {
        for value in [
            serde_json::json!({"chunkSize": null, "threadCount": 2}),
            serde_json::json!({"chunkSize": 48, "threadCount": null}),
            serde_json::json!({"chunkSize": 64, "threadCount": 1}),
        ] {
            assert!(
                serde_json::from_value::<LoadOptions>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
}
