#[allow(dead_code)]
#[path = "../src/render/mod.rs"]
mod render;

use litematica_preview_core::{MeshBatch, MeshPart, PreviewInfo, Texture};
use render::camera::FrameCamera;
use render::pipelines::Pipelines;
use render::scene::Scene;
use render::upload::{DeviceStatus, UploadWindow};
use std::sync::Arc;

const SIZE: u32 = 64;
const ROW_BYTES: u32 = SIZE * 4;

/// Reservations account for exactly what a batch stages: the geometry buffer
/// and every first-use texture upload.
fn staged_bytes(batch: &MeshBatch) -> usize {
    batch
        .textures
        .iter()
        .try_fold(batch.geometry.len(), |bytes, texture| {
            bytes.checked_add(texture.pixels.len())
        })
        .expect("the regression batch fits the staging arithmetic")
}

fn quad(compact: bool, alpha_mode: u32, alpha: u8) -> (MeshBatch, FrameCamera) {
    let (origin, extent, scale) = if compact {
        ([32.0, 41.0, 53.0], [150.0, 120.0, 0.0], 0.01)
    } else {
        ([320.0, 410.0, 530.0], [300.0, 240.0, 0.0], 0.005)
    };
    let stride = if compact { 24 } else { 28 };
    let mut geometry = Vec::new();
    for [x, y] in [
        [0u16, 0u16],
        [u16::MAX, 0],
        [u16::MAX, u16::MAX],
        [0, u16::MAX],
    ] {
        if compact {
            for value in [x, y, 0, 0] {
                geometry.extend_from_slice(&value.to_le_bytes());
            }
        } else {
            for value in [
                origin[0] + f32::from(x) / 65535.0 * extent[0],
                origin[1] + f32::from(y) / 65535.0 * extent[1],
                origin[2],
            ] {
                geometry.extend_from_slice(&value.to_le_bytes());
            }
        }
        for value in [0.5f32, 0.5] {
            geometry.extend_from_slice(&value.to_le_bytes());
        }
        geometry.extend_from_slice(&[0, 0, 127, 0]);
        geometry.extend_from_slice(&[255; 4]);
    }
    let index_offset = geometry.len();
    for index in [0u16, 1, 2, 0, 2, 3] {
        geometry.extend_from_slice(&index.to_le_bytes());
    }
    let info = PreviewInfo {
        block_count: 1,
        triangle_count: 2,
        part_count: 1,
        texture_count: 1,
        min: origin,
        max: std::array::from_fn(|axis| origin[axis] + extent[axis]),
        ..PreviewInfo::default()
    };
    let batch = MeshBatch {
        geometry,
        vertex_stride: stride,
        parts: vec![MeshPart {
            vertex_count: 4,
            index_count: 6,
            texture_index: 7,
            alpha_mode,
            vertex_offset: 0,
            index_offset,
            index_size: 2,
            min: info.min,
            max: info.max,
        }],
        textures: vec![Texture {
            index: 7,
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, alpha],
            repeat: false,
        }],
        origin,
        extent,
        source_min: info.min,
        source_max: info.max,
        chunk_coord: None,
        vertex_count: 4,
        index_count: 6,
        info,
    };
    // Both Core layouts describe the same screen-space quad after centering.
    // A real nonzero mesh origin catches accidentally transforming wide
    // absolute positions a second time; a planar extent catches vec3 padding.
    let camera = FrameCamera {
        view_projection: [
            scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0, 0.01, 0.0, -0.75, -0.6, 0.25, 1.0,
        ],
        offset: origin.map(|value| -value),
    };
    (batch, camera)
}

fn material_strips(compact: bool) -> (MeshBatch, FrameCamera) {
    let (mut batch, camera) = quad(compact, 0, 255);
    batch.geometry.clear();
    batch.parts.clear();
    batch.textures.clear();
    for (strip_index, alpha_mode) in [0, 1, 2, 0].into_iter().enumerate() {
        let alpha = if alpha_mode == 1 { 0 } else { 128 };
        let (mut strip, _) = quad(compact, alpha_mode, alpha);
        let mut part = strip.parts.remove(0);
        for (vertex, right) in [false, true, true, false].into_iter().enumerate() {
            let fraction = (strip_index + usize::from(right)) as f32 / 4.0;
            let at = vertex * strip.vertex_stride as usize;
            if compact {
                let x = (fraction * 65535.0).round() as u16;
                strip.geometry[at..at + 2].copy_from_slice(&x.to_le_bytes());
            } else {
                let x = strip.origin[0] + fraction * strip.extent[0];
                strip.geometry[at..at + 4].copy_from_slice(&x.to_le_bytes());
            }
        }
        part.min[0] = strip.origin[0] + strip_index as f32 / 4.0 * strip.extent[0];
        part.max[0] = part.min[0] + strip.extent[0] / 4.0;
        part.vertex_offset += batch.geometry.len();
        part.index_offset += batch.geometry.len();
        part.texture_index += strip_index as u32;
        let mut texture = strip.textures.remove(0);
        texture.index = part.texture_index;
        if strip_index == 3 {
            texture.pixels = vec![0, 255, 0, alpha];
        }
        batch.geometry.extend_from_slice(&strip.geometry);
        batch.parts.push(part);
        batch.textures.push(texture);
    }
    batch.vertex_count = 16;
    batch.index_count = 24;
    batch.info.triangle_count = 8;
    batch.info.part_count = 4;
    batch.info.texture_count = 4;
    (batch, camera)
}

fn render_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &Pipelines,
    scene: &Scene,
    camera: FrameCamera,
) -> Vec<u8> {
    pipelines.write_cameras(queue, camera, camera);
    let descriptor = wgpu::TextureDescriptor {
        label: Some("scene regression target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let target = device.create_texture(&descriptor);
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene regression depth"),
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        ..descriptor
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scene regression readback"),
        size: u64::from(ROW_BYTES * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene regression draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        scene.draw(pipelines, &mut pass, false).unwrap();
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW_BYTES),
                rows_per_image: Some(SIZE),
            },
        },
        target.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |result| {
        sender.send(result).unwrap();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let pixels = readback.get_mapped_range(..).unwrap().to_vec();
    readback.unmap();
    pixels
}

#[test]
#[ignore = "Requires a Windows Direct3D 12 hardware adapter"]
fn dx12_uploaded_meshes_render_with_wgsl_part_layout() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("A hardware Direct3D 12 adapter is required");
    println!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("scene rendering regression"),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipelines = Pipelines::new(
        &device,
        1,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Depth24Plus,
    );
    let uploads = Arc::new(UploadWindow::with_device(
        DeviceStatus::new(device.clone()),
        1,
    ));
    for (compact, alpha_mode, alpha) in [
        (true, 0, 255),
        (false, 0, 255),
        (true, 1, 0),
        (true, 2, 128),
    ] {
        let (batch, camera) = quad(compact, alpha_mode, alpha);
        let permit = uploads.acquire(staged_bytes(&batch), &|| Ok(())).unwrap();
        let mut scene = Scene::new(device.clone(), queue.clone());
        scene.attach_batch(&pipelines, &batch, permit).unwrap();
        drop(batch);
        // Upload-only progress must complete without relying on a rendered frame.
        uploads.drain(&|| Ok(())).unwrap();
        let image = render_pixels(&device, &queue, &pipelines, &scene, camera);
        for (x, y) in [(16, 16), (32, 32), (47, 47)] {
            let at = (y * ROW_BYTES + x * 4) as usize;
            let pixel = &image[at..at + 4];
            match alpha_mode {
                1 => assert_eq!(pixel, &[0; 4], "Cutout should discard: {pixel:?}"),
                2 => assert!(
                    (150..=200).contains(&pixel[0])
                        && pixel[1..3] == [0, 0]
                        && (63..=65).contains(&pixel[3]),
                    "Blended part should preserve texture alpha: {pixel:?}"
                ),
                _ => assert!(
                    pixel[0] > 200 && pixel[1..] == [0, 0, 255],
                    "Uploaded quad should cover ({x}, {y}), compact={compact}: {pixel:?}"
                ),
            }
        }
        for (x, y) in [(4, 32), (32, 4), (59, 32), (32, 59)] {
            let at = (y * ROW_BYTES + x * 4) as usize;
            assert_eq!(
                &image[at..at + 4],
                &[0; 4],
                "Quad should not cover ({x}, {y}), compact={compact}"
            );
        }
        println!("Rendered Core layout: compact={compact}, alpha_mode={alpha_mode}");
    }
    for compact in [true, false] {
        let (batch, camera) = material_strips(compact);
        let permit = uploads.acquire(staged_bytes(&batch), &|| Ok(())).unwrap();
        let mut scene = Scene::new(device.clone(), queue.clone());
        scene.attach_batch(&pipelines, &batch, permit).unwrap();
        drop(batch);
        uploads.drain(&|| Ok(())).unwrap();
        let image = render_pixels(&device, &queue, &pipelines, &scene, camera);
        let pixel = |x: u32| {
            let at = (32 * ROW_BYTES + x * 4) as usize;
            &image[at..at + 4]
        };
        let opaque = pixel(14);
        assert!(
            opaque[0] > 200 && opaque[1..] == [0, 0, 255],
            "Opaque must ignore texture alpha: {opaque:?}, compact={compact}",
        );
        assert_eq!(pixel(26), &[0; 4], "Cutout must discard, compact={compact}");
        let blended = pixel(38);
        assert!(
            (150..=200).contains(&blended[0])
                && blended[1..3] == [0, 0]
                && (63..=65).contains(&blended[3]),
            "Blended must retain its alpha mode: {blended:?}, compact={compact}",
        );
        let reused_opaque = pixel(50);
        assert!(
            reused_opaque[0] == 0
                && reused_opaque[1] > 200
                && reused_opaque[2..] == [0, 255],
            "Shared alpha uniforms must preserve each texture: {reused_opaque:?}, compact={compact}",
        );
    }
    let error = pollster::block_on(errors.pop());
    assert!(error.is_none(), "Scene rendering validation: {error:?}");
}
