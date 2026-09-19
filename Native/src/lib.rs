//! In-process mesh buffers for the C# renderer. The owning preview outlives
//! all borrowed views; the host frees it immediately after GPU upload.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::{ptr, slice};

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

#[repr(C)]
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

#[repr(C)]
#[derive(Default)]
pub struct PartView {
    pub positions: *const f32,
    pub normals: *const f32,
    pub uvs: *const f32,
    pub colors: *const f32,
    pub indices: *const u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub texture_index: u32,
    // 0 opaque, 1 alpha test, 2 alpha blend. Atlas clamps; greedy tiles repeat.
    pub alpha_mode: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct TextureView {
    pub pixels: *const u8,
    pub byte_count: usize,
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct ErrorBuffer {
    pub data: *mut u8,
    pub len: usize,
}

pub fn mesh_config() -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(2_048)
}

// One iterator defines what both the ABI and accounting expose. Greedy
// geometry has separate, repeating textures and must never use atlas UVs.
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

// Status 0 succeeds; any other value has an owned UTF-8 error. Callers pass
// valid pointer/length pairs. Output pointers are always cleared on entry.
unsafe fn guarded(error: *mut ErrorBuffer, f: impl FnOnce() -> Result<(), String>) -> i32 {
    if !error.is_null() {
        *error = ErrorBuffer::default();
    }
    let result = catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        Err("The schematic is too detailed to preview, or its data is invalid.".into())
    });
    match result {
        Ok(()) => 0,
        Err(message) => {
            if !error.is_null() {
                let bytes = message.into_bytes().into_boxed_slice();
                *error = ErrorBuffer {
                    len: bytes.len(),
                    data: Box::into_raw(bytes) as *mut u8,
                };
            }
            1
        }
    }
}

/// # Safety
/// data points to len readable bytes; out/error are writable. Free the pack
/// once, after its last load call. The host serializes calls sharing a pack.
#[no_mangle]
pub unsafe extern "C" fn lp_pack_open(
    data: *const u8,
    len: usize,
    out: *mut *mut ResourcePackSource,
    error: *mut ErrorBuffer,
) -> i32 {
    if !out.is_null() {
        *out = ptr::null_mut();
    }
    guarded(error, || {
        if data.is_null() || out.is_null() {
            return Err("Missing resource pack.".into());
        }
        let pack = ResourcePackSource::from_bytes(slice::from_raw_parts(data, len))
            .map_err(|e| format!("The bundled block resources are invalid: {e}"))?;
        *out = Box::into_raw(Box::new(pack));
        Ok(())
    })
}

/// # Safety
/// pack must be null or an unfreed pack from lp_pack_open, not in use.
#[no_mangle]
pub unsafe extern "C" fn lp_pack_free(pack: *mut ResourcePackSource) {
    if !pack.is_null() {
        drop(Box::from_raw(pack));
    }
}

/// # Safety
/// pack is live, data points to len bytes, out/error are writable. Free the
/// returned preview only after all borrowed views and GPU uploads finish.
#[no_mangle]
pub unsafe extern "C" fn lp_preview_load(
    pack: *const ResourcePackSource,
    data: *const u8,
    len: usize,
    out: *mut *mut Preview,
    error: *mut ErrorBuffer,
) -> i32 {
    if !out.is_null() {
        *out = ptr::null_mut();
    }
    guarded(error, || {
        if pack.is_null() || data.is_null() || out.is_null() {
            return Err("Missing schematic data.".into());
        }
        let preview = load(slice::from_raw_parts(data, len), &*pack)?;
        *out = Box::into_raw(Box::new(preview));
        Ok(())
    })
}

/// # Safety
/// preview must be null or an unfreed owning preview, no borrowed view in use.
#[no_mangle]
pub unsafe extern "C" fn lp_preview_free(preview: *mut Preview) {
    if !preview.is_null() {
        drop(Box::from_raw(preview));
    }
}

/// # Safety
/// preview is live and out is writable.
#[no_mangle]
pub unsafe extern "C" fn lp_preview_info(preview: *const Preview, out: *mut PreviewInfo) -> i32 {
    if preview.is_null() || out.is_null() {
        return 1;
    }
    *out = (*preview).info;
    0
}

/// # Safety
/// preview is live; out is writable. Its pointers borrow from preview.
#[no_mangle]
pub unsafe extern "C" fn lp_preview_part(
    preview: *const Preview,
    index: u32,
    out: *mut PartView,
) -> i32 {
    if preview.is_null() || out.is_null() {
        return 1;
    }
    let Some((layer, texture_index, alpha_mode)) = parts(&(*preview).mesh).nth(index as usize)
    else {
        return 1;
    };
    *out = PartView {
        positions: layer.positions.as_ptr().cast(),
        normals: layer.normals.as_ptr().cast(),
        uvs: layer.uvs.as_ptr().cast(),
        colors: layer.colors.as_ptr().cast(),
        indices: layer.indices.as_ptr(),
        vertex_count: layer.positions.len() as u32,
        index_count: layer.indices.len() as u32,
        texture_index,
        alpha_mode,
    };
    0
}

/// # Safety
/// preview is live; out is writable. Pixel bytes borrow from preview.
#[no_mangle]
pub unsafe extern "C" fn lp_preview_texture(
    preview: *const Preview,
    index: u32,
    out: *mut TextureView,
) -> i32 {
    if preview.is_null() || out.is_null() {
        return 1;
    }
    let p = &*preview;
    let (pixels, width, height) = if index == 0 {
        (
            &p.mesh.atlas.pixels,
            p.mesh.atlas.width,
            p.mesh.atlas.height,
        )
    } else if let Some(t) = p.textures.get(index as usize - 1) {
        (&t.pixels, t.width, t.height)
    } else {
        return 1;
    };
    *out = TextureView {
        pixels: pixels.as_ptr(),
        byte_count: pixels.len(),
        width,
        height,
    };
    0
}

/// # Safety
/// data/len must be exactly an unfreed error allocation returned by this ABI.
#[no_mangle]
pub unsafe extern "C" fn lp_error_free(data: *mut u8, len: usize) {
    if !data.is_null() {
        drop(Box::from_raw(ptr::slice_from_raw_parts_mut(data, len)));
    }
}
