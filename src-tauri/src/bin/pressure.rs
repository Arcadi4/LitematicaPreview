//! Pressure suite for the million-block fixtures in `Fixtures/Pressure`.
//!
//! Three phases: the `core` phase decodes in-process with allocator charges
//! and pins the decode counts to `baselines.json`; the `cold` phase runs one
//! production load per fixture (fresh decoder child, upload window, headless
//! scene for timed frames) and reports full-render latency; the `warm` phase
//! reuses one decoder child across four Atrium loads to detect growth.
//!
//! A software adapter still decodes and consumes every batch, but skips GPU
//! uploads and frame timings: CPU rasterization makes them noise and dominates
//! wall time. `--no-render` does the same for frames on a hardware adapter.
//!
//! `--preview-worker <port>` plays the decoder child role. The cold pass
//! spawns this binary through `DecoderProcess`, so the branch must match
//! `main.rs`.
//!
//! `--compare <baseline.json>` reports only representative runtime metrics
//! that leave their noise band, so incidental jitter does not turn every
//! comparison into a wall of percentage changes.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use litematica_preview_core::{MeshBatch, PreviewOptions};
use nucleation::meshing::ResourcePackSource;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[path = "../preview.rs"]
mod preview;
#[path = "../preview_process.rs"]
mod preview_process;
#[path = "../preview_protocol.rs"]
mod preview_protocol;
#[path = "../render/mod.rs"]
mod render;

mod meter {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicIsize, Ordering};

    pub struct AllocationMeter;

    // Charges land on global atomics because a thread-local tally can
    // panic when the allocator is reentered during thread destruction.
    static LIVE: AtomicIsize = AtomicIsize::new(0);
    static PEAK: AtomicIsize = AtomicIsize::new(0);

    #[inline]
    fn charge(bytes: isize) {
        if bytes == 0 {
            return;
        }
        let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
        PEAK.fetch_max(live, Ordering::Relaxed);
    }

    impl AllocationMeter {
        pub fn live() -> usize {
            LIVE.load(Ordering::Relaxed).max(0) as usize
        }

        pub fn reset_peak() {
            PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
        }

        pub fn peak() -> usize {
            PEAK.load(Ordering::Relaxed).max(0) as usize
        }
    }

    unsafe impl GlobalAlloc for AllocationMeter {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            charge(layout.size() as isize);
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            charge(layout.size() as isize);
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            charge(new_size as isize - layout.size() as isize);
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            charge(-(layout.size() as isize));
            unsafe { System.dealloc(ptr, layout) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: meter::AllocationMeter = meter::AllocationMeter;

const FIXTURES: &[&str] = &["Atrium", "Valkyrie", "DenseNonSolid", "Concrete"];

/// Counts recorded once into `baselines.json`; runtime metrics move with the
/// machine and never pin.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Baseline {
    name: String,
    blocks: u64,
    block_entities: u64,
    content: [f64; 3],
    batches: u64,
    atlas: [u64; 2],
    greedy_textures: u64,
    triangles: u64,
    payload_bytes: u64,
    geometry_extent: [f64; 3],
}

#[derive(Debug, Serialize, Deserialize)]
struct BaselineFile {
    fixtures: Vec<Baseline>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("manifest lives in src-tauri")
        .to_path_buf()
}

fn fixture_path(name: &str) -> PathBuf {
    repo_root()
        .join("Fixtures/Pressure")
        .join(format!("{name}.litematic"))
}

fn pack_path() -> PathBuf {
    repo_root().join("Assets/pack.zip")
}

fn decode_pack() -> Result<ResourcePackSource, String> {
    let bytes = std::fs::read(pack_path())
        .map_err(|error| format!("Unable to read {}: {error}", pack_path().display()))?;
    ResourcePackSource::from_bytes(&bytes)
        .map_err(|error| format!("The bundled block resources are invalid: {error}"))
}

fn baselines_path() -> PathBuf {
    repo_root().join("Fixtures/Pressure/baselines.json")
}

fn load_baselines() -> Vec<Baseline> {
    let text = match std::fs::read_to_string(baselines_path()) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<BaselineFile>(&text)
        .expect("baselines.json")
        .fixtures
}

/// A meshing change that inflates the payload fails here before memory.
const PAYLOAD_HEADROOM: u64 = 5;
/// Triangle-count drift in hundredths of a percent; compact packing rounds
/// inside this band.
const TRIANGLE_TOLERANCE: u64 = 100;
/// Teardown gives the model back; leftover bytes grow across loads.
const RETAINED_BUDGET: usize = 2 * 1024 * 1024;
/// Windows caps the decoder child at 2 GiB with a job object.
const DECODER_CAP_BYTES: usize = 2 * 1024 * 1024 * 1024;
/// The first load may fill the child pack cache; later loads stay flat.
const REUSE_GROWTH_BUDGET: usize = 32 * 1024 * 1024;
/// Uniform writes and GPU staging sit outside the allocator, so the frame
/// loop allowance is loose.
const FRAME_STABLE_BUDGET: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CoreRecord {
    fixture: String,
    error: Option<String>,
    blocks: u64,
    block_entities: u64,
    content: [f64; 3],
    batches: u64,
    atlas: [u64; 2],
    triangles: u64,
    vertices: u64,
    payload_bytes: u64,
    greedy_textures: u64,
    geometry_extent: [f64; 3],
    decode_ms: f64,
    stream_open_ms: f64,
    stream_ms: f64,
    total_ms: f64,
    first_batch_ms: f64,
    decode_peak_bytes: usize,
    streaming_peak_bytes: usize,
    retained_bytes: usize,
    max_batch_gap_ms: f64,
    p95_batch_gap_ms: f64,
    blocks_per_second: f64,
}

fn check_baseline(baseline: &Baseline, record: &CoreRecord) -> Vec<String> {
    let mut breaches = Vec::new();
    if record.blocks != baseline.blocks {
        breaches.push(format!(
            "{}: decoded {} blocks, baseline pinned {}",
            baseline.name, record.blocks, baseline.blocks
        ));
    }
    if record.block_entities != baseline.block_entities {
        breaches.push(format!(
            "{}: decoded {} block entities, baseline pinned {}",
            baseline.name, record.block_entities, baseline.block_entities
        ));
    }
    if record.batches != baseline.batches {
        breaches.push(format!(
            "{}: the stream partitioned into {} batches, baseline pinned {}",
            baseline.name, record.batches, baseline.batches
        ));
    }
    if record.greedy_textures != baseline.greedy_textures {
        breaches.push(format!(
            "{}: planned {} greedy textures, baseline pinned {}",
            baseline.name, record.greedy_textures, baseline.greedy_textures
        ));
    }
    if record.atlas != baseline.atlas {
        breaches.push(format!(
            "{}: shared atlas {:?} changed from {:?}",
            baseline.name, record.atlas, baseline.atlas
        ));
    }
    let ceiling = baseline.payload_bytes * PAYLOAD_HEADROOM / 4;
    if record.payload_bytes > ceiling {
        breaches.push(format!(
            "{}: payload {} bytes exceeds the recorded ceiling {} (pinned {})",
            baseline.name, record.payload_bytes, ceiling, baseline.payload_bytes
        ));
    }
    let slack = (baseline.triangles * TRIANGLE_TOLERANCE / 10_000).max(1);
    if record.triangles.abs_diff(baseline.triangles) > slack {
        breaches.push(format!(
            "{}: triangles {} drifted outside {} +/- {}",
            baseline.name, record.triangles, baseline.triangles, slack
        ));
    }
    for axis in 0..3 {
        if (record.content[axis] - baseline.content[axis]).abs() > 0.5 {
            breaches.push(format!(
                "{}: content axis {axis} {} differs from the recorded {}",
                baseline.name, record.content[axis], baseline.content[axis]
            ));
        }
        if (record.geometry_extent[axis] - baseline.geometry_extent[axis]).abs() > 0.5 {
            breaches.push(format!(
                "{}: geometry extent axis {axis} {} differs from the recorded {}",
                baseline.name, record.geometry_extent[axis], baseline.geometry_extent[axis]
            ));
        }
    }
    breaches
}

fn retained_breach(label: &str, retained: usize) -> Vec<String> {
    if retained <= RETAINED_BUDGET {
        return Vec::new();
    }
    vec![format!(
        "{label}: {retained} bytes stayed live after teardown, over the \
         {RETAINED_BUDGET} byte budget",
    )]
}

struct Settings {
    fixtures: Vec<String>,
    threads: u8,
    chunk: u16,
    frames: u32,
    render: bool,
    out: PathBuf,
    compare: Option<PathBuf>,
    write_baselines: bool,
    passes: Vec<String>,
}

impl Settings {
    fn options(&self) -> PreviewOptions {
        PreviewOptions {
            memory_limit_mb: None,
            chunk_size: Some(self.chunk),
            thread_count: Some(self.threads),
            speed_first: true,
        }
    }
}

fn parse_args(arguments: &[String]) -> Result<Settings, String> {
    let mut settings = Settings {
        fixtures: FIXTURES.iter().map(|name| (*name).to_string()).collect(),
        threads: 4,
        chunk: 32,
        frames: 90,
        render: true,
        out: repo_root().join("target/pressure"),
        compare: None,
        write_baselines: false,
        passes: vec![
            String::from("core"),
            String::from("cold"),
            String::from("warm"),
        ],
    };
    let mut fixtures: Option<Vec<String>> = None;
    let mut index = 0;
    while index < arguments.len() {
        let value = arguments.get(index + 1).cloned().unwrap_or_default();
        match arguments[index].as_str() {
            "--fixtures" => {
                fixtures = Some(
                    value
                        .split(',')
                        .map(str::to_string)
                        .filter(|name| !name.is_empty())
                        .collect(),
                );
                index += 2;
            }
            "--threads" => {
                settings.threads = value
                    .parse()
                    .map_err(|_| format!("--threads wants 1..=8: {value}"))?;
                index += 2;
            }
            "--chunk" => {
                settings.chunk = value
                    .parse()
                    .map_err(|_| format!("--chunk wants a chunk edge: {value}"))?;
                index += 2;
            }
            "--frames" => {
                settings.frames = value
                    .parse()
                    .map_err(|_| format!("--frames wants a count: {value}"))?;
                index += 2;
            }
            "--out" => {
                settings.out = PathBuf::from(value);
                index += 2;
            }
            "--compare" => {
                settings.compare = Some(PathBuf::from(value));
                index += 2;
            }
            "--passes" => {
                settings.passes = value
                    .split(',')
                    .map(str::to_string)
                    .filter(|name| !name.is_empty())
                    .collect();
                index += 2;
            }
            "--no-render" => {
                settings.render = false;
                index += 1;
            }
            "--write-baselines" => {
                settings.write_baselines = true;
                index += 1;
            }
            other => return Err(format!("Unknown option {other}")),
        }
    }
    if let Some(names) = fixtures {
        settings.fixtures = names;
    }
    for name in &settings.fixtures {
        let path = fixture_path(name);
        if !path.is_file() {
            return Err(format!("Missing fixture {}", path.display()));
        }
    }
    for pass in &settings.passes {
        if !matches!(pass.as_str(), "core" | "cold" | "warm") {
            return Err(format!("Unknown pass {pass}; use core, cold, or warm"));
        }
    }
    Ok(settings)
}

fn round_at(value: f64, digits: i32) -> f64 {
    let scale = 10.0_f64.powi(digits);
    (value * scale).round() / scale
}

fn ms_since(start: Instant, stamp: Option<Instant>) -> f64 {
    match stamp {
        Some(later) => round_at(
            later.saturating_duration_since(start).as_secs_f64() * 1_000.0,
            3,
        ),
        None => 0.0,
    }
}

fn median(values: Vec<f64>) -> f64 {
    let mut values = values;
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

fn percentile(mut values: Vec<f64>, fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let index = ((values.len() as f64 - 1.0) * fraction).round() as usize;
    values[index.min(values.len() - 1)]
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

/// Inserts thousands separators into the integer part while leaving the sign
/// and fraction alone: `-1234567.5` becomes `-1,234,567.5`.
fn grouped(text: &str) -> String {
    let (sign, digits) = match text.as_bytes().first() {
        Some(b'-') => ("-", &text[1..]),
        Some(b'+') => ("+", &text[1..]),
        _ => ("", text),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let mut text = String::with_capacity(digits.len() + digits.len() / 3 + sign.len());
    text.push_str(sign);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            text.push(',');
        }
        text.push(digit);
    }
    if !fraction.is_empty() {
        text.push('.');
        text.push_str(fraction);
    }
    text
}

fn number(value: f64, digits: usize) -> String {
    grouped(&format!("{value:.digits$}"))
}

fn count(value: u64) -> String {
    grouped(&value.to_string())
}

fn mb_opt(bytes: Option<u64>) -> String {
    bytes
        .map(|value| number(value as f64 / 1024.0 / 1024.0, 1))
        .unwrap_or_else(|| String::from("-"))
}

fn num_opt(value: Option<f64>) -> String {
    value
        .map(|value| grouped(&value.to_string()))
        .unwrap_or_else(|| String::from("-"))
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    match panic.downcast_ref::<String>() {
        Some(text) => text.clone(),
        None => panic
            .downcast_ref::<&str>()
            .copied()
            .map(str::to_owned)
            .unwrap_or_else(|| String::from("Unknown native panic")),
    }
}

fn git_rev() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The production app negotiates this backend on each platform.
#[cfg(windows)]
const RIG_BACKENDS: wgpu::Backends = wgpu::Backends::DX12;
#[cfg(not(windows))]
const RIG_BACKENDS: wgpu::Backends = wgpu::Backends::METAL;

const FRAME_WIDTH: u32 = 1920;
const FRAME_HEIGHT: u32 = 1080;
const WAIT_TIMEOUT: Duration = Duration::from_secs(60);

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: render::pipelines::Pipelines,
    samples: u32,
    format: wgpu::TextureFormat,
    adapter: String,
    backend: String,
    /// Software adapters rasterize on the CPU, so their upload and frame
    /// latencies are noise and dominate wall time. Those loads keep the decode
    /// and memory path but skip GPU work.
    hardware: bool,
    target: Offscreen,
}

/// Offscreen frames replace the window surface, so the timed draws run the
/// same pass family, depth format, and MSAA rule that the app renders.
struct Offscreen {
    msaa: Option<wgpu::TextureView>,
    resolve: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl Offscreen {
    fn create(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let size = wgpu::Extent3d {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            depth_or_array_layers: 1,
        };
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("pressure depth"),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24Plus,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let resolve = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("pressure color"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let msaa = (samples > 1).then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("pressure msaa color"),
                    size,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        Self {
            msaa,
            resolve,
            depth,
        }
    }
}

fn request_rig() -> Result<(Rig, Option<String>), String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = RIG_BACKENDS;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        apply_limit_buckets: false,
        ..Default::default()
    }))
    .map_err(|error| format!("The platform adapter is unavailable: {error}"))?;
    let info = adapter.get_info();
    let hardware = info.device_type != wgpu::DeviceType::Cpu;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("pressure renderer"),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .map_err(|error| format!("The pressure device failed: {error}"))?;
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let samples = if adapter
        .get_texture_format_features(format)
        .flags
        .sample_count_supported(4)
    {
        4
    } else {
        1
    };
    let pipelines = render::pipelines::Pipelines::new(
        &device,
        samples,
        format,
        wgpu::TextureFormat::Depth24Plus,
    );
    let target = Offscreen::create(&device, format, samples);
    let notice = (!hardware).then(|| {
        format!(
            "{} is a software adapter; GPU uploads and frame timings are skipped, so only decode and memory metrics are reported",
            info.name
        )
    });
    Ok((
        Rig {
            adapter: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device,
            queue,
            pipelines,
            samples,
            format,
            hardware,
            target,
        },
        notice,
    ))
}

fn encode_frame(
    rig: &Rig,
    scene: &render::scene::Scene,
    camera: &render::camera::Camera,
) -> Result<(), String> {
    rig.pipelines.write_cameras(
        &rig.queue,
        camera.frame_camera(true),
        camera.frame_camera(false),
    );
    let mut encoder = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pressure frame"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pressure render"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: rig.target.msaa.as_ref().unwrap_or(&rig.target.resolve),
                depth_slice: None,
                resolve_target: rig.target.msaa.as_ref().map(|_| &rig.target.resolve),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: if rig.target.msaa.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &rig.target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        scene
            .draw(&rig.pipelines, &mut pass, false)
            .map_err(|error| error.to_string())?;
    }
    rig.queue.submit(Some(encoder.finish()));
    Ok(())
}

fn wait_gpu(rig: &Rig) -> Result<(), String> {
    rig.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(WAIT_TIMEOUT),
        })
        .map_err(|error| format!("The GPU work never completed: {error}"))?;
    Ok(())
}

#[derive(Clone)]
struct Sample {
    host: Option<u64>,
    process: Option<u64>,
}

#[derive(Default)]
struct SeriesSummary {
    samples: u32,
    host_peak: Option<u64>,
    process_peak: Option<u64>,
    child_peak: Option<u64>,
}

impl SeriesSummary {
    fn collect(samples: &[Sample]) -> SeriesSummary {
        let host_values = samples
            .iter()
            .filter_map(|sample| sample.host)
            .collect::<Vec<_>>();
        let child_values = samples
            .iter()
            .filter_map(|sample| match (sample.host, sample.process) {
                (Some(host), Some(process)) => process.checked_sub(host),
                _ => None,
            })
            .collect::<Vec<_>>();
        SeriesSummary {
            samples: samples.len() as u32,
            host_peak: host_values.iter().copied().max(),
            process_peak: samples.iter().filter_map(|sample| sample.process).max(),
            child_peak: child_values.iter().copied().max(),
        }
    }
}

struct Watch {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    series: Arc<Mutex<Vec<Sample>>>,
}

fn start_watch(worker: Arc<preview::PreviewWorker>, id: u64) -> Watch {
    let stop = Arc::new(AtomicBool::new(false));
    let series: Arc<Mutex<Vec<Sample>>> = Arc::new(Mutex::new(Vec::new()));
    let handle = std::thread::Builder::new()
        .name("pressure-watch".into())
        .spawn({
            let stop = Arc::clone(&stop);
            let series = Arc::clone(&series);
            move || loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let process = worker.preview_memory(id);
                if let (Some(process), Some(host)) = (process, host_private()) {
                    series.lock().push(Sample {
                        host: Some(host),
                        process: Some(process),
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
        .ok();
    Watch {
        stop,
        handle,
        series,
    }
}

fn summarize(series: &Mutex<Vec<Sample>>) -> SeriesSummary {
    SeriesSummary::collect(&series.lock().clone())
}

fn settle(watch: &mut Watch) -> SeriesSummary {
    watch.stop.store(true, Ordering::Release);
    if let Some(handle) = watch.handle.take() {
        let _ = handle.join();
    }
    summarize(&watch.series)
}

/// Windows reports both the host private working set and the decoder child;
/// other platforms get the allocator lens only.
#[cfg(windows)]
fn host_private() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(),
            std::mem::size_of_val(&counters) as u32,
        )
    };
    (ok != 0).then_some(counters.PrivateWorkingSetSize as u64)
}

#[cfg(not(windows))]
fn host_private() -> Option<u64> {
    None
}

#[derive(Debug, Serialize, Deserialize)]
struct OutcomeRow {
    mode: String,
    load: u64,
    fixture: String,
    error: Option<String>,
    adapter: Option<String>,
    retained_bytes: usize,
    decode_peak_bytes: usize,
    streaming_peak_bytes: usize,
    batches: u64,
    triangles: u64,
    vertices: u64,
    payload_bytes: u64,
    staged_bytes: u64,
    staged_max_batch: u64,
    connect_ms: f64,
    decode_ms: f64,
    stream_open_ms: f64,
    first_batch_ms: f64,
    first_frame_ms: f64,
    full_render_ms: f64,
    stream_ms: f64,
    frames_avg_ms: f64,
    frames_p50_ms: f64,
    frames_p95_ms: f64,
    frames_max_ms: f64,
    frames_growth_bytes: i64,
    /// Absent from older reports, which always rendered frames.
    #[serde(default)]
    frames_measured: u32,
    samples: u32,
    host_peak: Option<u64>,
    child_peak: Option<u64>,
    decoder_cap_percent: Option<f64>,
}

struct Stamps {
    job_start: Instant,
    connect: Option<Instant>,
    prepared: Option<Instant>,
    mesh_zero: Option<Instant>,
    first_batch: Option<Instant>,
    first_frame: Option<Instant>,
    final_frame: Option<Instant>,
    stream_end: Option<Instant>,
}

/// One production-path load. Every callback lands on the `preview-session`
/// thread that `PreviewWorker` runs, so the state has no cross-task races.
struct LoadState {
    rig: Option<Arc<Rig>>,
    upload: Option<Arc<render::upload::UploadWindow>>,
    scene: Option<render::scene::Scene>,
    render_frames: bool,
    camera: render::camera::Camera,
    stamps: Stamps,
    tally: Tally,
    decode_peak_bytes: usize,
    streaming_peak_bytes: usize,
    last_render: Option<Instant>,
    has_visible: bool,
    frames_target: u32,
    frames: Vec<f64>,
    frames_before: usize,
    frames_after: usize,
    retained_base: usize,
}

struct Tally {
    batches: u64,
    triangles: u64,
    vertices: u64,
    payload_bytes: u64,
    staged_bytes: u64,
    staged_max_batch: u64,
}

impl LoadState {
    fn new(rig: Option<Arc<Rig>>, threads: u8, frames_target: u32, render: bool) -> Self {
        let render_frames = render && rig.as_ref().is_some_and(|rig| rig.hardware);
        let device_rig = rig.as_ref().filter(|rig| rig.hardware);
        let upload = device_rig.map(|rig| {
            Arc::new(render::upload::UploadWindow::with_device(
                render::upload::DeviceStatus::new(rig.device.clone()),
                usize::from(threads),
            ))
        });
        let scene =
            device_rig.map(|rig| render::scene::Scene::new(rig.device.clone(), rig.queue.clone()));
        Self {
            rig,
            upload,
            scene,
            render_frames,
            camera: render::camera::Camera::new(),
            stamps: Stamps {
                job_start: Instant::now(),
                connect: None,
                prepared: None,
                mesh_zero: None,
                first_batch: None,
                first_frame: None,
                final_frame: None,
                stream_end: None,
            },
            tally: Tally {
                batches: 0,
                triangles: 0,
                vertices: 0,
                payload_bytes: 0,
                staged_bytes: 0,
                staged_max_batch: 0,
            },
            decode_peak_bytes: 0,
            streaming_peak_bytes: 0,
            last_render: None,
            has_visible: false,
            frames_target,
            frames: Vec::new(),
            frames_before: 0,
            frames_after: 0,
            retained_base: 0,
        }
    }

    fn prepare(&mut self) {
        self.retained_base = meter::AllocationMeter::live();
        meter::AllocationMeter::reset_peak();
    }

    fn connect(&mut self) {
        self.stamps.connect.get_or_insert_with(Instant::now);
    }

    fn on_prepared(&mut self, min: [f32; 3], max: [f32; 3]) {
        if let Some(stamp) = self.stamps.prepared.is_none().then_some(Instant::now()) {
            self.stamps.prepared = Some(stamp);
            self.decode_peak_bytes = meter::AllocationMeter::peak();
            meter::AllocationMeter::reset_peak();
            if self.rig.is_some() {
                self.camera.resize(FRAME_WIDTH as f32, FRAME_HEIGHT as f32);
                self.camera.set_model_bounds(min, max);
            }
        }
    }

    fn mesh_zero(&mut self) {
        self.stamps.mesh_zero.get_or_insert_with(Instant::now);
    }

    fn progress(&mut self, phase: &'static str, completed: u64) {
        if phase == "decode" && completed == 0 {
            self.connect();
        }
        if phase == "mesh" && completed == 0 {
            self.mesh_zero();
        }
    }

    fn consume(
        &mut self,
        current: &impl Fn() -> Result<(), String>,
        batch: &MeshBatch,
    ) -> Result<(), String> {
        let bytes = staged_bytes(batch);
        self.tally.batches += 1;
        self.tally.triangles += batch.info.triangle_count;
        self.tally.vertices += batch.vertex_count as u64;
        self.tally.payload_bytes += bytes as u64;
        self.tally.staged_bytes += bytes as u64;
        self.tally.staged_max_batch = self.tally.staged_max_batch.max(bytes as u64);
        self.stamps.first_batch.get_or_insert_with(Instant::now);
        let Some(upload) = self.upload.as_ref() else {
            return Ok(());
        };
        let permit = upload.acquire(bytes, current)?;
        let Some(rig) = &self.rig else {
            return Ok(());
        };
        let Some(scene) = self.scene.as_mut() else {
            return Ok(());
        };
        scene
            .attach_batch(&rig.pipelines, batch, permit)
            .map_err(|error| error.to_string())?;
        if !self.render_frames {
            return Ok(());
        }
        // Renders at most once every 50 ms during the stream, matching the
        // `NativeRenderer::attach_batch` cadence the app ships.
        let due = self.has_visible
            && self
                .last_render
                .is_none_or(|past| past.elapsed() >= Duration::from_millis(50));
        let first_visible = !batch.parts.is_empty() && !self.has_visible;
        if !batch.parts.is_empty() {
            self.has_visible = true;
        }
        if first_visible || due {
            let started = Instant::now();
            encode_frame(rig, scene, &self.camera)
                .map_err(|error| format!("The pressure frame failed: {error}"))?;
            self.stamps.first_frame.get_or_insert_with(|| started);
            self.last_render = Some(Instant::now());
        }
        Ok(())
    }

    /// The last full frame completes on the GPU, so the latency is a
    /// presented-frame number rather than a submit cost.
    fn finish(
        &mut self,
        current: &impl Fn() -> Result<(), String>,
        metadata: &preview::Metadata,
    ) -> Result<(), String> {
        current()?;
        if let Some(upload) = self.upload.as_ref() {
            upload.drain(current)?;
        }
        self.streaming_peak_bytes = meter::AllocationMeter::peak();
        if !self.render_frames {
            self.scene = None;
            self.stamps.stream_end = Some(Instant::now());
            return Ok(());
        }
        let Some(rig) = self.rig.as_ref() else {
            self.stamps.stream_end = Some(Instant::now());
            return Ok(());
        };
        let scene = self
            .scene
            .as_mut()
            .ok_or_else(|| String::from("The pressure scene vanished before the finish."))?;
        scene.finish(&metadata.info());
        self.camera.set_model_bounds(metadata.min, metadata.max);
        encode_frame(rig, scene, &self.camera)
            .map_err(|error| format!("The pressure frame failed: {error}"))?;
        wait_gpu(rig)?;
        let completed = Instant::now();
        self.stamps.final_frame = Some(completed);
        self.stamps.stream_end = Some(completed);
        self.frames_before = meter::AllocationMeter::live();
        for _ in 0..self.frames_target {
            let started = Instant::now();
            encode_frame(rig, scene, &self.camera)
                .map_err(|error| format!("The pressure frame failed: {error}"))?;
            wait_gpu(rig)?;
            self.frames
                .push(round_at(started.elapsed().as_secs_f64() * 1_000.0, 3));
        }
        self.frames_after = meter::AllocationMeter::live();
        self.scene = None;
        Ok(())
    }

    fn outcome(
        &self,
        summary: SeriesSummary,
        mode: &'static str,
        load: u64,
        fixture: &str,
        error: Option<String>,
    ) -> OutcomeRow {
        let frames_growth = self.frames_after.abs_diff(self.frames_before) as i64;
        OutcomeRow {
            mode: mode.to_string(),
            load,
            fixture: fixture.to_string(),
            error,
            adapter: self.rig.as_ref().map(|rig| rig.adapter.clone()),
            retained_bytes: meter::AllocationMeter::live().saturating_sub(self.retained_base),
            decode_peak_bytes: self.decode_peak_bytes,
            streaming_peak_bytes: self.streaming_peak_bytes,
            batches: self.tally.batches,
            triangles: self.tally.triangles,
            vertices: self.tally.vertices,
            payload_bytes: self.tally.payload_bytes,
            staged_bytes: self.tally.staged_bytes,
            staged_max_batch: self.tally.staged_max_batch,
            connect_ms: ms_since(self.stamps.job_start, self.stamps.connect),
            decode_ms: ms_since(
                self.stamps.connect.unwrap_or(self.stamps.job_start),
                self.stamps.prepared,
            ),
            stream_open_ms: ms_since(
                self.stamps.prepared.unwrap_or(self.stamps.job_start),
                self.stamps.mesh_zero,
            ),
            first_batch_ms: ms_since(
                self.stamps.mesh_zero.unwrap_or(self.stamps.job_start),
                self.stamps.first_batch,
            ),
            first_frame_ms: ms_since(self.stamps.job_start, self.stamps.first_frame),
            full_render_ms: ms_since(self.stamps.job_start, self.stamps.final_frame),
            stream_ms: ms_since(
                self.stamps.mesh_zero.unwrap_or(self.stamps.job_start),
                self.stamps.stream_end,
            ),
            frames_avg_ms: self.frames.iter().sum::<f64>() / self.frames.len().max(1) as f64,
            frames_p50_ms: median(self.frames.clone()),
            frames_p95_ms: percentile(self.frames.clone(), 0.95),
            frames_max_ms: self.frames.iter().cloned().fold(0.0, f64::max),
            frames_growth_bytes: frames_growth,
            frames_measured: self.frames.len() as u32,
            samples: summary.samples,
            host_peak: summary.host_peak,
            child_peak: summary.child_peak,
            decoder_cap_percent: summary
                .child_peak
                .map(|peak| round_at(peak as f64 * 100.0 / DECODER_CAP_BYTES as f64, 1)),
        }
    }
}

fn staged_bytes(batch: &MeshBatch) -> usize {
    batch
        .textures
        .iter()
        .try_fold(batch.geometry.len(), |bytes, texture| {
            bytes.checked_add(texture.pixels.len())
        })
        .unwrap_or(usize::MAX)
}

fn run_load(
    worker: Arc<preview::PreviewWorker>,
    id: u64,
    mode: &'static str,
    load: u64,
    fixture: &str,
    rig: Option<Arc<Rig>>,
    settings: &Settings,
) -> OutcomeRow {
    let state = Arc::new(Mutex::new(LoadState::new(
        rig,
        settings.threads,
        settings.frames,
        settings.render,
    )));
    let (sender, receiver) = mpsc::channel::<OutcomeRow>();
    let mut watch = start_watch(Arc::clone(&worker), id);
    let prepare = {
        let state = Arc::clone(&state);
        move || {
            state.lock().prepare();
            Ok(())
        }
    };
    let job = Box::new({
        let state = Arc::clone(&state);
        let sender = sender;
        let path = fixture_path(fixture);
        let packed = pack_path();
        let options = settings.options();
        let worker = Arc::clone(&worker);
        let fixture_text = fixture.to_string();
        let job_stop = Arc::clone(&watch.stop);
        let job_series = Arc::clone(&watch.series);
        move |decoder: &mut Option<preview_process::DecoderProcess>| {
            let started = Instant::now();
            let outcome_result = catch_unwind(AssertUnwindSafe(|| {
                let current = || worker.ensure_current(id);
                current()?;
                let metadata = preview_process::load(
                    decoder,
                    &*worker,
                    id,
                    &path,
                    &packed,
                    &[],
                    options.clone(),
                    {
                        let state = Arc::clone(&state);
                        move |min, max| -> Result<(), String> {
                            state.lock().on_prepared(min, max);
                            Ok(())
                        }
                    },
                    {
                        let state = Arc::clone(&state);
                        let current = || worker.ensure_current(id);
                        move |batch: MeshBatch| -> Result<(), String> {
                            let result = state.lock().consume(&current, &batch);
                            drop(batch);
                            result
                        }
                    },
                    {
                        let state = Arc::clone(&state);
                        move |phase, completed, _total| {
                            state.lock().progress(phase, completed);
                        }
                    },
                )?;
                state.lock().finish(&current, &metadata)?;
                worker.release(id);
                Ok(())
            }))
            .unwrap_or_else(|panic| Err(panic_text(&panic)));
            let error = match outcome_result {
                Ok(()) => None,
                Err(failure) => Some(failure),
            };
            worker.release(id);
            job_stop.store(true, Ordering::Release);
            let summary = summarize(&job_series);
            let row = state
                .lock()
                .outcome(summary, mode, load, &fixture_text, error);
            let _ = started;
            let _ = sender.send(row);
        }
    });
    let received = if let Err(failure) = worker.submit(id, prepare, job) {
        Err(failure)
    } else {
        receiver
            .recv()
            .map_err(|_| String::from("The load job closed without a record."))
    };
    let summary = settle(&mut watch);
    match received {
        Ok(row) => row,
        Err(failure) => state
            .lock()
            .outcome(summary, mode, load, fixture, Some(failure)),
    }
}

fn cold_load(fixture: &str, rig: Option<Arc<Rig>>, settings: &Settings) -> OutcomeRow {
    // The app mints request ids per `start_preview`, and the worker stores the
    // generation on submit, so a fresh worker starts at id 1.
    let worker = Arc::new(preview::PreviewWorker::default());
    run_load(worker, 1, "cold", 1, fixture, rig, settings)
}

fn warm_loads(settings: &Settings) -> Vec<OutcomeRow> {
    let worker = Arc::new(preview::PreviewWorker::default());
    (1..=4)
        .map(|load| {
            run_load(
                Arc::clone(&worker),
                load,
                "warm",
                load,
                "Atrium",
                None,
                settings,
            )
        })
        .collect()
}

fn check_cold(row: &OutcomeRow, frames_reported: bool) -> Vec<String> {
    let mut breaches = retained_breach(&format!("cold {}", row.fixture), row.retained_bytes);
    if let Some(percent) = row.decoder_cap_percent {
        if percent >= 95.0 {
            breaches.push(format!(
                "cold {}: decoder child private working set sits at {percent}% of the decoder memory cap",
                row.fixture
            ));
        }
    }
    if frames_reported && row.frames_growth_bytes.abs() as usize > FRAME_STABLE_BUDGET {
        breaches.push(format!(
            "cold {}: the frame loop changed host allocations by {} bytes, over the \
             {FRAME_STABLE_BUDGET} byte allowance",
            row.fixture, row.frames_growth_bytes
        ));
    }
    breaches
}

fn check_warm(rows: &[OutcomeRow]) -> Vec<String> {
    let mut breaches = Vec::new();
    for row in rows {
        breaches.extend(retained_breach(
            &format!("warm load {} of {}", row.load, row.fixture),
            row.retained_bytes,
        ));
    }
    let baseline = rows
        .iter()
        .find(|row| row.load == 2)
        .and_then(|row| row.child_peak);
    for row in rows.iter().filter(|row| row.load >= 3) {
        if let (Some(base), Some(child)) = (baseline, row.child_peak) {
            if child > base + REUSE_GROWTH_BUDGET as u64 {
                breaches.push(format!(
                    "warm load {} of {}: decoder child reached {} bytes, \
                     {} bytes over the load-2 baseline plus the reuse allowance",
                    row.load,
                    row.fixture,
                    child,
                    child - base,
                ));
            }
        }
    }
    breaches
}

struct CoreRun {
    record: CoreRecord,
    before: usize,
    decode_start: Instant,
    prepared: Option<Instant>,
    opened: Option<Instant>,
    first_batch: Option<Instant>,
    last_batch: Option<Instant>,
    gaps: Vec<f64>,
    geometry_min: [f64; 3],
    geometry_max: [f64; 3],
    chunk_seen: HashSet<(i16, i16, i16)>,
    duplicate_chunks: u64,
    greedy: HashSet<u32>,
    decode_peak: usize,
    part_seen: bool,
}

impl CoreRun {
    fn new(name: &str, before: usize) -> Self {
        Self {
            record: CoreRecord {
                fixture: name.to_string(),
                ..CoreRecord::default()
            },
            before,
            decode_start: Instant::now(),
            prepared: None,
            opened: None,
            first_batch: None,
            last_batch: None,
            gaps: Vec::new(),
            geometry_min: [f64::INFINITY; 3],
            geometry_max: [f64::NEG_INFINITY; 3],
            chunk_seen: HashSet::new(),
            duplicate_chunks: 0,
            greedy: HashSet::new(),
            decode_peak: 0,
            part_seen: false,
        }
    }

    fn on_prepared(&mut self) {
        if self.prepared.is_none() {
            self.prepared = Some(Instant::now());
            self.record.decode_ms = ms_since(self.decode_start, Some(Instant::now()));
            self.decode_peak = meter::AllocationMeter::peak();
            meter::AllocationMeter::reset_peak();
        }
    }

    fn mesh_zero(&mut self) {
        self.opened.get_or_insert_with(Instant::now);
    }

    fn progress(&mut self, phase: &'static str, completed: u64) {
        if phase == "mesh" && completed == 0 {
            self.mesh_zero();
        }
    }

    fn consume(&mut self, batch: &MeshBatch) {
        let now = Instant::now();
        if self.opened.is_none() {
            self.opened = Some(now);
        }
        if let Some(previous) = self.last_batch {
            self.gaps.push(ms_since(previous, Some(now)));
        }
        self.first_batch.get_or_insert(now);
        self.last_batch = Some(now);
        self.record.batches += 1;
        self.record.triangles += batch.info.triangle_count;
        self.record.vertices += batch.vertex_count as u64;
        self.record.payload_bytes += staged_bytes(batch) as u64;
        if !batch.parts.is_empty() {
            self.part_seen = true;
            for axis in 0..3 {
                self.geometry_min[axis] = self.geometry_min[axis].min(batch.info.min[axis] as f64);
                self.geometry_max[axis] = self.geometry_max[axis].max(batch.info.max[axis] as f64);
            }
        }
        if let Some(texture) = batch.textures.iter().find(|texture| texture.index == 0) {
            if self.record.atlas == [0, 0] {
                self.record.atlas = [texture.width as u64, texture.height as u64];
            }
        }
        for texture in &batch.textures {
            if texture.index != 0 {
                self.greedy.insert(texture.index);
            }
        }
        self.record.greedy_textures = self.greedy.len() as u64;
        if let Some((x, y, z)) = batch.chunk_coord {
            let scaled = (x as i16, y as i16, z as i16);
            if !self.chunk_seen.insert(scaled) {
                self.duplicate_chunks += 1;
            }
        }
    }

    fn finish(&mut self, error: Option<String>, elapsed: Duration) {
        self.record.error = error;
        self.record.streaming_peak_bytes = meter::AllocationMeter::peak();
        for axis in 0..3 {
            if self.part_seen {
                self.record.geometry_extent[axis] =
                    self.geometry_max[axis] - self.geometry_min[axis];
            }
        }
        self.record.total_ms = round_at(elapsed.as_secs_f64() * 1_000.0, 3);
        self.record.stream_open_ms = ms_since(self.decode_start, self.opened);
        self.record.stream_ms = self
            .last_batch
            .map(|last| ms_since(self.opened.unwrap_or(self.decode_start), Some(last)))
            .unwrap_or(0.0);
        self.record.first_batch_ms =
            ms_since(self.opened.unwrap_or(self.decode_start), self.first_batch);
        self.record.decode_peak_bytes = self.decode_peak;
        self.record.retained_bytes = meter::AllocationMeter::live().saturating_sub(self.before);
        self.record.max_batch_gap_ms = self.gaps.iter().cloned().fold(0.0, f64::max);
        self.record.p95_batch_gap_ms = percentile(self.gaps.clone(), 0.95);
        self.record.blocks_per_second = round_at(
            self.record.blocks as f64 / self.record.decode_ms.max(0.001) * 1_000.0,
            0,
        );
    }
}

fn run_core_pass(
    settings: &Settings,
    names: &[String],
    baselines: &[Baseline],
) -> (Vec<CoreRecord>, Vec<String>, Vec<String>) {
    let mut records = Vec::new();
    let mut breaches = Vec::new();
    let mut notes = Vec::new();
    // The decoder child parses the pack once and reuses it across loads, so
    // the core pass parses once too and keeps the per-fixture retained
    // budget to the model alone.
    let mut packs = match decode_pack() {
        Ok(parsed) => preview::PackCache::with_base(parsed),
        Err(error) => return (records, vec![error], notes),
    };
    for name in names {
        let before = meter::AllocationMeter::live();
        meter::AllocationMeter::reset_peak();
        let run = Rc::new(RefCell::new(CoreRun::new(name, before)));
        let result = preview::decode(
            &fixture_path(name),
            &pack_path(),
            &[],
            &mut packs,
            settings.options(),
            || Ok(()),
            {
                let run = Rc::clone(&run);
                move |_min, _max| -> Result<(), String> {
                    run.borrow_mut().on_prepared();
                    Ok(())
                }
            },
            {
                let run = Rc::clone(&run);
                move |batch: MeshBatch| -> Result<(), String> {
                    run.borrow_mut().consume(&batch);
                    drop(batch);
                    Ok(())
                }
            },
            {
                let run = Rc::clone(&run);
                move |phase, completed, _total| {
                    run.borrow_mut().progress(phase, completed);
                }
            },
        );
        let elapsed = run.borrow().decode_start.elapsed();
        let failure = result.as_ref().err().cloned();
        {
            let mut run_ref = run.borrow_mut();
            if failure.is_none() {
                if let Ok(metadata) = &result {
                    run_ref.record.blocks = metadata.block_count.max(0) as u64;
                    run_ref.record.block_entities = metadata.block_entity_count.max(0) as u64;
                    for axis in 0..3 {
                        run_ref.record.content[axis] =
                            (metadata.max[axis] - metadata.min[axis]) as f64;
                    }
                }
            }
            run_ref.finish(failure.clone(), elapsed);
        }
        let record = run.borrow().record.clone();
        if result.is_err() {
            breaches.push(format!(
                "{}: decode failed: {}",
                name,
                record.error.clone().unwrap_or_default()
            ));
        } else {
            if run.borrow().duplicate_chunks > 0 {
                breaches.push(format!(
                    "{}: {} batches repeated a chunk coordinate",
                    name,
                    run.borrow().duplicate_chunks
                ));
            }
            match baselines.iter().find(|entry| entry.name == *name) {
                Some(baseline) => breaches.extend(check_baseline(baseline, &record)),
                None => notes.push(format!(
                    "{name}: no pinned baseline entry yet; run --write-baselines"
                )),
            }
            breaches.extend(retained_breach(name, record.retained_bytes));
        }
        records.push(record);
    }
    (records, breaches, notes)
}

fn load_options(settings: &Settings) -> PreviewOptions {
    settings.options()
}

#[derive(Debug, Serialize, Deserialize)]
struct GpuMeta {
    adapter: String,
    /// Absent from older reports, which recorded only the adapter debug dump.
    #[serde(default)]
    backend: String,
    samples: u32,
    format: String,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct MetaReport {
    generated_unix_ms: u128,
    os: String,
    rev: Option<String>,
    cores: usize,
    threads: u8,
    chunk: u16,
    frames: u32,
    adapter: Option<GpuMeta>,
    passes_ran: Vec<String>,
    failures: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Report {
    meta: MetaReport,
    core: Vec<CoreRecord>,
    loads: Vec<OutcomeRow>,
}

/// Shared runners vary by tens of percent from run to run. Only changes that
/// clear these bands are shown; the bands are advisory and never gate a run.
const TIMING_TOLERANCE: f64 = 20.0;
const SHORT_TIMING_TOLERANCE: f64 = 25.0;
const MEMORY_TOLERANCE: f64 = 10.0;

#[derive(Clone)]
struct MetricPoint {
    key: String,
    phase: &'static str,
    target: String,
    metric: &'static str,
    digits: usize,
    value: f64,
    tolerance: f64,
}

fn push_metric(
    rows: &mut Vec<MetricPoint>,
    phase: &'static str,
    target: &str,
    metric: &'static str,
    digits: usize,
    value: f64,
    tolerance: f64,
) {
    rows.push(MetricPoint {
        key: format!("{phase}/{target}/{metric}"),
        phase,
        target: target.to_string(),
        metric,
        digits,
        value,
        tolerance,
    });
}

/// The runtime metrics worth comparing. Block, batch, and triangle counts are
/// pinned in `baselines.json`; timing and memory peaks are the values that
/// move with the code. The warm pass reports its two ends: the first load
/// fills the child pack cache and the last one is the steady state.
fn report_metrics(report: &Report) -> Vec<MetricPoint> {
    let mut rows = Vec::new();
    for record in report.core.iter().filter(|record| record.error.is_none()) {
        let target = record.fixture.as_str();
        push_metric(
            &mut rows,
            "core",
            target,
            "decode (ms)",
            0,
            record.decode_ms,
            TIMING_TOLERANCE,
        );
        push_metric(
            &mut rows,
            "core",
            target,
            "mesh (ms)",
            0,
            record.stream_ms,
            TIMING_TOLERANCE,
        );
        push_metric(
            &mut rows,
            "core",
            target,
            "total (ms)",
            0,
            record.total_ms,
            TIMING_TOLERANCE,
        );
        push_metric(
            &mut rows,
            "core",
            target,
            "peak decode (MB)",
            1,
            mb(record.decode_peak_bytes),
            MEMORY_TOLERANCE,
        );
        push_metric(
            &mut rows,
            "core",
            target,
            "peak mesh (MB)",
            1,
            mb(record.streaming_peak_bytes),
            MEMORY_TOLERANCE,
        );
    }
    for row in report
        .loads
        .iter()
        .filter(|row| row.mode == "cold" && row.error.is_none())
    {
        let target = row.fixture.as_str();
        push_metric(
            &mut rows,
            "cold",
            target,
            "decode (ms)",
            0,
            row.decode_ms,
            TIMING_TOLERANCE,
        );
        push_metric(
            &mut rows,
            "cold",
            target,
            "first frame (ms)",
            0,
            row.first_batch_ms,
            SHORT_TIMING_TOLERANCE,
        );
        if row.full_render_ms > 0.0 {
            push_metric(
                &mut rows,
                "cold",
                target,
                "full render (ms)",
                0,
                row.full_render_ms,
                TIMING_TOLERANCE,
            );
        }
        if let Some(bytes) = row.child_peak {
            push_metric(
                &mut rows,
                "cold",
                target,
                "child peak (MB)",
                1,
                mb_bytes(bytes),
                MEMORY_TOLERANCE,
            );
        }
        if row.frames_measured > 0 {
            push_metric(
                &mut rows,
                "cold",
                target,
                "frames p95 (ms)",
                1,
                row.frames_p95_ms,
                TIMING_TOLERANCE,
            );
        }
    }
    for row in report
        .loads
        .iter()
        .filter(|row| row.mode == "warm" && row.error.is_none() && matches!(row.load, 1 | 4))
    {
        let target = format!("load {}", row.load);
        push_metric(
            &mut rows,
            "warm",
            &target,
            "decode (ms)",
            0,
            row.decode_ms,
            TIMING_TOLERANCE,
        );
        push_metric(
            &mut rows,
            "warm",
            &target,
            "mesh (ms)",
            0,
            row.stream_ms,
            TIMING_TOLERANCE,
        );
        if let Some(bytes) = row.child_peak {
            push_metric(
                &mut rows,
                "warm",
                &target,
                "child peak (MB)",
                1,
                mb_bytes(bytes),
                MEMORY_TOLERANCE,
            );
        }
    }
    rows
}

fn mb_bytes(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

fn markdown_row(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

fn markdown_table(headers: &[&str], layout: impl Fn(usize) -> Vec<String>, rows: usize) -> String {
    let mut text = String::new();
    text.push_str(&markdown_row(
        &headers
            .iter()
            .map(|header| header.to_string())
            .collect::<Vec<_>>(),
    ));
    text.push('\n');
    text.push_str(&markdown_row(
        &headers
            .iter()
            .map(|_| String::from("---"))
            .collect::<Vec<_>>(),
    ));
    text.push('\n');
    for index in 0..rows {
        text.push_str(&markdown_row(&layout(index)));
        text.push('\n');
    }
    text
}

fn rendered_markdown(report: &Report, comparison: Option<&Comparison>) -> String {
    let mut text = String::new();
    let meta = &report.meta;
    text.push_str("## Million-block pressure report\n\n");
    text.push_str(&format!(
        "- Host: {} with {} logical cores; revision `{}`\n",
        meta.os,
        meta.cores,
        meta.rev.clone().unwrap_or_else(|| String::from("unknown"))
    ));
    text.push_str(&format!(
        "- Run: {} worker threads, chunk size {}, {} timed frames per load\n",
        meta.threads, meta.chunk, meta.frames
    ));
    match &meta.adapter {
        Some(gpu) => {
            let backend = if gpu.backend.is_empty() {
                String::new()
            } else {
                format!(" ({})", gpu.backend)
            };
            text.push_str(&format!(
                "- GPU: {}{backend}, {}×{}, {}× MSAA, {}\n",
                gpu.adapter, gpu.width, gpu.height, gpu.samples, gpu.format
            ));
        }
        None => text.push_str("- GPU: unavailable; frame and device metrics are omitted\n"),
    }
    text.push('\n');
    if !report.core.is_empty() {
        text.push_str("### Core decode (in-process, allocator-measured memory)\n\n");
        let decoded: Vec<&CoreRecord> = report
            .core
            .iter()
            .filter(|record| record.error.is_none())
            .collect();
        text.push_str(&markdown_table(
            &[
                "fixture",
                "blocks",
                "batches",
                "triangles (M)",
                "payload (MB)",
                "decode (ms)",
                "stream open (ms)",
                "stream (ms)",
                "total (ms)",
                "peak decode (MB)",
                "peak stream (MB)",
                "retained (MB)",
            ],
            |index| {
                let record = decoded[index];
                vec![
                    record.fixture.clone(),
                    count(record.blocks),
                    count(record.batches),
                    number(record.triangles as f64 / 1_000_000.0, 2),
                    number(mb_bytes(record.payload_bytes), 1),
                    number(record.decode_ms, 0),
                    number(record.stream_open_ms, 0),
                    number(record.stream_ms, 0),
                    number(record.total_ms, 0),
                    number(mb(record.decode_peak_bytes), 1),
                    number(mb(record.streaming_peak_bytes), 1),
                    number(mb(record.retained_bytes), 2),
                ]
            },
            decoded.len(),
        ));
        for record in report.core.iter().filter(|record| record.error.is_some()) {
            let error = record.error.as_deref().unwrap_or_default();
            text.push_str(&format!(
                "- **{} failed:** {}\n",
                record.fixture,
                error.lines().next().unwrap_or_default()
            ));
        }
        text.push('\n');
    }
    let cold: Vec<&OutcomeRow> = report
        .loads
        .iter()
        .filter(|row| row.mode == "cold")
        .collect();
    if !cold.is_empty() {
        text.push_str("### Cold pass (fresh decoder child per fixture, full production load)\n\n");
        let frames_reported = cold.iter().any(|row| row.frames_measured > 0);
        let mut headers = vec!["fixture", "connect (ms)", "decode (ms)", "stream open (ms)"];
        if frames_reported {
            headers.extend(["first frame (ms)", "full render (ms)"]);
        }
        headers.extend(["batches", "decoder child peak (MB)", "decoder cap use (%)"]);
        if frames_reported {
            headers.extend(["frames p50/p95 (ms)", "frame-loop growth (KB)"]);
        }
        headers.extend(["host peak (MB)", "retained (MB)"]);
        text.push_str(&markdown_table(
            &headers,
            |index| {
                let row = cold[index];
                let mut cells = vec![
                    row.fixture.clone(),
                    number(row.connect_ms, 0),
                    number(row.decode_ms, 0),
                    number(row.stream_open_ms, 0),
                ];
                if frames_reported {
                    cells.push(cell_option_time(row.first_frame_ms));
                    cells.push(cell_option_time(row.full_render_ms));
                }
                cells.push(count(row.batches));
                cells.push(mb_opt(row.child_peak));
                cells.push(num_opt(row.decoder_cap_percent));
                if frames_reported {
                    cells.push(if row.frames_measured == 0 {
                        String::from("-")
                    } else {
                        format!(
                            "{} / {}",
                            number(row.frames_p50_ms, 1),
                            number(row.frames_p95_ms, 1)
                        )
                    });
                    cells.push(if row.frames_measured == 0 {
                        String::from("-")
                    } else {
                        number(row.frames_growth_bytes as f64 / 1024.0, 1)
                    });
                }
                cells.push(mb_opt(row.host_peak));
                cells.push(number(mb(row.retained_bytes), 2));
                cells
            },
            cold.len(),
        ));
        text.push('\n');
    }
    let warm: Vec<&OutcomeRow> = report
        .loads
        .iter()
        .filter(|row| row.mode == "warm")
        .collect();
    if !warm.is_empty() {
        text.push_str("### Warm pass (one decoder child reused across four Atrium loads)\n\n");
        text.push_str(&markdown_table(
            &[
                "load",
                "decode (ms)",
                "stream open (ms)",
                "stream (ms)",
                "decoder child peak (MB)",
                "host peak (MB)",
                "retained (MB)",
            ],
            |index| {
                let row = warm[index];
                vec![
                    row.load.to_string(),
                    number(row.decode_ms, 0),
                    number(row.stream_open_ms, 0),
                    number(row.stream_ms, 0),
                    mb_opt(row.child_peak),
                    mb_opt(row.host_peak),
                    number(mb(row.retained_bytes), 2),
                ]
            },
            warm.len(),
        ));
        text.push('\n');
    }
    if !report.meta.failures.is_empty() {
        text.push_str("### Breaches\n\n");
        for failure in &report.meta.failures {
            text.push_str(&format!("- {failure}\n"));
        }
        text.push('\n');
    }
    if let Some(comparison) = comparison {
        text.push_str("### Change vs baseline\n\n");
        text.push_str(&format!(
            "- Baseline `{}`; {} representative metrics compared.\n",
            comparison.baseline_rev.as_deref().unwrap_or("unknown"),
            comparison.compared,
        ));
        text.push_str(&format!(
            "- Noise bands: timing {TIMING_TOLERANCE}%, short timings {SHORT_TIMING_TOLERANCE}%, \
             memory {MEMORY_TOLERANCE}%; positive means slower or more memory.\n\n"
        ));
        if comparison.deltas.is_empty() {
            text.push_str("No representative metric moved outside its noise band.\n\n");
        } else {
            text.push_str(&markdown_table(
                &[
                    "phase", "target", "metric", "baseline", "this run", "change",
                ],
                |index| {
                    let delta = &comparison.deltas[index];
                    vec![
                        delta.phase.to_string(),
                        delta.target.clone(),
                        delta.metric.to_string(),
                        number(delta.baseline, delta.digits),
                        number(delta.current, delta.digits),
                        format!("{}%", grouped(&format!("{:+.1}", delta.change))),
                    ]
                },
                comparison.deltas.len(),
            ));
            text.push('\n');
        }
    }
    if !report.meta.warnings.is_empty() {
        text.push_str(&format!(
            "> **Warnings:** {}\n",
            report.meta.warnings.join("; ")
        ));
    }
    text
}

fn cell_option_time(value: f64) -> String {
    if value <= 0.0 {
        String::from("-")
    } else {
        number(value, 0)
    }
}

struct Comparison {
    baseline_rev: Option<String>,
    compared: usize,
    deltas: Vec<Delta>,
}

struct Delta {
    phase: &'static str,
    target: String,
    metric: &'static str,
    digits: usize,
    baseline: f64,
    current: f64,
    change: f64,
}

fn compare_reports(current: &Report, baseline: &Report) -> Comparison {
    let baseline_values: HashMap<String, f64> = report_metrics(baseline)
        .into_iter()
        .map(|point| (point.key, point.value))
        .collect();
    let mut compared = 0;
    let mut deltas = Vec::new();
    for point in report_metrics(current) {
        let Some(&base) = baseline_values.get(&point.key) else {
            continue;
        };
        if base <= 0.0 {
            continue;
        }
        compared += 1;
        let change = round_at((point.value - base) / base * 100.0, 1);
        if change.abs() > point.tolerance {
            deltas.push(Delta {
                phase: point.phase,
                target: point.target,
                metric: point.metric,
                digits: point.digits,
                baseline: base,
                current: point.value,
                change,
            });
        }
    }
    Comparison {
        baseline_rev: baseline.meta.rev.clone(),
        compared,
        deltas,
    }
}

fn write_report(report: &Report, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(&settings.out)
        .map_err(|error| format!("Unable to create {}: {error}", settings.out.display()))?;
    let json_path = settings.out.join("pressure.json");
    let markdown_path = settings.out.join("pressure.md");
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("Unable to serialize the report: {error}"))?;
    std::fs::write(&json_path, json)
        .map_err(|error| format!("Unable to write {}: {error}", json_path.display()))?;
    std::fs::write(&markdown_path, markdown_text_for(report, settings))
        .map_err(|error| format!("Unable to write {}: {error}", markdown_path.display()))?;
    println!("Wrote {}", json_path.display());
    println!("Wrote {}", markdown_path.display());
    Ok(())
}

fn markdown_text_for(report: &Report, settings: &Settings) -> String {
    let comparison = settings.compare.as_ref().and_then(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let baseline = serde_json::from_str::<Report>(&text).ok()?;
        Some(compare_reports(report, &baseline))
    });
    rendered_markdown(report, comparison.as_ref())
}

fn record_baselines(settings: &Settings) -> Result<(), String> {
    let (records, breaches, notes) = run_core_pass(settings, &settings.fixtures, &[]);
    if !breaches.is_empty() {
        for failure in &breaches {
            eprintln!("{failure}");
        }
        return Err(String::from("Baseline recording found decode breaches."));
    }
    let recorded: Vec<Baseline> = records
        .iter()
        .filter(|record| record.error.is_none())
        .map(|record| Baseline {
            name: record.fixture.clone(),
            blocks: record.blocks,
            block_entities: record.block_entities,
            content: record.content,
            batches: record.batches,
            atlas: record.atlas,
            greedy_textures: record.greedy_textures,
            triangles: record.triangles,
            payload_bytes: record.payload_bytes,
            geometry_extent: record.geometry_extent,
        })
        .collect();
    // A subset refresh must not drop the pinned entries it did not run.
    let mut fixtures = load_baselines();
    fixtures.retain(|entry| !recorded.iter().any(|record| record.name == entry.name));
    fixtures.extend(recorded);
    fixtures.sort_by_key(|entry| {
        FIXTURES
            .iter()
            .position(|name| *name == entry.name.as_str())
            .unwrap_or(usize::MAX)
    });
    let file = BaselineFile { fixtures };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|error| format!("Unable to serialize the baselines: {error}"))?;
    std::fs::write(baselines_path(), format!("{text}\n"))
        .map_err(|error| format!("Unable to write {}: {error}", baselines_path().display()))?;
    for note in notes {
        println!("{note}");
    }
    println!("Recorded {} pinned fixtures.", file.fixtures.len());
    Ok(())
}

fn run_suite(settings: &Settings) -> ExitCode {
    let baselines = load_baselines();
    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    let mut core = Vec::new();
    let mut loads = Vec::new();
    let mut adapter = None;
    let passes = &settings.passes;
    if passes.iter().any(|pass| pass == "core") {
        let (records, core_breaches, notes) =
            run_core_pass(settings, &settings.fixtures, &baselines);
        core.extend(records);
        failures.extend(core_breaches);
        warnings.extend(notes);
    }
    if passes.iter().any(|pass| pass == "cold") {
        let (rows, cold_breaches, gpu_notice, cold_adapter) =
            run_cold_pass(settings, &settings.fixtures);
        loads.extend(rows);
        failures.extend(cold_breaches);
        adapter = cold_adapter;
        if let Some(notice) = gpu_notice {
            warnings.push(notice);
        }
        if !settings.render {
            warnings.push(String::from("Frame timings skipped by --no-render"));
        }
    }
    if passes.iter().any(|pass| pass == "warm") {
        let rows = warm_loads(settings);
        for row in &rows {
            if let Some(error) = &row.error {
                failures.push(format!(
                    "warm load {} of {}: {error}",
                    row.load, row.fixture
                ));
            }
        }
        failures.extend(check_warm(&rows));
        loads.extend(rows);
    }
    let report = Report {
        meta: MetaReport {
            generated_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|span| span.as_millis())
                .unwrap_or_default(),
            os: std::env::consts::OS.to_string(),
            rev: git_rev(),
            cores: std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1),
            threads: settings.threads,
            chunk: settings.chunk,
            frames: settings.frames,
            adapter,
            passes_ran: settings.passes.clone(),
            failures,
            warnings,
        },
        core,
        loads,
    };
    if let Err(error) = write_report(&report, settings) {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    let failures = &report.meta.failures;
    for failure in failures {
        eprintln!("[pressure] {failure}");
    }
    ExitCode::from(failures.len().min(u8::MAX as usize) as u8)
}

fn run_cold_pass(
    settings: &Settings,
    names: &[String],
) -> (
    Vec<OutcomeRow>,
    Vec<String>,
    Option<String>,
    Option<GpuMeta>,
) {
    let mut rows = Vec::new();
    let mut breaches = Vec::new();
    let (rig, gpu_notice): (Option<Arc<Rig>>, Option<String>) = match request_rig() {
        Ok((rig, notice)) => (Some(Arc::new(rig)), notice),
        Err(notice) => (None, Some(notice)),
    };
    let frames_reported = settings.render && rig.as_ref().is_some_and(|rig| rig.hardware);
    let adapter = rig.as_ref().map(|rig| GpuMeta {
        adapter: rig.adapter.clone(),
        backend: rig.backend.clone(),
        samples: rig.samples,
        format: format!("{:?}", rig.format),
        width: FRAME_WIDTH,
        height: FRAME_HEIGHT,
    });
    for name in names {
        let row = cold_load(name, rig.clone(), settings);
        if let Some(error) = &row.error {
            breaches.push(format!("cold {}: {error}", row.fixture));
        }
        breaches.extend(check_cold(&row, frames_reported));
        rows.push(row);
    }
    (rows, breaches, gpu_notice, adapter)
}

pub fn main() -> ExitCode {
    let mut arguments = std::env::args_os();
    let _binary = arguments.next();
    let all: Vec<std::ffi::OsString> = arguments.collect();
    if all.first().map(|argument| argument.as_os_str()) == Some(OsStr::new("--preview-worker")) {
        let result = match all.get(1) {
            Some(port) => preview_process::run(port.as_os_str()),
            None => Err(String::from("Pass exactly one decoder connection port.")),
        };
        if let Err(error) = result {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    let rest: Vec<String> = all
        .iter()
        .map(|text| text.to_string_lossy().into_owned())
        .collect();
    let settings = match parse_args(&rest) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if settings.write_baselines {
        if let Err(error) = record_baselines(&settings) {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    run_suite(&settings)
}

#[cfg(test)]
mod tests {
    use super::{count, grouped, number};

    #[test]
    fn groups_thousands_without_touching_sign_or_fraction() {
        assert_eq!(count(1_121_671), "1,121,671");
        assert_eq!(count(791), "791");
        assert_eq!(number(1651.26, 1), "1,651.3");
        assert_eq!(number(-9.0, 2), "-9.00");
        assert_eq!(grouped("+1234567.5"), "+1,234,567.5");
        assert_eq!(grouped("0.02"), "0.02");
    }
}
