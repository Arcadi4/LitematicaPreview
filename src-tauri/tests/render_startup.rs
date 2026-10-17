#[allow(dead_code)]
#[path = "../src/render/mod.rs"]
mod render;

#[test]
#[ignore = "Requires a Windows Direct3D 12 hardware adapter"]
fn dx12_startup_pipelines_validate() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("A hardware Direct3D 12 adapter is required");
    println!("Adapter: {:?}", adapter.get_info());
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("startup pipeline validation"),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
        ..Default::default()
    }))
    .unwrap();
    for samples in [1, 4] {
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _pipelines = render::pipelines::Pipelines::new(
            &device,
            samples,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Depth24Plus,
        );
        let error = pollster::block_on(errors.pop());
        assert!(
            error.is_none(),
            "Startup pipeline validation (samples={samples}): {error:?}"
        );
        println!("Startup pipelines validated: samples={samples}");
    }
}
