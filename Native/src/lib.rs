//! Owned schematic mesh buffers shared directly with the Tauri Rust host.

use nucleation::meshing::{MeshConfig, MeshLayer, MeshOutput, ResourcePackSource};
use schematic_mesher::BoundingBox;

mod decode;
mod meshing;
pub use decode::{decode, DecodeFailure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewOptions {
    pub memory_limit_mb: Option<u16>,
    pub chunk_size: Option<u16>,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            memory_limit_mb: None,
            chunk_size: Some(64),
        }
    }
}

impl PreviewOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .memory_limit_mb
            .is_some_and(|limit| !(2048..=8192).contains(&limit))
        {
            return Err("The memory limit must be an integer from 2048 to 8192 MB.".into());
        }
        if self
            .chunk_size
            .is_some_and(|size| !matches!(size, 16 | 32 | 64 | 128 | 256))
        {
            return Err("The chunk size must be 16, 32, 64, 128 or 256 blocks.".into());
        }
        Ok(())
    }
}

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

/// Generate and synchronously hand off each configured spatial chunk, or one
/// complete geometry group when chunk separation is disabled. The caller owns
/// each result and should release it before accepting the next one.
/// Memory enforcement belongs to the isolated host process. Some formats still
/// decode densely, and an unseparated mesh can exhaust memory without that cap.
pub fn load_chunks(
    data: &[u8],
    pack: &ResourcePackSource,
    options: PreviewOptions,
    mut consume: impl FnMut(Preview) -> Result<(), String>,
    current: impl Fn() -> Result<(), String>,
) -> Result<PreviewInfo, String> {
    options.validate()?;
    current()?;
    if data.is_empty() {
        return Err("Choose a nonempty schematic.".into());
    }
    let chunk_size = options.chunk_size.map(i32::from);
    let source = decode::decode_preview(data, chunk_size, &current)?;
    current()?;
    let block_count = source.block_count();
    let block_entity_count = source.block_entity_count();
    if block_count == 0 {
        return Err("The schematic must contain at least one block.".into());
    }
    let mut chunks =
        meshing::ChunkMeshes::from_source(source, pack, &mesh_config(), chunk_size, &current)?;
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
