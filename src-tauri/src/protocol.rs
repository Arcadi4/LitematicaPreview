use std::io::{self, Read, Write};

use litematica_preview_core::{MeshBatch, PreviewInfo};
use serde::{Deserialize, Serialize};

pub const FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_READ_RANGES: usize = 256;
const END: u8 = 0;
const ERROR: u8 = 1;
const TEXTURE: u8 = 2;
const PART: u8 = 3;
const DATA: u8 = 4;
const CHECKPOINT: u8 = 5;
const PROGRESS: u8 = 6;
const CHUNK: u8 = 7;
const GEOMETRY: u8 = 8;
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
    pub geometry_byte_length: usize,
    pub byte_length: usize,
    pub textures: Vec<TextureMetadata>,
    pub parts: Vec<PartMetadata>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub batch_id: u64,
    pub texture_offset: usize,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StreamEvent {
    Batch { batch: Batch },
    Complete { metadata: Metadata },
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartMetadata {
    pub vertex_count: u32,
    pub index_count: u32,
    pub texture_index: u32,
    pub alpha_mode: u32,
    pub vertex_offset: usize,
    pub index_offset: usize,
    pub vertex_stride: u32,
    pub index_size: u32,
    pub origin: [f32; 3],
    pub extent: [f32; 3],
}

#[derive(Serialize, Deserialize)]
struct TextureRecord {
    index: u32,
    width: u32,
    height: u32,
    byte_length: usize,
    repeat: bool,
}

#[derive(Serialize, Deserialize)]
struct Summary {
    block_count: i64,
    block_entity_count: i64,
    triangle_count: u64,
    min: [f32; 3],
    max: [f32; 3],
}

/// Stores one logical renderer payload as allocation-sized segments without
/// flattening it into a contiguous model. Packed IPC reads copy at most 1 MiB per page.
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

impl Buffer {
    fn append(&mut self, other: Self) -> Result<(), String> {
        let base = self.length;
        self.length = base.checked_add(other.length).ok_or(TOO_LARGE)?;
        self.ends
            .extend(other.ends.into_iter().map(|end| base + end));
        self.segments.extend(other.segments);
        Ok(())
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadRange {
    pub buffer_id: usize,
    pub offset: usize,
    pub length: usize,
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

impl Payload {
    pub fn read_ranges(&self, ranges: &[ReadRange]) -> Result<Vec<u8>, String> {
        if ranges.is_empty() || ranges.len() > MAX_READ_RANGES {
            return Err("Invalid preview range.".into());
        }
        let mut total = 0usize;
        for range in ranges {
            let buffer = self
                .buffers
                .get(range.buffer_id)
                .ok_or("Invalid preview range.")?;
            let end = range
                .offset
                .checked_add(range.length)
                .ok_or("Invalid preview range.")?;
            if range.length == 0 || end > buffer.length {
                return Err("Invalid preview range.".into());
            }
            total = align4(total)
                .and_then(|total| total.checked_add(range.length))
                .filter(|total| *total <= FRAME_BYTES)
                .ok_or("Invalid preview range.")?;
        }

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(total)
            .map_err(|_| "There is not enough memory to upload this preview.")?;
        bytes.resize(total, 0);
        let mut destination = 0usize;
        for range in ranges {
            destination = align4(destination).expect("Validated page offset");
            let buffer = &self.buffers[range.buffer_id];
            let end = range.offset + range.length;
            let mut cursor = range.offset;
            let mut index = buffer
                .ends
                .partition_point(|segment_end| *segment_end <= cursor);
            while cursor < end {
                let base = if index == 0 {
                    0
                } else {
                    buffer.ends[index - 1]
                };
                let segment = &buffer.segments[index];
                let start = cursor - base;
                let count = (segment.len() - start).min(end - cursor);
                bytes[destination..destination + count]
                    .copy_from_slice(&segment[start..start + count]);
                cursor += count;
                destination += count;
                index += 1;
            }
        }
        Ok(bytes)
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
}

impl<'a, S: Read + Write> Encoder<'a, S> {
    pub fn new(stream: &'a mut S) -> Self {
        Self { stream }
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

    pub fn chunk(&mut self, batch: MeshBatch) -> Result<(), String> {
        for texture in &batch.textures {
            self.record(
                TEXTURE,
                &TextureRecord {
                    index: texture.index,
                    width: texture.width,
                    height: texture.height,
                    byte_length: texture.pixels.len(),
                    repeat: texture.repeat,
                },
            )?;
            self.bytes(&texture.pixels)?;
        }
        for part in &batch.parts {
            self.record(
                PART,
                &PartMetadata {
                    vertex_count: part.vertex_count,
                    index_count: part.index_count,
                    texture_index: part.texture_index,
                    alpha_mode: part.alpha_mode,
                    vertex_offset: part.vertex_offset,
                    index_offset: part.index_offset,
                    vertex_stride: batch.vertex_stride,
                    index_size: part.index_size,
                    origin: batch.origin,
                    extent: batch.extent,
                },
            )?;
        }
        self.send(GEOMETRY, &(batch.geometry.len() as u64).to_le_bytes())?;
        self.bytes(&batch.geometry)?;
        self.record(
            CHUNK,
            &Summary {
                block_count: batch.info.block_count,
                block_entity_count: batch.info.block_entity_count,
                triangle_count: batch.info.triangle_count,
                min: batch.info.min,
                max: batch.info.max,
            },
        )
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

/// Receives and assembles one logical renderer payload.
///
/// Every non-terminal packet requires an acknowledgement. Cancellation is observed
/// before the next packet, after which the decoder emits an `ERROR` terminator before
/// accepting another request. A malformed stream invalidates the connection.
pub fn receive(
    stream: &mut (impl Read + Write),
    current: impl Fn() -> bool,
    on_progress: impl FnMut(u64, u64),
) -> io::Result<Result<Payload, String>> {
    let mut buffers = vec![Buffer::default()];
    let result = receive_stream(stream, &current, on_progress, |_, payload| {
        let mut source = payload.buffers.into_iter();
        buffers[0].append(source.next().expect("A chunk has geometry"))?;
        buffers.extend(source);
        Ok(())
    })?;
    Ok(result.map(|metadata| Payload { metadata, buffers }))
}

pub fn receive_stream(
    stream: &mut (impl Read + Write),
    current: impl Fn() -> bool,
    mut on_progress: impl FnMut(u64, u64),
    mut on_chunk: impl FnMut(usize, Payload) -> Result<(), String>,
) -> io::Result<Result<Metadata, String>> {
    let mut textures = Vec::new();
    let mut parts: Vec<PartMetadata> = Vec::new();
    let mut buffers = vec![Buffer::default()];
    let mut pending: Option<usize> = None;
    let mut geometry_end = 0usize;
    let mut total = 0usize;
    let mut cancelled = false;
    let mut progress = None;
    let mut published: Option<Metadata> = None;
    let mut texture_offset = 0usize;
    let mut consumer_error = None;
    loop {
        let (kind, bytes) = read_packet(stream)?;
        cancelled |= !current();
        if kind == ERROR {
            let error =
                String::from_utf8(bytes).map_err(|_| invalid("Invalid decoder error text."))?;
            return Ok(Err(consumer_error.unwrap_or_else(|| {
                if cancelled {
                    "Cancelled".into()
                } else {
                    error
                }
            })));
        }
        if kind == END {
            if cancelled {
                return Ok(Err(consumer_error.unwrap_or_else(|| "Cancelled".into())));
            }
            if pending.is_some()
                || buffers.len() != 1
                || buffers[0].length != 0
                || !parts.is_empty()
                || !textures.is_empty()
            {
                return Err(invalid("The decoder preview is truncated."));
            }
            if progress.is_some_and(|(completed, total)| completed != total) {
                return Err(invalid("The decoder mesh progress is incomplete."));
            }
            let metadata = published.ok_or_else(|| invalid("The decoder returned no chunks."))?;
            let summary = parse_summary(&bytes, metadata.triangle_count)?;
            if summary.block_count != metadata.block_count
                || summary.block_entity_count != metadata.block_entity_count
                || summary.min != metadata.min
                || summary.max != metadata.max
            {
                return Err(invalid("The decoder summary does not match its chunks."));
            }
            return Ok(Ok(metadata));
        }
        if cancelled {
            buffers.clear();
            textures.clear();
            parts.clear();
            pending = None;
            published = None;
            stream.write_all(&[CANCEL])?;
            continue;
        }
        match kind {
            CHUNK if pending.is_none() && buffers[0].length != 0 => {
                let counted = parts.iter().try_fold(0u64, |count, part| {
                    validate_indices(&buffers[0], part)?;
                    count
                        .checked_add(u64::from(part.index_count / 3))
                        .ok_or_else(|| invalid(TOO_LARGE))
                })?;
                let summary = parse_summary(&bytes, counted)?;
                let metadata = Metadata {
                    block_count: summary.block_count,
                    block_entity_count: summary.block_entity_count,
                    triangle_count: counted,
                    min: summary.min,
                    max: summary.max,
                    geometry_byte_length: buffers[0].length,
                    byte_length: std::mem::take(&mut total),
                    textures: std::mem::take(&mut textures),
                    parts: std::mem::take(&mut parts),
                };
                let aggregate = published.get_or_insert_with(|| Metadata {
                    block_count: metadata.block_count,
                    block_entity_count: metadata.block_entity_count,
                    triangle_count: 0,
                    min: metadata.min,
                    max: metadata.max,
                    geometry_byte_length: 0,
                    byte_length: 0,
                    textures: Vec::new(),
                    parts: Vec::new(),
                });
                if aggregate.block_count != metadata.block_count
                    || aggregate.block_entity_count != metadata.block_entity_count
                {
                    return Err(invalid("The decoder chunk source counts changed."));
                }
                aggregate.triangle_count = aggregate
                    .triangle_count
                    .checked_add(counted)
                    .ok_or_else(|| invalid(TOO_LARGE))?;
                aggregate.byte_length = aggregate
                    .byte_length
                    .checked_add(metadata.byte_length)
                    .ok_or_else(|| invalid(TOO_LARGE))?;
                for axis in 0..3 {
                    aggregate.min[axis] = aggregate.min[axis].min(metadata.min[axis]);
                    aggregate.max[axis] = aggregate.max[axis].max(metadata.max[axis]);
                }
                for texture in &metadata.textures {
                    let mut texture = texture.clone();
                    texture.buffer_id = texture
                        .buffer_id
                        .checked_add(texture_offset)
                        .ok_or_else(|| invalid(TOO_LARGE))?;
                    aggregate.textures.push(texture);
                }
                for part in &metadata.parts {
                    let mut part = part.clone();
                    part.vertex_offset = part
                        .vertex_offset
                        .checked_add(aggregate.geometry_byte_length)
                        .ok_or_else(|| invalid(TOO_LARGE))?;
                    part.index_offset = part
                        .index_offset
                        .checked_add(aggregate.geometry_byte_length)
                        .ok_or_else(|| invalid(TOO_LARGE))?;
                    aggregate.parts.push(part);
                }
                aggregate.geometry_byte_length = aggregate
                    .geometry_byte_length
                    .checked_add(metadata.geometry_byte_length)
                    .ok_or_else(|| invalid(TOO_LARGE))?;
                let previous_textures = texture_offset;
                texture_offset = aggregate.textures.len();
                geometry_end = 0;
                let payload = Payload {
                    metadata,
                    buffers: std::mem::replace(&mut buffers, vec![Buffer::default()]),
                };
                if let Err(error) = on_chunk(previous_textures, payload) {
                    consumer_error = Some(error);
                    cancelled = true;
                }
            }
            TEXTURE if pending.is_none() && parts.is_empty() && buffers[0].length == 0 => {
                let record: TextureRecord =
                    serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
                let size = (record.width as usize)
                    .checked_mul(record.height as usize)
                    .and_then(|value| value.checked_mul(4));
                if record.width == 0
                    || record.height == 0
                    || size != Some(record.byte_length)
                    || record.index as usize != texture_offset + textures.len()
                {
                    return Err(invalid("The decoder returned an invalid texture."));
                }
                let id = add_buffer(&mut buffers, &mut total, record.byte_length)?;
                pending = Some(id);
                textures.push(TextureMetadata {
                    width: record.width,
                    height: record.height,
                    byte_length: record.byte_length,
                    buffer_id: id,
                    repeat: record.repeat,
                });
            }
            PART if pending.is_none() && buffers[0].length == 0 => {
                let part: PartMetadata =
                    serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
                geometry_end = validate_part(&part, geometry_end, texture_offset + textures.len())?;
                parts.push(part);
            }
            GEOMETRY
                if pending.is_none()
                    && !parts.is_empty()
                    && buffers[0].length == 0
                    && bytes.len() == 8 =>
            {
                let length = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| invalid(TOO_LARGE))?;
                if length == 0 || length != geometry_end {
                    return Err(invalid("The decoder returned an invalid geometry length."));
                }
                total = total
                    .checked_add(length)
                    .ok_or_else(|| invalid(TOO_LARGE))?;
                buffers[0].length = length;
                pending = Some(0);
            }
            DATA => {
                let id = pending.ok_or_else(|| invalid("Unexpected preview buffer."))?;
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
                buffer.segments.push(bytes);
                buffer.ends.push(received + expected);
                if received + expected == buffer.length {
                    pending = None;
                }
            }
            PROGRESS if pending.is_none() && bytes.len() == 16 => {
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
            CHECKPOINT if pending.is_none() && bytes.is_empty() => {}
            _ => return Err(invalid("Unexpected decoder record.")),
        }
        stream.write_all(&[if cancelled { CANCEL } else { CONTINUE }])?;
    }
}

fn validate_part(part: &PartMetadata, offset: usize, texture_count: usize) -> io::Result<usize> {
    if part.vertex_count == 0
        || part.vertex_count > i32::MAX as u32
        || part.index_count == 0
        || part.index_count > i32::MAX as u32
        || part.index_count % 3 != 0
        || part.alpha_mode > 2
        || part.texture_index as usize >= texture_count
        || !matches!(part.vertex_stride, 24 | 28)
        || !matches!(part.index_size, 2 | 4)
        || (part.index_size == 2 && part.vertex_count > 65_536)
        || part.vertex_offset != offset
        || part
            .origin
            .iter()
            .chain(&part.extent)
            .any(|value| !value.is_finite())
        || part.extent.iter().any(|&value| value < 0.0)
    {
        return Err(invalid("The decoder returned an invalid mesh part."));
    }
    let vertex_end = (part.vertex_count as usize)
        .checked_mul(part.vertex_stride as usize)
        .and_then(|length| part.vertex_offset.checked_add(length));
    if vertex_end != Some(part.index_offset) {
        return Err(invalid("The decoder returned overlapping geometry."));
    }
    (part.index_count as usize)
        .checked_mul(part.index_size as usize)
        .and_then(|length| part.index_offset.checked_add(length))
        .and_then(align4)
        .ok_or_else(|| invalid(TOO_LARGE))
}

fn validate_indices(geometry: &Buffer, part: &PartMetadata) -> io::Result<()> {
    let width = part.index_size as usize;
    let mut remaining = part.index_count as usize * width;
    let start = geometry
        .ends
        .partition_point(|&end| end <= part.index_offset);
    let base = if start == 0 {
        0
    } else {
        geometry.ends[start - 1]
    };
    let mut offset = part.index_offset - base;
    for segment in &geometry.segments[start..] {
        let length = remaining.min(segment.len() - offset);
        let bytes = &segment[offset..offset + length];
        if bytes.len() % width != 0 {
            return Err(invalid("The decoder returned truncated indices."));
        }
        for bytes in bytes.chunks_exact(width) {
            let index = if width == 2 {
                u32::from(u16::from_le_bytes(bytes.try_into().unwrap()))
            } else {
                u32::from_le_bytes(bytes.try_into().unwrap())
            };
            if index >= part.vertex_count {
                return Err(invalid("The decoder returned an out-of-range mesh index."));
            }
        }
        remaining -= length;
        if remaining == 0 {
            return Ok(());
        }
        offset = 0;
    }
    Err(invalid("The decoder returned truncated indices."))
}

fn parse_summary(bytes: &[u8], counted: u64) -> io::Result<Summary> {
    let summary: Summary = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    if summary.block_count <= 0
        || summary.block_entity_count < 0
        || counted == 0
        || summary.triangle_count != counted
        || !summary
            .min
            .iter()
            .chain(&summary.max)
            .all(|value| value.is_finite())
        || (0..3).any(|axis| summary.min[axis] > summary.max[axis])
    {
        return Err(invalid("The decoder returned invalid preview metadata."));
    }
    Ok(summary)
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
pub(crate) mod tests {
    use super::*;

    fn range(buffer_id: usize, offset: usize, length: usize) -> ReadRange {
        ReadRange {
            buffer_id,
            offset,
            length,
        }
    }

    #[test]
    fn native_mesh_encoder_delivers_reusable_textures_before_completion() {
        use litematica_preview_core::{load_chunks, PreviewOptions};
        use nucleation::{BlockState, UniversalSchematic};
        use std::net::{TcpListener, TcpStream};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let mut schematic = UniversalSchematic::new("pipeline regression".into());
        for x in [0, 16, 32] {
            schematic.set_block(x, 0, 0, &BlockState::new("minecraft:stone"));
            schematic.set_block(x + 1, 0, 0, &BlockState::new("minecraft:glass"));
        }
        let input = nucleation::formats::litematic::to_litematic(&schematic).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut receiver, _) = listener.accept().unwrap();
        sender
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let ended = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let producer = scope.spawn(|| {
                let pack_bytes = std::fs::read(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../Assets/pack.zip"),
                )
                .unwrap();
                let pack =
                    nucleation::meshing::ResourcePackSource::from_bytes(&pack_bytes).unwrap();
                let mut encoder = Encoder::new(&mut sender);
                let mut native_alpha = std::collections::BTreeSet::new();
                let info = load_chunks(
                    &input,
                    &pack,
                    PreviewOptions {
                        chunk_size: Some(16),
                        thread_count: (litematica_preview_core::max_worker_threads() >= 2)
                            .then_some(2),
                        ..PreviewOptions::default()
                    },
                    |preview| {
                        native_alpha.extend(preview.parts.iter().map(|part| part.alpha_mode));
                        encoder.chunk(preview)
                    },
                    |_, _| Ok(()),
                    || Ok(()),
                );
                drop(encoder);
                finish(&mut sender, info).unwrap();
                ended.store(true, Ordering::Release);
                native_alpha
            });
            let mut chunks = 0;
            let mut textures = 0;
            let mut bytes = 0;
            let mut triangles = 0;
            let mut alpha = std::collections::BTreeSet::new();
            let complete = receive_stream(
                &mut receiver,
                || true,
                |_, _| {},
                |offset, payload| {
                    if chunks == 0 {
                        assert!(!ended.load(Ordering::Acquire));
                    } else {
                        assert!(
                            payload.metadata.textures.is_empty(),
                            "shared textures were sent again"
                        );
                    }
                    assert_eq!(offset, textures);
                    textures += payload.metadata.textures.len();
                    bytes += payload.metadata.byte_length;
                    triangles += payload.metadata.triangle_count;
                    for part in &payload.metadata.parts {
                        assert!((part.texture_index as usize) < textures);
                        alpha.insert(part.alpha_mode);
                        let indices = payload
                            .read_ranges(&[range(
                                0,
                                part.index_offset,
                                part.index_count as usize * part.index_size as usize,
                            )])
                            .unwrap();
                        assert!(indices.chunks_exact(part.index_size as usize).all(|bytes| {
                            let index = if part.index_size == 2 {
                                u32::from(u16::from_le_bytes(bytes.try_into().unwrap()))
                            } else {
                                u32::from_le_bytes(bytes.try_into().unwrap())
                            };
                            index < part.vertex_count
                        }));
                    }
                    chunks += 1;
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
            let native_alpha = producer.join().unwrap();
            assert_eq!(chunks, 3);
            assert_eq!(complete.block_count, 6);
            assert_eq!(complete.triangle_count, triangles);
            assert_eq!(complete.byte_length, bytes);
            assert_eq!(complete.textures.len(), textures);
            assert_eq!(alpha, native_alpha);
        });
    }

    pub(crate) fn chunk_payload() -> Payload {
        receive(&mut wire(part_packets(0)), || true, |_, _| {})
            .unwrap()
            .unwrap()
    }

    pub(crate) fn stream_fixture(chunks: usize) -> Vec<u8> {
        let mut source = io::Cursor::new(part_packets(0));
        let mut first = Vec::new();
        let mut reused = Vec::new();
        loop {
            let (kind, bytes) = read_packet(&mut source).unwrap();
            if kind == END {
                break;
            }
            write_packet(&mut first, kind, &bytes).unwrap();
            if kind != TEXTURE && !(kind == DATA && reused.is_empty()) {
                write_packet(&mut reused, kind, &bytes).unwrap();
            }
        }
        let mut bytes = first;
        for _ in 1..chunks {
            bytes.extend_from_slice(&reused);
        }
        finish(
            &mut bytes,
            Ok(PreviewInfo {
                block_count: 1,
                triangle_count: chunks as u64,
                max: [1.0; 3],
                ..PreviewInfo::default()
            }),
        )
        .unwrap();
        bytes
    }

    #[test]
    fn streamed_chunks_reuse_textures_and_preserve_global_buffer_descriptors() {
        let mut batches = Vec::new();
        let metadata = receive_stream(
            &mut wire(stream_fixture(2)),
            || true,
            |_, _| {},
            |offset, payload| {
                batches.push((offset, payload));
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(batches[0].0, 0);
        assert_eq!(batches[1].0, 1);
        assert_eq!(batches[0].1.metadata.textures.len(), 1);
        assert!(batches[1].1.metadata.textures.is_empty());
        assert_eq!(batches[1].1.metadata.parts[0].texture_index, 0);
        assert_eq!(batches[1].1.metadata.parts[0].vertex_offset, 0);
        assert_eq!(metadata.parts[1].vertex_offset, 32);
        assert_eq!(metadata.geometry_byte_length, 64);
        assert_eq!(metadata.triangle_count, 2);
    }

    #[test]
    fn completed_chunks_preserve_local_indices_without_merging_or_repacking() {
        let payload = receive(&mut wire(stream_fixture(2)), || true, |_, _| {})
            .unwrap()
            .unwrap();
        assert_eq!(payload.metadata.parts.len(), 2);
        for part in &payload.metadata.parts {
            assert_eq!((part.vertex_count, part.index_count), (1, 3));
            let indices = payload
                .read_ranges(&[range(
                    0,
                    part.index_offset,
                    part.index_count as usize * part.index_size as usize,
                )])
                .unwrap();
            assert_eq!(indices, [0; 6]);
        }
        assert_eq!(payload.metadata.parts[1].vertex_offset, 32);
        assert_eq!(payload.metadata.textures[0].buffer_id, 1);
    }

    #[test]
    fn rejected_chunk_acknowledges_cancel_and_drains_before_next_request() {
        let mut input = io::Cursor::new(stream_fixture(1));
        let mut bytes = Vec::new();
        loop {
            let (kind, data) = read_packet(&mut input).unwrap();
            write_packet(&mut bytes, kind, &data).unwrap();
            if kind == CHUNK {
                break;
            }
        }
        finish(&mut bytes, Err("Cancelled".into())).unwrap();
        bytes.extend(stream_fixture(1));
        let mut connection = wire(bytes);
        let result = receive_stream(
            &mut connection,
            || true,
            |_, _| {},
            |_, _| Err("Upload failed".into()),
        )
        .unwrap();
        assert!(matches!(result, Err(error) if error == "Upload failed"));
        assert_eq!(connection.outgoing.last(), Some(&CANCEL));
        let next = receive(&mut connection, || true, |_, _| {})
            .unwrap()
            .unwrap();
        assert_eq!(next.read_ranges(&[range(1, 0, 4)]).unwrap(), [255; 4]);
    }

    #[test]
    fn malformed_late_terminator_never_completes_a_stream() {
        let mut bytes = stream_fixture(2);
        bytes.pop();
        let mut batches = 0;
        assert!(receive_stream(
            &mut wire(bytes),
            || true,
            |_, _| {},
            |_, _| {
                batches += 1;
                Ok(())
            }
        )
        .is_err());
        assert_eq!(batches, 2);
    }

    #[test]
    fn declared_buffers_above_two_gib_wait_for_payload_without_eager_allocation() {
        let mut bytes = Vec::new();
        let texture = TextureRecord {
            index: 0,
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
        for index in 0..100_001 {
            let texture = TextureRecord {
                index,
                width: 1,
                height: 1,
                byte_length: 4,
                repeat: index != 0,
            };
            write_packet(&mut bytes, TEXTURE, &serde_json::to_vec(&texture).unwrap()).unwrap();
            write_packet(&mut bytes, DATA, &[255; 4]).unwrap();
        }
        let mut suffix = io::Cursor::new(part_packets(0));
        read_packet(&mut suffix).unwrap();
        read_packet(&mut suffix).unwrap();
        bytes.extend_from_slice(&suffix.get_ref()[suffix.position() as usize..]);
        let payload = receive(&mut wire(bytes), || true, |_, _| {})
            .unwrap()
            .unwrap();
        assert_eq!(payload.metadata.textures.len(), 100_001);
        let last = payload.metadata.textures.last().unwrap();
        assert_eq!(
            payload.read_ranges(&[range(last.buffer_id, 0, 4)]).unwrap(),
            [255; 4]
        );
        assert_eq!(payload.metadata.triangle_count, 1);
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
                geometry_byte_length: FRAME_BYTES + 3,
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
            payload
                .read_ranges(&[range(0, FRAME_BYTES - 2, 5)])
                .unwrap(),
            [7, 7, 8, 9, 10]
        );
        assert!(payload.read_ranges(&[range(0, usize::MAX, 1)]).is_err());
        assert!(payload
            .read_ranges(&[range(0, 0, FRAME_BYTES + 1)])
            .is_err());
        assert!(payload.read_ranges(&[range(1, 0, 1)]).is_err());
    }

    #[test]
    fn packed_reads_align_ranges_cross_segments_and_preserve_sources() {
        let payload = Payload {
            metadata: Metadata {
                block_count: 0,
                block_entity_count: 0,
                triangle_count: 0,
                min: [0.0; 3],
                max: [0.0; 3],
                geometry_byte_length: 5,
                byte_length: 11,
                textures: vec![],
                parts: vec![],
            },
            buffers: vec![
                Buffer {
                    segments: vec![vec![1, 2], vec![3, 4, 5]],
                    ends: vec![2, 5],
                    length: 5,
                },
                Buffer {
                    segments: vec![vec![6, 7, 8, 9]],
                    ends: vec![4],
                    length: 4,
                },
            ],
        };
        assert_eq!(
            payload
                .read_ranges(&[range(0, 0, 3), range(1, 0, 4)])
                .unwrap(),
            [1, 2, 3, 0, 6, 7, 8, 9]
        );
        assert_eq!(
            payload
                .read_ranges(&[range(0, 1, 3), range(0, 1, 3), range(1, 2, 2)])
                .unwrap(),
            [2, 3, 4, 0, 2, 3, 4, 0, 8, 9]
        );
        assert_eq!(
            payload.read_ranges(&[range(0, 0, 5)]).unwrap(),
            [1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn packed_reads_validate_every_range_before_returning_a_page() {
        let payload = Payload {
            metadata: Metadata {
                block_count: 0,
                block_entity_count: 0,
                triangle_count: 0,
                min: [0.0; 3],
                max: [0.0; 3],
                geometry_byte_length: FRAME_BYTES,
                byte_length: FRAME_BYTES,
                textures: vec![],
                parts: vec![],
            },
            buffers: vec![Buffer {
                segments: vec![vec![42; FRAME_BYTES]],
                ends: vec![FRAME_BYTES],
                length: FRAME_BYTES,
            }],
        };
        assert_eq!(
            payload.read_ranges(&[range(0, 0, FRAME_BYTES)]).unwrap(),
            vec![42; FRAME_BYTES]
        );
        assert!(payload
            .read_ranges(&[range(0, 0, FRAME_BYTES - 3), range(0, 0, 1)])
            .is_err());
        assert!(payload.read_ranges(&[]).is_err());
        assert!(payload
            .read_ranges(&vec![range(0, 0, 1); MAX_READ_RANGES + 1])
            .is_err());
        for bad in [
            range(0, 0, 0),
            range(1, 0, 1),
            range(0, FRAME_BYTES, 1),
            range(0, usize::MAX, 2),
        ] {
            assert!(payload.read_ranges(&[range(0, 0, 1), bad]).is_err());
        }
        let page = payload
            .read_ranges(&[range(0, 0, 1); MAX_READ_RANGES])
            .unwrap();
        for (index, &value) in page.iter().enumerate() {
            assert_eq!(value, if index % 4 == 0 { 42 } else { 0 });
        }
    }

    #[test]
    fn read_ranges_rejects_invalid_json_fields_and_numeric_types() {
        assert!(serde_json::from_str::<ReadRange>(
            r#"{"bufferId":0,"offset":0,"length":1,"target":0}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<ReadRange>(r#"{"bufferId":-1,"offset":0,"length":1}"#).is_err()
        );
        assert!(
            serde_json::from_str::<ReadRange>(r#"{"bufferId":0,"offset":0.5,"length":1}"#).is_err()
        );
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
            index: 0,
            width: 1,
            height: 1,
            byte_length: 4,
            repeat: false,
        };
        write_packet(&mut bytes, TEXTURE, &serde_json::to_vec(&texture).unwrap()).unwrap();
        write_packet(&mut bytes, DATA, &[255; 4]).unwrap();
        let part = PartMetadata {
            vertex_count: 1,
            index_count: 3,
            texture_index: 0,
            alpha_mode: 0,
            vertex_offset: 0,
            index_offset: 24,
            vertex_stride: 24,
            index_size: 2,
            origin: [0.0; 3],
            extent: [1.0; 3],
        };
        write_packet(&mut bytes, PART, &serde_json::to_vec(&part).unwrap()).unwrap();
        let mut geometry = vec![0; 32];
        geometry[20..24].fill(255);
        for target in geometry[24..30].chunks_exact_mut(2) {
            target.copy_from_slice(&(index as u16).to_le_bytes());
        }
        write_packet(&mut bytes, GEOMETRY, &32u64.to_le_bytes()).unwrap();
        write_packet(&mut bytes, DATA, &geometry).unwrap();
        write_packet(
            &mut bytes,
            CHUNK,
            &serde_json::to_vec(&Summary {
                block_count: 1,
                block_entity_count: 0,
                triangle_count: 1,
                min: [0.0; 3],
                max: [1.0; 3],
            })
            .unwrap(),
        )
        .unwrap();
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
                .read_ranges(&[range(0, payload.metadata.parts[0].index_offset, 6)])
                .unwrap(),
            [0; 6]
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
        assert_eq!(worker.read(10, None, &[range(1, 0, 4)]).unwrap(), [255; 4]);
        worker.advance(11);
        assert!(worker.read(10, None, &[range(0, 0, 4)]).is_err());
        let payload = receive(&mut wire(part_packets(0)), || true, |_, _| {})
            .unwrap()
            .unwrap();
        worker.publish(11, payload).unwrap();
        worker.release(10);
        assert_eq!(worker.read(11, None, &[range(1, 0, 4)]).unwrap(), [255; 4]);
        worker.release(11);
        assert!(worker.read(11, None, &[range(0, 0, 4)]).is_err());
    }

    #[test]
    fn concurrent_upload_ranges_preserve_bytes_and_release_invalidates_reads() {
        let worker = crate::preview::PreviewWorker::default();
        worker.advance(10);
        let payload = receive(&mut wire(part_packets(0)), || true, |_, _| {})
            .unwrap()
            .unwrap();
        let texture = payload.metadata.textures[0].buffer_id;
        let positions = 0;
        worker.publish(10, payload).unwrap();
        let start = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            let readers: Vec<_> = (0..4)
                .map(|offset| {
                    let worker = &worker;
                    let start = &start;
                    scope.spawn(move || {
                        start.wait();
                        assert_eq!(
                            worker
                                .read(
                                    10,
                                    None,
                                    &[range(texture, offset, 1), range(positions, offset * 3, 3)]
                                )
                                .unwrap(),
                            [255, 0, 0, 0, 0, 0, 0]
                        );
                    })
                })
                .collect();
            for reader in readers {
                reader.join().unwrap();
            }
        });
        worker.release(10);
        assert!(worker.read(10, None, &[range(texture, 0, 4)]).is_err());
    }
}
