use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};

use litematica_preview_native::{parts, Preview, PreviewInfo};
use serde::{Deserialize, Serialize};

pub const FRAME_BYTES: usize = 1024 * 1024;
const END: u8 = 0;
const ERROR: u8 = 1;
const TEXTURE: u8 = 2;
const PART: u8 = 3;
const DATA: u8 = 4;
const CHECKPOINT: u8 = 5;
const PROGRESS: u8 = 6;
const CONTINUE: u8 = 1;
const CANCEL: u8 = 0;
const TOO_LARGE: &str = "The preview is too large to transfer to the graphics device.";

#[cfg(not(target_endian = "little"))]
compile_error!("The preview buffers require a little-endian target.");

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub block_count: i64,
    pub block_entity_count: i64,
    pub triangle_count: u64,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub byte_length: usize,
    pub textures: Vec<TextureMetadata>,
    pub parts: Vec<PartMetadata>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureMetadata {
    pub width: u32,
    pub height: u32,
    pub byte_length: usize,
    pub buffer_id: usize,
    pub repeat: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartMetadata {
    pub vertex_count: u32,
    pub index_count: u32,
    pub texture_index: u32,
    pub alpha_mode: u32,
    pub buffers: [usize; 5],
}

#[derive(Serialize, Deserialize)]
struct TextureRecord {
    width: u32,
    height: u32,
    byte_length: usize,
    repeat: bool,
}

#[derive(Serialize, Deserialize)]
struct PartRecord {
    vertex_count: u32,
    index_count: u32,
    texture_index: u32,
    alpha_mode: u32,
}

#[derive(Serialize, Deserialize)]
struct Summary {
    block_count: i64,
    block_entity_count: i64,
    triangle_count: u64,
    min: [f32; 3],
    max: [f32; 3],
}

// One logical renderer payload; allocation-sized segments never flatten into
// another full model. Only a requested IPC range is copied (at most 1 MiB).
pub struct Payload {
    pub metadata: Metadata,
    buffers: Vec<Buffer>,
}

#[derive(Default)]
struct Buffer {
    segments: Vec<Vec<u8>>,
    ends: Vec<usize>,
    length: usize,
}

impl Payload {
    pub fn read(&self, id: usize, offset: usize, length: usize) -> Result<Vec<u8>, String> {
        let buffer = self
            .buffers
            .get(id)
            .ok_or("The preview buffer is unavailable.")?;
        let end = offset.checked_add(length).ok_or("Invalid preview range.")?;
        if length == 0 || length > FRAME_BYTES || end > buffer.length {
            return Err("Invalid preview range.".into());
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| "There is not enough memory to upload this preview.")?;
        let mut cursor = offset;
        let mut index = buffer.ends.partition_point(|end| *end <= offset);
        while cursor < end {
            let base = if index == 0 {
                0
            } else {
                buffer.ends[index - 1]
            };
            let segment = &buffer.segments[index];
            let start = cursor - base;
            let count = (segment.len() - start).min(end - cursor);
            bytes.extend_from_slice(&segment[start..start + count]);
            cursor += count;
            index += 1;
        }
        Ok(bytes)
    }

    fn assemble(
        mut metadata: Metadata,
        mut source: Vec<Buffer>,
        current: impl Fn() -> bool,
    ) -> io::Result<Self> {
        let mut buffers = Vec::new();
        for texture in &mut metadata.textures {
            let buffer = std::mem::take(&mut source[texture.buffer_id]);
            texture.buffer_id = buffers.len();
            buffers.push(buffer);
        }
        let mut merged: Vec<PartMetadata> = Vec::new();
        let mut groups = std::collections::HashMap::new();
        for part in metadata.parts {
            if !current() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
            }
            let key = (part.texture_index, part.alpha_mode);
            let existing = groups.get(&key).copied().filter(|&index| {
                let target: &PartMetadata = &merged[index];
                target
                    .vertex_count
                    .checked_add(part.vertex_count)
                    .is_some_and(|count| count <= i32::MAX as u32)
                    && target
                        .index_count
                        .checked_add(part.index_count)
                        .is_some_and(|count| count <= i32::MAX as u32)
            });
            let group = if let Some(index) = existing {
                index
            } else {
                // Keep GPU-addressable draw batches rather than limiting the model.
                let index = merged.len();
                let base = buffers.len();
                buffers.extend((0..5).map(|_| Buffer::default()));
                merged.push(PartMetadata {
                    vertex_count: 0,
                    index_count: 0,
                    texture_index: part.texture_index,
                    alpha_mode: part.alpha_mode,
                    buffers: [base, base + 1, base + 2, base + 3, base + 4],
                });
                groups.insert(key, index);
                index
            };
            let target = &mut merged[group];
            let vertex_offset = target.vertex_count;
            target.vertex_count = target
                .vertex_count
                .checked_add(part.vertex_count)
                .filter(|n| *n <= i32::MAX as u32)
                .ok_or_else(|| invalid(TOO_LARGE))?;
            target.index_count = target
                .index_count
                .checked_add(part.index_count)
                .filter(|n| *n <= i32::MAX as u32)
                .ok_or_else(|| invalid(TOO_LARGE))?;
            for attribute in 0..5 {
                let mut buffer = std::mem::take(&mut source[part.buffers[attribute]]);
                if attribute == 4 && vertex_offset != 0 {
                    for segment in &mut buffer.segments {
                        for bytes in segment.chunks_exact_mut(4) {
                            let index =
                                u32::from_le_bytes(bytes.try_into().unwrap()) + vertex_offset;
                            bytes.copy_from_slice(&index.to_le_bytes());
                        }
                    }
                }
                let destination = &mut buffers[target.buffers[attribute]];
                let base = destination.length;
                destination.length = base
                    .checked_add(buffer.length)
                    .ok_or_else(|| invalid(TOO_LARGE))?;
                destination
                    .ends
                    .extend(buffer.ends.into_iter().map(|end| base + end));
                destination.segments.extend(buffer.segments);
            }
        }
        metadata.parts = merged;
        Ok(Self { metadata, buffers })
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn write_packet(stream: &mut impl Write, kind: u8, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > FRAME_BYTES {
        return Err(invalid("The decoder packet is too large."));
    }
    stream.write_all(&[kind])?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(bytes)
}

fn read_packet(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0; 5];
    stream.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header[1..].try_into().unwrap()) as usize;
    if length > FRAME_BYTES {
        return Err(invalid("The decoder packet is too large."));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| invalid("There is not enough memory to receive this preview."))?;
    bytes.resize(length, 0);
    stream.read_exact(&mut bytes)?;
    Ok((header[0], bytes))
}

pub struct Encoder<'a, S> {
    stream: &'a mut S,
    textures: Vec<(u64, TextureRecord, Vec<u8>)>,
}

impl<'a, S: Read + Write> Encoder<'a, S> {
    pub fn new(stream: &'a mut S) -> Self {
        Self {
            stream,
            textures: Vec::new(),
        }
    }

    pub fn checkpoint(&mut self) -> Result<(), String> {
        self.send(CHECKPOINT, &[])
    }

    pub fn progress(&mut self, completed: u64, total: u64) -> Result<(), String> {
        let mut bytes = [0; 16];
        bytes[..8].copy_from_slice(&completed.to_le_bytes());
        bytes[8..].copy_from_slice(&total.to_le_bytes());
        self.send(PROGRESS, &bytes)
    }

    fn send(&mut self, kind: u8, bytes: &[u8]) -> Result<(), String> {
        write_packet(self.stream, kind, bytes).map_err(|e| e.to_string())?;
        let mut ack = [0];
        self.stream
            .read_exact(&mut ack)
            .map_err(|e| e.to_string())?;
        match ack[0] {
            CONTINUE => Ok(()),
            CANCEL => Err("Cancelled".into()),
            _ => Err("Invalid decoder acknowledgement.".into()),
        }
    }

    fn record(&mut self, kind: u8, value: &impl Serialize) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        self.send(kind, &bytes)
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        for segment in bytes.chunks(FRAME_BYTES) {
            self.send(DATA, segment)?;
        }
        Ok(())
    }

    fn texture(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
        repeat: bool,
    ) -> Result<u32, String> {
        let mut hash = DefaultHasher::new();
        (width, height, repeat, pixels).hash(&mut hash);
        let key = hash.finish();
        if let Some(index) = self.textures.iter().position(|(h, record, prior)| {
            *h == key
                && record.width == width
                && record.height == height
                && record.repeat == repeat
                && prior == pixels
        }) {
            return u32::try_from(index).map_err(|_| TOO_LARGE.into());
        }
        let index = u32::try_from(self.textures.len()).map_err(|_| TOO_LARGE)?;
        let record = TextureRecord {
            width,
            height,
            byte_length: pixels.len(),
            repeat,
        };
        self.record(TEXTURE, &record)?;
        self.bytes(pixels)?;
        self.textures.push((key, record, pixels.to_vec()));
        Ok(index)
    }

    pub fn chunk(&mut self, preview: Preview) -> Result<(), String> {
        let atlas = &preview.mesh.atlas;
        let mut textures = Vec::with_capacity(preview.textures.len() + 1);
        textures.push(self.texture(atlas.width, atlas.height, &atlas.pixels, false)?);
        for texture in &preview.textures {
            textures.push(self.texture(texture.width, texture.height, &texture.pixels, true)?);
        }
        for (part, texture, alpha_mode) in parts(&preview.mesh) {
            let record = PartRecord {
                vertex_count: u32::try_from(part.positions.len()).map_err(|_| TOO_LARGE)?,
                index_count: u32::try_from(part.indices.len()).map_err(|_| TOO_LARGE)?,
                texture_index: textures[texture as usize],
                alpha_mode,
            };
            self.record(PART, &record)?;
            self.bytes(bytemuck::cast_slice(&part.positions))?;
            self.quantized(part.normals.iter().flatten().copied(), true)?;
            self.bytes(bytemuck::cast_slice(&part.uvs))?;
            self.quantized(part.colors.iter().flatten().copied(), false)?;
            self.bytes(bytemuck::cast_slice(&part.indices))?;
        }
        Ok(())
    }

    fn quantized(&mut self, values: impl Iterator<Item = f32>, signed: bool) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(FRAME_BYTES);
        for value in values {
            if !value.is_finite() {
                return Err("The mesher produced a non-finite attribute.".into());
            }
            bytes.push(if signed {
                (value.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8
            } else {
                (value.clamp(0.0, 1.0) * 255.0).round() as u8
            });
            if bytes.len() == FRAME_BYTES {
                self.send(DATA, &bytes)?;
                bytes.clear();
            }
        }
        if !bytes.is_empty() {
            self.send(DATA, &bytes)?;
        }
        Ok(())
    }
}

pub fn finish(stream: &mut impl Write, result: Result<PreviewInfo, String>) -> io::Result<()> {
    match result {
        Ok(info) => {
            let summary = Summary {
                block_count: info.block_count,
                block_entity_count: info.block_entity_count,
                triangle_count: info.triangle_count,
                min: info.min,
                max: info.max,
            };
            write_packet(
                stream,
                END,
                &serde_json::to_vec(&summary).map_err(|e| invalid(e.to_string()))?,
            )
        }
        Err(error) => {
            let message = if error.len() <= FRAME_BYTES {
                error.as_bytes()
            } else {
                b"The decoder returned an oversized error."
            };
            write_packet(stream, ERROR, message)
        }
    }
}

// Non-terminal packets require an acknowledgement. Cancellation is observed at
// the next packet, and the decoder emits an ERROR terminator before accepting
// another request. Malformed streams instead invalidate the worker connection.
pub fn receive(
    stream: &mut (impl Read + Write),
    current: impl Fn() -> bool,
    mut on_progress: impl FnMut(u64, u64),
) -> io::Result<Result<Payload, String>> {
    let mut textures = Vec::new();
    let mut parts = Vec::new();
    let mut buffers: Vec<Buffer> = Vec::new();
    let mut pending = std::collections::VecDeque::new();
    let mut total = 0usize;
    let mut cancelled = false;
    let mut progress = None;
    loop {
        let (kind, bytes) = read_packet(stream)?;
        cancelled |= !current();
        if kind == ERROR {
            return Ok(Err(if cancelled {
                "Cancelled".into()
            } else {
                String::from_utf8(bytes).map_err(|_| invalid("Invalid decoder error text."))?
            }));
        }
        if kind == END {
            if cancelled {
                return Ok(Err("Cancelled".into()));
            }
            if !pending.is_empty() {
                return Err(invalid("The decoder preview is truncated."));
            }
            if progress.is_some_and(|(completed, total)| completed != total) {
                return Err(invalid("The decoder mesh progress is incomplete."));
            }
            let summary: Summary =
                serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
            let counted = parts
                .iter()
                .try_fold(0u64, |total, p: &PartMetadata| {
                    total.checked_add(u64::from(p.index_count / 3))
                })
                .ok_or_else(|| invalid(TOO_LARGE))?;
            if summary.block_count <= 0
                || summary.block_entity_count < 0
                || counted == 0
                || summary.triangle_count != counted
                || !summary
                    .min
                    .iter()
                    .chain(&summary.max)
                    .all(|v| v.is_finite())
                || (0..3).any(|i| summary.min[i] > summary.max[i])
            {
                return Err(invalid("The decoder returned invalid preview metadata."));
            }
            let metadata = Metadata {
                block_count: summary.block_count,
                block_entity_count: summary.block_entity_count,
                triangle_count: counted,
                min: summary.min,
                max: summary.max,
                byte_length: total,
                textures,
                parts,
            };
            return Payload::assemble(metadata, buffers, current).map(Ok);
        }
        if cancelled {
            buffers.clear();
            textures.clear();
            parts.clear();
            pending.clear();
            stream.write_all(&[CANCEL])?;
            continue;
        }
        match kind {
            TEXTURE if pending.is_empty() => {
                let record: TextureRecord =
                    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
                let size = (record.width as usize)
                    .checked_mul(record.height as usize)
                    .and_then(|n| n.checked_mul(4));
                if record.width == 0 || record.height == 0 || size != Some(record.byte_length) {
                    return Err(invalid("The decoder returned an invalid texture."));
                }
                let id = add_buffer(&mut buffers, &mut total, record.byte_length)?;
                pending.push_back((id, None));
                textures.push(TextureMetadata {
                    width: record.width,
                    height: record.height,
                    byte_length: record.byte_length,
                    buffer_id: id,
                    repeat: record.repeat,
                });
            }
            PART if pending.is_empty() => {
                let record: PartRecord =
                    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
                if record.vertex_count == 0
                    || record.vertex_count > i32::MAX as u32
                    || record.index_count == 0
                    || record.index_count > i32::MAX as u32
                    || record.index_count % 3 != 0
                    || record.texture_index as usize >= textures.len()
                    || record.alpha_mode > 2
                {
                    return Err(invalid("The decoder returned an invalid mesh part."));
                }
                let v = record.vertex_count as usize;
                let lengths = [
                    v.checked_mul(12),
                    v.checked_mul(3),
                    v.checked_mul(8),
                    v.checked_mul(4),
                    (record.index_count as usize).checked_mul(4),
                ];
                let mut ids = [0; 5];
                for (i, length) in lengths.into_iter().enumerate() {
                    let length = length.ok_or_else(|| invalid(TOO_LARGE))?;
                    ids[i] = add_buffer(&mut buffers, &mut total, length)?;
                    pending.push_back((
                        ids[i],
                        if i == 4 {
                            Some(record.vertex_count)
                        } else {
                            None
                        },
                    ));
                }
                parts.push(PartMetadata {
                    vertex_count: record.vertex_count,
                    index_count: record.index_count,
                    texture_index: record.texture_index,
                    alpha_mode: record.alpha_mode,
                    buffers: ids,
                });
            }
            DATA => {
                let &(id, index_limit) = pending
                    .front()
                    .ok_or_else(|| invalid("Unexpected preview buffer."))?;
                let buffer = &mut buffers[id];
                let received = buffer.ends.last().copied().unwrap_or(0);
                let expected = buffer
                    .length
                    .checked_sub(received)
                    .ok_or_else(|| invalid("The decoder buffer is oversized."))?
                    .min(FRAME_BYTES);
                if bytes.len() != expected {
                    return Err(invalid("The decoder buffer is truncated or oversized."));
                }
                if let Some(limit) = index_limit {
                    if bytes
                        .chunks_exact(4)
                        .any(|value| u32::from_le_bytes(value.try_into().unwrap()) >= limit)
                    {
                        return Err(invalid("The decoder returned an out-of-range mesh index."));
                    }
                }
                buffer.segments.push(bytes);
                buffer.ends.push(received + expected);
                if received + expected == buffer.length {
                    pending.pop_front();
                }
            }
            PROGRESS if pending.is_empty() && bytes.len() == 16 => {
                let completed = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                let count = u64::from_le_bytes(bytes[8..].try_into().unwrap());
                if count == 0
                    || completed > count
                    || progress.map_or(completed != 0, |(previous, total)| {
                        count != total || completed <= previous
                    })
                {
                    return Err(invalid("The decoder returned invalid mesh progress."));
                }
                progress = Some((completed, count));
                on_progress(completed, count);
            }
            CHECKPOINT if pending.is_empty() && bytes.is_empty() => {}
            _ => return Err(invalid("Unexpected decoder record.")),
        }
        stream.write_all(&[CONTINUE])?;
    }
}

fn add_buffer(buffers: &mut Vec<Buffer>, total: &mut usize, length: usize) -> io::Result<usize> {
    *total = total
        .checked_add(length)
        .ok_or_else(|| invalid(TOO_LARGE))?;
    if length == 0 {
        return Err(invalid("The decoder returned an empty buffer."));
    }
    let id = buffers.len();
    buffers.push(Buffer {
        segments: Vec::new(),
        ends: Vec::new(),
        length,
    });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_buffers_above_two_gib_wait_for_payload_without_eager_allocation() {
        let mut bytes = Vec::new();
        let texture = TextureRecord {
            width: 1 << 30,
            height: 1,
            byte_length: 4usize << 30,
            repeat: false,
        };
        write_packet(&mut bytes, TEXTURE, &serde_json::to_vec(&texture).unwrap()).unwrap();
        finish(&mut bytes, Err("Stopped before pixel data".into())).unwrap();
        let mut connection = wire(bytes);
        assert!(matches!(
            receive(&mut connection, || true, |_, _| {}).unwrap(),
            Err(error) if error == "Stopped before pixel data"
        ));
        assert_eq!(connection.outgoing, [CONTINUE]);

        let mut buffers = Vec::new();
        let mut total = usize::MAX - 1;
        assert!(add_buffer(&mut buffers, &mut total, 2).is_err());
    }

    #[test]
    fn preview_accepts_more_than_one_hundred_thousand_texture_records() {
        let mut bytes = Vec::new();
        let texture = serde_json::to_vec(&TextureRecord {
            width: 1,
            height: 1,
            byte_length: 4,
            repeat: false,
        })
        .unwrap();
        for _ in 0..100_000 {
            write_packet(&mut bytes, TEXTURE, &texture).unwrap();
            write_packet(&mut bytes, DATA, &[255; 4]).unwrap();
        }
        bytes.extend(part_packets(0));
        let payload = receive(&mut wire(bytes), || true, |_, _| {})
            .unwrap()
            .unwrap();
        assert_eq!(payload.metadata.textures.len(), 100_001);
        let last = payload.metadata.textures.last().unwrap();
        assert_eq!(payload.read(last.buffer_id, 0, 4).unwrap(), [255; 4]);
        assert_eq!(payload.metadata.triangle_count, 1);
    }

    #[test]
    fn gpu_batch_count_boundary_starts_a_new_draw_instead_of_rejecting_the_model() {
        let max_triangular_indices = i32::MAX as u32 - 1;
        let records = vec![
            PartMetadata {
                vertex_count: 1,
                index_count: max_triangular_indices,
                texture_index: 0,
                alpha_mode: 0,
                buffers: [0, 1, 2, 3, 4],
            },
            PartMetadata {
                vertex_count: 1,
                index_count: 3,
                texture_index: 0,
                alpha_mode: 0,
                buffers: [5, 6, 7, 8, 9],
            },
        ];
        let source = (0..10).map(|_| Buffer::default()).collect();
        let payload = Payload::assemble(
            Metadata {
                block_count: 2,
                block_entity_count: 0,
                triangle_count: u64::from(max_triangular_indices / 3) + 1,
                min: [0.0; 3],
                max: [1.0; 3],
                byte_length: 0,
                textures: vec![],
                parts: records,
            },
            source,
            || true,
        )
        .unwrap();
        assert_eq!(payload.metadata.parts.len(), 2);
        assert_eq!(
            payload.metadata.parts[0].index_count,
            max_triangular_indices
        );
        assert_eq!(payload.metadata.parts[1].index_count, 3);
    }

    #[test]
    fn range_reads_cross_segments_without_flattening_and_reject_overflow() {
        let payload = Payload {
            metadata: Metadata {
                block_count: 1,
                block_entity_count: 0,
                triangle_count: 1,
                min: [0.0; 3],
                max: [1.0; 3],
                byte_length: FRAME_BYTES + 3,
                textures: vec![],
                parts: vec![],
            },
            buffers: vec![Buffer {
                segments: vec![vec![7; FRAME_BYTES], vec![8, 9, 10]],
                ends: vec![FRAME_BYTES, FRAME_BYTES + 3],
                length: FRAME_BYTES + 3,
            }],
        };
        assert_eq!(
            payload.read(0, FRAME_BYTES - 2, 5).unwrap(),
            [7, 7, 8, 9, 10]
        );
        assert!(payload.read(0, usize::MAX, 1).is_err());
        assert!(payload.read(0, 0, FRAME_BYTES + 1).is_err());
        assert!(payload.read(1, 0, 1).is_err());
    }

    #[test]
    fn merging_chunks_rebases_indices_and_preserves_attribute_boundaries() {
        let mut buffers = Vec::new();
        let mut records = Vec::new();
        for value in [10u8, 20] {
            let base = buffers.len();
            for data in [
                vec![value; 12],
                vec![value; 3],
                vec![value; 8],
                vec![value; 4],
                vec![0; 12],
            ] {
                let length = data.len();
                buffers.push(Buffer {
                    segments: vec![data],
                    ends: vec![length],
                    length,
                });
            }
            records.push(PartMetadata {
                vertex_count: 1,
                index_count: 3,
                texture_index: 0,
                alpha_mode: 0,
                buffers: [base, base + 1, base + 2, base + 3, base + 4],
            });
        }
        let metadata = Metadata {
            block_count: 2,
            block_entity_count: 0,
            triangle_count: 2,
            min: [0.0; 3],
            max: [1.0; 3],
            byte_length: 78,
            textures: vec![],
            parts: records,
        };
        let payload = Payload::assemble(metadata, buffers, || true).unwrap();
        let part = &payload.metadata.parts[0];
        assert_eq!(payload.metadata.parts.len(), 1);
        assert_eq!((part.vertex_count, part.index_count), (2, 6));
        assert_eq!(payload.read(part.buffers[1], 2, 3).unwrap(), [10, 20, 20]);
        let indices = payload.read(part.buffers[4], 0, 24).unwrap();
        let values: Vec<_> = indices
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(values, [0, 0, 0, 1, 1, 1]);
    }

    struct Wire {
        incoming: io::Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }

    impl Read for Wire {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.incoming.read(out)
        }
    }

    impl Write for Wire {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.outgoing.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn wire(bytes: Vec<u8>) -> Wire {
        Wire {
            incoming: io::Cursor::new(bytes),
            outgoing: Vec::new(),
        }
    }

    fn part_packets(index: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let texture = TextureRecord {
            width: 1,
            height: 1,
            byte_length: 4,
            repeat: false,
        };
        write_packet(&mut bytes, TEXTURE, &serde_json::to_vec(&texture).unwrap()).unwrap();
        write_packet(&mut bytes, DATA, &[255; 4]).unwrap();
        let part = PartRecord {
            vertex_count: 1,
            index_count: 3,
            texture_index: 0,
            alpha_mode: 0,
        };
        write_packet(&mut bytes, PART, &serde_json::to_vec(&part).unwrap()).unwrap();
        for data in [
            vec![0; 12],
            vec![0; 3],
            vec![0; 8],
            vec![255; 4],
            [index.to_le_bytes(); 3].concat(),
        ] {
            write_packet(&mut bytes, DATA, &data).unwrap();
        }
        finish(
            &mut bytes,
            Ok(PreviewInfo {
                block_count: 1,
                triangle_count: 1,
                max: [1.0; 3],
                ..PreviewInfo::default()
            }),
        )
        .unwrap();
        bytes
    }

    #[test]
    fn malformed_geometry_is_rejected_before_exposing_buffers() {
        assert!(receive(&mut wire(part_packets(1)), || true, |_, _| {}).is_err());
        let mut truncated = part_packets(0);
        truncated.truncate(truncated.len() - 1);
        assert!(receive(&mut wire(truncated), || true, |_, _| {}).is_err());
        let oversized = [DATA, 1, 0, 16, 0];
        assert!(receive(&mut wire(oversized.to_vec()), || true, |_, _| {}).is_err());
    }

    #[test]
    fn cancellation_drains_terminator_before_the_next_preview() {
        let mut bytes = Vec::new();
        write_packet(&mut bytes, CHECKPOINT, &[]).unwrap();
        finish(&mut bytes, Err("Cancelled".into())).unwrap();
        bytes.extend(part_packets(0));
        let mut connection = wire(bytes);
        assert!(
            matches!(receive(&mut connection, || false, |_, _| {}).unwrap(), Err(e) if e == "Cancelled")
        );
        assert_eq!(connection.outgoing, [CANCEL]);
        let payload = receive(&mut connection, || true, |_, _| {})
            .unwrap()
            .unwrap();
        assert_eq!(
            payload
                .read(payload.metadata.parts[0].buffers[4], 0, 12)
                .unwrap(),
            [0; 12]
        );
    }

    #[test]
    fn mesh_progress_is_acknowledged_and_validated_between_previews() {
        let mut bytes = Vec::new();
        let mut initial = [0; 16];
        initial[8..].copy_from_slice(&1u64.to_le_bytes());
        write_packet(&mut bytes, PROGRESS, &initial).unwrap();
        let mut completed = [0; 16];
        completed[..8].copy_from_slice(&1u64.to_le_bytes());
        completed[8..].copy_from_slice(&1u64.to_le_bytes());
        write_packet(&mut bytes, PROGRESS, &completed).unwrap();
        bytes.extend(part_packets(0));
        let mut connection = wire(bytes);
        let mut observed = Vec::new();
        receive(
            &mut connection,
            || true,
            |done, total| observed.push((done, total)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(observed, [(0, 1), (1, 1)]);
        assert_eq!(&connection.outgoing[..2], &[CONTINUE, CONTINUE]);

        for invalid_bytes in [vec![0; 15], {
            let mut count = [0; 16];
            count[..8].copy_from_slice(&2u64.to_le_bytes());
            count[8..].copy_from_slice(&1u64.to_le_bytes());
            count.to_vec()
        }] {
            let mut bytes = Vec::new();
            write_packet(&mut bytes, PROGRESS, &invalid_bytes).unwrap();
            assert!(receive(&mut wire(bytes), || true, |_, _| {}).is_err());
        }
    }

    #[test]
    fn mesh_progress_rejects_incomplete_or_regressing_results() {
        let mut bytes = Vec::new();
        let mut first = [0; 16];
        first[8..].copy_from_slice(&2u64.to_le_bytes());
        write_packet(&mut bytes, PROGRESS, &first).unwrap();
        bytes.extend(part_packets(0));
        assert!(receive(&mut wire(bytes), || true, |_, _| {}).is_err());

        let mut bytes = Vec::new();
        write_packet(&mut bytes, PROGRESS, &first).unwrap();
        write_packet(&mut bytes, PROGRESS, &first).unwrap();
        assert!(receive(&mut wire(bytes), || true, |_, _| {}).is_err());
    }

    #[test]
    fn old_upload_release_cannot_free_a_new_request() {
        let worker = crate::preview::PreviewWorker::default();
        worker.advance(10);
        let payload = receive(&mut wire(part_packets(0)), || true, |_, _| {})
            .unwrap()
            .unwrap();
        worker.publish(10, payload).unwrap();
        worker.release(9);
        assert_eq!(worker.read(10, 0, 0, 4).unwrap(), [255; 4]);
        worker.advance(11);
        assert!(worker.read(10, 0, 0, 4).is_err());
        let payload = receive(&mut wire(part_packets(0)), || true, |_, _| {})
            .unwrap()
            .unwrap();
        worker.publish(11, payload).unwrap();
        worker.release(10);
        assert_eq!(worker.read(11, 0, 0, 4).unwrap(), [255; 4]);
        worker.release(11);
        assert!(worker.read(11, 0, 0, 4).is_err());
    }

    #[test]
    fn quantized_attributes_preserve_endpoints_and_rounding_error() {
        let mut connection = wire(vec![CONTINUE; 2]);
        let mut encoder = Encoder::new(&mut connection);
        encoder
            .quantized([-1.0, 0.0, 1.0, 0.5].into_iter(), true)
            .unwrap();
        encoder
            .quantized([0.0, 0.5, 1.0].into_iter(), false)
            .unwrap();
        let mut records = io::Cursor::new(connection.outgoing);
        assert_eq!(
            read_packet(&mut records).unwrap(),
            (DATA, vec![129, 0, 127, 64])
        );
        assert_eq!(
            read_packet(&mut records).unwrap(),
            (DATA, vec![0, 128, 255])
        );
        assert!((64.0f32 / 127.0 - 0.5).abs() <= 0.5 / 127.0 + f32::EPSILON);
        assert!((128.0f32 / 255.0 - 0.5).abs() <= 0.5 / 255.0 + f32::EPSILON);
    }
}
