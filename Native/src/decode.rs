use nucleation::formats::limits::DecodeLimits;
use nucleation::formats::manager::get_manager;
use nucleation::UniversalSchematic;
use regex::Regex;

#[path = "structure_nbt.rs"]
mod structure_nbt;

// Compressed schematic bytes accepted from the desktop host.
const MAX_INPUT_BYTES: usize = 1_024 * 1_024 * 1_024;
// Inflated NBT bytes Nucleation may allocate while decoding.
const MAX_DECOMPRESSED_BYTES: usize = 1_024 * 1_024 * 1_024;
const MAX_AXIS_LENGTH: usize = 4_096;
// Cells Nucleation may decode per schematic; mirrors the renderer budget.
const MAX_VOLUME: usize = 536_870_912;
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
    preview_limits()
        .check_input(bytes)
        .map_err(|e| DecodeFailure::Limit(e.to_string()))?;
    let manager = get_manager();
    let guard = manager
        .lock()
        .map_err(|_| DecodeFailure::Format("the format registry is unavailable".to_string()))?;

    if let Ok((_, schematic)) = guard.read_bounded_with_format(bytes, &preview_limits()) {
        return Ok(schematic);
    }

    // A readable document that only trips on Nucleation's brace block-state
    // spelling gets one normalized retry.
    if let Some(normalized) = normalize_structure_snbt(bytes) {
        if let Ok((_, schematic)) = guard.read_bounded_with_format(&normalized, &preview_limits()) {
            return Ok(schematic);
        }
    }

    // The binary fallback parses once, including bounded decompression.
    if let Some(result) = structure_nbt::try_load(bytes) {
        return result;
    }
    Err(DecodeFailure::Format(
        "This file is not a readable Minecraft schematic, or it exceeds the preview limits.".into(),
    ))
}
