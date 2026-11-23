use std::collections::HashMap;
use std::sync::Arc;

use rune::compose::{Compositor, DeviceKey, Frame};

use crate::app::{App, Preview, VIEWPORT};
use crate::view::SCENE_EFFECT;

#[derive(Default)]
pub(crate) struct ViewerCompositor {
    device: Option<DeviceKey>,
    blit: Option<Blit>,
}

impl Compositor<App> for ViewerCompositor {
    fn render(&mut self, frame: &mut Frame<'_, App>) {
        let device_key = frame.device_key();
        if self.device != Some(device_key) {
            self.device = Some(device_key);
            self.blit = None;
            frame.rt.state.set_gpu(frame.device, frame.queue);
        }
        let Some((rect, _)) = frame.rt.presented(VIEWPORT) else {
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
        blit.pipeline(frame.device, frame.format);
        blit.pipeline(frame.device, wgpu::TextureFormat::Rgba16Float);
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
                if site.mark != SCENE_EFFECT {
                    return;
                }
                let [x, y, width, height] = site.instance.rect;
                let x = (x - site.origin[0]).max(0.0);
                let y = (y - site.origin[1]).max(0.0);
                let width = width.min(site.size[0] as f32 - x);
                let height = height.min(site.size[1] as f32 - y);
                if width <= 0.0 || height <= 0.0 {
                    return;
                }
                pass.set_viewport(x, y, width, height, 0.0, 1.0);
                pass.set_pipeline(&blit.pipelines[&site.format]);
                pass.set_bind_group(
                    0,
                    &blit.binding.as_ref().expect("bound preview texture").1,
                    &[],
                );
                pass.draw(0..3, 0..1);
            });
    }

    fn draws_own(&self) -> bool {
        self.blit.is_some()
    }
}

struct Blit {
    shader: wgpu::ShaderModule,
    bindings: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    binding: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
}

impl Blit {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("schematic viewport blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
        });
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bindings)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            shader,
            bindings,
            layout,
            sampler,
            binding: None,
            pipelines: HashMap::new(),
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

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("schematic viewport blit"),
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
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        });
    }
}
