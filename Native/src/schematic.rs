// Sponge schematic decoding, adapted from Nucleation 0.10.14's
// src/formats/schematic.rs. Copyright (c) 2025 Schem-at, MIT licensed.
// The upstream license is retained in the application's third-party notices.
//
// BlockData stays packed until each value is written into the final Region.
// Only the small file-palette -> Region-palette mapping is materialized.

use nucleation::block_entity::BlockEntity;
use nucleation::formats::limits::DecodeLimits;
use nucleation::{BlockState, Entity, Region, UniversalSchematic};
use quartz_nbt::{NbtCompound, NbtList, NbtTag};

use super::bounded_nbt::gzip_root;

pub(super) fn read(data: &[u8], limits: &DecodeLimits) -> Result<UniversalSchematic, String> {
    let root = gzip_root(data, limits)?;
    let schem = root.get::<_, &NbtCompound>("Schematic").unwrap_or(&root);
    let version = schem.get::<_, i32>("Version").map_err(|e| e.to_string())?;
    let width = schem.get::<_, i16>("Width").map_err(|e| e.to_string())?;
    let height = schem.get::<_, i16>("Height").map_err(|e| e.to_string())?;
    let length = schem.get::<_, i16>("Length").map_err(|e| e.to_string())?;
    let volume = limits
        .check_dimensions((i64::from(width), i64::from(height), i64::from(length)))
        .map_err(|e| e.to_string())?;

    // Preserve the upstream layout dispatch, including its v3-shaped inputs
    // whose Version is not 2. Offset is likewise not applied by that reader.
    let blocks = if version == 2 {
        schem
    } else {
        schem
            .get::<_, &NbtCompound>("Blocks")
            .map_err(|e| e.to_string())?
    };
    let palette = blocks
        .get::<_, &NbtCompound>("Palette")
        .map_err(|e| e.to_string())?;
    if palette.is_empty() || palette.len() > limits.max_palette_entries {
        return Err("palette limit exceeded or palette is empty".into());
    }
    let palette_max = blocks
        .get::<_, i32>("PaletteMax")
        .unwrap_or(palette.len() as i32);
    if palette_max < 0 || palette_max as usize > limits.max_palette_entries {
        return Err("palette limit exceeded".into());
    }
    let mut max_id = 0;
    for value in palette.inner().values() {
        let NbtTag::Int(id) = value else {
            return Err("palette index is not an integer".into());
        };
        // Upstream allocates PaletteMax + 1 slots, so retain its acceptance of
        // an explicitly defined entry at PaletteMax, within the shared limit.
        if *id < 0 || *id > palette_max || *id as usize >= limits.max_palette_entries {
            return Err("palette index is outside the declared palette or limit".into());
        }
        max_id = max_id.max(*id as usize);
    }
    let block_entities = optional_list(blocks, "BlockEntities")?;
    let entities = optional_list(schem, "Entities")?;
    if block_entities.is_some_and(|values| values.len() > limits.max_block_entities) {
        return Err("block-entity limit exceeded".into());
    }
    if entities.is_some_and(|values| values.len() > limits.max_entities) {
        return Err("entity limit exceeded".into());
    }
    let block_data = blocks
        .get::<_, &Vec<i8>>("BlockData")
        .or_else(|_| blocks.get::<_, &Vec<i8>>("Data"))
        .map_err(|e| e.to_string())?;
    if block_data.len() < volume || block_data.len() / 5 > volume {
        return Err("block data length cannot encode the declared volume".into());
    }

    let mut schematic = UniversalSchematic::new("Unnamed".into());
    if let Ok(metadata) = schem.get::<_, &NbtCompound>("Metadata") {
        // Metadata's type is private upstream; its public fields preserve the
        // same conversion without reaching through Nucleation's module boundary.
        schematic.metadata.name = metadata.get::<_, &str>("Name").ok().map(str::to_owned);
        schematic.metadata.author = metadata.get::<_, &str>("Author").ok().map(str::to_owned);
        schematic.metadata.description = metadata
            .get::<_, &str>("Description")
            .ok()
            .map(str::to_owned);
        schematic.metadata.created = metadata
            .get::<_, i64>("TimeCreated")
            .ok()
            .map(|value| value as u64);
        schematic.metadata.modified = metadata
            .get::<_, i64>("TimeModified")
            .ok()
            .map(|value| value as u64);
        schematic.metadata.lm_version = metadata.get::<_, i32>("lm_version").ok();
        schematic.metadata.mc_version = metadata.get::<_, i32>("mc_version").ok();
        schematic.metadata.we_version = metadata.get::<_, i32>("we_version").ok();
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
            if let Ok(definitions) = serde_json::from_str(json) {
                schematic.definition_regions = definitions;
            }
        }
        schematic.metadata.cell_contract = metadata
            .get::<_, &str>("NucleationCellContract")
            .ok()
            .map(str::to_owned);
    }
    if schematic.metadata.name.is_none() {
        schematic.metadata.name = Some("Unnamed".into());
    }
    schematic.metadata.embedded_test = root
        .get::<_, &NbtCompound>("NucleationTest")
        .ok()
        .and_then(|test| test.get::<_, &str>("Spec").ok())
        .map(str::to_owned);
    let data_version = schem.get::<_, i32>("DataVersion").ok();
    schematic.metadata.mc_version = data_version.or(schematic.metadata.mc_version);
    schematic.metadata.source_data_version = data_version;

    let mut region = Region::try_new(
        "Main".into(),
        (0, 0, 0),
        (i32::from(width), i32::from(height), i32::from(length)),
    )?;
    let mut remap = vec![usize::MAX; max_id + 1];
    for (state, value) in palette.inner() {
        let NbtTag::Int(id) = value else {
            unreachable!("palette preflight")
        };
        let target = &mut remap[*id as usize];
        if *target != usize::MAX {
            return Err("duplicate palette index".into());
        }
        *target = region.get_or_insert_palette_by_state(&parse_block_state(state));
    }
    let mut remaining = block_data.as_slice();
    for index in 0..volume {
        let source = read_varint(&mut remaining)? as usize;
        let target = remap
            .get(source)
            .copied()
            .filter(|value| *value != usize::MAX)
            .ok_or_else(|| format!("block {index} refers to undefined palette index {source}"))?;
        // Region starts as air (index 0). Skipping it retains the initialized
        // count/bounds; all other writes use the public bookkeeping setter.
        if target != 0 {
            let (x, y, z) = region.index_to_coords(index);
            region.set_block_at_index_unchecked(target, x, y, z);
        }
    }
    if !remaining.is_empty() {
        return Err("block data exceeds the declared volume".into());
    }

    if let Some(values) = block_entities {
        for tag in values {
            if let NbtTag::Compound(compound) = tag {
                region.add_block_entity(parse_block_entity(compound)?);
            }
        }
    }
    if let Some(values) = entities {
        for tag in values {
            if let NbtTag::Compound(compound) = tag {
                region.add_entity(parse_entity(compound)?);
            }
        }
    }
    schematic.add_region(region);
    drop(root);
    super::validate_with_implicit_air(&schematic, limits)?;
    Ok(schematic)
}

fn optional_list<'a>(compound: &'a NbtCompound, key: &str) -> Result<Option<&'a NbtList>, String> {
    if compound.contains_key(key) {
        compound
            .get::<_, &NbtList>(key)
            .map(Some)
            .map_err(|e| e.to_string())
    } else {
        Ok(None)
    }
}

// Preserve the original Sponge parser's property order and permissive spelling.
fn parse_block_state(input: &str) -> BlockState {
    if let Some((name, properties)) = input.split_once('[') {
        BlockState {
            name: name.into(),
            properties: properties
                .trim_end_matches(']')
                .split(',')
                .filter_map(|property| {
                    let (key, value) = property.split_once('=')?;
                    Some((key.trim().into(), value.trim().into()))
                })
                .collect(),
        }
    } else {
        BlockState::new(input)
    }
}

fn read_varint(bytes: &mut &[i8]) -> Result<u32, String> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let Some((&byte, rest)) = bytes.split_first() else {
            return Err("truncated block-data VarInt".into());
        };
        *bytes = rest;
        let byte = byte as u8;
        if shift == 28 && byte & 0xf0 != 0 {
            return Err("block-data VarInt exceeds 32 bits".into());
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("block-data VarInt exceeds 32 bits".into())
}

fn parse_block_entity(compound: &NbtCompound) -> Result<BlockEntity, String> {
    let flattened;
    let source = if compound.contains_key("Data") {
        // Upstream gives Data precedence when flattening block entities.
        let mut flat = NbtCompound::new();
        for (key, value) in compound.inner() {
            if key != "Data" {
                flat.insert(key, value.clone());
            }
        }
        if let Ok(data) = compound.get::<_, &NbtCompound>("Data") {
            for (key, value) in data.inner() {
                flat.insert(key, value.clone());
            }
        }
        flattened = flat;
        &flattened
    } else {
        compound
    };
    // Public BlockEntity::from_nbt indexes the first three elements directly.
    if source
        .get::<_, &Vec<i32>>("Pos")
        .is_ok_and(|pos| pos.len() < 3)
    {
        return Err("block entity Pos must contain three coordinates".into());
    }
    Ok(BlockEntity::from_nbt(source))
}

fn parse_entity(compound: &NbtCompound) -> Result<Entity, String> {
    if let Ok(data) = compound.get::<_, &NbtCompound>("Data") {
        let mut merged = data.clone();
        if let Ok(id) = compound.get::<_, &str>("Id") {
            merged.insert("Id", id);
        }
        if let Ok(pos) = compound.get::<_, &NbtList>("Pos") {
            merged.insert("Pos", pos.clone());
        }
        Entity::from_nbt(&merged)
    } else {
        Entity::from_nbt(compound)
    }
}
