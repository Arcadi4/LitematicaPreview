use litematica_preview_native::{parts, Preview};
use serde::Serialize;
use std::mem::size_of_val;

#[cfg(not(target_endian = "little"))]
compile_error!("The LPV1 renderer protocol requires a little-endian target.");

const MAGIC: u32 = 0x3156_504c;
const TOO_LARGE: &str = "The preview is too large to transfer to the graphics device.";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    block_count: i64,
    block_entity_count: i64,
    triangle_count: u64,
    min: [f32; 3],
    max: [f32; 3],
    textures: Vec<TextureMetadata>,
    parts: Vec<PartMetadata>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextureMetadata {
    width: u32,
    height: u32,
    byte_length: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PartMetadata {
    vertex_count: u32,
    index_count: u32,
    texture_index: u32,
    alpha_mode: u32,
}

pub fn serialize(
    preview: &Preview,
    current: impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    current()?;
    let atlas = &preview.mesh.atlas;
    let mut textures = Vec::with_capacity(preview.textures.len() + 1);
    let mut body_length = atlas.pixels.len();
    textures.push(TextureMetadata {
        width: atlas.width,
        height: atlas.height,
        byte_length: atlas.pixels.len(),
    });
    for texture in &preview.textures {
        body_length = body_length
            .checked_add(texture.pixels.len())
            .ok_or(TOO_LARGE)?;
        textures.push(TextureMetadata {
            width: texture.width,
            height: texture.height,
            byte_length: texture.pixels.len(),
        });
    }
    let mut mesh_parts = Vec::with_capacity(preview.info.part_count as usize);
    for (part, texture_index, alpha_mode) in parts(&preview.mesh) {
        current()?;
        mesh_parts.push(PartMetadata {
            vertex_count: u32::try_from(part.positions.len()).map_err(|_| TOO_LARGE)?,
            index_count: u32::try_from(part.indices.len()).map_err(|_| TOO_LARGE)?,
            texture_index,
            alpha_mode,
        });
        for length in [
            size_of_val(part.positions.as_slice()),
            size_of_val(part.normals.as_slice()),
            size_of_val(part.uvs.as_slice()),
            size_of_val(part.colors.as_slice()),
            size_of_val(part.indices.as_slice()),
        ] {
            body_length = body_length.checked_add(length).ok_or(TOO_LARGE)?;
        }
    }
    let metadata = Metadata {
        block_count: preview.info.block_count,
        block_entity_count: preview.info.block_entity_count,
        triangle_count: preview.info.triangle_count,
        min: preview.info.min,
        max: preview.info.max,
        textures,
        parts: mesh_parts,
    };
    let json = serde_json::to_vec(&metadata)
        .map_err(|e| format!("Unable to describe the preview: {e}"))?;
    let json_length = u32::try_from(json.len()).map_err(|_| TOO_LARGE)?;
    let header_length = json.len().checked_add(11).ok_or(TOO_LARGE)? & !3;
    let payload_length = header_length.checked_add(body_length).ok_or(TOO_LARGE)?;
    current()?;
    // Reserve the final IPC payload once. Cast POD slices directly instead of
    // expanding geometry into JSON numbers or an intermediate flattened copy.
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(payload_length)
        .map_err(|_| "There is not enough memory to upload this preview.".to_string())?;
    payload.extend_from_slice(&MAGIC.to_le_bytes());
    payload.extend_from_slice(&json_length.to_le_bytes());
    payload.extend_from_slice(&json);
    payload.resize(header_length, 0);
    payload.extend_from_slice(&atlas.pixels);
    for texture in &preview.textures {
        current()?;
        payload.extend_from_slice(&texture.pixels);
    }
    for (part, _, _) in parts(&preview.mesh) {
        current()?;
        payload.extend_from_slice(bytemuck::cast_slice(&part.positions));
        payload.extend_from_slice(bytemuck::cast_slice(&part.normals));
        payload.extend_from_slice(bytemuck::cast_slice(&part.uvs));
        payload.extend_from_slice(bytemuck::cast_slice(&part.colors));
        payload.extend_from_slice(bytemuck::cast_slice(&part.indices));
    }
    current()?;
    Ok(payload)
}
