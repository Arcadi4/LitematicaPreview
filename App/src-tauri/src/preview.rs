use std::fs::File;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use nucleation::meshing::ResourcePackSource;

use crate::protocol;

const MAX_INPUT_BYTES: usize = 1_024 * 1_024 * 1_024;

#[derive(Default)]
pub struct PreviewWorker {
    generation: AtomicU64,
    pack: Mutex<Option<ResourcePackSource>>,
}

impl PreviewWorker {
    pub fn begin_session(&self) -> u64 {
        // A WebView reload replaces JS state but not this worker. Invalidate
        // previous-session work and give the new frontend its request baseline.
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn advance(&self, request_id: u64) {
        // IPC arrival order is not guaranteed: an older load/cancel cannot
        // replace a newer generation, even while waiting for the worker.
        self.generation.fetch_max(request_id, Ordering::AcqRel);
    }

    pub fn ensure_current(&self, request_id: u64) -> Result<(), String> {
        if self.generation.load(Ordering::Acquire) == request_id {
            Ok(())
        } else {
            Err("Cancelled".into())
        }
    }

    pub fn load(&self, path: &Path, pack_path: &Path, request_id: u64) -> Result<Vec<u8>, String> {
        self.ensure_current(request_id)?;
        let mut pack = self
            .pack
            .lock()
            .map_err(|_| "The preview worker is unavailable. Restart the app.".to_string())?;
        self.ensure_current(request_id)?;

        // Catch inside the guard's lifetime: recoverable decoder/mesher panics
        // do not unwind through the mutex or poison the cached resource pack.
        let result = catch_unwind(AssertUnwindSafe(|| {
            let data = read_bounded(path, || self.ensure_current(request_id))?;
            if pack.is_none() {
                let bytes = read_bounded(pack_path, || self.ensure_current(request_id))?;
                let loaded = ResourcePackSource::from_bytes(&bytes)
                    .map_err(|e| format!("The bundled block resources are invalid: {e}"))?;
                *pack = Some(loaded);
            }
            self.ensure_current(request_id)?;
            let preview = litematica_preview_native::load(
                &data,
                pack.as_ref()
                    .ok_or("The bundled block resources are unavailable.")?,
            )?;
            drop(data);
            self.ensure_current(request_id)?;
            let bytes = protocol::serialize(&preview, || self.ensure_current(request_id))?;
            self.ensure_current(request_id)?;
            Ok(bytes)
        }));
        self.ensure_current(request_id)?;
        result.unwrap_or_else(|_| {
            Err(
                "The decoder could not preview this schematic. Try a smaller or different file."
                    .into(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PreviewWorker;

    #[test]
    fn reload_and_delayed_requests_cannot_revive_stale_work() {
        let worker = PreviewWorker::default();
        worker.advance(20);
        worker.advance(19);
        assert!(worker.ensure_current(20).is_ok());
        assert!(worker.ensure_current(19).is_err());

        let reloaded = worker.begin_session();
        assert!(worker.ensure_current(20).is_err());
        worker.advance(reloaded + 1);
        worker.advance(20);
        assert!(worker.ensure_current(reloaded + 1).is_ok());
        assert!(worker.ensure_current(reloaded).is_err());
    }
}

fn read_bounded(path: &Path, current: impl Fn() -> Result<(), String>) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("Unable to open {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Unable to inspect {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err("Choose a schematic file, not a folder or device.".into());
    }
    if metadata.len() == 0 || metadata.len() > MAX_INPUT_BYTES as u64 {
        return Err("Choose a nonempty schematic no larger than 1 GiB.".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| "There is not enough memory to read this file.".to_string())?;
    // The handle, not a prior path stat, owns the size check. Still cap actual
    // reads: another process can grow the file after metadata was inspected.
    let mut reader = file.take(MAX_INPUT_BYTES as u64 + 1);
    let mut chunk = [0u8; 64 * 1_024];
    loop {
        current()?;
        let count = match reader.read(&mut chunk) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Unable to read {}: {error}", path.display())),
        };
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_INPUT_BYTES {
            return Err("Choose a schematic no larger than 1 GiB.".into());
        }
        bytes
            .try_reserve_exact(count)
            .map_err(|_| "There is not enough memory to read this file.".to_string())?;
        bytes.extend_from_slice(&chunk[..count]);
    }
    if bytes.is_empty() {
        return Err("Choose a nonempty schematic.".into());
    }
    Ok(bytes)
}
