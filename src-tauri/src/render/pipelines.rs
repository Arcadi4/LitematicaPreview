//! Render pipelines, bind group layouts and the shared camera uniforms.
//!
//! Ported from the WebGL renderer's `Pipeline` object: one scene program for
//! block geometry and one grid program, each bound to a camera uniform. WebGL
//! toggled `gl.enable(BLEND)` and `gl.enable(CULL_FACE)` per part and used one
//! program for both grid and scene; wgpu expresses both as pipeline state, so
//! the same two shader modules are baked into multiple pipelines:
//!
//! * `grid` — grid lines, position-only layout, depth recorded, no blending;
//! * `compact_*` / `float_*` — the 24-byte and 28-byte vertex layouts, each
//!   with back-face culling (opaque), no culling (cutout) and alpha blending
//!   with depth writes off (transparent), matching renderer.ts's per-part
//!   `gl.enable(gl.CULL_FACE)` / `gl.enable(gl.BLEND)` toggles.

use crate::render::camera::FrameCamera;

/// Uniform byte length of the shader `Camera` struct: `mat4x4<f32>` plus the
/// 16-aligned `vec3<f32>` offset.
pub const CAMERA_UNIFORM_BYTES: u64 = 80;
/// Uniform byte length of the shader `Part` struct: two `vec3<f32>` plus an
/// `f32`, padded to the struct's 16-byte alignment.
pub const PART_UNIFORM_BYTES: u64 = 32;

/// 24-byte compact vertex: UNORM16x4 position, float2 uv, SNORM8x4 normal,
/// UNORM8x4 color — the attribute offsets from core's `MeshPart` packing.
const COMPACT_VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 24,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &[
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm16x4,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 8,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Snorm8x4,
            offset: 16,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 20,
            shader_location: 3,
        },
    ],
};

/// 28-byte wide vertex: float3 position, float2 uv, SNORM8x4 normal, UNORM8x4
/// color — the absolute-position layout used above the quantization extent.
const FLOAT_VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 28,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &[
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Snorm8x4,
            offset: 20,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 24,
            shader_location: 3,
        },
    ],
};

/// Grid vertex: a bare float3 position.
const GRID_VERTEX_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 12,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &[wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }],
};

pub struct Pipelines {
    /// Per-part uniform (origin/extent/alpha_mode) plus texture and sampler.
    part_layout: wgpu::BindGroupLayout,
    /// Two camera uniform instances: the grid draws with a zero offset, the
    /// scene with `-centre`, mirroring renderer.ts's per-pass uniform writes.
    grid_camera_buffer: wgpu::Buffer,
    scene_camera_buffer: wgpu::Buffer,
    grid_camera: wgpu::BindGroup,
    scene_camera: wgpu::BindGroup,
    clamp_sampler: wgpu::Sampler,
    repeat_sampler: wgpu::Sampler,
    grid_pipeline: wgpu::RenderPipeline,
    compact_back: wgpu::RenderPipeline,
    compact_nocull: wgpu::RenderPipeline,
    compact_blend: wgpu::RenderPipeline,
    float_back: wgpu::RenderPipeline,
    float_nocull: wgpu::RenderPipeline,
    float_blend: wgpu::RenderPipeline,
}

impl Pipelines {
    /// `samples` is the MSAA count the renderer negotiated (1 or 4);
    /// `surface_format` must be an sRGB format and `depth_format` the depth
    /// format the targets use.
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let model = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene shaders"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "model.wgsl"
            ))),
        });
        let grid = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid shaders"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("grid.wgsl"))),
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let part_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("part layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // sRGB textures decode to linear in the shader.
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let create_camera = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: CAMERA_UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let grid_camera_buffer = create_camera("grid camera uniform");
        let scene_camera_buffer = create_camera("scene camera uniform");
        let grid_camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: grid_camera_buffer.as_entire_binding(),
            }],
        });
        let scene_camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_camera_buffer.as_entire_binding(),
            }],
        });
        // renderer.ts set NEAREST magnification/minification with no mips;
        // only the address mode differs (atlas clamps, materials repeat).
        let sampler = |label, mode| wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: mode,
            address_mode_v: mode,
            address_mode_w: mode,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        };
        let clamp_sampler = device.create_sampler(&sampler(
            "block atlas sampler",
            wgpu::AddressMode::ClampToEdge,
        ));
        let repeat_sampler = device.create_sampler(&sampler(
            "block material sampler",
            wgpu::AddressMode::Repeat,
        ));

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&part_layout)],
            immediate_size: 0,
        });
        let grid_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid layout"),
            bind_group_layouts: &[Some(&camera_layout)],
            immediate_size: 0,
        });
        let depth_stencil = wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let multisample = wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        // Straight-alpha source-over, matching
        // `gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)`. The sRGB
        // render target blends in linear space, as the WebGL sRGB attachment
        // did.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let target = wgpu::ColorTargetState {
            format: surface_format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        };
        let blend_target = wgpu::ColorTargetState {
            format: surface_format,
            blend: Some(blend),
            write_mask: wgpu::ColorWrites::ALL,
        };
        let scene_pipeline = |label: &'static str,
                              layout: &wgpu::PipelineLayout,
                              entry_point: &'static str,
                              vertex_layout: wgpu::VertexBufferLayout<'static>,
                              cull: Option<wgpu::Face>,
                              blended: bool,
                              multisample: wgpu::MultisampleState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &model,
                    entry_point: Some(entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(vertex_layout)],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: cull,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    depth_write_enabled: Some(!blended),
                    ..depth_stencil.clone()
                }),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &model,
                    entry_point: Some("fragment_scene"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(if blended {
                        blend_target.clone()
                    } else {
                        target.clone()
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let compact_back = scene_pipeline(
            "opaque compact mesh",
            &scene_layout,
            "vertex_compact",
            COMPACT_VERTEX_LAYOUT,
            Some(wgpu::Face::Back),
            false,
            multisample,
        );
        let compact_nocull = scene_pipeline(
            "cutout compact mesh",
            &scene_layout,
            "vertex_compact",
            COMPACT_VERTEX_LAYOUT,
            None,
            false,
            multisample,
        );
        let compact_blend = scene_pipeline(
            "transparent compact mesh",
            &scene_layout,
            "vertex_compact",
            COMPACT_VERTEX_LAYOUT,
            None,
            true,
            multisample,
        );
        let float_back = scene_pipeline(
            "opaque float mesh",
            &scene_layout,
            "vertex_float",
            FLOAT_VERTEX_LAYOUT,
            Some(wgpu::Face::Back),
            false,
            multisample,
        );
        let float_nocull = scene_pipeline(
            "cutout float mesh",
            &scene_layout,
            "vertex_float",
            FLOAT_VERTEX_LAYOUT,
            None,
            false,
            multisample,
        );
        let float_blend = scene_pipeline(
            "transparent float mesh",
            &scene_layout,
            "vertex_float",
            FLOAT_VERTEX_LAYOUT,
            None,
            true,
            multisample,
        );
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid pipeline"),
            layout: Some(&grid_layout),
            vertex: wgpu::VertexState {
                module: &grid,
                entry_point: Some("vertex_grid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(GRID_VERTEX_LAYOUT)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                depth_write_enabled: Some(true),
                ..depth_stencil
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &grid,
                entry_point: Some("fragment_grid"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(target)],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            part_layout,
            grid_camera_buffer,
            scene_camera_buffer,
            grid_camera,
            scene_camera,
            clamp_sampler,
            repeat_sampler,
            grid_pipeline,
            compact_back,
            compact_nocull,
            compact_blend,
            float_back,
            float_nocull,
            float_blend,
        }
    }

    /// Uploads both camera uniform instances. The grid receives its zero
    /// offset and the scene its `-centre` offset, so each pass binds the
    /// buffer the WebGL renderer's per-pass uniform write would have produced.
    pub(crate) fn write_cameras(&self, queue: &wgpu::Queue, grid: FrameCamera, scene: FrameCamera) {
        queue.write_buffer(&self.grid_camera_buffer, 0, &grid.encode());
        queue.write_buffer(&self.scene_camera_buffer, 0, &scene.encode());
    }

    pub(crate) fn grid_camera(&self) -> &wgpu::BindGroup {
        &self.grid_camera
    }

    pub(crate) fn scene_camera(&self) -> &wgpu::BindGroup {
        &self.scene_camera
    }

    pub(crate) fn part_layout(&self) -> &wgpu::BindGroupLayout {
        &self.part_layout
    }

    pub(crate) fn clamp_sampler(&self) -> &wgpu::Sampler {
        &self.clamp_sampler
    }

    pub(crate) fn repeat_sampler(&self) -> &wgpu::Sampler {
        &self.repeat_sampler
    }

    pub(crate) fn grid_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.grid_pipeline
    }

    /// The scene pipeline for a part's vertex stride and alpha mode.
    pub(crate) fn scene_pipeline(&self, compact: bool, alpha_mode: u32) -> &wgpu::RenderPipeline {
        match (compact, alpha_mode) {
            (true, 2) => &self.compact_blend,
            (true, 0) => &self.compact_back,
            (true, _) => &self.compact_nocull,
            (false, 2) => &self.float_blend,
            (false, 0) => &self.float_back,
            (false, _) => &self.float_nocull,
        }
    }
}
