use nucleation::formats::limits::DecodeLimits;
use nucleation::formats::{
    classic_schematic, manager::SchematicImporter, mcstructure, snapshot, structure_snbt, world,
};
use nucleation::UniversalSchematic;
use regex::Regex;

#[path = "bounded_nbt.rs"]
mod bounded_nbt;
#[path = "litematic.rs"]
mod litematic;
#[path = "schematic.rs"]
mod schematic;
#[path = "structure_nbt.rs"]
mod structure_nbt;

// Compressed schematic bytes accepted from the desktop host.
const MAX_INPUT_BYTES: usize = 1_024 * 1_024 * 1_024;
// Inflated NBT bytes Nucleation may allocate while decoding.
const MAX_DECOMPRESSED_BYTES: usize = 1_024 * 1_024 * 1_024;
const MAX_AXIS_LENGTH: usize = 4_096;
// One GiB for all dense usize Region indices leaves room in the 2 GiB worker
// for the independently bounded input/NBT, chunk index, resources and mesh.
// On x64 the 134,217,728-cell ceiling admits the 95,722,550-cell target.
const MAX_DENSE_INDEX_BYTES: usize = 1_024 * 1_024 * 1_024;
const MAX_VOLUME: usize = MAX_DENSE_INDEX_BYTES / std::mem::size_of::<usize>();
const MAX_REGIONS: usize = 64;
const MAX_PALETTE_ENTRIES: usize = 4_096;
const MAX_ENTITIES: usize = 100_000;
const MAX_BLOCK_ENTITIES: usize = 100_000;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_STRING_BYTES: usize = 1_000_000;
// A Sponge `BlockData` array declares one VarInt per padded cell, so the
// collection allowance is the volume widened by a VarInt's width.
const MAX_NBT_COLLECTION_ITEMS: usize = MAX_VOLUME * 2;
const MAX_NBT_NODES: usize = 4_194_304;

// Decode limits bounding what a preview accepts.
pub fn preview_limits() -> DecodeLimits {
    DecodeLimits {
        max_input_bytes: MAX_INPUT_BYTES,
        max_decompressed_bytes: MAX_DECOMPRESSED_BYTES,
        max_dimension: MAX_AXIS_LENGTH,
        max_volume: MAX_VOLUME,
        max_regions: MAX_REGIONS,
        max_palette_entries: MAX_PALETTE_ENTRIES,
        max_entities: MAX_ENTITIES,
        max_block_entities: MAX_BLOCK_ENTITIES,
        max_nbt_depth: MAX_NBT_DEPTH,
        max_nbt_string_bytes: MAX_NBT_STRING_BYTES,
        max_nbt_collection_items: MAX_NBT_COLLECTION_ITEMS,
        max_nbt_nodes: MAX_NBT_NODES,
    }
}

// Rewrites the brace block-state spelling some structure SNBT uses (for
// example `state: "minecraft:oak_log{axis=y}"`) into the bracket form
// Nucleation's reader accepts, for one retry after a failed import.
// Restricted to `state` values so nothing else in the document is touched.
// `None` skips the retry.
fn normalize_structure_snbt(bytes: &[u8]) -> Option<Vec<u8>> {
    static BRACE_STATE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r#"state:\s*"([\w.-]+:[\w/.-]+)\{([\w.-]+[=:][\w.\-/]+(?:,[\w.-]+[=:][\w.\-/]+)*)\}""#,
        )
        .expect("brace-state pattern is static")
    });
    let text = std::str::from_utf8(bytes).ok()?;
    let brace_state = &*BRACE_STATE;

    if !brace_state.is_match(text) {
        return None;
    }
    Some(
        brace_state
            .replace_all(text, r#"state:"$1[$2]""#)
            .into_owned()
            .into_bytes(),
    )
}

#[derive(Debug)]
pub enum DecodeFailure {
    Format(String),
    Limit(String),
}

// Decode schematic `bytes`, refusing anything outside the preview budget.
//
// Keep the bounded reader and two required compatibility fallbacks. The old
// diagnostic retry with relaxed limits did extra work only to refine an error.
pub fn decode(bytes: &[u8]) -> Result<UniversalSchematic, DecodeFailure> {
    let limits = preview_limits();
    limits
        .check_input(bytes)
        .map_err(|error| DecodeFailure::Limit(error.to_string()))?;

    if let Ok(schematic) = read_bounded(bytes, &limits) {
        return Ok(schematic);
    }

    // Retry the existing brace-state compatibility spelling only once.
    if let Some(normalized) = normalize_structure_snbt(bytes) {
        if let Ok(schematic) = read_bounded(&normalized, &limits) {
            return Ok(schematic);
        }
    }
    if let Some(result) = structure_nbt::try_load(bytes, &limits) {
        return result;
    }
    Err(DecodeFailure::Format(
        "This file is not a readable Minecraft schematic, or it exceeds the preview limits.".into(),
    ))
}

fn read_bounded(bytes: &[u8], limits: &DecodeLimits) -> Result<UniversalSchematic, String> {
    limits
        .check_input(bytes)
        .map_err(|error| error.to_string())?;
    // Preserve registry precedence and retain the first successful decode.
    // A failed parse continues probing, just as the original bounded detector.
    if let Ok(schematic) = litematic::read(bytes, limits) {
        return Ok(schematic);
    }
    if let Ok(schematic) = schematic::read(bytes, limits) {
        return Ok(schematic);
    }
    if let Ok(schematic) = mcstructure::from_mcstructure_bounded(bytes, limits) {
        return Ok(schematic);
    }
    // Header-detected formats keep their terminal read-error semantics.
    if snapshot::SnapshotFormat.detect_bounded(bytes, limits) {
        return snapshot::from_snapshot_bounded(bytes, limits).map_err(|error| error.to_string());
    }
    if let Ok(schematic) = structure_snbt::from_structure_snbt_bounded(bytes, limits) {
        return Ok(schematic);
    }
    if let Ok(schematic) = classic_schematic::from_classic_schematic_bounded(bytes, limits) {
        return Ok(schematic);
    }
    if world::McaFormat.detect_bounded(bytes, limits) {
        return world::McaFormat
            .read_bounded(bytes, limits)
            .map_err(|error| error.to_string());
    }
    if world::WorldZipFormat.detect_bounded(bytes, limits) {
        return world::WorldZipFormat
            .read_bounded(bytes, limits)
            .map_err(|error| error.to_string());
    }
    Err("Unknown or unsupported schematic format".into())
}

// Native readers enforce the exact source palette cap before constructing a
// Region. Its public constructor adds one ordinary-air entry even when the
// source contains no air; that implementation detail must not reject the file.
fn validate_with_implicit_air(
    schematic: &UniversalSchematic,
    limits: &DecodeLimits,
) -> Result<(), String> {
    let mut internal_limits = limits.clone();
    internal_limits.max_palette_entries = limits.max_palette_entries.saturating_add(1);
    internal_limits
        .validate_schematic(schematic)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quartz_nbt::{NbtCompound, NbtList, NbtTag};

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../Fixtures/Formats")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn native_dispatch_enforces_aggregate_volume_at_the_boundary() {
        let snbt = normalize_structure_snbt(&fixture("Structure.snbt")).unwrap();
        let cases = [
            fixture("Classic.schematic"),
            fixture("Sponge.schem"),
            fixture("Bedrock.mcstructure"),
            fixture("Snapshot.nusn"),
            snbt,
        ];
        for bytes in cases {
            let schematic = read_bounded(&bytes, &preview_limits()).unwrap();
            let volume = std::iter::once(&schematic.default_region)
                .chain(schematic.other_regions.values())
                .map(|region| region.volume())
                .sum();
            let mut limits = DecodeLimits {
                max_volume: volume,
                ..preview_limits()
            };
            let bounded = read_bounded(&bytes, &limits).unwrap();
            assert_eq!(bounded.total_blocks(), schematic.total_blocks());
            limits.max_volume -= 1;
            assert!(read_bounded(&bytes, &limits).is_err());
        }
    }

    #[test]
    fn litematic_preflight_counts_negative_extents_and_all_region_budgets() {
        let mut regions = NbtCompound::new();
        for (index, x) in [-2, 2].into_iter().enumerate() {
            let mut size = NbtCompound::new();
            size.insert("x", x);
            size.insert("y", 2);
            size.insert("z", 2);
            let mut position = NbtCompound::new();
            for key in ["x", "y", "z"] {
                position.insert(key, 0);
            }
            let mut air = NbtCompound::new();
            air.insert("Name", "minecraft:air");
            let mut region = NbtCompound::new();
            region.insert("Size", size);
            region.insert("Position", position);
            region.insert(
                "BlockStatePalette",
                NbtList::from(vec![NbtTag::Compound(air)]),
            );
            region.insert("BlockStates", NbtTag::LongArray(vec![0]));
            let mut entity = NbtCompound::new();
            entity.insert("id", "minecraft:pig");
            entity.insert("Pos", NbtList::from(vec![NbtTag::Double(0.0); 3]));
            region.insert("Entities", NbtList::from(vec![NbtTag::Compound(entity)]));
            let mut chest = NbtCompound::new();
            chest.insert("id", "minecraft:chest");
            region.insert("TileEntities", NbtList::from(vec![NbtTag::Compound(chest)]));
            regions.insert(format!("region{index}"), region);
        }
        let mut root = NbtCompound::new();
        root.insert("Version", 6);
        root.insert("Metadata", NbtCompound::new());
        root.insert("Regions", regions);
        let mut bytes = Vec::new();
        quartz_nbt::io::write_nbt(
            &mut bytes,
            None,
            &root,
            quartz_nbt::io::Flavor::GzCompressed,
        )
        .unwrap();
        let limits = DecodeLimits {
            max_volume: 16,
            max_regions: 2,
            max_entities: 2,
            max_block_entities: 2,
            ..preview_limits()
        };
        let schematic = read_bounded(&bytes, &limits).unwrap();
        assert_eq!(schematic.total_volume(), 16);
        assert_eq!(
            schematic.default_region.entities.len()
                + schematic
                    .other_regions
                    .values()
                    .map(|region| region.entities.len())
                    .sum::<usize>(),
            2
        );
        assert_eq!(schematic.get_block_entities_as_list().len(), 2);
        for limits in [
            DecodeLimits {
                max_volume: 15,
                ..limits.clone()
            },
            DecodeLimits {
                max_regions: 1,
                ..limits.clone()
            },
            DecodeLimits {
                max_entities: 1,
                ..limits.clone()
            },
            DecodeLimits {
                max_block_entities: 1,
                ..limits.clone()
            },
        ] {
            assert!(read_bounded(&bytes, &limits).is_err());
        }
    }
}
