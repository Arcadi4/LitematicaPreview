//! How a streamed load becomes GPU state.
//!
//! [`protocol::Batch`] carries one geometry buffer addressed per part; each
//! part draws a slice of that buffer. [`protocol::PartMetadata`] and
//! `protocol::validate_part` define the wire metadata and validation.
//! `PartMetadata::texture_index` is stream-global, so a `Part` binds whatever
//! its slot holds. The grid draws first; parts follow in two passes,
//! non-blended then alpha-blended with depth writes off.

use crate::protocol;
use crate::render::context::RenderError;
use crate::render::pipelines::{Pipelines, PART_UNIFORM_BYTES};

/// One uploaded texture: a repeating or clamping texture view slot that part
/// bind groups reference (`repeat` selects the material sampler; the atlas
/// clamps).
struct Texture {
    view: wgpu::TextureView,
    repeat: bool,
}

/// One mesh part.
struct Part {
    alpha_mode: u32,
    /// `true` draws the 24-byte compact pipeline family with the
    /// origin/extent uniforms; `false` the 28-byte absolute-position family
    /// with 0/1 uniforms. `Pipelines::scene_pipeline` maps this pair.
    compact: bool,
    vertex_offset: u64,
    vertex_length: u64,
    index_offset: u64,
    index_count: u32,
    /// `2` or `4` (`PartMetadata::index_size`), kept so the index slice can be
    /// sized without deriving it from `index_format`.
    index_size: u32,
    index_format: wgpu::IndexFormat,
    bind: wgpu::BindGroup,
}

/// One wire batch's retained GPU geometry.
struct GeometryBatch {
    geometry: wgpu::Buffer,
    parts: Vec<Part>,
}

/// The ground-grid geometry built once from the completed model bounds.
struct Grid {
    buffer: wgpu::Buffer,
    count: u32,
}

/// One batch's texture pixel bytes, in batch metadata order: `pixels[i]`
/// uploads into the stream-global slot `texture_offset + i`; the remaining
/// fields come from the parallel `metadata.textures[i]`.
pub struct TextureBytes<'a> {
    pub pixels: &'a [u8],
}

/// A per-batch view of the wire payload: what the decoder published for this
/// batch, with each segment-joined buffer ready for upload. `metadata` is the
/// batch-local metadata; its zeros-offset parts index `geometry`, and its
/// texture record order pairs with `texture_pixels`.
pub struct BatchSource<'a> {
    /// Stream-global texture base offset for the batch (`Batch::textureOffset`).
    pub texture_offset: usize,
    /// Joined bytes of the batch's geometry buffer (payload buffer `0`).
    pub geometry: &'a [u8],
    /// Joined bytes of the batch's texture pixels, parallel to
    /// `metadata.textures`.
    pub texture_pixels: &'a [TextureBytes<'a>],
    /// The batch metadata wire-serialized (`Payload::metadata`).
    pub metadata: &'a protocol::Metadata,
}

/// The GPU side of one preview load: streaming batches land here and the
/// completed model swaps into the renderer at `finish`.
pub struct Scene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Stream-global texture slots; a slot fills with its first-use pixels.
    textures: Vec<Option<Texture>>,
    batches: Vec<GeometryBatch>,
    grid: Option<Grid>,
}

impl Scene {
    pub(crate) fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            textures: Vec::new(),
            batches: Vec::new(),
            grid: None,
        }
    }

    /// Uploads one wire batch: its textures into their stream-global slots,
    /// then one geometry buffer the parts draw slices of. An error makes the
    /// renderer discard the staged load.
    pub(crate) fn attach_batch(
        &mut self,
        pipelines: &Pipelines,
        batch: &BatchSource,
    ) -> Result<(), RenderError> {
        let metadata = batch.metadata;
        if metadata.textures.len() != batch.texture_pixels.len() {
            return Err(RenderError::malformed_batch(
                "the batch texture bytes do not match its texture records",
            ));
        }
        // Texture slots are stream-global; partial loads are discarded wholesale
        // on error.
        let needed = batch
            .texture_offset
            .checked_add(metadata.textures.len())
            .ok_or_else(|| RenderError::malformed_batch("the texture range overflows"))?;
        while self.textures.len() < needed {
            self.textures.push(None);
        }
        for (slot, texture) in metadata.textures.iter().enumerate() {
            let global = batch.texture_offset + slot;
            if self.textures[global].is_some() {
                // Keep the first pixels uploaded to this stream-global slot.
                continue;
            }
            if texture.width == 0
                || texture.height == 0
                || batch.texture_pixels[slot].pixels.len() as u64
                    != texture.width as u64 * texture.height as u64 * 4
            {
                return Err(RenderError::malformed_batch(
                    "the batch contained an unexpected texture size",
                ));
            }
            let gpu = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("block texture"),
                size: wgpu::Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // The sampler decodes sRGB so later math runs in linear space.
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = gpu.create_view(&wgpu::TextureViewDescriptor::default());
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                batch.texture_pixels[slot].pixels,
                wgpu::TexelCopyBufferLayout {
                    bytes_per_row: Some(texture.width * 4),
                    rows_per_image: Some(texture.height),
                    offset: 0,
                },
                wgpu::Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
            );
            self.textures[global] = Some(Texture {
                view,
                repeat: texture.repeat,
            });
        }

        // The batch's parts live at wire offsets inside one geometry buffer;
        // upload the buffer whole and slice per part on draw.
        let geometry = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh batch geometry"),
            size: batch.geometry.len() as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&geometry, 0, batch.geometry);
        let mut parts = Vec::with_capacity(metadata.parts.len());
        for part in metadata.parts.iter() {
            let vertex_length = part.vertex_count as u64 * part.vertex_stride as u64;
            let vertex_end = part.vertex_offset as u64 + vertex_length;
            let index_end =
                part.index_offset as u64 + part.index_count as u64 * part.index_size as u64;
            if (batch.geometry.len() as u64) < index_end || index_end < vertex_end {
                return Err(RenderError::malformed_batch(
                    "the batch geometry is shorter than its parts",
                ));
            }
            // WGSL's two vec3s start at offsets 0 and 16; alpha occupies
            // offset 28, not the padding after origin. Wide vertices already
            // contain absolute positions, while their metadata keeps the real
            // bounds for framing, so they need an identity transform here.
            let compact = part.vertex_stride == 24;
            let (origin, extent) = if compact {
                (part.origin, part.extent)
            } else {
                ([0.0; 3], [1.0; 3])
            };
            let mut uniforms = [0u8; PART_UNIFORM_BYTES as usize];
            for (base, values) in [(0, origin), (16, extent)] {
                for (axis, value) in values.iter().enumerate() {
                    let at = base + axis * 4;
                    uniforms[at..at + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            uniforms[28..32].copy_from_slice(&(part.alpha_mode as f32).to_le_bytes());
            let uniform = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh part uniform"),
                size: PART_UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&uniform, 0, &uniforms);
            let global = part.texture_index as usize;
            let bind_texture = self
                .textures
                .get(global)
                .and_then(|slot| slot.as_ref())
                .ok_or_else(|| {
                    RenderError::malformed_batch(
                        "the part references a texture slot no batch uploaded",
                    )
                })?;
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh part bind group"),
                layout: pipelines.part_layout(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&bind_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(if bind_texture.repeat {
                            pipelines.repeat_sampler()
                        } else {
                            pipelines.clamp_sampler()
                        }),
                    },
                ],
            });
            parts.push(Part {
                alpha_mode: part.alpha_mode,
                compact,
                vertex_offset: part.vertex_offset as u64,
                vertex_length,
                index_offset: part.index_offset as u64,
                index_count: part.index_count,
                index_size: part.index_size,
                index_format: if part.index_size == 2 {
                    wgpu::IndexFormat::Uint16
                } else {
                    wgpu::IndexFormat::Uint32
                },
                bind,
            });
        }
        self.batches.push(GeometryBatch { geometry, parts });
        Ok(())
    }

    /// Builds the ground grid from the completed model bounds.
    pub(crate) fn finish(&mut self, metadata: &protocol::Metadata) {
        // The grid uses a 15%-padded extent, a step that bounds line count,
        // and a floor half a block below the model's centre line.
        let extent =
            (metadata.max[0] - metadata.min[0]).max(metadata.max[2] - metadata.min[2]) * 1.15;
        let extent = extent.max(16.0);
        let step = (extent as f32 / 128.0).ceil().max(1.0);
        let half_steps = (extent as f32 / (2.0 * step)).ceil() as i32;
        let edge = half_steps as f32 * step;
        let y = (metadata.min[1] - metadata.max[1]) * 0.5 - 0.02;
        let mut lines = Vec::with_capacity((half_steps * 2 + 1) as usize * 6);
        for line in -half_steps..=half_steps {
            let offset = line as f32 * step;
            lines.extend_from_slice(&[
                offset, y, -edge, offset, y, edge, -edge, y, offset, edge, y, offset,
            ]);
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ground grid"),
            size: lines.len() as u64 * 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes = lines
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<u8>>();
        self.queue.write_buffer(&buffer, 0, &bytes);
        self.grid = Some(Grid {
            buffer,
            count: lines.len() as u32 / 3,
        });
    }

    /// Draws the grid and parts; `grid_visible` comes from the camera.
    pub(crate) fn draw(
        &self,
        pipelines: &Pipelines,
        pass: &mut wgpu::RenderPass,
        grid_visible: bool,
    ) -> Result<(), RenderError> {
        if grid_visible {
            if let Some(grid) = self.grid.as_ref() {
                pass.set_pipeline(pipelines.grid_pipeline());
                pass.set_bind_group(0, pipelines.grid_camera(), &[]);
                pass.set_vertex_buffer(0, grid.buffer.slice(..));
                pass.draw(0..grid.count, 0..1);
            }
        }
        for blended in [false, true] {
            for batch in &self.batches {
                for part in &batch.parts {
                    if (part.alpha_mode == 2) != blended {
                        continue;
                    }
                    pass.set_pipeline(pipelines.scene_pipeline(part.compact, part.alpha_mode));
                    pass.set_bind_group(0, pipelines.scene_camera(), &[]);
                    pass.set_bind_group(1, &part.bind, &[]);
                    pass.set_vertex_buffer(
                        0,
                        batch
                            .geometry
                            .slice(part.vertex_offset..part.vertex_offset + part.vertex_length),
                    );
                    pass.set_index_buffer(
                        batch.geometry.slice(
                            part.index_offset
                                ..part.index_offset
                                    + part.index_count as u64 * part.index_size as u64,
                        ),
                        part.index_format,
                    );
                    pass.draw_indexed(0..part.index_count, 0, 0..1);
                }
            }
        }
        Ok(())
    }
}
