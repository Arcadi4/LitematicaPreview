use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use parking_lot::Mutex;
use rune::{Key, KeyEvent, Tx, Waker};

use crate::browse::{self, BrowseSelection};
use crate::browse_thumbnail::{ThumbnailCache, ThumbnailSource};
use crate::loader::Loader;
use crate::preferences::{Action, Preferences};
use crate::preview::{self, Metadata, PreviewRequest};
use crate::render::camera::Camera;
use crate::render::capture::CaptureJob;
use crate::render::context::{LiveScene, ThumbnailRenderer};
use crate::texture_packs::{TexturePack, TexturePackLibrary};
use crate::{file_dialog, formats, preferences_ui, preview_image, resources, update};

pub(crate) const VIEWPORT: &str = "schematic-viewport";
/// Siblings on each side of the selection whose thumbnails are rendered.
const THUMBNAIL_RADIUS: usize = 4;
/// Thumbnails kept resident; past it the ones farthest from the selection are dropped.
const RESIDENT_THUMBNAILS: usize = 64;
/// The sibling indices whose thumbnails are rendered: [`THUMBNAIL_RADIUS`] either side of
/// `index`, shifted in at either end so the window keeps its width while the list is longer.
fn thumbnail_range(len: usize, index: usize) -> Range<usize> {
    let span = (THUMBNAIL_RADIUS * 2 + 1).min(len);
    let start = index.saturating_sub(THUMBNAIL_RADIUS).min(len - span);
    start..start + span
}
/// Seconds a notice toast stays up before it dismisses itself.
const NOTICE_SECONDS: f64 = 5.0;

type Handler = Box<dyn FnOnce(&mut App, &mut Tx) -> bool + Send>;

#[derive(Clone)]
struct Dispatch {
    send: Sender<Handler>,
    wake: Waker,
}

impl Dispatch {
    fn update(&self, f: impl FnOnce(&mut App, &mut Tx) -> bool + Send + 'static) {
        if self.send.send(Box::new(f)).is_ok() {
            (self.wake)();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Viewer,
    Preferences,
    TexturePacks,
}

pub(crate) struct Tab {
    pub id: u64,
    pub tab_token: String,
    pub path: PathBuf,
    pub camera: Camera,
    pub interacted: bool,
    nearby: Arc<Nearby>,
    nearby_index: usize,
}

struct Nearby {
    paths: Vec<PathBuf>,
    directory: Option<PathBuf>,
}

impl Nearby {
    fn new(paths: Vec<PathBuf>) -> Self {
        let directory = paths
            .first()
            .and_then(|path| path.parent())
            .filter(|parent| paths.iter().all(|path| path.parent() == Some(*parent)))
            .map(Path::to_path_buf);
        Self { paths, directory }
    }
}

#[derive(Default)]
pub(crate) enum Preview {
    #[default]
    None,
    Loading(&'static str, u64, u64),
    Ready(Metadata),
    Failed(String),
}

pub(crate) enum Capture {
    Copy,
    Save(PathBuf),
}

pub(crate) struct App {
    pub(crate) preferences: Preferences,
    pub(crate) preferences_path: PathBuf,
    pub(crate) update_mode: update::Mode,
    pub(crate) update_state: update::State,
    pub(crate) recording: Option<Action>,
    pub(crate) reset_confirming: bool,
    pub(crate) notice: Option<String>,
    pub(crate) reduced_motion: Cell<bool>,
    /// The notice on screen and the poll clock time it appeared, which starts its timeout.
    notice_shown: Option<(String, f64)>,
    pub(crate) preferences_section: preferences_ui::PreferencesSection,
    pub(crate) sidebar: rune_kit::shell::PaneState,
    pub(crate) page: Page,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: Option<u64>,
    pub(crate) preview: Preview,
    pub(crate) scene: Option<Arc<Mutex<LiveScene>>>,
    pub(crate) scene_size: Option<(u32, u32)>,
    pub(crate) capture: Option<Capture>,
    pub(crate) packs: Vec<TexturePack>,
    pub(crate) pack_busy: bool,
    pub(crate) choosing: bool,
    pub(crate) dropping: bool,
    pub(crate) spectator: bool,
    pub(crate) grid: bool,
    pub(crate) drag_previous: [f32; 2],
    pub(crate) look_previous: Option<[f32; 2]>,
    pub(crate) demos: Vec<PathBuf>,
    pub(crate) thumbnails: HashMap<PathBuf, rune::Image>,
    gpu: Option<(wgpu::Device, wgpu::Queue)>,
    scene_factory: Option<LiveScene>,
    loader: Loader,
    thumbnail_loader: Loader,
    images: Option<rune::ImageCache>,
    pub(crate) logo: Option<rune::Image>,
    cache_dir: PathBuf,
    pack_library: TexturePackLibrary,
    send: Sender<Handler>,
    receive: Receiver<Handler>,
    dispatch: Option<Dispatch>,
    pending_paths: Vec<PathBuf>,
    next_tab: u64,
    load_id: u64,
    last_poll: Option<f64>,
    movement: HashSet<Action>,
    started: bool,
    auto_check_pending: bool,
    update_choice: (bool, update::Source),
    update_epoch: u64,
}

impl App {
    pub(crate) fn new() -> Self {
        let paths = directories::BaseDirs::new();
        let data_dir = paths
            .as_ref()
            .map(|paths| paths.data_local_dir().join("dev.litematica.preview"))
            .unwrap_or_else(|| std::env::temp_dir().join("litematica-preview"));
        let cache_dir = paths
            .as_ref()
            .map(|paths| paths.cache_dir().join("dev.litematica.preview"))
            .unwrap_or_else(|| data_dir.join("cache"));
        let preferences_path = data_dir.join("preferences.json");
        let (preferences, mut notice) = match Preferences::load(&preferences_path) {
            Ok(value) => (value, None),
            Err(error) => (Preferences::default(), Some(error)),
        };
        let pack_library = TexturePackLibrary::new(data_dir.join("Texture Packs"));
        let packs = pack_library
            .state()
            .map(|state| state.packs)
            .unwrap_or_else(|error| {
                notice = Some(error);
                Vec::new()
            });
        let (send, receive) = mpsc::channel();
        let auto_check_pending = preferences.auto_update && !cfg!(debug_assertions);
        let update_choice = (preferences.auto_update, preferences.update_source);
        Self {
            preferences,
            preferences_path,
            update_mode: update::Mode::current(),
            update_state: update::State::Idle,
            recording: None,
            reset_confirming: false,
            notice,
            reduced_motion: Cell::new(false),
            notice_shown: None,
            page: Page::Viewer,
            tabs: Vec::new(),
            active: None,
            preview: Preview::None,
            scene: None,
            scene_size: None,
            preferences_section: preferences_ui::PreferencesSection::default(),
            sidebar: rune_kit::shell::PaneState::new(216.0).range(176.0, 320.0),
            capture: None,
            packs,
            pack_busy: false,
            choosing: false,
            dropping: false,
            spectator: false,
            grid: true,
            drag_previous: [0.0; 2],
            look_previous: None,
            demos: Vec::new(),
            thumbnails: HashMap::new(),
            gpu: None,
            loader: Loader::new(),
            scene_factory: None,
            thumbnail_loader: Loader::new(),
            images: None,
            logo: None,
            cache_dir,
            pack_library,
            send,
            receive,
            dispatch: None,
            next_tab: 0,
            load_id: 0,
            last_poll: None,
            pending_paths: Vec::new(),
            movement: HashSet::new(),
            started: false,
            auto_check_pending,
            update_choice,
            update_epoch: 0,
        }
    }

    pub(crate) fn setup(&mut self, wake: Waker) {
        let images = rune::ImageCache::new(wake.clone());
        self.logo = Some(images.load(resources::APP_ICON));
        self.images = Some(images);
        self.dispatch = Some(Dispatch {
            send: self.send.clone(),
            wake,
        });
    }

    pub(crate) fn request_frame(&self) {
        if let Some(dispatch) = &self.dispatch {
            dispatch.update(|_, _| true);
        }
    }

    pub(crate) fn queue_open(&mut self, path: PathBuf) {
        self.pending_paths.push(path);
    }

    pub(crate) fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        let mut changed = false;
        while let Ok(apply) = self.receive.try_recv() {
            changed |= apply(self, tx);
        }
        if !self.started && self.dispatch.is_some() {
            self.started = true;
            self.pending_paths.extend(crate::initial_paths());
            let demos_dir = self.cache_dir.join("demos");
            self.background(
                move || resources::demos(&demos_dir),
                |s, result, _| match result {
                    Ok(demos) => s.demos = demos,
                    Err(error) => s.notice = Some(error),
                },
            );
            if self.auto_check_pending && self.update_mode == update::Mode::SelfUpdate {
                self.auto_check_pending = false;
                self.check_update();
                changed = true;
            }
        }
        if self.dispatch.is_some() && !self.pending_paths.is_empty() {
            let paths = std::mem::take(&mut self.pending_paths);
            self.open_paths(paths);
            changed = true;
        }
        changed |= self.expire_notice(now, tx);
        let dt = self
            .last_poll
            .replace(now)
            .map_or(0.0, |last| (now - last).clamp(0.0, 0.1));
        if self.spectator && self.page == Page::Viewer && !self.movement.is_empty() {
            use Action::*;
            let axis = |plus, minus| {
                f32::from(self.movement.contains(&plus)) - f32::from(self.movement.contains(&minus))
            };
            let (forward, right, up) = (
                axis(FlyForward, FlyBackward),
                axis(FlyRight, FlyLeft),
                axis(FlyUp, FlyDown),
            );
            if let Some(camera) = self.camera_mut() {
                camera.fly_move(forward, right, up, dt as f32);
                changed = true;
            }
            tx.wake_after(std::time::Duration::from_millis(16));
        }
        if let Some(camera) = self.active_tab_mut().map(|tab| &mut tab.camera) {
            changed |= camera.advance(now);
            if camera.is_animating() {
                tx.wake_after(std::time::Duration::from_millis(16));
            }
        }
        changed
    }

    fn expire_notice(&mut self, now: f64, tx: &mut Tx) -> bool {
        let Some(notice) = &self.notice else {
            self.notice_shown = None;
            return false;
        };
        let since = match &self.notice_shown {
            Some((shown, since)) if shown == notice => *since,
            _ => {
                self.notice_shown = Some((notice.clone(), now));
                now
            }
        };
        let left = since + NOTICE_SECONDS - now;
        if left > 0.0 {
            tx.wake_after(std::time::Duration::from_secs_f64(left));
            return false;
        }
        self.notice = None;
        self.notice_shown = None;
        tx.animated();
        true
    }

    fn background<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
        apply: impl FnOnce(&mut App, Result<T, String>, &mut Tx) + Send + 'static,
    ) {
        let Some(dispatch) = self.dispatch.clone() else {
            return;
        };
        if let Err(error) = std::thread::Builder::new()
            .name("preview-task".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                    .unwrap_or_else(|_| Err("The background task stopped unexpectedly.".into()));
                dispatch.update(move |s, tx| {
                    // Rune lands commits no input caused at once; these answer the user's action.
                    tx.animated();
                    apply(s, result, tx);
                    true
                });
            })
        {
            self.notice = Some(format!("Could not start the background task: {error}"));
        }
    }

    pub(crate) fn set_gpu(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.gpu = Some((device.clone(), queue.clone()));
        self.scene_factory = Some(LiveScene::new(device, queue));
        if self.active.is_some() {
            self.begin_preview();
        }
    }

    pub(crate) fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| Some(tab.id) == self.active)
    }

    pub(crate) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| Some(tab.id) == self.active)
    }

    pub(crate) fn camera_mut(&mut self) -> Option<&mut Camera> {
        let tab = self.active_tab_mut()?;
        tab.interacted = true;
        Some(&mut tab.camera)
    }

    pub(crate) fn fit_camera(&mut self, tx: &mut Tx) {
        let reduced = self.reduced_motion.get();
        let Some(camera) = self.camera_mut() else {
            return;
        };
        if reduced {
            camera.fit();
        } else {
            camera.animate_fit(tx.clock());
            tx.wake_after(std::time::Duration::from_millis(16));
        }
    }

    pub(crate) fn zoom_camera(&mut self, factor: f32, tx: &mut Tx) {
        let reduced = self.reduced_motion.get();
        let Some(camera) = self.camera_mut() else {
            return;
        };
        if reduced {
            camera.zoom(factor);
        } else {
            camera.animate_zoom(factor, tx.clock());
            tx.wake_after(std::time::Duration::from_millis(16));
        }
    }

    pub(crate) fn open_dialog(&mut self) {
        if self.choosing {
            return;
        }
        self.choosing = true;
        self.background(
            || {
                let extensions: Vec<_> = formats::EXTENSIONS.iter().map(|ext| &ext[1..]).collect();
                let paths = file_dialog::pick_files(
                    "Open Minecraft schematics",
                    "Minecraft schematics",
                    &extensions,
                );
                if paths.is_empty() {
                    return Ok(None);
                }
                let many = paths.len() > 1;
                let selection = browse::selection_for_picked(paths)?;
                Ok(Some((selection, many)))
            },
            |s, result, tx| {
                s.choosing = false;
                s.apply_selection(result, tx);
            },
        );
    }

    pub(crate) fn open_folder(&mut self, tx: &mut Tx) {
        if self.choosing {
            return;
        }
        self.choosing = true;
        tx.open_folder(
            rune::FileDialogOptions {
                title: "Open schematic folder".into(),
                ..Default::default()
            },
            |s: &mut App, result, _| match result {
                Ok(Some(folder)) => s.background(
                    move || {
                        browse::selection_for_directory(&folder).map(|value| Some((value, false)))
                    },
                    |s, result, tx| {
                        s.choosing = false;
                        s.apply_selection(result, tx);
                    },
                ),
                Ok(None) => s.choosing = false,
                Err(error) => {
                    s.choosing = false;
                    s.notice = Some(error.to_string());
                }
            },
        );
    }

    pub(crate) fn open_paths(&mut self, paths: Vec<PathBuf>) {
        self.background(
            move || {
                if paths.len() == 1 && paths[0].is_dir() {
                    return browse::selection_for_directory(&paths[0])
                        .map(|value| Some((value, false)));
                }
                let paths: Vec<_> = paths
                    .into_iter()
                    .filter(|path| formats::is_supported_path(path) && path.is_file())
                    .collect();
                if paths.is_empty() {
                    return Err("Choose a supported schematic file.".into());
                }
                let many = paths.len() > 1;
                browse::selection_for_picked(paths).map(|value| Some((value, many)))
            },
            |s, result, tx| s.apply_selection(result, tx),
        );
    }

    fn apply_selection(
        &mut self,
        result: Result<Option<(BrowseSelection, bool)>, String>,
        tx: &mut Tx,
    ) {
        let (selection, many) = match result {
            Ok(Some(value)) => value,
            Ok(None) => return,
            Err(error) => {
                self.notice = Some(error);
                return;
            }
        };
        let nearby = Arc::new(Nearby::new(selection.paths));
        let mut selected = None;
        if many {
            for index in 0..nearby.paths.len() {
                let id = self.add_tab(index, Arc::clone(&nearby));
                if index == selection.index {
                    selected = Some(id);
                }
            }
        }
        let id = selected.unwrap_or_else(|| self.add_tab(selection.index, nearby));
        self.activate(id, tx);
    }

    fn add_tab(&mut self, index: usize, nearby: Arc<Nearby>) -> u64 {
        let path = &nearby.paths[index];
        if let Some(tab) = self.tabs.iter_mut().find(|tab| &tab.path == path) {
            tab.nearby = nearby;
            tab.nearby_index = index;
            return tab.id;
        }
        let id = self.next_tab;
        self.next_tab += 1;
        let mut camera = Camera::new();
        camera.set_grid(self.grid);
        self.tabs.push(Tab {
            id,
            tab_token: id.to_string(),
            path: path.clone(),
            camera,
            interacted: false,
            nearby,
            nearby_index: index,
        });
        id
    }

    pub(crate) fn activate(&mut self, id: u64, tx: &mut Tx) {
        self.page = Page::Viewer;
        if self.active != Some(id) {
            self.active = Some(id);
            self.begin_preview();
        }
        tx.scroll_into_view_in("tab-strip", format!("tab-{id}"));
        if let Some((_, index)) = self.nearby() {
            tx.scroll_into_view_in("schematic-browser", format!("browse-{index}"));
        }
        tx.focus(VIEWPORT, false);
    }

    pub(crate) fn close_tab(&mut self, id: u64, tx: &mut Tx) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active == Some(id) {
            if let Some(next) = self.tabs.get(index.min(self.tabs.len().saturating_sub(1))) {
                self.activate(next.id, tx);
            } else {
                self.active = None;
                self.begin_preview();
            }
        }
    }

    pub(crate) fn home(&mut self) {
        self.tabs.clear();
        self.active = None;
        self.page = Page::Viewer;
        self.begin_preview();
    }

    pub(crate) fn nearby(&self) -> Option<(&[PathBuf], usize)> {
        let tab = self.active_tab()?;
        Some((&tab.nearby.paths, tab.nearby_index))
    }

    pub(crate) fn nearby_directory(&self) -> Option<&Path> {
        self.active_tab()?.nearby.directory.as_deref()
    }

    /// The sibling range whose thumbnails are rendered: [`THUMBNAIL_RADIUS`] either side of the
    /// selection, shifted in at either end instead of shrinking.
    pub(crate) fn thumbnail_window(&self) -> Option<&[PathBuf]> {
        let (paths, index) = self.nearby()?;
        Some(&paths[thumbnail_range(paths.len(), index)])
    }

    pub(crate) fn browse_at(&mut self, index: usize, tx: &mut Tx) {
        if let Some(tab) = self.active_tab()
            && index < tab.nearby.paths.len()
        {
            let id = self.add_tab(index, Arc::clone(&tab.nearby));
            self.activate(id, tx);
        }
    }

    fn browse_step(&mut self, delta: isize, tx: &mut Tx) -> bool {
        let Some((paths, index)) = self.nearby() else {
            return false;
        };
        if paths.len() <= 1 {
            return false;
        }
        if let Some(next) = index.checked_add_signed(delta)
            && next < paths.len()
        {
            self.browse_at(next, tx);
        }
        true
    }

    fn begin_preview(&mut self) {
        for tab in &mut self.tabs {
            tab.camera.settle_motion();
        }
        self.loader.cancel();
        self.thumbnail_loader.cancel();
        self.stop_flying();
        self.load_id += 1;
        self.scene = None;
        self.scene_size = None;
        self.capture = None;
        self.preview = Preview::None;
        let Some(path) = self.active_tab().map(|tab| tab.path.clone()) else {
            return;
        };
        self.preview = Preview::Loading("Opening schematic", 0, 0);
        let (Some(factory), Some(dispatch)) = (&self.scene_factory, self.dispatch.clone()) else {
            return;
        };
        let layers = match self.pack_library.layers() {
            Ok(layers) => layers,
            Err(error) => {
                self.preview = Preview::Failed(error);
                return;
            }
        };
        let load_id = self.load_id;
        let scene = Arc::new(Mutex::new(factory.fork()));
        let options = self.preferences.preview_options();
        let (max_texture_side, upload) = {
            let scene = scene.lock();
            (
                scene.max_texture_side(),
                scene.upload_window(usize::from(options.thread_count.unwrap_or(1))),
            )
        };
        self.scene = Some(Arc::clone(&scene));
        self.loader.submit(move |packs, current| {
            let check = || current();
            let request = PreviewRequest {
                path: &path,
                layers: &layers,
                options,
                max_texture_side,
            };
            let mut first_visible = true;
            let result = preview::decode(
                request,
                packs,
                check,
                |min, max| {
                    scene.lock().prepare(min, max);
                    dispatch.update(move |s, _| {
                        if s.load_id != load_id {
                            return false;
                        }
                        if let Some(tab) = s.active_tab_mut() {
                            if tab.interacted {
                                tab.camera.update_model_bounds(min, max);
                            } else {
                                tab.camera.set_model_bounds(min, max);
                            }
                        }
                        true
                    });
                    Ok(())
                },
                |batch| {
                    let bytes = batch
                        .textures
                        .iter()
                        .fold(batch.geometry.len(), |size, texture| {
                            size.saturating_add(texture.pixels.len())
                        });
                    let permit = upload.acquire(bytes, &check)?;
                    check()?;
                    scene
                        .lock()
                        .attach(&batch, permit)
                        .map_err(|error| error.to_string())?;
                    if first_visible && !batch.parts.is_empty() {
                        first_visible = false;
                        dispatch.update(move |s, _| s.load_id == load_id);
                    }
                    Ok(())
                },
                |phase, have, total| {
                    dispatch.update(move |s, _| {
                        if s.load_id != load_id {
                            return false;
                        }
                        s.preview = Preview::Loading(phase, have, total);
                        true
                    });
                },
            )
            .and_then(|metadata| {
                upload.drain(&check)?;
                scene.lock().finish(&metadata.info());
                Ok(metadata)
            });
            if check().is_err() {
                return;
            }
            dispatch.update(move |s, _| {
                if s.load_id != load_id {
                    return false;
                }
                match result {
                    Ok(metadata) => {
                        if let Some(tab) = s.active_tab_mut() {
                            if tab.interacted {
                                tab.camera.update_model_bounds(metadata.min, metadata.max);
                            } else {
                                tab.camera.set_model_bounds(metadata.min, metadata.max);
                            }
                        }
                        s.preview = Preview::Ready(metadata);
                        s.queue_thumbnails();
                    }
                    Err(error) => {
                        s.scene = None;
                        s.preview = Preview::Failed(error);
                    }
                }
                true
            });
        });
    }

    fn queue_thumbnails(&mut self) {
        let Some(window) = self.thumbnail_window().map(|window| window.to_vec()) else {
            return;
        };
        self.trim_thumbnails();
        let paths: Vec<_> = window
            .into_iter()
            .filter(|path| !self.thumbnails.contains_key(path))
            .collect();
        if paths.is_empty() {
            return;
        }
        let (Some((device, queue)), Some(dispatch)) = (self.gpu.clone(), self.dispatch.clone())
        else {
            return;
        };
        let directory = self.cache_dir.clone();
        let load_id = self.load_id;
        self.thumbnail_loader.submit(move |packs, current| {
            let Ok(cache) = ThumbnailCache::from_path(directory) else {
                return;
            };
            let mut renderer = None;
            for path in paths {
                if current().is_err() {
                    break;
                }
                let Ok(Some(source)) = ThumbnailSource::open_checked(&path, current) else {
                    continue;
                };
                let png = cache.lookup(source.key()).or_else(|| {
                    let renderer = renderer
                        .get_or_insert_with(|| ThumbnailRenderer::from_device(&device, &queue));
                    thumbnail(&source, &path, renderer, packs, &cache, current).ok()
                });
                if let Some(png) = png {
                    dispatch.update(move |s, _| {
                        if s.load_id != load_id {
                            return false;
                        }
                        if let Some(images) = &s.images {
                            s.thumbnails
                                .insert(path, images.load(Arc::<[u8]>::from(png)));
                        }
                        true
                    });
                }
            }
        });
    }

    /// Keeps thumbnails for the active tab's siblings, dropping those farthest from the
    /// selection once more than [`RESIDENT_THUMBNAILS`] are held.
    fn trim_thumbnails(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let nearby = Arc::clone(&tab.nearby);
        let index = tab.nearby_index;
        self.thumbnails
            .retain(|path, _| nearby.paths.contains(path));
        let excess = self.thumbnails.len().saturating_sub(RESIDENT_THUMBNAILS);
        if excess == 0 {
            return;
        }
        let mut farthest: Vec<(usize, PathBuf)> = self
            .thumbnails
            .keys()
            .filter_map(|path| {
                nearby
                    .paths
                    .iter()
                    .position(|candidate| candidate == path)
                    .map(|at| (at.abs_diff(index), path.clone()))
            })
            .collect();
        farthest.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (_, path) in farthest.into_iter().take(excess) {
            self.thumbnails.remove(&path);
        }
    }

    pub(crate) fn finish_capture(&mut self, capture: Capture, job: CaptureJob) {
        self.background(
            move || {
                let image = job.finish().map_err(|error| error.to_string())?;
                match capture {
                    Capture::Copy => image.png_bytes().map(Some),
                    Capture::Save(path) => preview_image::save(&image, path).map(|_| None),
                }
            },
            |s, result, tx| match result {
                Ok(Some(png)) => {
                    if let Some(png) = rune::ClipboardImage::from_png(png) {
                        tx.copy_rich(rune::ClipboardContent::default().with_image(png));
                        s.notice = Some("Preview image copied.".into());
                    }
                }
                Ok(None) => s.notice = Some("Preview image saved.".into()),
                Err(error) => s.notice = Some(error),
            },
        );
    }

    pub(crate) fn save_image(&mut self, tx: &mut Tx) {
        if !matches!(self.preview, Preview::Ready(_)) {
            return;
        }
        let name = self
            .active_tab()
            .map_or_else(|| "Schematic".into(), |tab| title(&tab.path));
        tx.save_file(
            rune::FileDialogOptions {
                title: "Save preview image".into(),
                file_name: Some(format!("{name}.png")),
                ..Default::default()
            }
            .filter("PNG or JPEG", &["png", "jpg", "jpeg"]),
            |s: &mut App, result, _| match result {
                Ok(Some(path)) => s.capture = Some(Capture::Save(path)),
                Ok(None) => (),
                Err(error) => s.notice = Some(error.to_string()),
            },
        );
    }

    pub(crate) fn toggle_grid(&mut self) {
        self.grid = !self.grid;
        for tab in &mut self.tabs {
            tab.camera.set_grid(self.grid);
        }
    }

    pub(crate) fn toggle_flying(&mut self, tx: &mut Tx) {
        if self.spectator {
            self.stop_flying();
        } else if matches!(self.preview, Preview::Ready(_)) {
            self.spectator = true;
            self.look_previous = None;
            tx.focus(VIEWPORT, false);
        }
    }

    pub(crate) fn stop_flying(&mut self) {
        self.spectator = false;
        self.movement.clear();
        self.look_previous = None;
    }

    pub(crate) fn open_page(&mut self, page: Page) {
        self.stop_flying();
        self.page = page;
        self.recording = None;
        self.reset_confirming = false;
    }

    pub(crate) fn save_preferences(&mut self) {
        let next = (self.preferences.auto_update, self.preferences.update_source);
        if std::mem::replace(&mut self.update_choice, next) != next {
            self.update_epoch += 1;
            if !matches!(
                self.update_state,
                update::State::Installing | update::State::Installed
            ) {
                self.update_state = update::State::Idle;
            }
        }
        if let Err(error) = self.preferences.save(&self.preferences_path) {
            self.notice = Some(error);
        }
    }

    pub(crate) fn check_update(&mut self) {
        use update::{Mode, State};
        if matches!(self.update_state, State::Checking | State::Installing) {
            return;
        }
        if self.update_mode == Mode::AppImage {
            self.update_state = State::Checking;
            self.background(update::appimage_update, |s, result, _| {
                s.update_state = result.map_or_else(State::Failed, |()| State::Idle);
            });
        } else if self.update_mode == Mode::SelfUpdate && self.preferences.auto_update {
            self.update_state = State::Checking;
            let source = self.preferences.update_source;
            let epoch = self.update_epoch;
            self.background(
                move || update::check(source),
                move |s, result, _| {
                    if s.update_epoch == epoch {
                        s.update_state = match result {
                            Ok(Some(release)) => State::Available(release),
                            Ok(None) => State::Current,
                            Err(error) => State::Failed(error),
                        };
                    }
                },
            );
        }
    }

    pub(crate) fn install_update(&mut self) {
        use update::{Mode, State};
        if self.update_mode != Mode::SelfUpdate || !self.preferences.auto_update {
            return;
        }
        let State::Available(release) = &self.update_state else {
            return;
        };
        let release = release.clone();
        self.update_state = State::Installing;
        self.background(
            move || update::install(&release),
            |s, result, _| {
                s.update_state = result.map_or_else(State::Failed, |()| State::Installed);
            },
        );
    }

    pub(crate) fn show_texture_picker(&mut self, tx: &mut Tx) {
        tx.open_file(
            rune::FileDialogOptions {
                title: "Import a Minecraft resource pack".into(),
                ..Default::default()
            }
            .filter("Resource pack ZIP", &["zip"]),
            |s: &mut App, result, _| match result {
                Ok(Some(path)) => s.change_packs(move |library| {
                    let bytes = std::fs::read(&path)
                        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
                    let name = title(&path);
                    library.import(&name, &bytes, |bytes| {
                        preview::validate_texture_pack(&name, bytes)
                    })?;
                    Ok(())
                }),
                Ok(None) => (),
                Err(error) => s.notice = Some(error.to_string()),
            },
        );
    }

    pub(crate) fn change_packs(
        &mut self,
        work: impl FnOnce(&TexturePackLibrary) -> Result<(), String> + Send + 'static,
    ) {
        if self.pack_busy {
            return;
        }
        self.pack_busy = true;
        let library = self.pack_library.clone();
        self.background(
            move || {
                work(&library)?;
                library.state().map(|state| state.packs)
            },
            |s, result, _| {
                s.pack_busy = false;
                match result {
                    Ok(packs) => {
                        s.packs = packs;
                        s.begin_preview();
                    }
                    Err(error) => s.notice = Some(error),
                }
            },
        );
    }

    pub(crate) fn key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        use Action::*;
        if self.recording.is_some() {
            return preferences_ui::recording_key(self, event, tx);
        }
        if event.is_command('o') && !event.mods.alt {
            if event.mods.shift {
                self.open_folder(tx);
            } else {
                self.open_dialog();
            }
            return true;
        }
        if event.is_command('w')
            && self.page == Page::Viewer
            && let Some(id) = self.active
        {
            self.close_tab(id, tx);
            return true;
        }
        if event.is_command(',') {
            self.open_page(Page::Preferences);
            return true;
        }
        if self.page != Page::Viewer {
            if matches!(event.key, Key::Escape) {
                self.open_page(Page::Viewer);
                return true;
            }
            return false;
        }
        if self.active.is_none() {
            return false;
        }
        let bound = |action| self.preferences.binding(action).matches(event, tx);
        if self.spectator {
            if bound(FlyExit) {
                self.stop_flying();
                return true;
            }
            for action in [FlyForward, FlyBackward, FlyLeft, FlyRight, FlyUp, FlyDown] {
                if bound(action) {
                    self.movement.insert(action);
                    return true;
                }
            }
            return false;
        }
        if bound(ToggleSpectator) {
            self.toggle_flying(tx);
            return true;
        }
        if matches!(event.key, Key::Escape) && matches!(self.preview, Preview::Loading(..)) {
            if let Some(id) = self.active {
                self.close_tab(id, tx);
            }
            return true;
        }
        if bound(Fit) {
            self.fit_camera(tx);
            return true;
        }
        if bound(ZoomIn) || bound(ZoomOut) {
            let factor = if bound(ZoomIn) { 0.88 } else { 1.12 };
            self.zoom_camera(factor, tx);
            return true;
        }
        if tx.focused().is_some_and(|key| key != VIEWPORT) {
            return false;
        }
        if !event.mods.shift && !event.mods.alt && !event.mods.cmd && !event.mods.ctrl {
            if matches!(event.key, Key::Left) && self.browse_step(-1, tx) {
                return true;
            }
            if matches!(event.key, Key::Right) && self.browse_step(1, tx) {
                return true;
            }
        }
        for (action, dx, dy) in [
            (OrbitLeft, -18.0, 0.0),
            (OrbitRight, 18.0, 0.0),
            (OrbitUp, 0.0, -18.0),
            (OrbitDown, 0.0, 18.0),
        ] {
            if self.preferences.binding(action).matches(event, tx) {
                if let Some(camera) = self.camera_mut() {
                    camera.arrow(dx, dy, event.mods.shift);
                }
                return true;
            }
        }
        false
    }
    pub(crate) fn key_up(&mut self, event: &KeyEvent, _: &mut Tx) -> bool {
        let before = self.movement.len();
        self.movement
            .retain(|action| !self.preferences.binding(*action).matches_release(event));
        self.movement.len() != before
    }
}

fn thumbnail(
    source: &ThumbnailSource,
    path: &Path,
    renderer: &mut ThumbnailRenderer,
    packs: &mut preview::PackCache,
    cache: &ThumbnailCache,
    current: &dyn Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    use crate::browse_thumbnail::THUMBNAIL_SIDE;
    use std::cell::RefCell;
    let check = || current();
    let max_texture_side = renderer.max_texture_side();
    renderer.reset();
    let work = RefCell::new(renderer);
    let request = PreviewRequest {
        path,
        layers: &[],
        options: Default::default(),
        max_texture_side,
    };
    let metadata = preview::decode(
        request,
        packs,
        check,
        |min, max| {
            work.borrow_mut().prepare(min, max);
            Ok(())
        },
        |batch| {
            work.borrow_mut()
                .attach_checked(&batch, check)
                .map_err(|e| e.to_string())
        },
        |_, _, _| (),
    )?;
    let renderer = work.into_inner();
    renderer.drain_checked(check).map_err(|e| e.to_string())?;
    renderer.finish(&metadata.info());
    let capture = renderer
        .begin_capture_checked(THUMBNAIL_SIDE, check)
        .map_err(|e| e.to_string())?;
    let png = capture.finish().map_err(|e| e.to_string())?.png_bytes()?;
    if !crate::browse_thumbnail::store_source_thumbnail_checked(cache, source, &png, check)? {
        return Err("The schematic changed while building its thumbnail.".into());
    }
    Ok(png)
}

pub(crate) fn title(path: &Path) -> String {
    path.file_stem()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::thumbnail_range;

    #[test]
    fn thumbnail_range_keeps_its_width_at_both_ends() {
        assert_eq!(thumbnail_range(0, 0), 0..0);
        assert_eq!(thumbnail_range(7, 0), 0..7);
        assert_eq!(thumbnail_range(7, 6), 0..7);
        assert_eq!(thumbnail_range(20, 0), 0..9);
        assert_eq!(thumbnail_range(20, 10), 6..15);
        assert_eq!(thumbnail_range(20, 19), 11..20);
    }
}
