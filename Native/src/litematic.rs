//! Single-storage Litematic import through Nucleation's public Region API.
//!
//! Format and metadata handling adapted from Nucleation 0.10.14,
//! Copyright (c) 2025 Schem-at, MIT licensed (see NOTICE).

use nucleation::block_entity::BlockEntity;
use nucleation::formats::limits::DecodeLimits;
use nucleation::BoundingBox;
use nucleation::{BlockState, Entity, Region, UniversalSchematic};
use quartz_nbt::{NbtCompound, NbtList, NbtTag};

use super::bounded_nbt;

struct PreparedRegion {
    name: String,
    position: (i32, i32, i32),
    size: (i32, i32, i32),
    volume: usize,
    bits: usize,
    palette: Vec<BlockState>,
    packed: Vec<i64>,
    nbt: NbtCompound,
}

pub(super) fn read(data: &[u8], limits: &DecodeLimits) -> Result<UniversalSchematic, String> {
    let mut root = bounded_nbt::gzip_root(data, limits)?;
    let mut schematic = UniversalSchematic::new("Unnamed".into());
    metadata(&root, &mut schematic)?;
    let Some(NbtTag::Compound(regions)) = root.inner_mut().shift_remove("Regions") else {
        return Err("missing Litematic Regions compound".into());
    };
    drop(root);
    // Match the original importer: the first serialized region names the default.
    if let Some(name) = regions.inner().keys().next() {
        schematic.default_region_name = name.clone();
    }
    let regions = preflight(regions, &schematic.default_region_name, limits)?;
    for prepared in regions {
        schematic.add_region(read_region(prepared)?);
    }
    super::validate_with_implicit_air(&schematic, limits)?;
    Ok(schematic)
}

// All regions, their aggregate budgets and every packed index are checked before
// the first volume-sized Region is allocated. Only small palettes are decoded here.
fn preflight(
    regions: NbtCompound,
    default_name: &str,
    limits: &DecodeLimits,
) -> Result<Vec<PreparedRegion>, String> {
    if regions.len() > limits.max_regions {
        return Err("region limit exceeded".into());
    }
    let mut prepared = Vec::with_capacity(regions.len());
    let mut total_volume = 0usize;
    let mut total_entities = 0usize;
    let mut total_block_entities = 0usize;
    let mut has_default = false;
    for (name, tag) in regions.into_inner() {
        let NbtTag::Compound(mut nbt) = tag else {
            continue;
        };
        let position = triple(&nbt, "Position")?;
        let size = triple(&nbt, "Size")?;
        let volume = limits
            .check_dimensions((
                i64::from(size.0).abs(),
                i64::from(size.1).abs(),
                i64::from(size.2).abs(),
            ))
            .map_err(|error| error.to_string())?;
        let bounds = BoundingBox::try_from_position_and_size(position, size)?;
        total_volume = total_volume
            .checked_add(volume)
            .ok_or("total volume overflow")?;
        if total_volume > limits.max_volume {
            return Err("total volume limit exceeded".into());
        }
        let Some(NbtTag::List(palette_nbt)) = nbt.inner_mut().shift_remove("BlockStatePalette")
        else {
            return Err("missing Litematic BlockStatePalette".into());
        };
        if palette_nbt.is_empty() || palette_nbt.len() > limits.max_palette_entries {
            return Err("empty palette or palette limit exceeded".into());
        }
        let mut palette = Vec::with_capacity(palette_nbt.len());
        for tag in palette_nbt.into_inner() {
            let NbtTag::Compound(state) = tag else {
                return Err("invalid palette state".into());
            };
            palette.push(BlockState::from_nbt(&state)?);
        }
        let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(2) as usize;
        let Some(NbtTag::LongArray(packed)) = nbt.inner_mut().shift_remove("BlockStates") else {
            return Err("missing Litematic BlockStates".into());
        };
        let total_bits = volume
            .checked_mul(bits)
            .ok_or("packed state size overflow")?;
        let required_longs = total_bits
            .checked_add(63)
            .ok_or("packed state size overflow")?
            / 64;
        if packed.len() != required_longs {
            return Err("packed state length does not match region volume".into());
        }
        for index in 0..volume {
            if packed_index(&packed, bits, index) >= palette.len() {
                return Err("packed block palette index out of range".into());
            }
        }
        if let Ok(entities) = nbt.get::<_, &NbtList>("Entities") {
            total_entities = total_entities
                .checked_add(entities.len())
                .ok_or("entity count overflow")?;
        }
        if let Ok(entities) = nbt.get::<_, &NbtList>("TileEntities") {
            total_block_entities = total_block_entities
                .checked_add(entities.len())
                .ok_or("block-entity count overflow")?;
            for tag in entities.iter() {
                if let NbtTag::Compound(entity) = tag {
                    // BlockEntity::from_nbt indexes Pos directly; validate its
                    // array and translated coordinates before calling that API.
                    let relative = block_entity_position(entity)?;
                    offset_position(relative, bounds.min)?;
                }
            }
        }
        if total_entities > limits.max_entities || total_block_entities > limits.max_block_entities
        {
            return Err("entity or block-entity limit exceeded".into());
        }
        has_default |= name == default_name;
        prepared.push(PreparedRegion {
            name,
            position,
            size,
            volume,
            bits,
            palette,
            packed,
            nbt,
        });
    }
    // Preserve the original empty/default-region behavior even when the first
    // Regions entry is not a compound and gets skipped by the importer.
    if !has_default {
        total_volume = total_volume.checked_add(1).ok_or("total volume overflow")?;
        if prepared.len() >= limits.max_regions || total_volume > limits.max_volume {
            return Err("default region exceeds region or volume limit".into());
        }
    }
    Ok(prepared)
}

#[inline]
fn packed_index(packed: &[i64], bits: usize, index: usize) -> usize {
    let bit = index * bits; // Preflight checked volume * bits and the long count.
    let word = bit / 64;
    let shift = bit % 64;
    let mut value = (packed[word] as u64) >> shift;
    if shift + bits > 64 {
        value |= (packed[word + 1] as u64) << (64 - shift);
    }
    (value & ((1u64 << bits) - 1)) as usize
}

fn read_region(prepared: PreparedRegion) -> Result<Region, String> {
    let PreparedRegion {
        name,
        position,
        size,
        volume,
        bits,
        palette,
        packed,
        nbt,
    } = prepared;
    let mut region = Region::try_new(name, position, size)?;
    let mapping: Vec<usize> = palette
        .iter()
        .map(|state| region.get_or_insert_palette_by_state(state))
        .collect();
    drop(palette);
    for index in 0..volume {
        let palette_index = mapping[packed_index(&packed, bits, index)];
        // The final dense allocation already contains ordinary air (index zero).
        if palette_index != 0 {
            let (x, y, z) = region.index_to_coords(index);
            region.set_block_at_index_unchecked(palette_index, x, y, z);
        }
    }
    drop(packed);
    let min_corner = region.get_bounding_box().min;
    for (key, tag) in nbt.into_inner() {
        match (key.as_str(), tag) {
            ("Entities", NbtTag::List(entities)) => {
                for tag in entities.into_inner() {
                    if let NbtTag::Compound(entity) = tag {
                        if let Ok(mut entity) = Entity::from_nbt(&entity) {
                            // Ordinary entities are relative to the signed region
                            // origin, not the normalized minimum corner.
                            entity.position.0 += f64::from(position.0);
                            entity.position.1 += f64::from(position.1);
                            entity.position.2 += f64::from(position.2);
                            region.entities.push(entity);
                        }
                    }
                }
            }
            ("TileEntities", NbtTag::List(entities)) => {
                for tag in entities.into_inner() {
                    if let NbtTag::Compound(entity) = tag {
                        let mut entity = BlockEntity::from_nbt(&entity);
                        entity.position = offset_position(entity.position, min_corner)?;
                        region.block_entities.insert(entity.position, entity);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(region)
}

fn triple(nbt: &NbtCompound, key: &str) -> Result<(i32, i32, i32), String> {
    let value = nbt
        .get::<_, &NbtCompound>(key)
        .map_err(|error| error.to_string())?;
    Ok((
        value
            .get::<_, i32>("x")
            .map_err(|error| error.to_string())?,
        value
            .get::<_, i32>("y")
            .map_err(|error| error.to_string())?,
        value
            .get::<_, i32>("z")
            .map_err(|error| error.to_string())?,
    ))
}

fn block_entity_position(nbt: &NbtCompound) -> Result<(i32, i32, i32), String> {
    if let Ok(position) = nbt.get::<_, &[i32]>("Pos") {
        if position.len() < 3 {
            return Err("truncated block entity position".into());
        }
        return Ok((position[0], position[1], position[2]));
    }
    let integer = |key| match nbt.inner().get(key) {
        Some(NbtTag::Byte(value)) => Some(i32::from(*value)),
        Some(NbtTag::Short(value)) => Some(i32::from(*value)),
        Some(NbtTag::Int(value)) => Some(*value),
        _ => None,
    };
    Ok(match (integer("x"), integer("y"), integer("z")) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => (0, 0, 0),
    })
}

fn offset_position(
    position: (i32, i32, i32),
    offset: (i32, i32, i32),
) -> Result<(i32, i32, i32), String> {
    Ok((
        position
            .0
            .checked_add(offset.0)
            .ok_or("block entity X overflow")?,
        position
            .1
            .checked_add(offset.1)
            .ok_or("block entity Y overflow")?,
        position
            .2
            .checked_add(offset.2)
            .ok_or("block entity Z overflow")?,
    ))
}

fn metadata(root: &NbtCompound, schematic: &mut UniversalSchematic) -> Result<(), String> {
    if let Ok(version) = root.get::<_, i32>("MinecraftDataVersion") {
        schematic.metadata.mc_version = Some(version);
        schematic.metadata.source_data_version = Some(version);
    }
    if let Ok(test) = root.get::<_, &NbtCompound>("NucleationTest") {
        schematic.metadata.embedded_test = test.get::<_, &str>("Spec").ok().map(String::from);
    }
    let metadata = root
        .get::<_, &NbtCompound>("Metadata")
        .map_err(|error| error.to_string())?;
    schematic.metadata.name = metadata.get::<_, &str>("Name").ok().map(String::from);
    schematic.metadata.description = metadata
        .get::<_, &str>("Description")
        .ok()
        .map(String::from);
    schematic.metadata.author = metadata.get::<_, &str>("Author").ok().map(String::from);
    schematic.metadata.created = metadata
        .get::<_, i64>("TimeCreated")
        .ok()
        .map(|value| value as u64);
    schematic.metadata.modified = metadata
        .get::<_, i64>("TimeModified")
        .ok()
        .map(|value| value as u64);
    schematic.metadata.provenance = metadata
        .get::<_, &str>("NucleationProvenance")
        .ok()
        .and_then(|json| nucleation::SchematicProvenance::from_json(json).ok());
    schematic.metadata.transformation_history = metadata
        .get::<_, &str>("NucleationTransformationHistory")
        .ok()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    if let Ok(json) = metadata.get::<_, &str>("NucleationDefinitions") {
        if let Ok(regions) = serde_json::from_str(json) {
            schematic.definition_regions = regions;
        }
    }
    Ok(())
}
