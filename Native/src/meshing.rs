//! Application-owned compact chunk scheduling over Nucleation's public data model.
//!
//! Block/entity adapters and chunk scheduling are adapted from Nucleation 0.10.14,
//! Copyright (c) 2025 Schem-at, under the MIT license. See
//! ../../ThirdParty/Nucleation-LICENSE.txt for the complete license.

use nucleation::meshing::{MeshConfig, MeshOutput, ResourcePackSource};
use nucleation::{BlockState, Entity, NbtValue, UniversalSchematic};
use schematic_mesher::{
    BlockPosition, BoundingBox, InputBlock, MesherConfig, ResourcePack, TextureAtlas,
};
use std::collections::{hash_map::Entry, HashMap};

mod atlas;
mod builder;

type ChunkCoord = (i32, i32, i32);
type IndexedBlock = (BlockPosition, u32);

struct CompactBlocks {
    chunks: Vec<(ChunkCoord, Vec<IndexedBlock>)>,
    palette: Vec<InputBlock>,
}

impl CompactBlocks {
    fn new(schematic: UniversalSchematic, chunk_size: i32) -> Result<Self, String> {
        if chunk_size <= 0 {
            return Err("The mesh chunk size must be positive.".into());
        }
        let UniversalSchematic {
            default_region,
            other_regions,
            ..
        } = schematic;
        let mut regions: Vec<_> = other_regions.into_iter().collect();
        regions.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        let mut chunks: HashMap<ChunkCoord, Vec<IndexedBlock>> = HashMap::new();
        let mut palette = Vec::new();
        let mut states = HashMap::new();
        // Preserve the flat source's additive geometry: overlapping blocks and
        // co-located entities all remain. For neighbor queries the mesher uses
        // the last co-located entry, so make the previously HashMap-dependent
        // order explicit: default region, other region keys, then each region's
        // volume scan and entities. Stable spatial sorting keeps those ties.
        for region in std::iter::once(default_region).chain(regions.into_iter().map(|(_, r)| r)) {
            // The public accessor clones a small palette once per region. Its
            // entries move into the deduplication map, never into each voxel.
            let region_palette = region.get_palette();
            let mut remap = Vec::with_capacity(region_palette.len());
            for state in region_palette {
                if is_air(&state.name) {
                    remap.push(None);
                    continue;
                }
                let index = match states.entry(state) {
                    Entry::Occupied(entry) => *entry.get(),
                    Entry::Vacant(entry) => {
                        let index = u32::try_from(palette.len())
                            .map_err(|_| "The schematic has too many block states.")?;
                        palette.push(block_state_to_input_block(entry.key()));
                        entry.insert(index);
                        index
                    }
                };
                remap.push(Some(index));
            }
            for (index, &state) in region.blocks.iter().enumerate() {
                let Some(&state) = remap.get(state) else {
                    return Err("The schematic contains an invalid palette index.".into());
                };
                if let Some(state) = state {
                    let (x, y, z) = region.index_to_coords(index);
                    let position = BlockPosition::new(x, y, z);
                    chunks
                        .entry(chunk_coord(position, chunk_size))
                        .or_default()
                        .push((position, state));
                }
            }
            for entity in &region.entities {
                let position = BlockPosition::new(
                    entity.position.0.floor() as i32,
                    entity.position.1.floor() as i32,
                    entity.position.2.floor() as i32,
                );
                let index = u32::try_from(palette.len())
                    .map_err(|_| "The schematic has too many block states.")?;
                palette.push(entity_to_input_block(entity));
                chunks
                    .entry(chunk_coord(position, chunk_size))
                    .or_default()
                    .push((position, index));
            }
            // Moving regions through this loop releases each dense block array
            // now, while the remaining regions and compact index still coexist.
        }
        drop(states);
        let mut chunks: Vec<_> = chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|(coord, _)| *coord);
        for (_, blocks) in &mut chunks {
            // Stable ties preserve region/entity order at duplicate positions.
            blocks.sort_by_key(|(pos, _)| (pos.y, pos.z, pos.x));
        }
        Ok(Self { chunks, palette })
    }

    fn atlas(
        &self,
        pack: &ResourcePackSource,
        config: &MesherConfig,
    ) -> Result<TextureAtlas, String> {
        let positioned: Vec<_> = self
            .palette
            .iter()
            .map(|block| block.name.starts_with("entity:") || atlas::position_dependent(block))
            .collect();
        let representatives = self
            .palette
            .iter()
            .zip(&positioned)
            .filter(|(_, positioned)| !**positioned)
            .map(|(block, _)| (BlockPosition::new(0, 0, 0), block));
        // Static palettes need no additional volume scan. Entities and generated
        // position-keyed textures must be discovered at every actual position.
        let chunks = if positioned.iter().any(|&value| value) {
            self.chunks.as_slice()
        } else {
            &[]
        };
        let actual_positions = chunks.iter().flat_map(|(_, blocks)| {
            let positioned = &positioned;
            blocks.iter().filter_map(move |&(pos, index)| {
                positioned[index as usize].then_some((pos, &self.palette[index as usize]))
            })
        });
        atlas::build(pack.pack(), config, representatives.chain(actual_positions))
            .map_err(|error| format!("Unable to prepare schematic textures: {error}"))
    }

    fn context(&self, coord: ChunkCoord, chunk_size: i32) -> Vec<(BlockPosition, &InputBlock)> {
        let (min, max) = chunk_bounds(coord, chunk_size);
        let capacity = self
            .chunks
            .binary_search_by_key(&coord, |(coord, _)| *coord)
            .map_or(0, |index| self.chunks[index].1.len());
        let mut context = Vec::with_capacity(capacity);
        // Face culling, corner AO, and liquid heights (including one block above
        // diagonals) all need the complete one-block Chebyshev halo.
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                for dz in -1i32..=1 {
                    let (Some(x), Some(y), Some(z)) = (
                        coord.0.checked_add(dx),
                        coord.1.checked_add(dy),
                        coord.2.checked_add(dz),
                    ) else {
                        continue;
                    };
                    let Ok(index) = self
                        .chunks
                        .binary_search_by_key(&(x, y, z), |(coord, _)| *coord)
                    else {
                        continue;
                    };
                    let blocks = &self.chunks[index].1;
                    let start = blocks.partition_point(|(pos, _)| i64::from(pos.y) < min[1] - 1);
                    let end = blocks.partition_point(|(pos, _)| i64::from(pos.y) < max[1] + 1);
                    for &(pos, state) in &blocks[start..end] {
                        if i64::from(pos.x) >= min[0] - 1
                            && i64::from(pos.x) < max[0] + 1
                            && i64::from(pos.z) >= min[2] - 1
                            && i64::from(pos.z) < max[2] + 1
                        {
                            context.push((pos, &self.palette[state as usize]));
                        }
                    }
                }
            }
        }
        context
    }
}

fn chunk_coord(pos: BlockPosition, size: i32) -> ChunkCoord {
    (
        pos.x.div_euclid(size),
        pos.y.div_euclid(size),
        pos.z.div_euclid(size),
    )
}

fn chunk_bounds(coord: ChunkCoord, size: i32) -> ([i64; 3], [i64; 3]) {
    let size = i64::from(size);
    let min = [coord.0, coord.1, coord.2].map(|value| i64::from(value) * size);
    (min, min.map(|value| value + size))
}

/// Retain compact source data through the final chunk for backward-looking halo
/// queries. Only the current output is owned by the caller; no mesh is retained.
pub(super) struct ChunkMeshes<'a> {
    source: CompactBlocks,
    index: usize,
    chunk_size: i32,
    pack: &'a ResourcePack,
    config: MesherConfig,
    atlas: TextureAtlas,
    atlas_only: Vec<bool>,
}

impl<'a> ChunkMeshes<'a> {
    pub(super) fn new(
        schematic: UniversalSchematic,
        pack: &'a ResourcePackSource,
        config: &MeshConfig,
        chunk_size: i32,
        current: impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        let source = CompactBlocks::new(schematic, chunk_size)?;
        current()?;
        let config = mesher_config(config);
        let atlas = source.atlas(pack, &config)?;
        let atlas_only = source
            .palette
            .iter()
            .map(|block| config.greedy_meshing && builder::atlas_only(pack.pack(), block))
            .collect();
        current()?;
        Ok(Self {
            source,
            index: 0,
            chunk_size,
            pack: pack.pack(),
            config,
            atlas,
            atlas_only,
        })
    }
}

impl Iterator for ChunkMeshes<'_> {
    type Item = Result<MeshOutput, String>;

    fn next(&mut self) -> Option<Self::Item> {
        let (coord, blocks) = self.source.chunks.get(self.index)?;
        self.index += 1;
        let (min, max) = chunk_bounds(*coord, self.chunk_size);
        let bounds = BoundingBox::new(min.map(|value| value as f32), max.map(|value| value as f32));
        // Context and output both borrow the palette. No full InputBlock map or
        // geometry is retained after this one chunk has been consumed.
        let context = self.source.context(*coord, self.chunk_size);
        Some(
            builder::mesh(
                self.pack,
                &self.config,
                &self.atlas,
                &self.source.palette,
                &self.atlas_only,
                blocks,
                &context,
                bounds,
            )
            .map(|mut mesh| {
                mesh.chunk_coord = Some(*coord);
                mesh
            }),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.source.chunks.len() - self.index;
        (remaining, Some(remaining))
    }
}

fn mesher_config(config: &MeshConfig) -> MesherConfig {
    // Map every public MeshConfig field explicitly. Remaining mesher settings
    // retain Nucleation 0.10.14's choices: padding=1, no air, no block/sky light,
    // sky level=15, particles enabled, and the default tint provider.
    let mut result = MesherConfig {
        cull_hidden_faces: config.cull_hidden_faces,
        ambient_occlusion: config.ambient_occlusion,
        ao_intensity: config.ao_intensity,
        atlas_max_size: config.atlas_max_size,
        cull_occluded_blocks: config.cull_occluded_blocks,
        greedy_meshing: config.greedy_meshing,
        ..MesherConfig::default()
    };
    if let Some(biome) = &config.biome {
        result = result.with_biome(biome);
    }
    result
}

fn is_air(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}

fn block_state_to_input_block(state: &BlockState) -> InputBlock {
    let mut input = InputBlock::new(state.name.to_string());
    // A bare id denotes its default state. Explicit schematic properties win.
    if let Some(facts) = nucleation::blockpedia::get_block(&state.name) {
        for (key, value) in facts.default_state {
            input.properties.insert(key.to_string(), value.to_string());
        }
    }
    for (key, value) in &state.properties {
        input.properties.insert(key.to_string(), value.to_string());
    }
    input
}

fn entity_to_input_block(entity: &Entity) -> InputBlock {
    let entity_id = entity.id.strip_prefix("minecraft:").unwrap_or(&entity.id);
    let mesher_id = match entity_id {
        "furnace_minecart"
        | "chest_minecart"
        | "tnt_minecart"
        | "hopper_minecart"
        | "spawner_minecart"
        | "command_block_minecart" => "minecart",
        id if id.ends_with("_chest_boat") || id.ends_with("_chest_raft") => "chest_boat",
        id if id.ends_with("_boat") || id.ends_with("_raft") => "boat",
        id => id,
    };
    let mut input = InputBlock::new(format!("entity:{mesher_id}"));
    if let Some(NbtValue::List(rotation)) = entity.nbt.get("Rotation") {
        if let Some(NbtValue::Float(yaw)) = rotation.first() {
            input
                .properties
                .insert("facing".into(), yaw_to_facing(*yaw).into());
        }
    }
    if let Some(NbtValue::Compound(item)) = entity.nbt.get("Item") {
        if let Some(NbtValue::String(id)) = item.get("id") {
            input.properties.insert("item".into(), id.clone());
        }
    }
    if matches!(entity.nbt.get("IsBaby"), Some(NbtValue::Byte(1)))
        || matches!(entity.nbt.get("Age"), Some(NbtValue::Int(age)) if *age < 0)
    {
        input.properties.insert("is_baby".into(), "true".into());
    }
    if let Some(NbtValue::Byte(color)) = entity.nbt.get("Color") {
        input
            .properties
            .insert("color".into(), dye_color_name(*color as u8).into());
    }
    for pose_key in [
        "RightArmPose",
        "LeftArmPose",
        "RightLegPose",
        "LeftLegPose",
        "HeadPose",
        "BodyPose",
    ] {
        if let Some(NbtValue::List(angles)) = entity.nbt.get(pose_key) {
            use std::fmt::Write;
            let mut pose = String::new();
            for angle in angles {
                if let NbtValue::Float(angle) = angle {
                    if !pose.is_empty() {
                        pose.push(',');
                    }
                    write!(&mut pose, "{angle}").expect("writing to a String cannot fail");
                }
            }
            if !pose.is_empty() {
                input.properties.insert(pose_key.into(), pose);
            }
        }
    }
    if let Some(NbtValue::List(items)) = entity.nbt.get("ArmorItems") {
        for (index, property) in ["boots", "leggings", "chestplate", "helmet"]
            .iter()
            .enumerate()
        {
            if let Some(NbtValue::Compound(item)) = items.get(index) {
                if let Some(NbtValue::String(id)) = item.get("id") {
                    input.properties.insert((*property).into(), id.clone());
                }
            }
        }
    }
    input
}

fn yaw_to_facing(yaw: f32) -> &'static str {
    let normalized = ((yaw % 360.0) + 360.0) % 360.0;
    if !(45.0..315.0).contains(&normalized) {
        "south"
    } else if (45.0..135.0).contains(&normalized) {
        "west"
    } else if (135.0..225.0).contains(&normalized) {
        "north"
    } else {
        "east"
    }
}

fn dye_color_name(color: u8) -> &'static str {
    match color {
        0 => "white",
        1 => "orange",
        2 => "magenta",
        3 => "light_blue",
        4 => "yellow",
        5 => "lime",
        6 => "pink",
        7 => "gray",
        8 => "light_gray",
        9 => "cyan",
        10 => "purple",
        11 => "blue",
        12 => "brown",
        13 => "green",
        14 => "red",
        15 => "black",
        _ => "white",
    }
}

#[cfg(test)]
mod tests;
