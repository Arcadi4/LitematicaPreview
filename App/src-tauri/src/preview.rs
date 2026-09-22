use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use nucleation::meshing::ResourcePackSource;

use crate::preview_process::DecoderProcess;
use crate::protocol;

const MAX_INPUT_BYTES: usize = 1_024 * 1_024 * 1_024;

#[derive(Default)]
pub struct PreviewWorker {
    generation: AtomicU64,
    process: Mutex<Option<DecoderProcess>>,
    payload: Mutex<Option<(u64, protocol::Payload)>>,
}

impl PreviewWorker {
    pub fn begin_session(&self) -> u64 {
        let request_id = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.clear_older_payloads(request_id);
        request_id
    }

    pub fn advance(&self, request_id: u64) {
        self.generation.fetch_max(request_id, Ordering::AcqRel);
        self.clear_older_payloads(self.generation.load(Ordering::Acquire));
    }

    pub fn ensure_current(&self, request_id: u64) -> Result<(), String> {
        if self.generation.load(Ordering::Acquire) == request_id {
            Ok(())
        } else {
            Err("Cancelled".into())
        }
    }

    pub fn load(
        &self,
        path: &Path,
        pack_path: &Path,
        request_id: u64,
    ) -> Result<protocol::Metadata, String> {
        self.ensure_current(request_id)?;
        let mut process = self
            .process
            .lock()
            .map_err(|_| "The preview worker is unavailable. Restart the app.".to_string())?;
        self.ensure_current(request_id)?;
        if process.is_none() {
            *process = Some(DecoderProcess::spawn()?);
        }
        let result = process
            .as_mut()
            .ok_or("The preview worker is unavailable.")?
            .load(path, pack_path, || self.ensure_current(request_id).is_ok());
        let payload = match result {
            Ok(result) => result?,
            Err(error) => {
                process.take();
                return Err(error);
            }
        };
        self.publish(request_id, payload)
    }

    pub fn read(
        &self,
        request_id: u64,
        buffer_id: usize,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>, String> {
        let stored = self
            .payload
            .lock()
            .map_err(|_| "The preview buffers are unavailable.")?;
        self.ensure_current(request_id)?;
        let (_, payload) = stored
            .as_ref()
            .filter(|(id, _)| *id == request_id)
            .ok_or("The preview buffers have been released.")?;
        payload.read(buffer_id, offset, length)
    }

    pub fn release(&self, request_id: u64) {
        // A stale upload's finally block must never release the newer model.
        if let Ok(mut stored) = self.payload.lock() {
            if stored.as_ref().is_some_and(|(id, _)| *id == request_id) {
                stored.take();
            }
        }
    }

    fn clear_older_payloads(&self, request_id: u64) {
        if let Ok(mut stored) = self.payload.lock() {
            if stored.as_ref().is_some_and(|(id, _)| *id < request_id) {
                stored.take();
            }
        }
    }

    pub(crate) fn publish(
        &self,
        request_id: u64,
        payload: protocol::Payload,
    ) -> Result<protocol::Metadata, String> {
        let mut stored = self
            .payload
            .lock()
            .map_err(|_| "The preview buffers are unavailable.")?;
        self.ensure_current(request_id)?;
        let metadata = payload.metadata.clone();
        *stored = Some((request_id, payload));
        Ok(metadata)
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

// Runs only in the isolated decoder. Each chunk is sent synchronously and
// dropped before the next one; only the reusable resource pack survives loads.
pub(crate) fn decode(
    path: &Path,
    pack_path: &Path,
    pack: &mut Option<ResourcePackSource>,
    stream: &mut (impl Read + Write),
) -> Result<litematica_preview_native::PreviewInfo, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let encoder = RefCell::new(protocol::Encoder::new(stream));
        let current = || encoder.borrow_mut().checkpoint();
        let data = read_bounded(path, &current)?;
        if pack.is_none() {
            let bytes = read_bounded(pack_path, &current)?;
            *pack = Some(
                ResourcePackSource::from_bytes(&bytes)
                    .map_err(|e| format!("The bundled block resources are invalid: {e}"))?,
            );
        }
        litematica_preview_native::load_chunks(
            &data,
            pack.as_ref()
                .ok_or("The bundled block resources are unavailable.")?,
            |preview| encoder.borrow_mut().chunk(preview),
            current,
        )
    }));
    result.unwrap_or_else(|panic| {
        // Do not reuse native state that was being mutated during a panic.
        *pack = None;
        let detail = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("Unknown native panic");
        Err(format!(
            "The decoder encountered an internal error: {detail}"
        ))
    })
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
