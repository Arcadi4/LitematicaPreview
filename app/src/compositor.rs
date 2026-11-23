mod sky;

use std::collections::HashMap;
use std::sync::Arc;

use rune::compose::{Compositor, DeviceKey, Frame};

use crate::app::{App, Preview, VIEWPORT};
use crate::preferences_ui::SKY;
use crate::view::{LIGHTING_SCENE, SCENE_EFFECT};

#[derive(Default)]
pub(crate) struct ViewerCompositor {
    device: Option<DeviceKey>,
    blit: Option<Blit>,
    sky: Option<sky::Sky>,
}

impl Compositor<App> for ViewerCompositor {
    fn render(&mut self, frame: &mut Frame<'_, App>) {
        let device_key = frame.device_key();
        if self.device != Some(device_key) {
            self.device = Some(device_key);
            self.blit = None;
            self.sky = None;
            frame.rt.state.set_gpu(frame.device, frame.queue);
        }
        if let Some((rect, _)) = frame.rt.presented(SKY) {
            self.blit = None;
            self.sky
                .get_or_insert_with(|| sky::Sky::new(frame.device, frame.queue))
                .draw(frame, rect);
            return;
        }
        self.sky = None;
        let viewport = frame
            .rt
            .presented(VIEWPORT)
            .or_else(|| frame.rt.presented(LIGHTING_SCENE));
        let Some((rect, _)) = viewport else {
            self.blit = None;
            frame.draw_ui();
            return;
        };
        let [_, _, width, height] = rect;
        let scale = frame.rt.env.scale;
        if width < 1.0 || height < 1.0 {
            frame.draw_ui();
            return;
        }
        let size = (
            (width * scale).round() as u32,
            (height * scale).round() as u32,
        );
        let s = &mut frame.rt.state;
        let Some(scene) = s.scene.as_ref().map(Arc::clone) else {
            self.blit = None;
            frame.draw_ui();
            return;
        };
        let resized = s.scene_size.replace(size) != Some(size);
        let capture = s.capture.take();
        let fov = if s.spectator { s.preferences.fov } else { 28.0 };
        let Some(tab) = s.active_tab_mut() else {
            frame.draw_ui();
            return;
        };
        tab.camera.set_fov(fov);
        if resized {
            tab.camera.resize(width, height);
        }
        let mut scene = scene.lock();
        let captured = capture.map(|capture| (capture, scene.capture(size, &tab.camera)));
        let rendered = scene.encode(frame.encoder, size, &tab.camera);
        let texture = match rendered {
            Ok(texture) => texture,
            Err(error) => {
                drop(scene);
                s.scene = None;
                s.preview = Preview::Failed(error.to_string());
                s.request_frame();
                frame.draw_ui();
                return;
            }
        };
        let blit = self.blit.get_or_insert_with(|| Blit::new(frame.device));
        blit.bind(frame.device, texture);
        drop(scene);
        if let Some((capture, result)) = captured {
            match result {
                Ok(job) => s.finish_capture(capture, job),
                Err(error) => {
                    s.notice = Some(error.to_string());
                    s.request_frame();
                }
            }
        }
        blit.slot.draw(
            frame,
            &blit.binding.as_ref().expect("bound preview texture").1,
            |mark| (mark == SCENE_EFFECT).then_some(0),
        );
    }

    fn animating(&self) -> bool {
        self.sky.as_ref().is_some_and(|sky| sky.moving)
    }

    fn draws_own(&self) -> bool {
        self.blit.is_some() || self.sky.is_some()
    }
}

struct Blit {
    slot: SlotPipeline,
    bindings: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    binding: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

impl Blit {
    fn new(device: &wgpu::Device) -> Self {
        let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("schematic target texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let slot = SlotPipeline::new(
            device,
            "schematic viewport blit",
            include_str!("viewport.wgsl"),
            &bindings,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            slot,
            bindings,
            sampler,
            binding: None,
        }
    }

    fn bind(&mut self, device: &wgpu::Device, view: &wgpu::TextureView) {
        if self.binding.as_ref().is_some_and(|(old, _)| old == view) {
            return;
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.bindings,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.binding = Some((view.clone(), bind));
    }
}

struct SlotPipeline {
    label: &'static str,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl SlotPipeline {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        wgsl: &'static str,
        bindings: &wgpu::BindGroupLayout,
    ) -> Self {
        Self {
            label,
            shader: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            }),
            layout: device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bindings)],
                immediate_size: 0,
            }),
            pipelines: HashMap::new(),
        }
    }

    fn draw(
        &mut self,
        frame: &mut Frame<'_, App>,
        binding: &wgpu::BindGroup,
        layer: impl Fn(u32) -> Option<u32>,
    ) {
        let clear = frame.rt.clear_color();
        frame.ui.set_fade_stops(frame.rt.fade_stops());
        frame.ui.prepare(
            frame.device,
            frame.queue,
            &mut frame.rt.text.atlas,
            &frame.rt.instances,
            frame.size,
        );
        frame
            .ui
            .draw_with(frame.encoder, frame.target, clear, &mut |pass, site| {
                let Some(index) = layer(site.mark) else {
                    return;
                };
                let [x, y, width, height] = site.instance.rect;
                let x = (x - site.origin[0]).max(0.0);
                let y = (y - site.origin[1]).max(0.0);
                let width = width.min(site.size[0] as f32 - x);
                let height = height.min(site.size[1] as f32 - y);
                if width > 0.0 && height > 0.0 {
                    pass.set_viewport(x, y, width, height, 0.0, 1.0);
                    pass.set_pipeline(self.pipeline(frame.device, site.format));
                    pass.set_bind_group(0, binding, &[]);
                    pass.draw(0..3, index..index + 1);
                }
            });
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(self.label),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}
