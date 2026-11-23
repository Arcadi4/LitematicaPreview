//! One in-process decode worker. The thread owns the parsed pack cache, accepts
//! only the newest job, and cooperatively cancels any job a newer submission or
//! an explicit cancel has superseded. A panicking job is contained and the
//! cache is discarded, so the worker and the next request survive.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread;

use parking_lot::{Condvar, Mutex};

use crate::preview::PackCache;

/// Work accepted by [`Loader`]. The check must be polled by long-running work;
/// it returns `Err("Cancelled")` once a newer job or a cancel supersedes this one.
pub type Job = Box<dyn FnOnce(&mut PackCache, &dyn Fn() -> Result<(), String>) + Send + 'static>;

struct Queue {
    pending: Option<(u64, Job)>,
    stopped: bool,
}

struct Inner {
    generation: AtomicU64,
    queue: Mutex<Queue>,
    wake: Condvar,
}

pub struct Loader {
    inner: Arc<Inner>,
}

impl Loader {
    pub fn new() -> Self {
        let inner = Arc::new(Inner {
            generation: AtomicU64::new(0),
            queue: Mutex::new(Queue {
                pending: None,
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let worker = Arc::downgrade(&inner);
        thread::Builder::new()
            .name("preview-loader".into())
            .spawn(move || run(worker))
            .expect("Unable to start the preview loader");
        Self { inner }
    }

    /// Queues `job` as the only pending work. A job still queued from an older
    /// submission is dropped, and a running older job sees cancellation on its
    /// next check.
    pub fn submit(
        &self,
        job: impl FnOnce(&mut PackCache, &dyn Fn() -> Result<(), String>) + Send + 'static,
    ) {
        let mut queue = self.inner.queue.lock();
        // Assign the generation under the queue lock so two submitters cannot
        // leave an older id in the pending slot.
        let id = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        queue.pending = Some((id, Box::new(job)));
        drop(queue);
        self.inner.wake.notify_one();
    }

    /// Drops queued work and invalidates the running job. Finished cache
    /// entries survive, so the next job keeps the parsed packs warm.
    pub fn cancel(&self) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        let mut queue = self.inner.queue.lock();
        queue.pending = None;
        drop(queue);
        self.inner.wake.notify_one();
    }
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.inner.generation.fetch_add(1, Ordering::AcqRel);
        let mut queue = self.inner.queue.lock();
        queue.pending = None;
        queue.stopped = true;
        drop(queue);
        self.inner.wake.notify_all();
    }
}

fn run(worker: Weak<Inner>) {
    let mut packs = PackCache::default();
    loop {
        let Some(inner) = worker.upgrade() else {
            return;
        };
        let (id, job) = {
            let mut queue = inner.queue.lock();
            loop {
                if queue.stopped {
                    return;
                }
                if let Some(job) = queue.pending.take() {
                    break job;
                }
                inner.wake.wait(&mut queue);
            }
        };
        // A job replaced before the worker reached it is stale and never runs.
        if inner.generation.load(Ordering::Acquire) != id {
            continue;
        }
        let current = || {
            if inner.generation.load(Ordering::Acquire) == id {
                Ok(())
            } else {
                Err("Cancelled".to_string())
            }
        };
        if catch_unwind(AssertUnwindSafe(|| job(&mut packs, &current))).is_err() {
            packs = PackCache::default();
        }
    }
}
