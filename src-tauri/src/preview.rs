use std::fs::File;
use std::io::Read;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use litematica_preview_core::{MeshBatch, MeshStream, PreviewInfo, PreviewOptions};
use nucleation::formats::limits::DecodeLimits;
use nucleation::meshing::ResourcePackSource;
use serde::Serialize;

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

#[derive(Clone, Debug, Serialize)]
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

type Job = Box<dyn FnOnce(&mut Option<ResourcePackSource>) + Send>;

#[derive(Default)]
struct Schedule {
    pending: Option<Job>,
    running: bool,
    pack: Option<ResourcePackSource>,
}

#[derive(Default)]
pub struct PreviewWorker {
    generation: AtomicU64,
    active_request: AtomicU64,
    schedule: Mutex<Schedule>,
}

impl PreviewWorker {
    pub fn begin_session(&self) -> u64 {
        let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        let id = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        schedule.pending = None;
        self.active_request.store(0, Ordering::Release);
        id
    }

    /// Generation changes and scene mutations share this lock. An old worker can
    /// finish its native call, but cannot mutate a newer scene or emit its result.
    pub fn with_current<T>(
        &self,
        id: u64,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _schedule = self
            .schedule
            .lock()
            .map_err(|_| "The preview scheduler is unavailable.")?;
        self.ensure_current(id)?;
        catch_unwind(AssertUnwindSafe(action)).unwrap_or_else(|panic| Err(panic_message(panic)))
    }

    pub fn submit(
        self: &Arc<Self>,
        id: u64,
        prepare: impl FnOnce() -> Result<(), String>,
        job: Job,
    ) -> Result<(), String> {
        let mut schedule = self
            .schedule
            .lock()
            .map_err(|_| "The preview scheduler is unavailable.")?;
        if id <= self.generation.load(Ordering::Acquire) {
            return Err("Cancelled".into());
        }
        self.generation.store(id, Ordering::Release);
        schedule.pending = None;
        self.active_request.store(id, Ordering::Release);
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
    }

    fn run(&self) {
        let mut pack = self
            .schedule
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pack
            .take();
        loop {
            let job = {
                let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
                match schedule.pending.take() {
                    Some(job) => job,
                    None => {
                        schedule.pack = pack;
                        schedule.running = false;
                        return;
                    }
                }
            };
            job(&mut pack);
        }
    }

    pub fn cancel(
        &self,
        id: u64,
        clear: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut schedule = self
            .schedule
            .lock()
            .map_err(|_| "The preview scheduler is unavailable.")?;
        if id < self.generation.load(Ordering::Acquire) {
            return Ok(());
        }
        self.generation.store(id, Ordering::Release);
        self.active_request.store(0, Ordering::Release);
        schedule.pending = None;
        catch_unwind(AssertUnwindSafe(clear)).unwrap_or_else(|panic| Err(panic_message(panic)))
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
        let memory = host_memory();
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

/// The decoded source and cached pack outlive the borrowing stream on this
/// stack. Only the batch crosses into the renderer; no self-reference is stored.
pub fn load(
    path: &Path,
    pack_path: &Path,
    pack: &mut Option<ResourcePackSource>,
    options: PreviewOptions,
    current: impl Fn() -> Result<(), String>,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    mut consume: impl FnMut(MeshBatch) -> Result<(), String>,
    mut progress: impl FnMut(&'static str, u64, u64),
) -> Result<Metadata, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        options.validate()?;
        progress("decode", 0, 0);
        let data = read_file(path, &current)?;
        if pack.is_none() {
            let bytes = read_file(pack_path, &current)?;
            *pack = Some(
                ResourcePackSource::from_bytes(&bytes)
                    .map_err(|e| format!("The bundled block resources are invalid: {e}"))?,
            );
        }
        // Match Core's representation limits and independent NBT stack bound;
        // memory enforcement is deliberately suspended for this in-process path.
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
            pack.as_ref()
                .ok_or("The bundled block resources are unavailable.")?,
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
        *pack = None;
        Err(panic_message(panic))
    })
}

fn read_file(path: &Path, current: impl Fn() -> Result<(), String>) -> Result<Vec<u8>, String> {
    let mut file =
        File::open(path).map_err(|e| format!("Unable to open {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Unable to inspect {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err("Choose a schematic file, not a folder or device.".into());
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
        return Err("Choose a nonempty schematic.".into());
    }
    Ok(bytes)
}

#[cfg(windows)]
fn host_memory() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            std::mem::size_of_val(&counters) as u32,
        )
    };
    (ok != 0).then_some(counters.PrivateWorkingSetSize as u64)
}

#[cfg(not(windows))]
fn host_memory() -> Option<u64> {
    None
}
