use std::io::{self, Read, Write};

use litematica_preview_core::{MeshBatch, MeshPart, PreviewInfo, Texture};

use crate::preview::Metadata;

const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024 * 1024;
const MAX_DESCRIPTORS: usize = 65_536;

pub enum Event {
    Progress(&'static str, u64, u64),
    Prepared([f32; 3], [f32; 3]),
    Batch(MeshBatch),
    Finished(Result<Metadata, String>),
}

pub fn progress(mut output: impl Write, phase: &str, completed: u64, total: u64) -> io::Result<()> {
    let phase = match phase {
        "decode" => 0,
        "mesh" => 1,
        "upload" => 2,
        _ => return Err(invalid("Unknown decoder progress phase")),
    };
    output.write_all(&[1, phase])?;
    output.write_all(&completed.to_le_bytes())?;
    output.write_all(&total.to_le_bytes())?;
    output.flush()
}

pub fn prepared(mut output: impl Write, min: [f32; 3], max: [f32; 3]) -> io::Result<()> {
    output.write_all(&[2])?;
    output.vec3(min)?;
    output.vec3(max)?;
    output.flush()
}

pub fn batch(mut output: impl Write, batch: &MeshBatch) -> io::Result<()> {
    validate_counts(
        batch.geometry.len(),
        batch.parts.len(),
        batch.textures.len(),
    )?;
    if !matches!(batch.vertex_stride, 24 | 28) {
        return Err(invalid("The decoder produced an unsupported vertex layout"));
    }
    let mut budget = MAX_BATCH_BYTES;
    charge(&mut budget, batch.geometry.len())?;
    charge(
        &mut budget,
        batch.parts.len() * std::mem::size_of::<MeshPart>(),
    )?;
    charge(
        &mut budget,
        batch.textures.len() * std::mem::size_of::<Texture>(),
    )?;
    for texture in &batch.textures {
        if texture_bytes(texture.width, texture.height)? != texture.pixels.len() {
            return Err(invalid("Invalid decoder texture length"));
        }
        charge(&mut budget, texture.pixels.len())?;
    }
    output.write_all(&[3])?;
    output.write_all(&(batch.geometry.len() as u64).to_le_bytes())?;
    output.write_all(&(batch.parts.len() as u32).to_le_bytes())?;
    output.write_all(&(batch.textures.len() as u32).to_le_bytes())?;
    output.write_all(&batch.vertex_stride.to_le_bytes())?;
    output.vec3(batch.origin)?;
    output.vec3(batch.extent)?;
    output.vec3(batch.source_min)?;
    output.vec3(batch.source_max)?;
    match batch.chunk_coord {
        Some((x, y, z)) => {
            output.write_all(&[1])?;
            for value in [x, y, z] {
                output.write_all(&value.to_le_bytes())?;
            }
        }
        None => output.write_all(&[0])?,
    }
    output.write_all(&batch.vertex_count.to_le_bytes())?;
    output.write_all(&batch.index_count.to_le_bytes())?;
    output.info(&batch.info)?;
    for part in &batch.parts {
        output.write_all(&(part.vertex_offset as u64).to_le_bytes())?;
        output.write_all(&(part.index_offset as u64).to_le_bytes())?;
        for value in [
            part.vertex_count,
            part.index_count,
            part.index_size,
            part.alpha_mode,
            part.texture_index,
        ] {
            output.write_all(&value.to_le_bytes())?;
        }
        output.vec3(part.min)?;
        output.vec3(part.max)?;
    }
    output.write_all(&batch.geometry)?;
    for texture in &batch.textures {
        for value in [texture.index, texture.width, texture.height] {
            output.write_all(&value.to_le_bytes())?;
        }
        output.write_all(&[u8::from(texture.repeat)])?;
        output.write_all(&texture.pixels)?;
    }
    output.flush()
}

pub fn finish(mut output: impl Write, result: Result<Metadata, String>) -> io::Result<()> {
    match result {
        Ok(metadata) => {
            let bytes = serde_json::to_vec(&metadata).map_err(io::Error::other)?;
            output.write_all(&[4])?;
            write_frame(&mut output, &bytes, MAX_METADATA_BYTES)?;
        }
        Err(mut error) => {
            let mut end = error.len().min(MAX_METADATA_BYTES);
            while !error.is_char_boundary(end) {
                end -= 1;
            }
            error.truncate(end);
            output.write_all(&[5])?;
            write_frame(&mut output, error.as_bytes(), MAX_METADATA_BYTES)?;
        }
    }
    output.flush()
}

pub fn receive(mut input: impl Read) -> io::Result<Event> {
    match input.u8()? {
        1 => {
            let phase = match input.u8()? {
                0 => "decode",
                1 => "mesh",
                2 => "upload",
                _ => return Err(invalid("Invalid decoder progress phase")),
            };
            Ok(Event::Progress(phase, input.u64()?, input.u64()?))
        }
        2 => Ok(Event::Prepared(input.vec3()?, input.vec3()?)),
        3 => receive_batch(&mut input).map(Event::Batch),
        4 => {
            let bytes = read_frame(&mut input, MAX_METADATA_BYTES)?;
            let metadata = serde_json::from_slice(&bytes).map_err(invalid)?;
            Ok(Event::Finished(Ok(metadata)))
        }
        5 => {
            let bytes = read_frame(&mut input, MAX_METADATA_BYTES)?;
            Ok(Event::Finished(Err(
                String::from_utf8(bytes).map_err(invalid)?
            )))
        }
        _ => Err(invalid("Invalid decoder event")),
    }
}

fn receive_batch(input: &mut impl Read) -> io::Result<MeshBatch> {
    let geometry_len = usize::try_from(input.u64()?).map_err(invalid)?;
    let part_count = input.u32()? as usize;
    let texture_count = input.u32()? as usize;
    validate_counts(geometry_len, part_count, texture_count)?;
    let vertex_stride = input.u32()?;
    if !matches!(vertex_stride, 24 | 28) || geometry_len % 4 != 0 {
        return Err(invalid("Invalid decoder geometry layout"));
    }
    let mut budget = MAX_BATCH_BYTES;
    charge(&mut budget, geometry_len)?;
    charge(&mut budget, part_count * std::mem::size_of::<MeshPart>())?;
    charge(&mut budget, texture_count * std::mem::size_of::<Texture>())?;
    let origin = input.vec3()?;
    let extent = input.vec3()?;
    let source_min = input.vec3()?;
    let source_max = input.vec3()?;
    let chunk_coord = match input.u8()? {
        0 => None,
        1 => Some((input.i32()?, input.i32()?, input.i32()?)),
        _ => return Err(invalid("Invalid decoder chunk coordinates")),
    };
    let vertex_count = input.u32()?;
    let index_count = input.u32()?;
    let info = input.info()?;
    let mut parts = reserved(part_count)?;
    for _ in 0..part_count {
        let part = MeshPart {
            vertex_offset: usize::try_from(input.u64()?).map_err(invalid)?,
            index_offset: usize::try_from(input.u64()?).map_err(invalid)?,
            vertex_count: input.u32()?,
            index_count: input.u32()?,
            index_size: input.u32()?,
            alpha_mode: input.u32()?,
            texture_index: input.u32()?,
            min: input.vec3()?,
            max: input.vec3()?,
        };
        let vertex_end = (part.vertex_offset as u64)
            .checked_add(part.vertex_count as u64 * vertex_stride as u64);
        let index_end = (part.index_offset as u64)
            .checked_add(part.index_count as u64 * part.index_size as u64);
        if !matches!(part.index_size, 2 | 4)
            || part.alpha_mode > 2
            || part.vertex_count == 0
            || part.index_count == 0
            || part.vertex_offset % 4 != 0
            || part.index_offset % 4 != 0
            || vertex_end.is_none_or(|end| end > part.index_offset as u64)
            || index_end.is_none_or(|end| end > geometry_len as u64)
        {
            return Err(invalid("Decoder part exceeds its geometry"));
        }
        parts.push(part);
    }
    let geometry = read_bytes(input, geometry_len)?;
    let mut textures = reserved(texture_count)?;
    for _ in 0..texture_count {
        let index = input.u32()?;
        let width = input.u32()?;
        let height = input.u32()?;
        let repeat = match input.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("Invalid decoder texture sampler")),
        };
        let length = texture_bytes(width, height)?;
        charge(&mut budget, length)?;
        textures.push(Texture {
            index,
            width,
            height,
            repeat,
            pixels: read_bytes(input, length)?,
        });
    }
    Ok(MeshBatch {
        geometry,
        vertex_stride,
        parts,
        textures,
        origin,
        extent,
        source_min,
        source_max,
        chunk_coord,
        vertex_count,
        index_count,
        info,
    })
}

fn validate_counts(geometry: usize, parts: usize, textures: usize) -> io::Result<()> {
    if geometry > MAX_BATCH_BYTES || parts > MAX_DESCRIPTORS || textures > MAX_DESCRIPTORS {
        return Err(invalid("The decoder batch exceeds the transfer limit"));
    }
    Ok(())
}

fn texture_bytes(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 {
        return Err(invalid("Invalid decoder texture dimensions"));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= MAX_BATCH_BYTES)
        .ok_or_else(|| invalid("The decoder texture exceeds the transfer limit"))
}

fn charge(budget: &mut usize, length: usize) -> io::Result<()> {
    *budget = budget
        .checked_sub(length)
        .ok_or_else(|| invalid("The decoder batch exceeds the transfer limit"))?;
    Ok(())
}

fn reserved<T>(length: usize) -> io::Result<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| io::Error::other("Not enough memory to receive the preview"))?;
    Ok(values)
}

fn read_bytes(input: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
    let mut bytes = reserved(length)?;
    bytes.resize(length, 0);
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn read_frame(input: &mut impl Read, maximum: usize) -> io::Result<Vec<u8>> {
    let length = input.u32()? as usize;
    if length > maximum {
        return Err(invalid("The decoder packet exceeds the transfer limit"));
    }
    read_bytes(input, length)
}

pub fn write_frame(output: &mut impl Write, bytes: &[u8], maximum: usize) -> io::Result<()> {
    if bytes.len() > maximum {
        return Err(invalid("The decoder packet exceeds the transfer limit"));
    }
    let length = u32::try_from(bytes.len()).map_err(invalid)?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(bytes)
}

fn invalid(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

trait ReadWire: Read {
    fn fixed<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.fixed::<1>()?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }
    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.fixed()?))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }
    fn vec3(&mut self) -> io::Result<[f32; 3]> {
        let mut values = [0.0; 3];
        for value in &mut values {
            *value = f32::from_le_bytes(self.fixed()?);
            if !value.is_finite() {
                return Err(invalid("Non-finite decoder bounds"));
            }
        }
        Ok(values)
    }
    fn info(&mut self) -> io::Result<PreviewInfo> {
        Ok(PreviewInfo {
            block_count: i64::from_le_bytes(self.fixed()?),
            block_entity_count: i64::from_le_bytes(self.fixed()?),
            triangle_count: self.u64()?,
            part_count: self.u32()?,
            texture_count: self.u32()?,
            min: self.vec3()?,
            max: self.vec3()?,
        })
    }
}
impl<T: Read + ?Sized> ReadWire for T {}

trait WriteWire: Write {
    fn vec3(&mut self, values: [f32; 3]) -> io::Result<()> {
        for value in values {
            self.write_all(&value.to_le_bytes())?;
        }
        Ok(())
    }
    fn info(&mut self, info: &PreviewInfo) -> io::Result<()> {
        self.write_all(&info.block_count.to_le_bytes())?;
        self.write_all(&info.block_entity_count.to_le_bytes())?;
        self.write_all(&info.triangle_count.to_le_bytes())?;
        self.write_all(&info.part_count.to_le_bytes())?;
        self.write_all(&info.texture_count.to_le_bytes())?;
        self.vec3(info.min)?;
        self.vec3(info.max)
    }
}
impl<T: Write + ?Sized> WriteWire for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> MeshBatch {
        let mut geometry = vec![0; 80];
        geometry[72..78].copy_from_slice(&[0, 0, 1, 0, 2, 0]);
        MeshBatch {
            geometry,
            vertex_stride: 24,
            parts: vec![MeshPart {
                vertex_offset: 0,
                index_offset: 72,
                vertex_count: 3,
                index_count: 3,
                index_size: 2,
                alpha_mode: 2,
                texture_index: 7,
                min: [-1.0, 0.0, 2.0],
                max: [1.0, 3.0, 2.0],
            }],
            textures: vec![Texture {
                index: 7,
                pixels: vec![12, 34, 56, 78],
                width: 1,
                height: 1,
                repeat: true,
            }],
            origin: [-1.0, 0.0, 2.0],
            extent: [2.0, 3.0, 0.0],
            source_min: [-16.0; 3],
            source_max: [16.0; 3],
            chunk_coord: Some((-1, 0, 1)),
            vertex_count: 3,
            index_count: 3,
            info: PreviewInfo {
                triangle_count: 1,
                part_count: 1,
                texture_count: 8,
                ..Default::default()
            },
        }
    }

    #[test]
    fn raw_transfer_preserves_draw_ranges_materials_and_texture_pixels() {
        for stride in [24, 28] {
            let mut expected = triangle();
            let index_offset = stride as usize * 3;
            expected.vertex_stride = stride;
            expected.geometry = vec![0; (index_offset + 6).next_multiple_of(4)];
            expected.geometry[index_offset..index_offset + 6].copy_from_slice(&[0, 0, 1, 0, 2, 0]);
            expected.parts[0].index_offset = index_offset;
            let mut bytes = Vec::new();
            batch(&mut bytes, &expected).unwrap();
            let Event::Batch(actual) = receive(bytes.as_slice()).unwrap() else {
                panic!("expected geometry");
            };
            assert_eq!(actual.geometry, expected.geometry);
            assert_eq!(actual.vertex_stride, expected.vertex_stride);
            assert_eq!(actual.parts[0].index_offset, expected.parts[0].index_offset);
            assert_eq!(actual.parts[0].index_size, 2);
            assert_eq!(actual.parts[0].alpha_mode, 2);
            assert_eq!(actual.parts[0].texture_index, actual.textures[0].index);
            assert_eq!(actual.parts[0].min, expected.parts[0].min);
            assert_eq!(actual.parts[0].max, expected.parts[0].max);
            assert_eq!(actual.textures[0].pixels, [12, 34, 56, 78]);
            assert!(actual.textures[0].repeat);
            assert_eq!(actual.chunk_coord, Some((-1, 0, 1)));
            assert_eq!(actual.origin, expected.origin);
            assert_eq!(actual.extent, expected.extent);
            assert_eq!(actual.info.triangle_count, 1);
        }
    }

    #[test]
    fn oversized_batch_is_rejected_before_reading_or_allocating_geometry() {
        let mut bytes = vec![3];
        bytes.extend_from_slice(&((MAX_BATCH_BYTES as u64) + 4).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(
            matches!(receive(bytes.as_slice()), Err(error) if error.kind() == io::ErrorKind::InvalidData)
        );
    }

    #[test]
    fn out_of_bounds_draw_and_truncated_pixels_fail_the_transfer() {
        let mut malformed = triangle();
        malformed.parts[0].index_offset = 80;
        let mut bytes = Vec::new();
        batch(&mut bytes, &malformed).unwrap();
        assert!(
            matches!(receive(bytes.as_slice()), Err(error) if error.kind() == io::ErrorKind::InvalidData)
        );
        bytes.clear();
        batch(&mut bytes, &triangle()).unwrap();
        bytes.pop();
        assert!(
            matches!(receive(bytes.as_slice()), Err(error) if error.kind() == io::ErrorKind::UnexpectedEof)
        );
    }
}
