use rune::compose::Frame;
use wgpu::util::DeviceExt;

use super::SlotPipeline;
use crate::app::App;
use crate::preferences_ui::{SKY_EFFECT, SKY_LIGHT_EFFECT, THEME_DOCK};

pub(super) struct Sky {
    slot: SlotPipeline,
    uniforms: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    bake: wgpu::ComputePipeline,
    heights: wgpu::TextureView,
    bodies: wgpu::TextureView,
    horizon: Option<Horizon>,
    pub(super) moving: bool,
}

struct Horizon {
    size: [u32; 2],
    night: Option<bool>,
    write: wgpu::BindGroup,
    read: wgpu::BindGroup,
}

impl Sky {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let sampled = |sample_type| wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("celestial scene"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    1,
                    sampled(wgpu::TextureSampleType::Float { filterable: false }),
                ),
                entry(
                    4,
                    sampled(wgpu::TextureSampleType::Float { filterable: false }),
                ),
            ],
        });
        let slot = SlotPipeline::new(device, "celestial scene", include_str!("sky.wgsl"), &layout);
        let bake = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("overworld horizon"),
            layout: None,
            module: &slot.shader,
            entry_point: Some("bake_horizon"),
            compilation_options: Default::default(),
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("celestial layout"),
            size: size_of::<[[f32; 4]; 6]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = |label, width, height, format, data: &[u8]| {
            device
                .create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    data,
                )
                .create_view(&Default::default())
        };
        let atlas = image::load_from_memory(crate::resources::SKY_BODIES)
            .expect("bundled sky sprites are a PNG")
            .into_rgba8();
        let bodies = texture(
            "sky sprites",
            atlas.width(),
            atlas.height(),
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &atlas,
        );
        let terrain = texture(
            "overworld heights",
            MAP_SIZE as u32,
            MAP_SIZE as u32,
            wgpu::TextureFormat::Rg8Uint,
            terrain().as_flattened(),
        );
        Self {
            slot,
            uniforms,
            layout,
            bake,
            heights: terrain,
            bodies,
            horizon: None,
            moving: false,
        }
    }

    pub(super) fn draw(&mut self, frame: &mut Frame<'_, App>, pane: [f32; 4]) {
        let rt = &frame.rt;
        let rect = |key| rt.presented(key).map_or([0.0; 4], |(r, _)| r);
        let scale = rt.env.scale;
        let night = rt.state.preferences.theme.is_dark(rt.env.system_dark);
        self.moving = !rt.reduced_motion;
        let uniforms = [
            [
                frame.size.0 as f32 / scale,
                frame.size.1 as f32 / scale,
                f32::from(night),
                if self.moving { rt.t as f32 } else { 0.0 },
            ],
            pane,
            rect(THEME_DOCK),
            rect("theme-dock/indicator"),
            rect("section-Appearance"),
            rect("back"),
        ];
        let bytes = uniforms.map(|v| v.map(f32::to_ne_bytes));
        frame.write_buffer(&self.uniforms, 0, bytes.as_flattened().as_flattened());
        let size = [
            pane[2].ceil().max(1.0) as u32,
            (pane[3] * 0.375).ceil().max(1.0) as u32,
        ];
        if self.horizon.as_ref().is_none_or(|h| h.size != size) {
            self.horizon = Some(self.horizon_target(frame.device, size));
        }
        let horizon = self.horizon.as_mut().unwrap();
        if horizon.night.replace(night) != Some(night) {
            let mut pass = frame
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("overworld horizon"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(&self.bake);
            pass.set_bind_group(0, &horizon.write, &[]);
            pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), 1);
        }
        self.slot.draw(frame, &horizon.read, |mark| match mark {
            SKY_EFFECT => Some(0),
            SKY_LIGHT_EFFECT => Some(1),
            _ => None,
        });
    }

    fn horizon_target(&self, device: &wgpu::Device, size: [u32; 2]) -> Horizon {
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("overworld horizon"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let bind = |layout, (first, a), (second, b)| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("overworld horizon"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: first,
                        resource: wgpu::BindingResource::TextureView(a),
                    },
                    wgpu::BindGroupEntry {
                        binding: second,
                        resource: wgpu::BindingResource::TextureView(b),
                    },
                ],
            })
        };
        Horizon {
            size,
            night: None,
            write: bind(
                &self.bake.get_bind_group_layout(0),
                (2, &self.heights),
                (3, &view),
            ),
            read: bind(&self.layout, (1, &self.bodies), (4, &view)),
        }
    }
}

const MAP_SIZE: usize = 128;

// Each column stores ground elevation and the top of its spruce canopy (zero for open sky).
fn terrain() -> [[u8; 2]; MAP_SIZE * MAP_SIZE] {
    let mut map = [[0; 2]; MAP_SIZE * MAP_SIZE];
    let step = std::f32::consts::TAU / MAP_SIZE as f32;
    for (i, cell) in map.iter_mut().enumerate() {
        let x = (i % MAP_SIZE) as f32 * step;
        let z = (i / MAP_SIZE) as f32 * step;
        let relief = 7.0
            + 3.4 * (2.0 * x + z.cos()).sin()
            + 2.8 * (2.0 * z - x).cos()
            + 1.7 * (4.0 * x + 3.0 * z).sin()
            + 0.8 * (6.0 * z - 3.0 * x).sin();
        cell[0] = relief.round() as u8;
    }
    let mut seed = 739u32;
    for _ in 0..120 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let x = (seed >> 8) as usize % MAP_SIZE;
        let z = (seed >> 20) as usize % MAP_SIZE;
        let ground = map[z * MAP_SIZE + x][0];
        if ground < 6 {
            continue;
        }
        for dz in -2i32..=2 {
            for dx in -2i32..=2 {
                let ring = dx.abs().max(dz.abs());
                if dx.abs() + dz.abs() == 4 {
                    continue;
                }
                let index = ((z as i32 + dz).rem_euclid(MAP_SIZE as i32) as usize) * MAP_SIZE
                    + (x as i32 + dx).rem_euclid(MAP_SIZE as i32) as usize;
                let top = ground + 7 - 2 * ring as u8;
                if top > map[index][1] & 0x7f {
                    map[index][1] = top | if ring == 0 { 0x80 } else { 0 };
                }
            }
        }
    }
    map
}
