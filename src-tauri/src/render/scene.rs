//! Live GPU geometry appended directly from Core mesh batches.

use crate::render::context::RenderError;
use crate::render::pipelines::{Pipelines, PART_UNIFORM_BYTES};
use crate::render::upload::UploadPermit;
use litematica_preview_core::{MeshBatch, PreviewInfo};

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
    /// Index width in bytes, used to slice the packed geometry buffer.
    index_size: u32,
    index_format: wgpu::IndexFormat,
    bind: wgpu::BindGroup,
}

/// One Core batch's retained GPU geometry.
struct GeometryBatch {
    geometry: wgpu::Buffer,
    parts: Vec<Part>,
}

/// The ground-grid geometry built once from the completed model bounds.
struct Grid {
    buffer: wgpu::Buffer,
    count: u32,
}

/// The GPU side of one progressively displayed preview load.
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

    /// First-use textures land before geometry. No CPU mesh bytes are retained.
    pub(crate) fn attach_batch(
        &mut self,
        pipelines: &Pipelines,
        batch: &MeshBatch,
        permit: UploadPermit,
    ) -> Result<(), RenderError> {
        permit.check(&self.device).map_err(RenderError::Device)?;
        self.validate_batch(batch)?;
        for texture in &batch.textures {
            let slot = texture.index as usize;
            if self.textures.len() <= slot {
                self.textures.resize_with(slot + 1, || None);
            }
            if self.textures[slot].is_some() {
                continue;
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

                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &texture.pixels,
                wgpu::TexelCopyBufferLayout {
                    bytes_per_row: Some(texture.width * 4),
                    rows_per_image: Some(texture.height),
                    offset: 0,
                },
                gpu.size(),
            );
            self.textures[slot] = Some(Texture {
                view: gpu.create_view(&wgpu::TextureViewDescriptor::default()),
                repeat: texture.repeat,
            });
        }

        if !batch.parts.is_empty() {
            let geometry = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh batch geometry"),
                size: batch.geometry.len() as u64,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::INDEX
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&geometry, 0, &batch.geometry);
            let compact = batch.vertex_stride == 24;
            let (origin, extent) = if compact {
                (batch.origin, batch.extent)
            } else {
                ([0.0; 3], [1.0; 3])
            };
            let mut parts = Vec::with_capacity(batch.parts.len());
            for part in &batch.parts {
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
                let texture = self.textures[part.texture_index as usize].as_ref().unwrap();
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
                            resource: wgpu::BindingResource::TextureView(&texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(if texture.repeat {
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
                    vertex_length: part.vertex_count as u64 * batch.vertex_stride as u64,
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
        }
        permit.submit(&self.queue).map_err(RenderError::Device)
    }

    fn validate_batch(&self, batch: &MeshBatch) -> Result<(), RenderError> {
        if !matches!(batch.vertex_stride, 24 | 28) || batch.geometry.len() % 4 != 0 {
            return Err(RenderError::malformed_batch(
                "the batch geometry layout is invalid",
            ));
        }
        let limits = self.device.limits();
        if batch.geometry.len() as u64 > limits.max_buffer_size {
            return Err(RenderError::malformed_batch(
                "the batch geometry exceeds the graphics device limit",
            ));
        }
        for texture in &batch.textures {
            if texture.width == 0
                || texture.height == 0
                || texture.width > limits.max_texture_dimension_2d
                || texture.height > limits.max_texture_dimension_2d
                || texture.pixels.len() as u64 != texture.width as u64 * texture.height as u64 * 4
            {
                return Err(RenderError::malformed_batch(
                    "the batch contained an unexpected texture size",
                ));
            }
        }
        for part in &batch.parts {
            let vertex_length = part.vertex_count as u64 * batch.vertex_stride as u64;
            let vertex_end = (part.vertex_offset as u64).checked_add(vertex_length);
            let index_end = (part.index_offset as u64)
                .checked_add(part.index_count as u64 * part.index_size as u64);
            if !matches!(part.index_size, 2 | 4)
                || part.alpha_mode > 2
                || part.vertex_count == 0
                || part.index_count == 0
                || part.vertex_offset % 4 != 0
                || part.index_offset % 4 != 0
                || vertex_end.is_none_or(|end| end > part.index_offset as u64)
                || index_end.is_none_or(|end| end > batch.geometry.len() as u64)
            {
                return Err(RenderError::malformed_batch(
                    "the batch geometry does not contain its parts",
                ));
            }
            let existing = self
                .textures
                .get(part.texture_index as usize)
                .is_some_and(Option::is_some);
            if !existing
                && !batch
                    .textures
                    .iter()
                    .any(|texture| texture.index == part.texture_index)
            {
                return Err(RenderError::malformed_batch(
                    "the part references a texture slot no batch uploaded",
                ));
            }
        }
        Ok(())
    }

    /// Builds the ground-grid geometry from the completed model bounds,
    /// mirroring renderer.ts's `buildGrid`.
    pub(crate) fn finish(&mut self, metadata: &PreviewInfo) {
        // Same numeric steps as `buildGrid`: a 15%-padded extent, a step that
        // keeps the line count bounded, and a floor half a block below the
        // model's centre line.
        let extent =
            (metadata.max[0] - metadata.min[0]).max(metadata.max[2] - metadata.min[2]) * 1.15;
        let extent = extent.max(16.0);
        let step = (extent / 128.0).ceil().max(1.0);
        let half_steps = (extent / (2.0 * step)).ceil() as i32;
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
