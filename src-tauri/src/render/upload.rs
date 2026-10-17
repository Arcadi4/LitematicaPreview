//! Session-local credits for queue-write staging. Wait only outside renderer locks.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const POLL_SLICE: Duration = Duration::from_millis(25);
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

type DeviceFailure = Arc<Mutex<Option<String>>>;

pub(crate) struct DeviceStatus {
    device: wgpu::Device,
    failure: DeviceFailure,
}

impl DeviceStatus {
    pub(crate) fn new(device: wgpu::Device) -> Arc<Self> {
        let status = Arc::new(Self {
            device,
            failure: Arc::new(Mutex::new(None)),
        });
        let failure = Arc::clone(&status.failure);
        status
            .device
            .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                fail(
                    &failure,
                    format!("The graphics device could not complete preview work: {error}"),
                );
            }));
        let failure = Arc::clone(&status.failure);
        status
            .device
            .set_device_lost_callback(move |reason, message| {
                fail(
                    &failure,
                    format!("The graphics device was lost ({reason:?}): {message}"),
                );
            });
        status
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        check(&self.failure)
    }
}

fn check(failure: &DeviceFailure) -> Result<(), String> {
    match failure.lock() {
        Ok(error) => error
            .as_ref()
            .map_or(Ok(()), |message| Err(message.clone())),
        Err(_) => Err(String::from(
            "The graphics device error state is unavailable.",
        )),
    }
}

fn fail(failure: &DeviceFailure, message: String) {
    if let Ok(mut error) = failure.lock() {
        if error.is_none() {
            *error = Some(message);
        }
    }
}

#[derive(Default)]
struct Progress {
    pending: usize,
    submitted: usize,
    waiting_since: Option<Instant>,
}

pub(crate) struct UploadState {
    progress: Mutex<Progress>,
    changed: Condvar,
    failure: DeviceFailure,
}

impl UploadState {
    fn release(&self, submitted: bool) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.pending -= 1;
            if submitted {
                progress.submitted -= 1;
                progress.waiting_since = None;
            }
        }
        self.changed.notify_all();
    }
}

pub struct UploadWindow {
    device: Arc<DeviceStatus>,
    capacity: usize,
    state: Arc<UploadState>,
}

impl UploadWindow {
    pub(crate) fn with_device(device: Arc<DeviceStatus>, capacity: usize) -> Self {
        let state = Arc::new(UploadState {
            progress: Mutex::new(Progress::default()),
            changed: Condvar::new(),
            failure: Arc::clone(&device.failure),
        });
        Self {
            device,
            capacity,
            state,
        }
    }

    /// Reserves staging capacity before taking the renderer lock or allocating GPU resources.
    pub fn acquire(
        self: &Arc<Self>,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<UploadPermit, String> {
        if self.capacity == 0 {
            return Err(String::from(
                "The GPU upload window must have at least one credit.",
            ));
        }
        loop {
            current()?;
            self.device.check()?;
            let mut progress = self
                .state
                .progress
                .lock()
                .map_err(|_| String::from("The GPU upload window is unavailable."))?;
            if progress.pending < self.capacity {
                progress.pending += 1;
                return Ok(UploadPermit {
                    device: self.device.device.clone(),
                    state: Some(Arc::clone(&self.state)),
                });
            }
            self.wait(progress, current)?;
        }
    }

    /// Waits for every reserved upload to complete; never call with the renderer locked.
    pub fn drain(&self, current: &impl Fn() -> Result<(), String>) -> Result<(), String> {
        loop {
            current()?;
            self.device.check()?;
            let progress = self
                .state
                .progress
                .lock()
                .map_err(|_| String::from("The GPU upload window is unavailable."))?;
            if progress.pending == 0 {
                return Ok(());
            }
            self.wait(progress, current)?;
        }
    }

    fn wait(
        &self,
        mut progress: std::sync::MutexGuard<'_, Progress>,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        if progress.submitted == 0 {
            // A producer owns a credit but has not submitted it yet.
            drop(
                self.state
                    .changed
                    .wait_timeout(progress, POLL_SLICE)
                    .map_err(|_| String::from("The GPU upload window is unavailable."))?,
            );
        } else {
            progress.waiting_since.get_or_insert_with(Instant::now);
            drop(progress);
            match self.device.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(POLL_SLICE),
            }) {
                Ok(_) => {}
                Err(wgpu::PollError::Timeout) => {
                    let progress = self
                        .state
                        .progress
                        .lock()
                        .map_err(|_| String::from("The GPU upload window is unavailable."))?;
                    let stalled = progress
                        .waiting_since
                        .is_some_and(|time| time.elapsed() >= STALL_TIMEOUT);
                    drop(progress);
                    if stalled {
                        fail(
                            &self.device.failure,
                            String::from("The graphics device stopped completing preview uploads."),
                        );
                    }
                }
                Err(error) => {
                    fail(
                        &self.device.failure,
                        format!("The graphics device could not complete an upload: {error}"),
                    );
                }
            }
        }
        current()?;
        self.device.check()
    }
}

/// An unused reservation returns on drop. Submitted credits return only from the GPU callback.
pub struct UploadPermit {
    device: wgpu::Device,
    state: Option<Arc<UploadState>>,
}

impl UploadPermit {
    pub(crate) fn check(&self, device: &wgpu::Device) -> Result<(), String> {
        if &self.device != device {
            return Err(String::from(
                "The upload credit belongs to another graphics device.",
            ));
        }
        check(&self.state.as_ref().unwrap().failure)
    }

    pub(crate) fn submit(mut self, queue: &wgpu::Queue) -> Result<(), String> {
        let state = Arc::clone(self.state.as_ref().unwrap());
        // Queue writes become submissions even when no frame is due.
        queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
        {
            let mut progress = state
                .progress
                .lock()
                .map_err(|_| String::from("The GPU upload window is unavailable."))?;
            progress.submitted += 1;
        }
        self.state.take();
        let completed = Arc::clone(&state);
        queue.on_submitted_work_done(move || completed.release(true));
        state.changed.notify_all();
        check(&state.failure)
    }
}

impl Drop for UploadPermit {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            state.release(false);
        }
    }
}
