//! Owned schematic mesh buffers shared directly with the Tauri Rust host.

use nucleation::meshing::{MeshConfig, MeshLayer, MeshOutput, ResourcePackSource};
use schematic_mesher::BoundingBox;

mod decode;
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

pub fn load(data: &[u8], pack: &ResourcePackSource) -> Result<Preview, String> {
    if data.is_empty() || data.len() > 1_024 * 1_024 * 1_024 {
        return Err("Choose a nonempty schematic smaller than 1 GiB.".into());
    }
    let schematic = decode(data).map_err(|e| match e {
        DecodeFailure::Format(m) | DecodeFailure::Limit(m) => m,
    })?;
    let block_count = i64::from(schematic.total_blocks());
    if block_count == 0 || block_count > 33_554_432 {
        return Err("The schematic must contain between 1 and 33,554,432 blocks.".into());
    }
    let block_entity_count = schematic.get_block_entities_as_list().len() as i64;
    let mesh = schematic
        .to_mesh(pack, &mesh_config())
        .map_err(|e| format!("This schematic is too detailed to preview: {e}"))?;
    // Release the decoded block volume before retaining GPU input buffers.
    drop(schematic);
    prepare(mesh, block_count, block_entity_count)
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
    // Nucleation's total_triangles/is_empty omit greedy materials. Count all
    // parts, and derive visible bounds from vertices rather than region padding.
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
