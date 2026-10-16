#[allow(dead_code)]
#[path = "../src/protocol.rs"]
mod protocol;
#[allow(dead_code)]
#[path = "../src/render/mod.rs"]
mod render;

use render::camera::FrameCamera;
use render::pipelines::Pipelines;
use render::scene::{BatchSource, Scene, TextureBytes};

const SIZE: u32 = 64;
const ROW_BYTES: u32 = SIZE * 4;

fn quad(compact: bool, alpha_mode: u32) -> (Vec<u8>, protocol::Metadata, FrameCamera) {
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
    let metadata = protocol::Metadata {
        block_count: 1,
        block_entity_count: 0,
        triangle_count: 2,
        min: origin,
        max: std::array::from_fn(|axis| origin[axis] + extent[axis]),
        geometry_byte_length: geometry.len(),
        byte_length: geometry.len() + 4,
        textures: vec![protocol::TextureMetadata {
            width: 1,
            height: 1,
            byte_length: 4,
            buffer_id: 1,
            repeat: false,
        }],
        parts: vec![protocol::PartMetadata {
            vertex_count: 4,
            index_count: 6,
            texture_index: 0,
            alpha_mode,
            vertex_offset: 0,
            index_offset,
            vertex_stride: stride,
            index_size: 2,
            origin,
            extent,
        }],
    };
    // Both layouts describe one quad; a nonzero origin catches double-transforming wide positions,
    // and a planar extent catches vec3 padding.
    let camera = FrameCamera {
        view_projection: [
            scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0, 0.01, 0.0, -0.75, -0.6, 0.25, 1.0,
        ],
        offset: origin.map(|value| -value),
    };
    (geometry, metadata, camera)
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
    for (compact, alpha_mode, alpha) in [
        (true, 0, 255),
        (false, 0, 255),
        (true, 1, 0),
        (true, 2, 128),
    ] {
        let (geometry, metadata, camera) = quad(compact, alpha_mode);
        let pixels = [255, 0, 0, alpha];
        let textures = [TextureBytes { pixels: &pixels }];
        let mut scene = Scene::new(device.clone(), queue.clone());
        scene
            .attach_batch(
                &pipelines,
                &BatchSource {
                    texture_offset: 0,
                    geometry: &geometry,
                    texture_pixels: &textures,
                    metadata: &metadata,
                },
            )
            .unwrap();
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
        println!("Rendered wire layout: compact={compact}, alpha_mode={alpha_mode}");
    }
    let error = pollster::block_on(errors.pop());
    assert!(error.is_none(), "Scene rendering validation: {error:?}");
}
