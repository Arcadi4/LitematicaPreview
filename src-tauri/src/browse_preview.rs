//! Speculative browse work after a foreground preview is already visible.
//!
//! Everything here is best effort and token-bounded: the next foreground
//! submit invalidates the preload token, which interrupts hashing and decoder
//! work. Failures and panics are swallowed so the successful preview and the
//! session worker's scheduling stay intact. A thumbnail is only stored and
//! announced when the PNG was actually written for the file that is still on
//! disk.
//!
//! Speculation borrows the session worker's mutable decoder, the same one
//! foreground loads use. A completed preload releases its claim, so the idle
//! decoder is not stopped at the end of this work; only work still in flight
//! when the token dies is interrupted.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use litematica_preview_core::PreviewOptions;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::browse_thumbnail::{
    ThumbnailCache, ThumbnailSource, finish_capture_png, store_source_thumbnail_checked,
};
use crate::formats;
use crate::preview::{PackLayer, PreviewWorker};
use crate::preview_process::{self, DecoderProcess, PreloadOutcome};
use crate::render::context::{NativeRenderer, ThumbnailRenderer};
use crate::resources;

/// The square edge the browse grid displays.
pub const THUMBNAIL_SIDE: u32 = 128;

/// The most neighbours that may follow a browsed file, on either side.
const MAX_NEIGHBORS: usize = 2;

/// A cheap identity for the opened file. It is captured before the foreground
/// decode so a file that changed while the preview loaded cannot be cached
/// under its new hash with the old geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceStat {
    len: u64,
    modified: Option<SystemTime>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowseThumbnail {
    path: String,
}

/// Resolves the frontend's neighbour list. `None` marks a demo or other
/// non-browse open, which does no speculative work at all. `Some([])` is a
/// single-file selection.
pub fn neighbors(active: &Path, requested: Option<Vec<String>>) -> Option<Vec<PathBuf>> {
    let requested = requested?;
    let mut paths: Vec<PathBuf> = Vec::with_capacity(MAX_NEIGHBORS);
    for requested in requested {
        let path = PathBuf::from(requested);
        if !formats::is_supported_path(&path) || same_path(active, &path) {
            continue;
        }
        if paths.iter().any(|kept| same_path(kept, &path)) {
            continue;
        }
        paths.push(path);
        if paths.len() == MAX_NEIGHBORS {
            break;
        }
    }
    Some(paths)
}

pub fn source_stat(path: &Path) -> Option<SourceStat> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(SourceStat {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Runs after a successful foreground preview was released and announced.
///
/// `decoder` is the session worker's mutable `Option`, so speculative decodes
/// reuse the same process instead of spawning another. The work runs
/// synchronously on the session worker thread and yields as soon as the
/// preload token stops being current.
#[allow(clippy::too_many_arguments)]
pub fn after_preview(
    app: &AppHandle,
    worker: &Arc<PreviewWorker>,
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    foreground_id: u64,
    active: &Path,
    neighbors: Option<&[PathBuf]>,
    pack_path: &Path,
    layers: &[PackLayer],
    options: PreviewOptions,
    max_texture_side: u32,
    active_stat: Option<SourceStat>,
    decoder: &mut Option<DecoderProcess>,
) {
    // Demos and other non-browse opens leave the cache and the GPU alone.
    let Some(neighbors) = neighbors else {
        return;
    };
    // The foreground is loaded and released; only now may speculation start.
    let Ok(token) = worker.begin_preload(foreground_id) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| {
        speculate(
            app,
            worker,
            renderer,
            foreground_id,
            token,
            active,
            neighbors,
            pack_path,
            layers,
            options,
            max_texture_side,
            active_stat,
            decoder,
        );
    }));
    // A finished preload has already released its claim, so this only drops
    // the token; a panicked load is interrupted here. Stored cache entries
    // survive either way.
    worker.cancel_preload();
}

#[allow(clippy::too_many_arguments)]
fn speculate(
    app: &AppHandle,
    worker: &PreviewWorker,
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    foreground_id: u64,
    token: u64,
    active: &Path,
    neighbors: &[PathBuf],
    pack_path: &Path,
    layers: &[PackLayer],
    options: PreviewOptions,
    max_texture_side: u32,
    active_stat: Option<SourceStat>,
    decoder: &mut Option<DecoderProcess>,
) {
    let cache = ThumbnailCache::from_app(app).ok();

    // The foreground already consumed whatever this list provides, so stale
    // meshes for other selections are safe to drop now.
    let mut keep = Vec::with_capacity(neighbors.len() + 1);
    keep.push(active.to_path_buf());
    keep.extend(neighbors.iter().cloned());
    preview_process::evict_except(&keep);

    let mut scratch = None;

    // The live scene is valid only until the next foreground submit calls
    // begin_load. Hash first so a cached thumbnail costs no GPU work, then
    // capture the opened vanilla scene only while the token is still current.
    if layers.is_empty() {
        if let Some(cache) = &cache {
            if let Some(source) = open_unchanged(worker, token, active, active_stat) {
                if !cache.contains(source.key()) {
                    if let Some(png) = capture_live(worker, renderer, foreground_id) {
                        store_and_emit(app, worker, token, cache, &source, &png, active);
                    }
                }
            }
        }
    }

    for neighbor in neighbors {
        if worker.ensure_preload(token).is_err() {
            return;
        }
        // Warm the file as the user would open it, with their pack stack.
        let warmed = preview_process::preload(
            decoder,
            worker,
            token,
            neighbor,
            pack_path,
            layers,
            options,
            max_texture_side,
            |_, _, _| {},
        );
        if !matches!(warmed, Ok(PreloadOutcome::Cached(_))) {
            continue;
        }
        let Some(cache) = &cache else {
            continue;
        };
        // The hash names the PNG by content. The cached mesh is keyed by path,
        // pack layers and options, and persisting revalidates this source, so
        // a stale image is never stored under fresh bytes.
        let Ok(source) =
            ThumbnailSource::open_checked(neighbor.to_path_buf(), || worker.ensure_preload(token))
        else {
            if worker.ensure_preload(token).is_err() {
                return;
            }
            continue;
        };
        let Some(source) = source else {
            continue;
        };
        if cache.contains(source.key()) {
            continue;
        }
        // Thumbnails always use bundled vanilla, so warm that geometry too
        // when the user's pack stack produced a different cache entry.
        if !layers.is_empty() {
            let vanilla = preview_process::preload(
                decoder,
                worker,
                token,
                neighbor,
                pack_path,
                &[],
                options,
                max_texture_side,
                |_, _, _| {},
            );
            if !matches!(vanilla, Ok(PreloadOutcome::Cached(_))) {
                continue;
            }
        }
        let Some(scratch) = thumbnail_scratch(&mut scratch, renderer) else {
            continue;
        };
        let rendered = preview_process::with_cached(
            neighbor,
            pack_path,
            &[],
            options,
            max_texture_side,
            |metadata, batches| -> Result<Vec<u8>, String> {
                let current = || worker.ensure_preload(token);
                current()?;
                scratch.reset();
                scratch.prepare(metadata.min, metadata.max);
                for batch in batches {
                    scratch
                        .attach_checked(batch, &current)
                        .map_err(|error| error.to_string())?;
                }
                scratch.finish(&metadata.info());
                let job = scratch
                    .begin_capture_checked(THUMBNAIL_SIDE, current)
                    .map_err(|error| error.to_string())?;
                finish_capture_png(job)
            },
        );
        if let Some(Ok(png)) = rendered {
            store_and_emit(app, worker, token, cache, &source, &png, neighbor);
        }
    }

    // A custom-pack preview keeps the live scene in the user's textures, so
    // its browse thumbnail is rendered separately, after the neighbours.
    if !layers.is_empty() {
        if worker.ensure_preload(token).is_err() {
            return;
        }
        let Some(cache) = &cache else {
            return;
        };
        let Some(source) = open_unchanged(worker, token, active, active_stat) else {
            return;
        };
        if cache.contains(source.key()) {
            return;
        }
        let Some(scratch) = thumbnail_scratch(&mut scratch, renderer) else {
            return;
        };
        let cell = RefCell::new(scratch);
        let thumbnail_options = PreviewOptions {
            thread_count: Some(1),
            speed_first: true,
            ..options
        };
        let metadata = preview_process::load_thumbnail(
            decoder,
            worker,
            token,
            active,
            pack_path,
            thumbnail_options,
            max_texture_side,
            |min, max| {
                let mut scratch = cell.borrow_mut();
                scratch.reset();
                scratch.prepare(min, max);
                Ok(())
            },
            |batch| {
                cell.borrow_mut()
                    .attach_checked(&batch, || worker.ensure_preload(token))
                    .map_err(|error| error.to_string())
            },
        );
        let Ok(metadata) = metadata else {
            return;
        };
        if worker.ensure_preload(token).is_err() {
            return;
        }
        let scratch = cell.into_inner();
        scratch.finish(&metadata.info());
        let job = scratch
            .begin_capture_checked(THUMBNAIL_SIDE, || worker.ensure_preload(token))
            .map_err(|error| error.to_string());
        let Ok(job) = job else {
            return;
        };
        let Ok(png) = finish_capture_png(job) else {
            return;
        };
        store_and_emit(app, worker, token, cache, &source, &png, active);
    }
}

/// Renders the live scene under the worker and renderer locks only for the
/// GPU submission; the readback wait runs without either lock.
fn capture_live(
    worker: &PreviewWorker,
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
    foreground_id: u64,
) -> Option<Vec<u8>> {
    let job = worker
        .with_current(foreground_id, || {
            let mut guard = renderer
                .lock()
                .map_err(|_| "The viewport is unavailable.".to_string())?;
            let renderer = guard
                .as_mut()
                .ok_or_else(|| "The viewport is unavailable.".to_string())?;
            renderer
                .begin_thumbnail_capture(THUMBNAIL_SIDE)
                .map_err(|error| error.to_string())
        })
        .ok()?;
    finish_capture_png(job).ok()
}

/// Hashes the opened file and accepts it only when its cheap identity did not
/// move while the foreground decode ran.
fn open_unchanged(
    worker: &PreviewWorker,
    token: u64,
    path: &Path,
    before: Option<SourceStat>,
) -> Option<ThumbnailSource> {
    let before = before?;
    let source = ThumbnailSource::open_checked(path.to_path_buf(), || worker.ensure_preload(token))
        .ok()??;
    (source_stat(path)? == before).then_some(source)
}

fn store_and_emit(
    app: &AppHandle,
    worker: &PreviewWorker,
    token: u64,
    cache: &ThumbnailCache,
    source: &ThumbnailSource,
    png: &[u8],
    path: &Path,
) {
    if worker.ensure_preload(token).is_err() || cache.contains(source.key()) {
        return;
    }
    let Ok(Some(_)) =
        store_source_thumbnail_checked(app, source, png, || worker.ensure_preload(token))
    else {
        return;
    };
    // A selection that arrived while the PNG was persisted must not receive
    // the previous selection's thumbnail event.
    if worker.ensure_preload(token).is_err() {
        return;
    }
    let Ok(path) = resources::path_string(path) else {
        return;
    };
    let _ = app.emit("browse-thumbnail", BrowseThumbnail { path });
}

/// Keeps one scratch renderer for every job when possible, sharing the live
/// device (and its device-wide error state) instead of requesting another
/// adapter.
fn thumbnail_scratch<'a>(
    slot: &'a mut Option<ThumbnailRenderer>,
    renderer: &Arc<Mutex<Option<NativeRenderer>>>,
) -> Option<&'a mut ThumbnailRenderer> {
    if slot.is_none() {
        let guard = renderer.lock().ok()?;
        let live = guard.as_ref()?;
        *slot = Some(ThumbnailRenderer::from_live(live));
    }
    slot.as_mut()
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(right.to_string_lossy().as_ref())
    }
    #[cfg(not(windows))]
    {
        false
    }
}
