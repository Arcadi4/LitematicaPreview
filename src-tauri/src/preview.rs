use std::fs::File;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use litematica_preview_core::{MeshBatch, MeshStream, PreviewInfo, PreviewOptions};
use nucleation::formats::limits::DecodeLimits;
use nucleation::meshing::ResourcePackSource;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadOptions {
    #[serde(deserialize_with = "required_nullable")]
    chunk_size: Option<u16>,
    #[serde(deserialize_with = "required_nullable")]
    thread_count: Option<u8>,
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
            memory_limit_mb: None,
            chunk_size: self.chunk_size,
            thread_count: self.thread_count,
            speed_first: self.thread_count.is_some(),
        };
        options.validate()?;
        Ok(options)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub block_count: i64,
    pub block_entity_count: i64,
    pub triangle_count: u64,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub geometry_byte_length: usize,
    pub byte_length: usize,
    pub texture_count: u32,
    pub part_count: u32,
}

impl Metadata {
    pub fn info(&self) -> PreviewInfo {
        PreviewInfo {
            block_count: self.block_count,
            block_entity_count: self.block_entity_count,
            triangle_count: self.triangle_count,
            min: self.min,
            max: self.max,
            texture_count: self.texture_count,
            part_count: self.part_count,
        }
    }
}

type Job = Box<dyn FnOnce(&mut Option<crate::preview_process::DecoderProcess>) + Send>;

#[derive(Default)]
struct Schedule {
    pending: Option<Job>,
    running: bool,
    decoder: Option<crate::preview_process::DecoderProcess>,
    control: Option<Arc<crate::preview_process::DecoderControl>>,
}

#[derive(Default)]
pub struct PreviewWorker {
    generation: AtomicU64,
    active_request: AtomicU64,
    schedule: Mutex<Schedule>,
}

impl PreviewWorker {
    pub fn begin_session(&self) -> u64 {
        let (id, control, decoder) = {
            let mut schedule = self.schedule.lock();
            let id = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
            schedule.pending = None;
            self.active_request.store(0, Ordering::Release);
            let control = schedule.control.take();
            if let Some(control) = &control {
                control.invalidate();
            }
            (id, control, schedule.decoder.take())
        };
        if let Some(control) = control {
            control.stop();
        }
        drop(decoder);
        id
    }

    /// Serializes generation changes with scene mutations and result emission.
    pub fn with_current<T>(
        &self,
        id: u64,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _schedule = self.schedule.lock();
        self.ensure_current(id)?;
        catch_unwind(AssertUnwindSafe(action)).unwrap_or_else(|panic| Err(panic_message(panic)))
    }

    pub fn submit(
        self: &Arc<Self>,
        id: u64,
        prepare: impl FnOnce() -> Result<(), String>,
        job: Job,
    ) -> Result<(), String> {
        let mut interrupted = None;
        let result = (|| {
            let mut schedule = self.schedule.lock();
            if id <= self.generation.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            self.generation.store(id, Ordering::Release);
            schedule.pending = None;
            if self.active_request.swap(id, Ordering::AcqRel) != 0 {
                interrupted = schedule.control.take();
                if let Some(control) = &interrupted {
                    control.invalidate();
                }
            }
            if let Err(error) = catch_unwind(AssertUnwindSafe(prepare))
                .unwrap_or_else(|panic| Err(panic_message(panic)))
            {
                self.active_request.store(0, Ordering::Release);
                return Err(error);
            }
            schedule.pending = Some(job);
            if !schedule.running {
                schedule.running = true;
                let worker = Arc::clone(self);
                if let Err(error) = std::thread::Builder::new()
                    .name("preview-session".into())
                    .spawn(move || worker.run())
                {
                    schedule.pending = None;
                    schedule.running = false;
                    self.active_request.store(0, Ordering::Release);
                    return Err(format!("Unable to start the preview worker: {error}"));
                }
            }
            Ok(())
        })();
        if let Some(control) = interrupted {
            control.stop();
        }
        result
    }

    fn run(&self) {
        let mut decoder = self.schedule.lock().decoder.take();
        loop {
            let job = {
                let mut schedule = self.schedule.lock();
                match schedule.pending.take() {
                    Some(job) => Some(job),
                    None => {
                        if schedule.control.is_some() {
                            schedule.decoder = decoder.take();
                        }
                        schedule.running = false;
                        None
                    }
                }
            };
            let Some(job) = job else {
                return;
            };
            job(&mut decoder);
        }
    }

    pub(crate) fn register_decoder(
        &self,
        id: u64,
        control: Arc<crate::preview_process::DecoderControl>,
    ) -> Result<(), String> {
        let mut schedule = self.schedule.lock();
        self.ensure_current(id)?;
        schedule.control = Some(control);
        Ok(())
    }

    pub fn cancel(
        &self,
        id: u64,
        clear: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let (result, control, decoder) = {
            let mut schedule = self.schedule.lock();
            if id < self.generation.load(Ordering::Acquire) {
                return Ok(());
            }
            self.generation.store(id, Ordering::Release);
            self.active_request.store(0, Ordering::Release);
            schedule.pending = None;
            let result = catch_unwind(AssertUnwindSafe(clear))
                .unwrap_or_else(|panic| Err(panic_message(panic)));
            let control = schedule.control.take();
            if let Some(control) = &control {
                control.invalidate();
            }
            (result, control, schedule.decoder.take())
        };
        if let Some(control) = control {
            control.stop();
        }
        drop(decoder);
        result
    }

    pub fn ensure_current(&self, id: u64) -> Result<(), String> {
        if self.generation.load(Ordering::Acquire) == id {
            Ok(())
        } else {
            Err("Cancelled".into())
        }
    }

    pub fn release(&self, id: u64) {
        let _ = self
            .active_request
            .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn preview_memory(&self, id: u64) -> Option<u64> {
        if self.active_request.load(Ordering::Acquire) != id || self.ensure_current(id).is_err() {
            return None;
        }
        let control = self.schedule.lock().control.clone();
        let memory = crate::preview_process::preview_memory(control.as_deref());
        (self.active_request.load(Ordering::Acquire) == id && self.ensure_current(id).is_ok())
            .then_some(memory)
            .flatten()
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    let detail = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("Unknown native panic");
    format!("The preview worker encountered an internal error: {detail}")
}

/// Parsed packs reused across decoder requests. The overlay rebuilds when
/// the selected archive changes.
#[derive(Default)]
pub struct PackCache {
    base: Option<ResourcePackSource>,
    overlay: Option<(PathBuf, ResourcePackSource)>,
}

impl PackCache {
    /// Seeds the cache with an already parsed base pack so a caller can keep
    /// that parse outside a measured window.
    #[allow(dead_code)] // The pressure binary compiles this module on its own.
    pub fn with_base(base: ResourcePackSource) -> Self {
        Self {
            base: Some(base),
            overlay: None,
        }
    }

    fn resolve(
        &mut self,
        base_path: &Path,
        custom_path: Option<&Path>,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<&ResourcePackSource, String> {
        if self.base.is_none() {
            let bytes = read_resource_pack(base_path, current)?;
            self.base =
                Some(ResourcePackSource::from_bytes(&bytes).map_err(|error| {
                    format!("The bundled block resources are invalid: {error}")
                })?);
        }
        let Some(custom_path) = custom_path else {
            self.overlay = None;
            return Ok(self
                .base
                .as_ref()
                .ok_or("The bundled block resources are unavailable.")?);
        };
        if self
            .overlay
            .as_ref()
            .is_none_or(|(cached, _)| cached != custom_path)
        {
            let bytes = read_resource_pack(custom_path, current)?;
            let merged = litematica_preview_core::resource_pack_overlay(
                self.base
                    .as_ref()
                    .ok_or("The bundled block resources are unavailable.")?,
                &bytes,
            )
            .map_err(|error| format!("The selected texture pack could not be used: {error}"))?;
            self.overlay = Some((custom_path.to_path_buf(), merged));
        }
        Ok(&self
            .overlay
            .as_ref()
            .ok_or("The selected texture pack is unavailable.")?
            .1)
    }
}

/// The decoded source and cached packs outlive the borrowing mesh stream.
pub fn decode(
    path: &Path,
    pack_path: &Path,
    custom_pack_path: Option<&Path>,
    packs: &mut PackCache,
    options: PreviewOptions,
    current: impl Fn() -> Result<(), String>,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    mut consume: impl FnMut(MeshBatch) -> Result<(), String>,
    mut progress: impl FnMut(&'static str, u64, u64),
) -> Result<Metadata, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        options.validate()?;
        progress("decode", 0, 0);
        let data = read_file(path, "schematic", &current)?;
        let pack = packs.resolve(pack_path, custom_pack_path, &current)?;
        // Representation bounds are independent of the Windows job's hard cap.
        let limits = DecodeLimits {
            max_input_bytes: isize::MAX as usize,
            max_decompressed_bytes: isize::MAX as usize,
            max_dimension: i32::MAX as usize,
            max_volume: isize::MAX as usize / std::mem::size_of::<usize>(),
            max_regions: usize::MAX,
            max_palette_entries: usize::MAX,
            max_entities: usize::MAX,
            max_block_entities: usize::MAX,
            max_nbt_depth: 64,
            max_nbt_string_bytes: isize::MAX as usize,
            max_nbt_collection_items: usize::MAX,
            max_nbt_nodes: usize::MAX,
        };
        let source =
            litematica_preview_core::decode_with_options(&data, &limits, options, &current)
                .map_err(|e| e.to_string())?;
        drop(data);
        if source.block_count() == 0 {
            return Err("The schematic must contain at least one block.".into());
        }
        let origin = source.content_origin();
        let dimensions = source.content_dimensions();
        let min = [
            origin.0 as f32 - 0.5,
            origin.1 as f32 - 0.5,
            origin.2 as f32 - 0.5,
        ];
        let max = [
            min[0] + dimensions.0 as f32,
            min[1] + dimensions.1 as f32,
            min[2] + dimensions.2 as f32,
        ];
        prepared(min, max)?;
        let mut stream = MeshStream::new(
            &source,
            pack,
            &litematica_preview_core::mesh_config(),
            usize::from(options.thread_count.unwrap_or(1)),
            &current,
        )?;
        let total = stream.batch_count() as u64;
        progress("mesh", 0, total);
        let mut atlas = Some(stream.take_atlas());
        let mut metadata = Metadata {
            block_count: source.block_count(),
            block_entity_count: source.block_entity_count(),
            triangle_count: 0,
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
            geometry_byte_length: 0,
            byte_length: 0,
            texture_count: 0,
            part_count: 0,
        };
        let mut completed = 0;
        let mut last_progress = Instant::now();
        while let Some(mut batch) = stream.next(&current)? {
            if let Some(texture) = atlas.take() {
                batch.textures.insert(0, texture);
            }
            metadata.triangle_count += batch.info.triangle_count;
            metadata.part_count += batch.info.part_count;
            metadata.texture_count = batch.info.texture_count;
            metadata.geometry_byte_length += batch.geometry.len();
            metadata.byte_length += batch.geometry.len()
                + batch
                    .textures
                    .iter()
                    .map(|texture| texture.pixels.len())
                    .sum::<usize>();
            if !batch.parts.is_empty() {
                for axis in 0..3 {
                    metadata.min[axis] = metadata.min[axis].min(batch.info.min[axis]);
                    metadata.max[axis] = metadata.max[axis].max(batch.info.max[axis]);
                }
            }
            consume(batch)?;
            completed += 1;
            if completed == total || last_progress.elapsed() >= Duration::from_millis(150) {
                progress("mesh", completed, total);
                last_progress = Instant::now();
            }
        }
        current()?;
        if metadata.part_count == 0 {
            return Err("The schematic contains no visible geometry.".into());
        }
        progress("upload", completed, total);
        Ok(metadata)
    }));
    result.unwrap_or_else(|panic| {
        *packs = PackCache::default();
        Err(panic_message(panic))
    })
}

fn read_resource_pack(
    path: &Path,
    current: impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    const LIMIT: u64 = 256 * 1024 * 1024;
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
    if metadata.len() > LIMIT {
        return Err("Choose a resource pack smaller than 256 MB.".into());
    }
    read_file(path, "resource pack", current)
}

fn read_file(
    path: &Path,
    kind: &str,
    current: impl Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let mut file =
        File::open(path).map_err(|e| format!("Unable to open {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Unable to inspect {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("Choose a {kind} file, not a folder or device."));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        current()?;
        let count = match file.read(&mut chunk) {
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
        return Err(format!("Choose a nonempty {kind}."));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn queued_reload_keeps_only_latest_and_never_overlaps_jobs() {
        let worker = Arc::new(PreviewWorker::default());
        let (started_tx, started) = mpsc::channel();
        let (resume_tx, resume) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let first_worker = Arc::clone(&worker);
        worker
            .submit(
                1,
                || Ok(()),
                Box::new(move |_| {
                    started_tx.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(5)).unwrap();
                    assert_eq!(first_worker.ensure_current(1).unwrap_err(), "Cancelled");
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let obsolete = done_tx.clone();
        worker
            .submit(
                2,
                || Ok(()),
                Box::new(move |_| {
                    obsolete.send(2).unwrap();
                }),
            )
            .unwrap();
        worker
            .submit(
                3,
                || Ok(()),
                Box::new(move |_| {
                    done_tx.send(3).unwrap();
                }),
            )
            .unwrap();
        assert!(done.try_recv().is_err());
        resume_tx.send(()).unwrap();
        assert_eq!(done.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
        assert!(done.recv_timeout(Duration::from_millis(20)).is_err());
    }

    #[test]
    fn cancelled_and_delayed_requests_cannot_modify_the_current_scene() {
        let worker = Arc::new(PreviewWorker::default());
        worker.cancel(4, || Ok(())).unwrap();
        assert_eq!(
            worker
                .with_current::<()>(3, || panic!("stale scene mutation"))
                .unwrap_err(),
            "Cancelled"
        );
        worker.cancel(2, || panic!("stale cancel")).unwrap();
        assert_eq!(
            worker
                .submit(
                    3,
                    || panic!("stale prepare"),
                    Box::new(|_| panic!("stale job"))
                )
                .unwrap_err(),
            "Cancelled"
        );
        assert!(worker.ensure_current(4).is_ok());
    }

    #[test]
    fn caught_native_panic_does_not_poison_generation_recovery() {
        let worker = PreviewWorker::default();
        worker.cancel(1, || Ok(())).unwrap();
        assert!(worker
            .with_current::<()>(1, || panic!("native failure"))
            .unwrap_err()
            .contains("native failure"));
        worker.cancel(2, || Ok(())).unwrap();
        assert_eq!(worker.with_current(2, || Ok(42)).unwrap(), 42);
    }

    #[test]
    fn cancel_discards_queued_work_and_allows_a_new_session_to_recover() {
        let worker = Arc::new(PreviewWorker::default());
        let (started_tx, started) = mpsc::channel();
        let (resume_tx, resume) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let active_worker = Arc::clone(&worker);
        worker
            .submit(
                1,
                || Ok(()),
                Box::new(move |_| {
                    started_tx.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(5)).unwrap();
                    assert_eq!(active_worker.ensure_current(1).unwrap_err(), "Cancelled");
                }),
            )
            .unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let obsolete_tx = done_tx.clone();
        worker
            .submit(
                2,
                || Ok(()),
                Box::new(move |_| {
                    obsolete_tx.send(2).unwrap();
                }),
            )
            .unwrap();
        let mut cleared = false;
        worker
            .cancel(3, || {
                cleared = true;
                Ok(())
            })
            .unwrap();
        assert!(cleared);
        worker
            .submit(
                4,
                || Ok(()),
                Box::new(move |_| {
                    done_tx.send(4).unwrap();
                }),
            )
            .unwrap();
        resume_tx.send(()).unwrap();
        assert_eq!(done.recv_timeout(Duration::from_secs(5)).unwrap(), 4);
        assert!(done.recv_timeout(Duration::from_millis(20)).is_err());
    }

    #[test]
    fn failed_prepare_does_not_start_its_job_or_block_the_next_request() {
        let worker = Arc::new(PreviewWorker::default());
        assert_eq!(
            worker
                .submit(
                    1,
                    || Err("viewport failure".into()),
                    Box::new(|_| {
                        panic!("failed preparation must not start a decoder");
                    })
                )
                .unwrap_err(),
            "viewport failure"
        );
        let (done_tx, done) = mpsc::channel();
        worker
            .submit(
                2,
                || Ok(()),
                Box::new(move |_| {
                    done_tx.send(()).unwrap();
                }),
            )
            .unwrap();
        done.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn selected_pack_decodes_and_meshes_a_fixture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let pack = root.join("../Assets/pack.zip");
        let schematic = root.join("../Fixtures/Formats/Sponge.schem");
        let mut packs = PackCache::default();
        let mut batches = 0;
        let metadata = decode(
            &schematic,
            &pack,
            Some(&pack),
            &mut packs,
            PreviewOptions {
                chunk_size: Some(32),
                thread_count: Some(2),
                ..PreviewOptions::default()
            },
            || Ok(()),
            |_, _| Ok(()),
            |_| {
                batches += 1;
                Ok(())
            },
            |_, _, _| {},
        )
        .expect("preview with the selected pack");
        assert!(batches > 0);
        assert!(metadata.part_count > 0);
    }

    #[test]
    fn selected_pack_is_validated_against_the_bundled_pack() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../Assets/pack.zip");
        let custom = std::env::temp_dir().join(format!(
            "litematica-preview-selected-pack-{}.zip",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&custom, b"not a zip").unwrap();
        let mut packs = PackCache::default();
        let current = || Ok(());
        assert!(packs.resolve(&base, None, &current).is_ok());
        let error = match packs.resolve(&base, Some(&custom), &current) {
            Err(error) => error,
            Ok(_) => panic!("a malformed selected pack must not be accepted"),
        };
        assert!(error.contains("selected texture pack"), "{error}");
        std::fs::remove_file(&custom).unwrap();
    }

    #[test]
    fn invalid_worker_and_chunk_combinations_are_rejected() {
        for value in [
            serde_json::json!({"chunkSize": null, "threadCount": 2}),
            serde_json::json!({"chunkSize": 48, "threadCount": null}),
            serde_json::json!({"chunkSize": 64, "threadCount": 1}),
        ] {
            assert!(serde_json::from_value::<LoadOptions>(value)
                .unwrap()
                .validate()
                .is_err());
        }
    }
}
