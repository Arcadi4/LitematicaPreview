use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::sync::Mutex;

use litematica_preview_native::PreviewOptions;
use nucleation::meshing::ResourcePackSource;

use crate::preview_process::DecoderProcess;
use crate::protocol;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadOptions {
    #[serde(rename = "memoryLimitMB", deserialize_with = "required_nullable")]
    memory_limit_mb: Option<u16>,
    #[serde(deserialize_with = "required_nullable")]
    chunk_size: Option<u16>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer)
}

impl LoadOptions {
    pub fn validate(self) -> Result<PreviewOptions, String> {
        let options = PreviewOptions {
            memory_limit_mb: self.memory_limit_mb,
            chunk_size: self.chunk_size,
        };
        options.validate()?;
        Ok(options)
    }
}

#[derive(Default)]
pub struct PreviewWorker {
    generation: AtomicU64,
    active_request: AtomicU64,
    worker_pid: AtomicU32,
    process: Mutex<Option<DecoderProcess>>,
    payload: Mutex<Option<(u64, protocol::Payload)>>,
}

impl PreviewWorker {
    pub fn begin_session(&self) -> u64 {
        let request_id = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.active_request.store(0, Ordering::Release);
        self.clear_older_payloads(request_id);
        request_id
    }

    pub fn advance(&self, request_id: u64) {
        self.generation.fetch_max(request_id, Ordering::AcqRel);
        if self.active_request.load(Ordering::Acquire) != self.generation.load(Ordering::Acquire) {
            self.active_request.store(0, Ordering::Release);
        }
        self.clear_older_payloads(self.generation.load(Ordering::Acquire));
    }

    pub fn ensure_current(&self, request_id: u64) -> Result<(), String> {
        if self.generation.load(Ordering::Acquire) == request_id {
            Ok(())
        } else {
            Err("Cancelled".into())
        }
    }

    pub fn decoder_working_set(&self, request_id: u64) -> Option<u64> {
        if self.generation.load(Ordering::Acquire) != request_id
            || self.active_request.load(Ordering::Acquire) != request_id
        {
            return None;
        }
        let pid = self.worker_pid.load(Ordering::Acquire);
        if pid == 0 {
            return None;
        }
        let bytes = crate::preview_process::working_set(pid)?;
        (self.generation.load(Ordering::Acquire) == request_id
            && self.active_request.load(Ordering::Acquire) == request_id
            && self.worker_pid.load(Ordering::Acquire) == pid)
            .then_some(bytes)
    }

    pub fn load(
        &self,
        path: &Path,
        pack_path: &Path,
        request_id: u64,
        options: PreviewOptions,
        on_progress: impl FnMut(u64, u64),
    ) -> Result<protocol::Metadata, String> {
        options.validate()?;
        self.ensure_current(request_id)?;
        let mut process = self
            .process
            .lock()
            .map_err(|_| "The preview worker is unavailable. Restart the app.".to_string())?;
        self.ensure_current(request_id)?;
        if process
            .as_ref()
            .is_some_and(|process| process.memory_limit_mb() != options.memory_limit_mb)
        {
            self.active_request.store(0, Ordering::Release);
            self.worker_pid.store(0, Ordering::Release);
            process.take();
        }
        if process.is_none() {
            *process = Some(DecoderProcess::spawn(options.memory_limit_mb)?);
        }
        self.worker_pid
            .store(process.as_ref().unwrap().pid(), Ordering::Release);
        self.ensure_current(request_id)?;
        self.active_request.store(request_id, Ordering::Release);
        let result = process
            .as_mut()
            .ok_or("The preview worker is unavailable.")?
            .load(
                path,
                pack_path,
                options.chunk_size,
                || self.ensure_current(request_id).is_ok(),
                on_progress,
            );
        let payload = match result {
            Ok(Err(error)) => {
                self.active_request.store(0, Ordering::Release);
                return Err(error);
            }
            Ok(Ok(payload)) => payload,
            Err(error) => {
                self.active_request.store(0, Ordering::Release);
                self.worker_pid.store(0, Ordering::Release);
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
        if self.active_request.load(Ordering::Acquire) == request_id {
            self.active_request.store(0, Ordering::Release);
        }
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
    use super::{read_bytes, LoadOptions, PreviewOptions, PreviewWorker};

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

    fn options(value: serde_json::Value) -> Result<PreviewOptions, String> {
        serde_json::from_value::<LoadOptions>(value)
            .map_err(|error| error.to_string())?
            .validate()
    }

    #[test]
    fn preview_options_require_explicit_nullable_integer_settings() {
        use serde_json::json;

        assert_eq!(
            options(json!({"memoryLimitMB": 2048, "chunkSize": 64})).unwrap(),
            PreviewOptions {
                memory_limit_mb: Some(2048),
                ..PreviewOptions::default()
            }
        );
        assert_eq!(
            options(json!({"memoryLimitMB": null, "chunkSize": null})).unwrap(),
            PreviewOptions {
                memory_limit_mb: None,
                chunk_size: None,
            }
        );
        for value in [
            json!({}),
            json!({"memoryLimitMB": 2048}),
            json!({"chunkSize": 64}),
            json!({"memoryLimitGiB": 2, "chunkSize": 64}),
            json!({"memoryLimitMB": 2047, "chunkSize": 64}),
            json!({"memoryLimitMB": 8193, "chunkSize": 64}),
            json!({"memoryLimitMB": 2048.5, "chunkSize": 64}),
            json!({"memoryLimitMB": "2048", "chunkSize": 64}),
            json!({"memoryLimitMB": 2048, "chunkSize": 48}),
            json!({"memoryLimitMB": 2048, "chunkSize": 64.5}),
            json!({"memoryLimitMB": 2048, "chunkSize": 64, "extra": true}),
        ] {
            assert!(options(value.clone()).is_err(), "accepted {value}");
        }
        for mb in [2048, 3072, 4096, 5120, 6144, 7168, 8192] {
            for size in [16, 32, 64, 128, 256] {
                assert_eq!(
                    options(json!({"memoryLimitMB": mb, "chunkSize": size})).unwrap(),
                    PreviewOptions {
                        memory_limit_mb: Some(mb),
                        chunk_size: Some(size),
                    }
                );
            }
        }
    }

    #[test]
    fn file_reads_follow_actual_bytes_instead_of_declared_file_quota() {
        let bytes = [1, 2, 3];
        assert_eq!(
            read_bytes(&bytes[..], 1024 * 1024 * 1024 + 1, || Ok(())).unwrap(),
            bytes
        );
        let error = read_bytes(&bytes[..], 1, || Err("Cancelled".into())).unwrap_err();
        assert_eq!(error, "Cancelled");
        assert!(read_bytes(&[][..], 3, || Ok(())).is_err());
        assert!(read_bytes(&bytes[..], u64::MAX, || Ok(())).is_err());
    }
}

// Runs only in the isolated decoder. Each chunk is sent synchronously and
// dropped before the next one; only the reusable resource pack survives loads.
pub(crate) fn decode(
    path: &Path,
    pack_path: &Path,
    pack: &mut Option<ResourcePackSource>,
    options: PreviewOptions,
    stream: &mut (impl Read + Write),
) -> Result<litematica_preview_native::PreviewInfo, String> {
    options.validate()?;
    let result = catch_unwind(AssertUnwindSafe(|| {
        let encoder = RefCell::new(protocol::Encoder::new(stream));
        let mut last_progress = None;
        let current = || encoder.borrow_mut().checkpoint();
        let data = read_file(path, &current)?;
        if pack.is_none() {
            let bytes = read_file(pack_path, &current)?;
            *pack = Some(
                ResourcePackSource::from_bytes(&bytes)
                    .map_err(|e| format!("The bundled block resources are invalid: {e}"))?,
            );
        }
        litematica_preview_native::load_chunks(
            &data,
            pack.as_ref()
                .ok_or("The bundled block resources are unavailable.")?,
            options,
            |preview| encoder.borrow_mut().chunk(preview),
            |completed, total| {
                let now = Instant::now();
                if completed == 0
                    || completed == total
                    || last_progress
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(150))
                {
                    encoder
                        .borrow_mut()
                        .progress(completed as u64, total as u64)?;
                    last_progress = Some(now);
                }
                Ok(())
            },
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

fn read_file(path: &Path, current: impl Fn() -> Result<(), String>) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("Unable to open {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Unable to inspect {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err("Choose a schematic file, not a folder or device.".into());
    }
    read_bytes(file, metadata.len(), current)
}

fn read_bytes(
    mut reader: impl Read,
    declared_length: u64,
    current: impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    if declared_length == 0 {
        return Err("Choose a nonempty schematic.".into());
    }
    usize::try_from(declared_length)
        .ok()
        .filter(|length| *length <= isize::MAX as usize)
        .ok_or("The file exceeds this platform's addressable memory.")?;
    // Grow from bytes actually read, not potentially stale or sparse-file metadata.
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1_024];
    loop {
        current()?;
        let count = match reader.read(&mut chunk) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Unable to read the file: {error}")),
        };
        if count == 0 {
            break;
        }
        bytes
            .try_reserve(count)
            .map_err(|_| "There is not enough memory to read this file.".to_string())?;
        bytes.extend_from_slice(&chunk[..count]);
    }
    if bytes.is_empty() {
        return Err("Choose a nonempty schematic.".into());
    }
    Ok(bytes)
}
