//! Regression coverage for the `UploadWindow` credit and staging-byte ledger
//! across real admission, waiting, cancellation, device failure, and GPU
//! callback release.

#[allow(dead_code)]
#[path = "../src/render/upload.rs"]
mod upload;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use upload::{DeviceStatus, UploadWindow};

const MIB: usize = 1024 * 1024;
/// Mirrors the staged-byte budget pinned in `render/upload.rs`.
const MAX_PENDING_BYTES: usize = 64 * MIB;

fn native_device() -> (wgpu::Device, wgpu::Queue) {
    let descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    let instance = wgpu::Instance::new(descriptor);
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a native graphics adapter is required");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("the native graphics adapter must provide a device")
}

/// The byte budget schools reservations beside credits: 32 MiB plus 31 MiB
/// stays on the budget, and one more MiB waits until a returned permit frees
/// the staged bytes, even though another credit was still free.
#[test]
#[ignore = "Requires a native graphics adapter"]
fn staged_byte_budget_blocks_until_a_permit_returns() {
    let (device, _queue) = native_device();
    let window = Arc::new(UploadWindow::with_device(DeviceStatus::new(device), 3));
    let ok = || Ok::<(), String>(());
    let first = window.acquire(32 * MIB, &ok).unwrap();
    let second = window.acquire(31 * MIB, &ok).unwrap();
    let (sender, receiver) = mpsc::channel();
    {
        let window = Arc::clone(&window);
        std::thread::spawn(move || {
            let _unused = sender.send(window.acquire(2 * MIB, &ok));
        });
    }
    assert!(
        receiver.recv_timeout(Duration::from_secs(1)).is_err(),
        "a reservation beyond the staging budget must wait"
    );
    drop(second);
    let third = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("returning a permit must unblock the waiting reservation")
        .expect("the waiting reservation must eventually win");
    drop(first);
    drop(third);
    window.drain(&ok).unwrap();
}

/// An oversized batch waits behind any pending reservation but still passes
/// through a window that became empty again.
#[test]
#[ignore = "Requires a native graphics adapter"]
fn one_oversized_batch_passes_only_through_an_empty_window() {
    let (device, _queue) = native_device();
    let window = Arc::new(UploadWindow::with_device(DeviceStatus::new(device), 2));
    let ok = || Ok::<(), String>(());
    let hold = window.acquire(1024, &ok).unwrap();
    let (sender, receiver) = mpsc::channel();
    {
        let window = Arc::clone(&window);
        std::thread::spawn(move || {
            let _unused = sender.send(window.acquire(MAX_PENDING_BYTES + MIB, &ok));
        });
    }
    assert!(
        receiver.recv_timeout(Duration::from_secs(1)).is_err(),
        "an oversized batch must wait behind pending reservations"
    );
    drop(hold);
    let oversized = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("an empty window must admit one oversized batch")
        .expect("the oversized batch must fit alone");
    drop(oversized);
    window.drain(&ok).unwrap();
}

/// A reservation waiting on the ledger must wake when its load generation
/// moves on, return the cancellation error, and leave the window's accounting
/// clean for the next reservation.
#[test]
#[ignore = "Requires a native graphics adapter"]
fn a_cancelled_reservation_wakes_without_consuming_its_credit() {
    let (device, _queue) = native_device();
    let window = Arc::new(UploadWindow::with_device(DeviceStatus::new(device), 1));
    let ok = || Ok::<(), String>(());
    let guard = window.acquire(1024, &ok).unwrap();
    let generation = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = mpsc::channel();
    {
        let generation = Arc::clone(&generation);
        let window = Arc::clone(&window);
        std::thread::spawn(move || {
            let current = move || {
                if generation.load(Ordering::SeqCst) == 1 {
                    Err(String::from("The load moved on."))
                } else {
                    Ok(())
                }
            };
            let _unused = sender.send(window.acquire(1024, &current));
        });
    }
    assert!(
        receiver.recv_timeout(Duration::from_secs(1)).is_err(),
        "the reservation must wait until its load generation moves"
    );
    generation.store(1, Ordering::SeqCst);
    drop(guard);
    let outcome = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("cancellation must wake the waiting reservation");
    assert!(matches!(outcome, Err(error) if error == "The load moved on."));
    let fresh = window.acquire(1024, &ok).unwrap();
    drop(fresh);
    window.drain(&ok).unwrap();
}

/// Invalid GPU work recorded on the device status must stop the window from
/// granting further reservations.
#[test]
#[ignore = "Requires a native graphics adapter"]
fn a_failed_device_stops_granting_reservations() {
    let (device, _queue) = native_device();
    let window = Arc::new(UploadWindow::with_device(
        DeviceStatus::new(device.clone()),
        1,
    ));
    let ok = || Ok::<(), String>(());
    let _rejected = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("regression oversized buffer"),
        size: u64::MAX,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let Err(error) = window.acquire(1024, &ok) else {
        panic!("a failed device must stop granting reservations");
    };
    assert!(
        error.starts_with("The graphics device could not complete preview work"),
        "{error}"
    );
}

/// Submitted staging bytes stay reserved until the GPU callback returns them;
/// `drain` may only return once that happened, and the credit works again.
#[test]
#[ignore = "Requires a native graphics adapter"]
fn drain_waits_for_the_gpu_callback_to_return_staged_bytes() {
    let (device, queue) = native_device();
    let window = Arc::new(UploadWindow::with_device(
        DeviceStatus::new(device.clone()),
        1,
    ));
    let ok = || Ok::<(), String>(());
    let permit = window.acquire(4096, &ok).unwrap();
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("regression staging"),
        size: 4096,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&staging, 0, &[7u8; 4096]);
    permit.submit(&queue).unwrap();
    window.drain(&ok).unwrap();
    let again = window.acquire(4096, &ok).unwrap();
    drop(again);
}

/// A window without credits refuses every reservation before touching the
/// device or the wait path.
#[test]
#[ignore = "Requires a native graphics adapter"]
fn an_empty_window_cannot_grant_reservations() {
    let (device, _queue) = native_device();
    let window = Arc::new(UploadWindow::with_device(DeviceStatus::new(device), 0));
    let ok = || Ok::<(), String>(());
    assert!(matches!(
        window.acquire(1024, &ok),
        Err(error) if error == "The GPU upload window must have at least one credit."
    ));
}
