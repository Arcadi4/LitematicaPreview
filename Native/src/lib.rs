//! Owned schematic mesh buffers shared directly with the Tauri Rust host.

use nucleation::meshing::{MeshConfig, MeshLayer, MeshOutput, ResourcePackSource};
use schematic_mesher::BoundingBox;

mod decode;
mod meshing;
pub use decode::{decode, DecodeFailure};

pub struct Preview {
    pub mesh: MeshOutput,
    pub textures: Vec<Texture>,
    pub info: PreviewInfo,
}

pub struct Texture {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Default)]
pub struct PreviewInfo {
    pub block_count: i64,
    pub block_entity_count: i64,
    pub triangle_count: u64,
    pub part_count: u32,
    pub texture_count: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

pub fn mesh_config() -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(2_048)
}

// One iterator defines both transport and accounting. Greedy geometry has
// separate, repeating textures and must never use atlas UVs.
pub fn parts(mesh: &MeshOutput) -> impl Iterator<Item = (&MeshLayer, u32, u32)> {
    [
        (&mesh.opaque, 0, 0),
        (&mesh.cutout, 0, 1),
        (&mesh.transparent, 0, 2),
    ]
    .into_iter()
    .chain(mesh.greedy_materials.iter().enumerate().flat_map(|(i, m)| {
        [
            (&m.opaque, i as u32 + 1, 0),
            (&m.transparent, i as u32 + 1, 2),
        ]
    }))
    .filter(|(layer, _, _)| !layer.indices.is_empty())
}

/// Generate and synchronously hand off one spatial chunk at a time. The callback
/// owns that chunk; callers must release it before accepting the next one.
pub fn load_chunks(
    data: &[u8],
    pack: &ResourcePackSource,
    mut consume: impl FnMut(Preview) -> Result<(), String>,
    current: impl Fn() -> Result<(), String>,
) -> Result<PreviewInfo, String> {
    current()?;
    if data.is_empty() || data.len() > 1_024 * 1_024 * 1_024 {
        return Err("Choose a nonempty schematic smaller than 1 GiB.".into());
    }
    let schematic = decode(data).map_err(|e| match e {
        DecodeFailure::Format(m) | DecodeFailure::Limit(m) => m,
    })?;
    current()?;
    let block_count = i64::from(schematic.total_blocks());
    if block_count == 0 || block_count > 33_554_432 {
        return Err("The schematic must contain between 1 and 33,554,432 blocks.".into());
    }
    let block_entity_count = std::iter::once(&schematic.default_region)
        .chain(schematic.other_regions.values())
        .map(|region| region.block_entities.len() as i64)
        .sum();
    let mut chunks = meshing::ChunkMeshes::new(schematic, pack, &mesh_config(), 64, &current)?;
    current()?;
    let mut info = PreviewInfo {
        block_count,
        block_entity_count,
        texture_count: 1,
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
        ..PreviewInfo::default()
    };
    loop {
        current()?;
        let Some(mesh) = chunks.next() else {
            break;
        };
        let mesh = mesh.map_err(|e| format!("This schematic is too detailed to preview: {e}"))?;
        current()?;
        if parts(&mesh).next().is_none() {
            continue;
        }
        let preview = prepare(mesh, block_count, block_entity_count)?;
        info.triangle_count = info
            .triangle_count
            .checked_add(preview.info.triangle_count)
            .ok_or("The schematic has too many triangles.")?;
        info.part_count = info
            .part_count
            .checked_add(preview.info.part_count)
            .ok_or("The schematic has too many mesh parts.")?;
        info.texture_count = info
            .texture_count
            .checked_add(preview.info.texture_count - 1)
            .ok_or("The schematic has too many textures.")?;
        for axis in 0..3 {
            info.min[axis] = info.min[axis].min(preview.info.min[axis]);
            info.max[axis] = info.max[axis].max(preview.info.max[axis]);
        }
        consume(preview)?;
        current()?;
    }
    if info.part_count == 0 {
        return Err("The schematic contains no visible geometry.".into());
    }
    Ok(info)
}

pub fn prepare(
    mesh: MeshOutput,
    block_count: i64,
    block_entity_count: i64,
) -> Result<Preview, String> {
    let mut triangle_count = 0u64;
    let mut part_count = 0u32;
    for (layer, _, _) in parts(&mesh) {
        let n = layer.positions.len();
        if n > i32::MAX as usize
            || layer.indices.len() > i32::MAX as usize
            || layer.normals.len() != n
            || layer.uvs.len() != n
            || layer.colors.len() != n
            || layer.indices.len() % 3 != 0
            || layer.indices.iter().any(|&i| i as usize >= n)
        {
            return Err("The mesher produced invalid or oversized geometry.".into());
        }
        triangle_count += (layer.indices.len() / 3) as u64;
        part_count += 1;
    }
    // Include greedy materials and derive visible bounds from emitted vertices.
    let bounds =
        BoundingBox::from_points(parts(&mesh).flat_map(|(p, _, _)| p.positions.iter().copied()))
            .ok_or("The schematic contains no visible geometry.")?;
    if bounds.min.iter().chain(&bounds.max).any(|v| !v.is_finite()) {
        return Err("The mesher produced invalid bounds.".into());
    }
    let mut textures = Vec::with_capacity(mesh.greedy_materials.len());
    for material in &mesh.greedy_materials {
        let image =
            image::load_from_memory_with_format(&material.texture_png, image::ImageFormat::Png)
                .map_err(|e| format!("Unable to read a block texture: {e}"))?
                .into_rgba8();
        let (width, height) = image.dimensions();
        textures.push(Texture {
            pixels: image.into_raw(),
            width,
            height,
        });
    }
    let info = PreviewInfo {
        block_count,
        block_entity_count,
        triangle_count,
        part_count,
        texture_count: textures.len() as u32 + 1,
        min: bounds.min,
        max: bounds.max,
    };
    Ok(Preview {
        mesh,
        textures,
        info,
    })
}
