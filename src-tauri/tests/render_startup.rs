#[allow(dead_code)]
#[path = "../src/render/mod.rs"]
mod render;

use render::upload::{DeviceStatus, UploadWindow};
use std::sync::Arc;

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

#[test]
#[ignore = "Requires a Windows Direct3D 12 hardware adapter"]
fn dx12_upload_credits_complete_cancel_and_remember_device_loss() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("A hardware Direct3D 12 adapter is required");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let status = DeviceStatus::new(device.clone());
    let first = Arc::new(UploadWindow::with_device(Arc::clone(&status), 1));
    let reserved = first.acquire(&|| Ok(())).unwrap();
    let cancelled = std::cell::Cell::new(false);
    let result = first.acquire(&|| {
        if cancelled.replace(true) {
            Err(String::from("cancelled"))
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(error) if error == "cancelled"));
    drop(reserved);

    first.acquire(&|| Ok(())).unwrap().submit(&queue).unwrap();
    // A separate session must not receive the old session's completion credit.
    let next = Arc::new(UploadWindow::with_device(Arc::clone(&status), 1));
    let reserved = next.acquire(&|| Ok(())).unwrap();
    first.drain(&|| Ok(())).unwrap();
    let cancelled = std::cell::Cell::new(false);
    let result = next.acquire(&|| {
        if cancelled.replace(true) {
            Err(String::from("cancelled"))
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(error) if error == "cancelled"));
    drop(reserved);
    next.acquire(&|| Ok(())).unwrap().submit(&queue).unwrap();
    next.acquire(&|| Ok(())).unwrap().submit(&queue).unwrap();
    next.drain(&|| Ok(())).unwrap();

    device.destroy();
    let _ = device.poll(wgpu::PollType::Poll);
    assert!(next.acquire(&|| Ok(())).is_err());
    let after_loss = Arc::new(UploadWindow::with_device(status, 1));
    assert!(after_loss.acquire(&|| Ok(())).is_err());
}
