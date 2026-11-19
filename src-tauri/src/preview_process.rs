use std::cell::RefCell;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};

use litematica_preview_core::{MeshBatch, MeshPart, PreviewOptions, Texture};
use parking_lot::Mutex;

use crate::preview::{Metadata, PackLayer, PreviewRequest, PreviewWorker};
use crate::preview_protocol::{self as wire, Event, Purpose};

const TOKEN_BYTES: usize = 32;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Only upload-ready batches from admitted small models are retained, and the
/// whole host-side cache is capped independently of how navigation walks files.
const PRELOAD_FILE_BYTES: usize = 8 * 1024 * 1024;
const PRELOAD_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const PRELOAD_MAX_ENTRIES: usize = 6;
#[cfg(windows)]
const DECODER_MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Terminal result of one decoder stream, kept separate from transport errors
/// so a speculative admission rejection is not reported as a failure.
enum LoadOutcome {
    Finished(Result<Metadata, String>),
    Skipped(String),
}

struct ProcessState {
    child: Option<Child>,
    input: Option<ChildStdin>,
    stream: Option<TcpStream>,
}

pub(crate) struct DecoderControl {
    stopped: AtomicBool,
    state: Mutex<ProcessState>,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl DecoderControl {
    pub(crate) fn invalidate(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    /// Interrupts native work and IPC without waiting for process teardown.
    pub(crate) fn stop(&self) {
        self.invalidate();
        let (input, stream) = {
            let mut state = self.state.lock();
            if let Some(child) = state.child.as_mut() {
                let _ = child.kill();
            }
            (state.input.take(), state.stream.take())
        };
        drop(input);
        if let Some(stream) = stream {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn connect(&self, stream: &TcpStream) -> io::Result<()> {
        let stream = stream.try_clone()?;
        let mut state = self.state.lock();
        if self.stopped.load(Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }
        state.stream = Some(stream);
        Ok(())
    }

    fn exited(&self) -> Result<Option<std::process::ExitStatus>, String> {
        self.state
            .lock()
            .child
            .as_mut()
            .ok_or_else(|| "The decoder process is unavailable.".to_string())?
            .try_wait()
            .map_err(|e| format!("Unable to inspect the decoder process: {e}"))
    }

    #[cfg(windows)]
    fn memory(&self) -> Option<u64> {
        use std::os::windows::io::AsRawHandle;
        let state = self.state.lock();
        let child = state.child.as_ref()?;
        private_working_set(child.as_raw_handle())
    }
}

pub struct DecoderProcess {
    control: Arc<DecoderControl>,
    stream: Option<BufReader<TcpStream>>,
}

impl DecoderProcess {
    fn spawn(
        register: impl FnOnce(Arc<DecoderControl>) -> Result<(), String>,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Unable to create the decoder connection: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = authentication_token()?;
        let executable = std::env::current_exe()
            .map_err(|e| format!("Unable to locate the decoder executable: {e}"))?;
        let mut command = Command::new(executable);
        command
            .arg("--preview-worker")
            .arg(port.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        #[cfg(windows)]
        let job = decoder_job()?;
        let mut child = command
            .spawn()
            .map_err(|e| format!("Unable to start the decoder process: {e}"))?;
        let input = child.stdin.take();
        let mut process = Self {
            control: Arc::new(DecoderControl {
                stopped: AtomicBool::new(false),
                state: Mutex::new(ProcessState {
                    child: Some(child),
                    input,
                    stream: None,
                }),
                #[cfg(windows)]
                _job: job,
            }),
            stream: None,
        };
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
            let state = process.control.state.lock();
            let child = state
                .child
                .as_ref()
                .ok_or("The decoder process is unavailable.")?;
            if unsafe {
                AssignProcessToJobObject(
                    process.control._job.as_raw_handle(),
                    child.as_raw_handle(),
                )
            } == 0
            {
                return Err(format!(
                    "Unable to isolate the decoder process: {}",
                    io::Error::last_os_error()
                ));
            }
        }
        register(Arc::clone(&process.control))?;
        // The bounded authentication write fits in the empty inherited pipe.
        let mut input = process
            .control
            .state
            .lock()
            .input
            .take()
            .ok_or("The decoder input pipe is unavailable.")?;
        let authenticated = input.write_all(&token);
        {
            let mut state = process.control.state.lock();
            if !process.control.stopped.load(Ordering::Acquire) {
                state.input = Some(input);
            }
        }
        authenticated.map_err(|e| format!("Unable to authenticate the decoder: {e}"))?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if process.control.stopped.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            if let Some(status) = process.control.exited()? {
                return Err(format!(
                    "The decoder process stopped during startup ({status})."
                ));
            }
            if Instant::now() >= deadline {
                return Err("The decoder process did not connect within ten seconds.".into());
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Darwin inherits the listener's nonblocking mode. Reads
                    // must block between streamed frames until cancellation
                    // shuts the owned socket down.
                    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                    process
                        .control
                        .connect(&stream)
                        .map_err(|e| e.to_string())?;
                    stream
                        .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
                        .map_err(|e| e.to_string())?;
                    let mut received = [0; TOKEN_BYTES];
                    if stream.read_exact(&mut received).is_err() || received != token {
                        continue;
                    }
                    stream
                        .set_read_timeout(None)
                        .and_then(|()| stream.set_nodelay(true))
                        .map_err(|e| e.to_string())?;
                    process.stream = Some(BufReader::new(stream));
                    return Ok(process);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(format!("Unable to connect to the decoder: {error}")),
            }
        }
    }

    fn load(
        &mut self,
        request: PreviewRequest<'_>,
        purpose: Purpose,
        current: impl Fn() -> Result<(), String>,
        prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
        mut consume: impl FnMut(MeshBatch) -> Result<(), String>,
        mut progress: impl FnMut(&'static str, u64, u64),
    ) -> Result<LoadOutcome, String> {
        let PreviewRequest {
            path,
            pack_path,
            layers,
            options,
            max_texture_side,
        } = request;
        let request = wire::Request {
            path: path.to_path_buf(),
            pack_path: pack_path.to_path_buf(),
            layers: layers.to_vec(),
            chunk_size: options.chunk_size,
            thread_count: options.thread_count,
            max_texture_side,
            purpose,
        };
        let request_bytes = serde_json::to_vec(&request)
            .map_err(|e| format!("Unable to describe the decoder request: {e}"))?;
        let control = &self.control;
        let stream = self
            .stream
            .as_mut()
            .ok_or("The decoder is not connected.")?;
        wire::write_frame(stream.get_mut(), &request_bytes, MAX_REQUEST_BYTES)
            .map_err(|e| format!("Unable to send the decoder request: {e}"))?;
        let mut prepared = Some(prepared);
        loop {
            current()?;
            let event = wire::receive(&mut *stream).map_err(|error| {
                let detail = if cfg!(windows) {
                    " Decoder private memory is capped at 2 GiB; an unexpected exit alone does not establish that the cap was reached."
                } else { "" };
                match control.exited() {
                    Ok(Some(status)) => format!("The decoder process or connection failed ({status}): {error}.{detail}"),
                    _ => format!("The decoder process or connection failed: {error}.{detail}"),
                }
            })?;
            current()?;
            match event {
                Event::Progress(phase, completed, total) => progress(phase, completed, total),
                Event::Prepared(min, max) => {
                    prepared.take().ok_or("Duplicate decoder source bounds.")?(min, max)?
                }
                Event::Batch(batch) => {
                    if prepared.is_some() {
                        return Err("The decoder sent geometry before source bounds.".into());
                    }
                    consume(batch)?;
                }
                Event::Skipped(reason) => return Ok(LoadOutcome::Skipped(reason)),
                Event::Finished(result) => return Ok(LoadOutcome::Finished(result)),
            }
        }
    }
}

impl Drop for DecoderProcess {
    fn drop(&mut self) {
        self.control.stop();
        self.stream.take();
        let child = self.control.state.lock().child.take();
        if let Some(mut child) = child {
            let _ = child.wait();
        }
    }
}

/// Identity of a retained preview. Thread count and speed mode are excluded
/// because they only change scheduling; chunk size and atlas limit do change
/// the produced batches and stay part of the key.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PreviewKey {
    path: PathBuf,
    pack_path: PathBuf,
    layers: Vec<PackLayer>,
    chunk_size: Option<u16>,
    max_texture_side: u32,
}

impl PreviewKey {
    fn new(
        path: &Path,
        pack_path: &Path,
        layers: &[PackLayer],
        options: PreviewOptions,
        max_texture_side: u32,
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            pack_path: pack_path.to_path_buf(),
            layers: layers.to_vec(),
            chunk_size: options.chunk_size,
            max_texture_side,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn file_stamp(path: &Path) -> Result<FileStamp, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
    Ok(FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Stamps every input so a cached entry is discarded if the schematic, base
/// pack, or any overlay changed on disk.
fn key_stamps(key: &PreviewKey) -> Result<Vec<(PathBuf, FileStamp)>, String> {
    let mut paths: Vec<&Path> = vec![&key.path, &key.pack_path];
    for layer in &key.layers {
        if !paths
            .iter()
            .any(|candidate| *candidate == layer.path.as_path())
        {
            paths.push(&layer.path);
        }
    }
    paths
        .into_iter()
        .map(|path| Ok((path.to_path_buf(), file_stamp(path)?)))
        .collect()
}

fn batch_byte_length(batch: &MeshBatch) -> usize {
    std::mem::size_of::<MeshBatch>()
        .saturating_add(batch.geometry.len())
        .saturating_add(
            batch
                .parts
                .len()
                .saturating_mul(std::mem::size_of::<MeshPart>()),
        )
        .saturating_add(batch.textures.iter().fold(0usize, |bytes, texture| {
            bytes
                .saturating_add(std::mem::size_of::<Texture>())
                .saturating_add(texture.pixels.len())
        }))
}

/// One successful speculative mesh, ready to be moved into a foreground
/// upload without decoding or re-meshing.
struct CachedPreview {
    metadata: Metadata,
    prepared: ([f32; 3], [f32; 3]),
    batches: Vec<MeshBatch>,
    byte_length: usize,
    stamps: Vec<(PathBuf, FileStamp)>,
}

/// Bounded, process-local cache of completed speculative previews. Entries are
/// independent of any decoder process, so preempting one neighbor keeps every
/// finished neighbor.
struct PreviewCache {
    entries: Mutex<Vec<(PreviewKey, CachedPreview)>>,
}

impl PreviewCache {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn fresh_index(
        entries: &mut Vec<(PreviewKey, CachedPreview)>,
        key: &PreviewKey,
    ) -> Option<usize> {
        let index = entries.iter().position(|(candidate, _)| candidate == key)?;
        let fresh = key_stamps(key).is_ok_and(|stamps| entries[index].1.stamps == stamps);
        if fresh {
            Some(index)
        } else {
            entries.remove(index);
            None
        }
    }

    fn take_fresh(&self, key: &PreviewKey) -> Option<CachedPreview> {
        let mut entries = self.entries.lock();
        let index = Self::fresh_index(&mut entries, key)?;
        Some(entries.remove(index).1)
    }

    fn with_cached<T>(
        &self,
        key: &PreviewKey,
        visit: impl FnOnce(&Metadata, &[MeshBatch]) -> T,
    ) -> Option<T> {
        let mut entries = self.entries.lock();
        let index = Self::fresh_index(&mut entries, key)?;
        let entry = &entries[index].1;
        Some(visit(&entry.metadata, &entry.batches))
    }

    fn insert(&self, key: PreviewKey, preview: CachedPreview) -> bool {
        if preview.byte_length > PRELOAD_FILE_BYTES {
            return false;
        }
        let mut entries = self.entries.lock();
        let mut total = entries.iter().fold(0usize, |bytes, (_, entry)| {
            bytes.saturating_add(entry.byte_length)
        });
        while !entries.is_empty()
            && (entries.len() >= PRELOAD_MAX_ENTRIES
                || total.saturating_add(preview.byte_length) > PRELOAD_TOTAL_BYTES)
        {
            let (_, evicted) = entries.remove(0);
            total = total.saturating_sub(evicted.byte_length);
        }
        if let Some(index) = entries.iter().position(|(candidate, _)| candidate == &key) {
            entries.remove(index);
        }
        entries.push((key, preview));
        true
    }

    /// Keeps only neighbors still reachable from the current navigation set.
    fn retain_paths(&self, paths: &[PathBuf]) {
        self.entries
            .lock()
            .retain(|(key, _)| paths.iter().any(|path| path == &key.path));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().len()
    }
}

static CACHE: LazyLock<PreviewCache> = LazyLock::new(PreviewCache::new);

fn cache() -> &'static PreviewCache {
    &CACHE
}

/// Drops retained previews outside the current directory/navigation set.
pub fn evict_except(paths: &[PathBuf]) {
    cache().retain_paths(paths);
}

/// Visits one retained preview without removing it. The callback runs under the
/// cache lock, so it must not call back into the cache.
pub fn with_cached<T>(
    path: &Path,
    pack_path: &Path,
    layers: &[PackLayer],
    options: PreviewOptions,
    max_texture_side: u32,
    visit: impl FnOnce(&Metadata, &[MeshBatch]) -> T,
) -> Option<T> {
    let key = PreviewKey::new(path, pack_path, layers, options, max_texture_side);
    cache().with_cached(&key, visit)
}

pub fn load(
    decoder: &mut Option<DecoderProcess>,
    worker: &PreviewWorker,
    id: u64,
    request: PreviewRequest<'_>,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    mut consume: impl FnMut(MeshBatch) -> Result<(), String>,
    mut progress: impl FnMut(&'static str, u64, u64),
) -> Result<Metadata, String> {
    let PreviewRequest {
        path,
        pack_path,
        layers,
        options,
        max_texture_side,
    } = request;
    worker.ensure_current(id)?;
    // A foreground load preempts speculative work before it competes for CPU.
    worker.cancel_preload();
    let key = PreviewKey::new(path, pack_path, layers, options, max_texture_side);
    if let Some(cached) = cache().take_fresh(&key) {
        let CachedPreview {
            metadata,
            prepared: (min, max),
            batches,
            ..
        } = cached;
        prepared(min, max)?;
        let total = batches.len() as u64;
        progress("upload", 0, total);
        for batch in batches {
            consume(batch)?;
        }
        progress("upload", total, total);
        worker.ensure_current(id)?;
        return Ok(metadata);
    }
    if decoder
        .as_ref()
        .is_some_and(|process| process.control.stopped.load(Ordering::Acquire))
    {
        *decoder = None;
    }
    if decoder.is_none() {
        *decoder = Some(DecoderProcess::spawn(|control| {
            worker.register_decoder(id, control)
        })?);
    }
    let process = decoder
        .as_mut()
        .ok_or("The decoder process is unavailable.")?;
    worker.register_decoder(id, Arc::clone(&process.control))?;
    let result = process.load(
        request,
        Purpose::Foreground,
        || worker.ensure_current(id),
        prepared,
        consume,
        progress,
    );
    match result {
        Ok(LoadOutcome::Finished(result)) => result,
        Ok(LoadOutcome::Skipped(reason)) => {
            Err(format!("The preview could not be prepared: {reason}"))
        }
        Err(error) => {
            *decoder = None;
            Err(error)
        }
    }
}

/// Result of one speculative preparation. `Cancelled` means foreground work
/// or a newer neighbor invalidated the token; it is not a failure.
#[derive(Clone, Debug)]
pub enum PreloadOutcome {
    Cached(Metadata),
    Skipped(String),
    Cancelled,
}

fn load_speculative(
    decoder: &mut Option<DecoderProcess>,
    worker: &PreviewWorker,
    token: u64,
    path: &Path,
    pack_path: &Path,
    layers: &[PackLayer],
    options: PreviewOptions,
    max_texture_side: u32,
    purpose: Purpose,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    consume: impl FnMut(MeshBatch) -> Result<(), String>,
    progress: impl FnMut(&'static str, u64, u64),
) -> Result<LoadOutcome, String> {
    worker.ensure_preload(token)?;
    if decoder
        .as_ref()
        .is_some_and(|process| process.control.stopped.load(Ordering::Acquire))
    {
        *decoder = None;
    }
    if decoder.is_none() {
        *decoder = Some(DecoderProcess::spawn(|control| {
            worker.register_preload(token, control)
        })?);
    }
    let control = match decoder.as_ref() {
        Some(process) => Arc::clone(&process.control),
        None => return Err("The decoder process is unavailable.".into()),
    };
    if let Err(error) = worker.register_preload(token, control) {
        *decoder = None;
        return Err(error);
    }
    let result = match decoder.as_mut() {
        Some(process) => process.load(
            PreviewRequest {
                path,
                pack_path,
                layers,
                options,
                max_texture_side,
            },
            purpose,
            || worker.ensure_preload(token),
            prepared,
            consume,
            progress,
        ),
        None => return Err("The decoder process is unavailable.".into()),
    };
    match result {
        Ok(outcome) => {
            worker.release_preload(token);
            Ok(outcome)
        }
        Err(error) => {
            *decoder = None;
            Err(error)
        }
    }
}

/// Prepares one neighbor without any GPU or renderer work and retains its
/// upload-ready batches in the bounded host cache. The caller owns the decoder
/// (shared with the session or dedicated) and must serialize speculative calls.
pub fn preload(
    decoder: &mut Option<DecoderProcess>,
    worker: &PreviewWorker,
    token: u64,
    path: &Path,
    pack_path: &Path,
    layers: &[PackLayer],
    options: PreviewOptions,
    max_texture_side: u32,
    mut progress: impl FnMut(&'static str, u64, u64),
) -> Result<PreloadOutcome, String> {
    worker.ensure_preload(token)?;
    let key = PreviewKey::new(path, pack_path, layers, options, max_texture_side);
    if let Some(metadata) = cache()
        .with_cached(&key, |metadata, batches| {
            (!batches.is_empty()).then(|| metadata.clone())
        })
        .flatten()
    {
        return Ok(PreloadOutcome::Cached(metadata));
    }
    let stamps = match key_stamps(&key) {
        Ok(stamps) => stamps,
        Err(error) => return Ok(PreloadOutcome::Skipped(error)),
    };
    // One mesh worker bounds speculative CPU. The retained key keeps the
    // caller's chunk size, so the foreground load still matches.
    let speculative = PreviewOptions {
        thread_count: None,
        speed_first: false,
        ..options
    };
    let mut bounds = None;
    let mut batches = Vec::new();
    let mut byte_length = 0usize;
    let mut overflowed = false;
    let outcome = load_speculative(
        decoder,
        worker,
        token,
        path,
        pack_path,
        layers,
        speculative,
        max_texture_side,
        Purpose::Preload,
        |min, max| {
            bounds = Some((min, max));
            Ok(())
        },
        |batch| {
            byte_length = byte_length.saturating_add(batch_byte_length(&batch));
            if byte_length > PRELOAD_FILE_BYTES {
                overflowed = true;
                return Err("The preloaded preview exceeds the speculative memory budget.".into());
            }
            batches.push(batch);
            Ok(())
        },
        &mut progress,
    );
    if overflowed {
        return Ok(PreloadOutcome::Skipped(
            "The preloaded preview exceeds the speculative memory budget.".into(),
        ));
    }
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            return if worker.ensure_preload(token).is_err() {
                Ok(PreloadOutcome::Cancelled)
            } else {
                Err(error)
            };
        }
    };
    match outcome {
        LoadOutcome::Finished(Ok(metadata)) => {
            if worker.ensure_preload(token).is_err() {
                return Ok(PreloadOutcome::Cancelled);
            }
            let Some(prepared) = bounds else {
                return Ok(PreloadOutcome::Skipped(
                    "The decoder sent no source bounds for the preloaded preview.".into(),
                ));
            };
            let preview = CachedPreview {
                metadata: metadata.clone(),
                prepared,
                batches,
                byte_length,
                stamps,
            };
            if cache().insert(key, preview) {
                Ok(PreloadOutcome::Cached(metadata))
            } else {
                Ok(PreloadOutcome::Skipped(
                    "The preloaded preview exceeds the speculative memory budget.".into(),
                ))
            }
        }
        LoadOutcome::Finished(Err(error)) => {
            if worker.ensure_preload(token).is_err() {
                Ok(PreloadOutcome::Cancelled)
            } else {
                Err(error)
            }
        }
        LoadOutcome::Skipped(reason) => Ok(PreloadOutcome::Skipped(reason)),
    }
}

/// Streams a vanilla-pack preview for a thumbnail without the speculative
/// admission cap. It shares the preload token and cancellation path, so
/// foreground work preempts it and only one speculative decoder is retained.
pub fn load_thumbnail(
    decoder: &mut Option<DecoderProcess>,
    worker: &PreviewWorker,
    token: u64,
    path: &Path,
    pack_path: &Path,
    options: PreviewOptions,
    max_texture_side: u32,
    prepared: impl FnOnce([f32; 3], [f32; 3]) -> Result<(), String>,
    consume: impl FnMut(MeshBatch) -> Result<(), String>,
) -> Result<Metadata, String> {
    let speculative = PreviewOptions {
        thread_count: None,
        speed_first: false,
        ..options
    };
    match load_speculative(
        decoder,
        worker,
        token,
        path,
        pack_path,
        &[],
        speculative,
        max_texture_side,
        Purpose::Foreground,
        prepared,
        consume,
        |_, _, _| {},
    )? {
        LoadOutcome::Finished(Ok(metadata)) => Ok(metadata),
        LoadOutcome::Finished(Err(error)) => Err(error),
        LoadOutcome::Skipped(reason) => Err(format!("The thumbnail preview was skipped: {reason}")),
    }
}

pub fn run(port: &std::ffi::OsStr) -> Result<(), String> {
    let port = port
        .to_str()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .ok_or("Invalid decoder connection port.")?;
    let mut token = [0; TOKEN_BYTES];
    io::stdin()
        .read_exact(&mut token)
        .map_err(|e| format!("Unable to read decoder authentication: {e}"))?;
    std::thread::Builder::new()
        .name("preview-host-lifetime".into())
        .spawn(|| {
            let mut byte = [0];
            let mut input = io::stdin().lock();
            loop {
                match input.read(&mut byte) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    _ => std::process::exit(0),
                }
            }
        })
        .map_err(|e| format!("Unable to monitor the preview host: {e}"))?;
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        STARTUP_TIMEOUT,
    )
    .map_err(|e| format!("Unable to connect to the preview host: {e}"))?;
    stream
        .set_nodelay(true)
        .and_then(|()| stream.write_all(&token))
        .map_err(|e| e.to_string())?;
    let mut input = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let output = RefCell::new(BufWriter::new(stream));
    let mut packs = crate::preview::PackCache::default();
    loop {
        let request = wire::read_frame(&mut input, MAX_REQUEST_BYTES).map_err(|e| e.to_string())?;
        let request: wire::Request = serde_json::from_slice(&request)
            .map_err(|e| format!("Invalid decoder request: {e}"))?;
        let options = PreviewOptions {
            memory_limit_mb: None,
            chunk_size: request.chunk_size,
            thread_count: request.thread_count,
            speed_first: request.thread_count.is_some(),
        };
        let purpose = request.purpose;
        let budget = if purpose == Purpose::Preload {
            crate::preview::DecodeBudget::preload()
        } else {
            crate::preview::DecodeBudget::foreground()
        };
        let mut skipped = None;
        let admit = |metrics: &crate::preview::DecodedMetrics| {
            if purpose != Purpose::Preload {
                return Ok(());
            }
            match crate::preview::preload_admission(metrics) {
                Ok(()) => Ok(()),
                Err(reason) => {
                    skipped = Some(reason);
                    Err("The schematic exceeds the speculative preload budget.".to_string())
                }
            }
        };
        let transport_error = RefCell::new(None);
        let result = crate::preview::decode_admitted(
            &request.path,
            &request.pack_path,
            &request.layers,
            &mut packs,
            options,
            request.max_texture_side,
            &budget,
            admit,
            || match transport_error.borrow().as_ref() {
                Some(error) => Err(format!("The decoder connection failed: {error}")),
                None => Ok(()),
            },
            |min, max| {
                wire::prepared(&mut *output.borrow_mut(), min, max).map_err(|e| e.to_string())
            },
            |batch| wire::batch(&mut *output.borrow_mut(), &batch).map_err(|e| e.to_string()),
            |phase, completed, total| {
                if let Err(error) =
                    wire::progress(&mut *output.borrow_mut(), phase, completed, total)
                {
                    *transport_error.borrow_mut() = Some(error);
                }
            },
        );
        if let Some(reason) = skipped {
            wire::skipped(&mut *output.borrow_mut(), &reason)
                .map_err(|e| format!("Unable to send the decoder skip: {e}"))?;
        } else {
            wire::finish(&mut *output.borrow_mut(), result)
                .map_err(|e| format!("Unable to send the decoder result: {e}"))?;
        }
    }
}

pub(crate) fn preview_memory(control: Option<&DecoderControl>) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let host = private_working_set(unsafe { GetCurrentProcess() })?;
        let decoder = match control {
            Some(control) => control.memory()?,
            None => 0,
        };
        host.checked_add(decoder)
    }
    #[cfg(not(windows))]
    {
        let _ = control;
        None
    }
}

#[cfg(windows)]
fn private_working_set(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            handle,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            std::mem::size_of_val(&counters) as u32,
        )
    };
    (ok != 0).then_some(counters.PrivateWorkingSetSize as u64)
}

#[cfg(windows)]
fn decoder_job() -> Result<std::os::windows::io::OwnedHandle, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(format!(
            "Unable to create the decoder job: {}",
            io::Error::last_os_error()
        ));
    }
    let job = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit = DECODER_MEMORY_BYTES;
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return Err(format!(
            "Unable to configure decoder isolation: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(job)
}

#[cfg(windows)]
fn authentication_token() -> Result<[u8; TOKEN_BYTES], String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    let mut token = [0; TOKEN_BYTES];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            token.as_mut_ptr(),
            token.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(format!(
            "Unable to authenticate the decoder process (OS status {status})."
        ));
    }
    Ok(token)
}

#[cfg(not(windows))]
fn authentication_token() -> Result<[u8; TOKEN_BYTES], String> {
    let mut token = [0; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut token))
        .map_err(|e| format!("Unable to authenticate the decoder process: {e}"))?;
    Ok(token)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn stopping_a_busy_process_interrupts_ipc_without_waiting_for_its_work() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut child = Command::new("/bin/sleep")
            .arg("60")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let control = Arc::new(DecoderControl {
            stopped: AtomicBool::new(false),
            state: Mutex::new(ProcessState {
                child: Some(child),
                input,
                stream: Some(stream.try_clone().unwrap()),
            }),
        });
        let process = DecoderProcess {
            control: Arc::clone(&control),
            stream: None,
        };
        let (started_tx, started) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut stream = stream;
            started_tx.send(()).unwrap();
            let error = stream.read_exact(&mut [0]).unwrap_err();
            done_tx.send(error.kind()).unwrap();
        });
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        control.stop();
        let error = done.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            error,
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::NotConnected
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = control.exited().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(Instant::now() < deadline, "the decoder was not killed");
            std::thread::sleep(Duration::from_millis(1));
        }
        reader.join().unwrap();
        drop(process);
        drop(peer);
    }

    fn cache_fixture() -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir();
        let path = directory.join(format!(
            "litematica-preload-cache-{}.schem",
            uuid::Uuid::new_v4()
        ));
        let pack_path = directory.join(format!(
            "litematica-preload-pack-{}.zip",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"schematic").unwrap();
        std::fs::write(&pack_path, b"pack").unwrap();
        (path, pack_path)
    }

    fn cached_fixture(path: &Path, pack_path: &Path) -> (PreviewKey, CachedPreview) {
        let key = PreviewKey::new(path, pack_path, &[], PreviewOptions::default(), 2048);
        let batch = MeshBatch {
            geometry: vec![0; 4],
            vertex_stride: 24,
            parts: Vec::new(),
            textures: Vec::new(),
            origin: [0.0; 3],
            extent: [0.0; 3],
            source_min: [-1.0; 3],
            source_max: [1.0; 3],
            chunk_coord: None,
            vertex_count: 0,
            index_count: 0,
            info: litematica_preview_core::PreviewInfo::default(),
        };
        let byte_length = batch_byte_length(&batch);
        let preview = CachedPreview {
            metadata: Metadata {
                block_count: 1,
                block_entity_count: 0,
                triangle_count: 0,
                min: [-0.5; 3],
                max: [0.5; 3],
                geometry_byte_length: 0,
                byte_length: 0,
                texture_count: 0,
                part_count: 1,
            },
            prepared: ([-1.0; 3], [1.0; 3]),
            batches: vec![batch],
            byte_length,
            stamps: key_stamps(&key).unwrap(),
        };
        (key, preview)
    }

    #[test]
    fn completed_preload_is_reused_once_within_bounds() {
        let (path, pack_path) = cache_fixture();
        let (key, preview) = cached_fixture(&path, &pack_path);
        let cache = PreviewCache::new();
        assert!(cache.insert(key.clone(), preview));
        assert_eq!(
            cache.with_cached(&key, |metadata, batches| (
                metadata.part_count,
                batches.len()
            )),
            Some((1, 1))
        );
        assert_eq!(cache.take_fresh(&key).unwrap().batches.len(), 1);
        assert!(
            cache.take_fresh(&key).is_none(),
            "reuse must move batches out instead of copying them"
        );
        let mut oversized = cached_fixture(&path, &pack_path).1;
        oversized.byte_length = PRELOAD_FILE_BYTES + 1;
        assert!(!cache.insert(key, oversized));
        assert_eq!(cache.len(), 0);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&pack_path).unwrap();
    }

    #[test]
    fn cancelling_speculation_preserves_finished_cache_entries() {
        let (path, pack_path) = cache_fixture();
        let (key, preview) = cached_fixture(&path, &pack_path);
        assert!(cache().insert(key.clone(), preview));
        let worker = PreviewWorker::default();
        worker.cancel(1, || Ok(())).unwrap();
        let token = worker.begin_preload(1).unwrap();
        assert!(
            worker.begin_preload(2).is_err(),
            "a stale id must not claim"
        );
        worker.cancel_preload();
        assert!(worker.ensure_preload(token).is_err());
        assert_eq!(
            cache().with_cached(&key, |_, batches| batches.len()),
            Some(1),
            "preemption must not discard completed neighbors"
        );
        evict_except(&[]);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&pack_path).unwrap();
    }
}
