//! Persisted user preferences: appearance, camera feel, preview worker preset, update
//! behaviour, and the keys that drive the preview and spectator cameras.
//!
//! The file is JSON at a path the app chooses. A missing file means defaults; a file that
//! cannot be parsed is moved aside as `<name>.bad` and defaults are used, so a hand-edited
//! file never stops the app from starting. Saves go through a temporary file and a rename.
//!
//! Bindings are stored as Rune [`Chord`] specs (`"f"`, `"shift+minus"`, `"space"`), so they
//! follow keyboard layouts through [`Chord::matches_tx`]. A whole modifier held on its own
//! (the default Control for descending in spectator mode) is stored as `"ctrl"` and matched
//! against Rune's `Key::Modifier` events. A chord without Shift also matches its Shift-held
//! form, so `f` accepts both `f` and `F`, and `left` accepts Shift+Left, which the viewer
//! reads as a pan.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use litematica_preview_core::PreviewOptions;
use rune::{
    Chord, ChordKey, ChordMod, Key, KeyEvent, KeyPlatform, Modifier, Mods, PhysicalKeyCode,
    Shortcut, Tx,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The shipped vertical field of view, in degrees.
pub const DEFAULT_FOV_DEGREES: f32 = 28.0;
/// The shipped spectator look multiplier.
pub const DEFAULT_SENSITIVITY: f32 = 1.0;
/// Narrowest and widest accepted field of view, matching the camera's own clamp.
pub const MIN_FOV_DEGREES: f32 = 5.0;
pub const MAX_FOV_DEGREES: f32 = 120.0;
/// Narrowest and widest accepted spectator look multiplier, matching the camera's own clamp.
pub const MIN_SENSITIVITY: f32 = 0.05;
pub const MAX_SENSITIVITY: f32 = 10.0;

/// The appearance the app paints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Theme::System => system_dark,
            Theme::Light => false,
            Theme::Dark => true,
        }
    }
}

/// Which camera's controls a binding belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Preview,
    Spectator,
}

impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Mode::Preview => "Preview",
            Mode::Spectator => "Spectator",
        }
    }
}

/// A configurable viewer action. Preview actions drive the orbit camera; spectator actions
/// drive the free-flight camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Fit the model in view.
    Fit,
    /// Enter spectator mode.
    ToggleSpectator,
    ZoomIn,
    ZoomOut,
    OrbitLeft,
    OrbitRight,
    OrbitUp,
    OrbitDown,
    /// Leave spectator mode.
    FlyExit,
    FlyForward,
    FlyBackward,
    FlyLeft,
    FlyRight,
    FlyUp,
    FlyDown,
}

pub const PREVIEW_ACTIONS: &[Action] = &[
    Action::Fit,
    Action::ToggleSpectator,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::OrbitLeft,
    Action::OrbitRight,
    Action::OrbitUp,
    Action::OrbitDown,
];

pub const SPECTATOR_ACTIONS: &[Action] = &[
    Action::FlyExit,
    Action::FlyForward,
    Action::FlyBackward,
    Action::FlyLeft,
    Action::FlyRight,
    Action::FlyUp,
    Action::FlyDown,
];

pub const ALL_ACTIONS: &[Action] = &[
    Action::Fit,
    Action::ToggleSpectator,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::OrbitLeft,
    Action::OrbitRight,
    Action::OrbitUp,
    Action::OrbitDown,
    Action::FlyExit,
    Action::FlyForward,
    Action::FlyBackward,
    Action::FlyLeft,
    Action::FlyRight,
    Action::FlyUp,
    Action::FlyDown,
];

impl Action {
    pub fn all(mode: Mode) -> &'static [Action] {
        match mode {
            Mode::Preview => PREVIEW_ACTIONS,
            Mode::Spectator => SPECTATOR_ACTIONS,
        }
    }

    pub fn mode(self) -> Mode {
        match self {
            Action::Fit
            | Action::ToggleSpectator
            | Action::ZoomIn
            | Action::ZoomOut
            | Action::OrbitLeft
            | Action::OrbitRight
            | Action::OrbitUp
            | Action::OrbitDown => Mode::Preview,
            Action::FlyExit
            | Action::FlyForward
            | Action::FlyBackward
            | Action::FlyLeft
            | Action::FlyRight
            | Action::FlyUp
            | Action::FlyDown => Mode::Spectator,
        }
    }

    /// The stable key the preferences file uses, matching the serde name.
    pub fn key(self) -> &'static str {
        match self {
            Action::Fit => "fit",
            Action::ToggleSpectator => "toggleSpectator",
            Action::ZoomIn => "zoomIn",
            Action::ZoomOut => "zoomOut",
            Action::OrbitLeft => "orbitLeft",
            Action::OrbitRight => "orbitRight",
            Action::OrbitUp => "orbitUp",
            Action::OrbitDown => "orbitDown",
            Action::FlyExit => "flyExit",
            Action::FlyForward => "flyForward",
            Action::FlyBackward => "flyBackward",
            Action::FlyLeft => "flyLeft",
            Action::FlyRight => "flyRight",
            Action::FlyUp => "flyUp",
            Action::FlyDown => "flyDown",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Action::Fit => "Fit model",
            Action::ToggleSpectator => "Spectator mode",
            Action::ZoomIn => "Zoom in",
            Action::ZoomOut => "Zoom out",
            Action::OrbitLeft => "Orbit left",
            Action::OrbitRight => "Orbit right",
            Action::OrbitUp => "Orbit up",
            Action::OrbitDown => "Orbit down",
            Action::FlyExit => "Leave spectator",
            Action::FlyForward => "Fly forward",
            Action::FlyBackward => "Fly back",
            Action::FlyLeft => "Fly left",
            Action::FlyRight => "Fly right",
            Action::FlyUp => "Rise",
            Action::FlyDown => "Descend",
        }
    }
}

/// A modifier held on its own as a binding, such as Control for descending in spectator mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoldKey {
    Control,
    Shift,
    Alt,
    Super,
    Fn,
}

impl HoldKey {
    fn parse(spec: &str) -> Option<Self> {
        match spec.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(HoldKey::Control),
            "shift" => Some(HoldKey::Shift),
            "alt" | "option" | "opt" => Some(HoldKey::Alt),
            "super" | "cmd" | "command" | "meta" | "win" | "logo" => Some(HoldKey::Super),
            "fn" | "function" => Some(HoldKey::Fn),
            _ => None,
        }
    }

    fn spec(self) -> &'static str {
        match self {
            HoldKey::Control => "ctrl",
            HoldKey::Shift => "shift",
            HoldKey::Alt => "alt",
            HoldKey::Super => "super",
            HoldKey::Fn => "fn",
        }
    }

    fn label(self) -> &'static str {
        match self {
            HoldKey::Control => {
                if cfg!(target_os = "macos") {
                    "⌃"
                } else {
                    "Ctrl"
                }
            }
            HoldKey::Shift => {
                if cfg!(target_os = "macos") {
                    "⇧"
                } else {
                    "Shift"
                }
            }
            HoldKey::Alt => {
                if cfg!(target_os = "macos") {
                    "⌥"
                } else {
                    "Alt"
                }
            }
            HoldKey::Super => {
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Super"
                }
            }
            HoldKey::Fn => "Fn",
        }
    }

    fn from_modifier(modifier: Modifier) -> Option<Self> {
        match modifier {
            Modifier::ControlLeft | Modifier::ControlRight => Some(HoldKey::Control),
            Modifier::ShiftLeft | Modifier::ShiftRight => Some(HoldKey::Shift),
            Modifier::AltLeft | Modifier::AltRight => Some(HoldKey::Alt),
            Modifier::SuperLeft | Modifier::SuperRight => Some(HoldKey::Super),
            Modifier::Fn => Some(HoldKey::Fn),
            _ => None,
        }
    }

    fn matches(self, modifier: Modifier) -> bool {
        Self::from_modifier(modifier) == Some(self)
    }
}

/// One key a binding accepts: a chord, or a modifier held alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyChord {
    Chord(Chord),
    Hold(HoldKey),
}

impl KeyChord {
    fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }
        if let Some(hold) = HoldKey::parse(spec) {
            return Some(KeyChord::Hold(hold));
        }
        Chord::parse(spec).ok().map(KeyChord::Chord)
    }

    fn spec(self) -> String {
        match self {
            KeyChord::Chord(chord) => chord.to_string(),
            KeyChord::Hold(hold) => hold.spec().to_owned(),
        }
    }

    pub fn label(self) -> String {
        match self {
            KeyChord::Chord(chord) => chord.label(),
            KeyChord::Hold(hold) => hold.label().to_owned(),
        }
    }

    /// Whether two stored keys would be matched by the same event. A chord without Shift also
    /// takes the Shift-held form, so `left` and `shift+left` conflict.
    fn conflicts(self, other: KeyChord) -> bool {
        match (self, other) {
            (KeyChord::Hold(a), KeyChord::Hold(b)) => a == b,
            (KeyChord::Chord(a), KeyChord::Chord(b)) => {
                let platform = KeyPlatform::current();
                if a.same_keys(&b, platform) {
                    return true;
                }
                if a.key() != b.key() {
                    return false;
                }
                let mut am = a.mods_on(platform);
                let mut bm = b.mods_on(platform);
                am.retain(|m| *m != ChordMod::Shift);
                bm.retain(|m| *m != ChordMod::Shift);
                am == bm
            }
            _ => false,
        }
    }

    /// Whether this key is the one the event presses. A chord without Shift also matches with
    /// Shift held, which keeps `f`/`F` and arrows/Shift+arrows on the same binding.
    fn matches(self, event: &KeyEvent, tx: &Tx) -> bool {
        match self {
            KeyChord::Chord(chord) => {
                if matches!(&event.key, Key::Modifier { .. }) {
                    return false;
                }
                if chord.matches_tx(event, tx) {
                    return true;
                }
                if !chord.has(ChordMod::Shift) && event.mods.shift {
                    let pressed = KeyEvent {
                        key: event.key.clone(),
                        mods: Mods {
                            shift: false,
                            ..event.mods
                        },
                    };
                    return chord.matches_tx(&pressed, tx);
                }
                false
            }
            KeyChord::Hold(hold) => match &event.key {
                Key::Modifier { modifier, pressed } => *pressed && hold.matches(*modifier),
                _ => false,
            },
        }
    }

    /// Whether this key is the one the event releases. Key-up events carry no pressed flag for
    /// ordinary keys, so only the key and not the modifiers are compared.
    fn matches_release(self, event: &KeyEvent) -> bool {
        match self {
            KeyChord::Hold(hold) => match &event.key {
                Key::Modifier { modifier, pressed } => !*pressed && hold.matches(*modifier),
                _ => false,
            },
            KeyChord::Chord(chord) => {
                let Some(key) = chord_key(&event.key) else {
                    return false;
                };
                match chord.key() {
                    ChordKey::Char(want) => match key {
                        ChordKey::Char(got) => got.eq_ignore_ascii_case(&want),
                        _ => false,
                    },
                    want => key == want,
                }
            }
        }
    }
}

/// A configurable action's keys. Empty means the action is unbound.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Binding {
    keys: Vec<KeyChord>,
}

impl Binding {
    fn one(key: KeyChord) -> Self {
        Self { keys: vec![key] }
    }

    /// Parses a comma-separated spec (`"f, home"`). Unknown parts are skipped so one bad token
    /// does not lose the other keys.
    fn parse(spec: &str) -> Self {
        let mut binding = Self {
            keys: spec.split(',').filter_map(KeyChord::parse).collect(),
        };
        binding.dedup();
        binding
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn matches(&self, event: &KeyEvent, tx: &Tx) -> bool {
        self.keys.iter().any(|key| key.matches(event, tx))
    }

    pub fn matches_release(&self, event: &KeyEvent) -> bool {
        self.keys.iter().any(|key| key.matches_release(event))
    }

    /// The keys as Rune keycap labels, one per key ("F", "Home", "⇧←").
    pub fn keycaps(&self) -> Vec<String> {
        self.keys.iter().map(|key| key.label()).collect()
    }

    pub fn label(&self) -> String {
        self.keycaps().join(" / ")
    }

    fn spec(&self) -> String {
        self.keys
            .iter()
            .map(|key| key.spec())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn dedup(&mut self) {
        let mut unique: Vec<KeyChord> = Vec::with_capacity(self.keys.len());
        for key in self.keys.drain(..) {
            if !unique.iter().any(|seen| seen.conflicts(key)) {
                unique.push(key);
            }
        }
        self.keys = unique;
    }
}

impl Serialize for Binding {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.spec())
    }
}

impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Binding::parse(&String::deserialize(deserializer)?))
    }
}

/// The preview worker preset, using the same field names and defaults as the old web
/// settings (`chunkingEnabled`, `chunkSize`, `multithreadingEnabled`, `threadCount`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewSettings {
    pub chunking_enabled: bool,
    pub chunk_size: u32,
    pub multithreading_enabled: bool,
    pub thread_count: u8,
}

impl PreviewSettings {
    pub const CHUNK_SIZES: [u32; 5] = [16, 32, 64, 128, 256];
    pub const MIN_THREADS: u8 = 2;
    pub const MAX_THREADS: u8 = 8;

    fn sanitized(mut self) -> Self {
        if !Self::CHUNK_SIZES.contains(&self.chunk_size) {
            self.chunk_size = 32;
        }
        self.thread_count = self
            .thread_count
            .clamp(Self::MIN_THREADS, Self::MAX_THREADS);
        if !self.chunking_enabled {
            self.multithreading_enabled = false;
        }
        self
    }
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            chunking_enabled: true,
            chunk_size: 32,
            multithreading_enabled: true,
            thread_count: 4,
        }
    }
}

/// Why a binding could not be assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The action that already uses the key.
    pub action: Action,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "That key is already used by {}.", self.action.title())
    }
}

impl std::error::Error for Conflict {}

/// Everything the app remembers between launches.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub theme: Theme,
    /// Spectator look multiplier applied on top of Rune's camera base speed.
    pub sensitivity: f32,
    /// Vertical field of view in degrees.
    pub fov: f32,
    pub auto_update: bool,
    pub update_source: crate::update::Source,
    pub preview: PreviewSettings,
    bindings: BTreeMap<String, Binding>,
}

impl Default for Preferences {
    fn default() -> Self {
        let bindings = ALL_ACTIONS
            .iter()
            .map(|action| (action.key().to_owned(), default_binding(*action)))
            .collect();
        Self {
            theme: Theme::System,
            sensitivity: DEFAULT_SENSITIVITY,
            fov: DEFAULT_FOV_DEGREES,
            auto_update: true,
            update_source: crate::update::Source::default(),
            preview: PreviewSettings::default(),
            bindings,
        }
    }
}

impl Preferences {
    /// Reads preferences from `path`. A missing file gives defaults. A file that does not
    /// parse is moved aside as `<name>.bad` and defaults are returned with an explanation;
    /// only an unreadable file is an error.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Could not read preferences: {error}")),
        };
        match serde_json::from_slice::<Preferences>(&bytes) {
            Ok(preferences) => Ok(preferences.normalized()),
            Err(error) => {
                let mut bad = path.as_os_str().to_owned();
                bad.push(".bad");
                let bad = PathBuf::from(bad);
                if std::fs::rename(path, &bad).is_err() {
                    let _ = std::fs::write(&bad, &bytes);
                }
                Err(format!(
                    "Preferences were reset because the file could not be read: {error}"
                ))
            }
        }
    }

    /// Writes preferences to `path`, replacing the previous file atomically.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|error| format!("Could not create the preferences folder: {error}"))?;
        }
        let json = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Could not encode preferences: {error}"))?;
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(format!(".{}.tmp", std::process::id()));
        let temporary = PathBuf::from(temporary);
        let result = (|| {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(&json)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|error| format!("Could not save preferences: {error}"))
    }

    pub fn binding(&self, action: Action) -> Binding {
        self.bindings.get(action.key()).cloned().unwrap_or_default()
    }

    /// Binds `action` to `key`, refusing a key already used by another action in the same mode.
    /// Re-recording a key the action already accepts is a no-op, so the shipped aliases (F and
    /// Home for Fit, for example) survive.
    pub fn assign_binding(&mut self, action: Action, key: KeyChord) -> Result<(), Conflict> {
        if self
            .binding(action)
            .keys
            .iter()
            .any(|bound| bound.conflicts(key))
        {
            return Ok(());
        }
        for other in Action::all(action.mode()) {
            if *other == action {
                continue;
            }
            if self
                .binding(*other)
                .keys
                .iter()
                .any(|bound| bound.conflicts(key))
            {
                return Err(Conflict { action: *other });
            }
        }
        self.bindings
            .insert(action.key().to_owned(), Binding::one(key));
        Ok(())
    }

    pub fn clear_binding(&mut self, action: Action) {
        self.bindings
            .insert(action.key().to_owned(), Binding::default());
    }

    /// Restores every shipped preference, including bindings.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn set_sensitivity(&mut self, multiplier: f32) {
        if multiplier.is_finite() {
            self.sensitivity = multiplier.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
        }
    }

    pub fn set_fov(&mut self, degrees: f32) {
        if degrees.is_finite() {
            self.fov = degrees.clamp(MIN_FOV_DEGREES, MAX_FOV_DEGREES);
        }
    }

    pub fn set_auto_update(&mut self, enabled: bool) {
        self.auto_update = enabled;
    }

    pub fn set_update_source(&mut self, source: crate::update::Source) {
        self.update_source = source;
    }

    pub fn set_chunking(&mut self, enabled: bool) {
        self.preview.chunking_enabled = enabled;
        if !enabled {
            self.preview.multithreading_enabled = false;
        }
    }

    pub fn set_chunk_size(&mut self, size: u32) {
        if PreviewSettings::CHUNK_SIZES.contains(&size) {
            self.preview.chunk_size = size;
        }
    }

    pub fn set_multithreading(&mut self, enabled: bool) {
        if enabled {
            self.preview.chunking_enabled = true;
        }
        self.preview.multithreading_enabled = enabled;
    }

    pub fn set_thread_count(&mut self, count: u8) {
        self.preview.thread_count =
            count.clamp(PreviewSettings::MIN_THREADS, PreviewSettings::MAX_THREADS);
    }

    /// The worker preset for the preview pipeline, in the core crate's shape. The configured
    /// count is capped at the machine's available worker count so a preset written on a larger
    /// machine stays valid here.
    pub fn preview_options(&self) -> PreviewOptions {
        let available = litematica_preview_core::max_worker_threads();
        let parallel =
            self.preview.chunking_enabled && self.preview.multithreading_enabled && available >= 2;
        let thread_count = parallel.then(|| self.preview.thread_count.clamp(2, available.max(2)));
        PreviewOptions {
            memory_limit_mb: None,
            chunk_size: self
                .preview
                .chunking_enabled
                .then_some(self.preview.chunk_size as u16),
            thread_count,
            speed_first: thread_count.is_some(),
        }
    }

    /// Fills in shipped bindings that a file predates and resolves overlaps that a hand-edited
    /// file introduced. The first action in canonical order keeps a key.
    fn normalized(mut self) -> Self {
        self.set_sensitivity(self.sensitivity);
        self.set_fov(self.fov);
        self.preview = self.preview.sanitized();
        for action in ALL_ACTIONS {
            self.bindings
                .entry(action.key().to_owned())
                .or_insert_with(|| default_binding(*action));
        }
        for mode in [Mode::Preview, Mode::Spectator] {
            let mut seen: Vec<KeyChord> = Vec::new();
            for action in Action::all(mode) {
                if let Some(binding) = self.bindings.get_mut(action.key()) {
                    binding.dedup();
                    binding
                        .keys
                        .retain(|key| !seen.iter().any(|taken| taken.conflicts(*key)));
                    seen.extend(binding.keys.iter().copied());
                }
            }
        }
        self
    }
}

fn default_binding(action: Action) -> Binding {
    let spec = match action {
        Action::Fit => "f, home",
        Action::ToggleSpectator => "v",
        Action::ZoomIn => "plus, equal",
        Action::ZoomOut => "minus, _",
        Action::OrbitLeft => "left",
        Action::OrbitRight => "right",
        Action::OrbitUp => "up",
        Action::OrbitDown => "down",
        Action::FlyExit => "escape",
        Action::FlyForward => "w",
        Action::FlyBackward => "s",
        Action::FlyLeft => "a",
        Action::FlyRight => "d",
        Action::FlyUp => "space",
        Action::FlyDown => "ctrl",
    };
    Binding::parse(spec)
}

/// The chord key an event's key names, without its modifiers. Letters are lowercased so a
/// shifted capital compares equal to the unshifted letter.
fn chord_key(key: &Key) -> Option<ChordKey> {
    Some(match key {
        Key::Text(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(character), None) => ChordKey::Char(lower(character)),
                _ => return None,
            }
        }
        Key::Command(character) => ChordKey::Char(lower(*character)),
        Key::Enter => ChordKey::Enter,
        Key::Backspace => ChordKey::Backspace,
        Key::Delete => ChordKey::Delete,
        Key::Left => ChordKey::Left,
        Key::Right => ChordKey::Right,
        Key::Up => ChordKey::Up,
        Key::Down => ChordKey::Down,
        Key::Home => ChordKey::Home,
        Key::End => ChordKey::End,
        Key::PageUp => ChordKey::PageUp,
        Key::PageDown => ChordKey::PageDown,
        Key::Insert => ChordKey::Insert,
        Key::PrintScreen => ChordKey::PrintScreen,
        Key::ScrollLock => ChordKey::ScrollLock,
        Key::Pause => ChordKey::Pause,
        Key::Escape => ChordKey::Escape,
        Key::Tab => ChordKey::Tab,
        Key::Function(number) => ChordKey::Function(*number),
        Key::ContextMenu => ChordKey::ContextMenu,
        Key::Modifier { .. } => return None,
        _ => return None,
    })
}

fn lower(character: char) -> char {
    if character.is_ascii() {
        character.to_ascii_lowercase()
    } else {
        let mut lowered = character.to_lowercase();
        match (lowered.next(), lowered.next()) {
            (Some(character), None) => character,
            _ => character,
        }
    }
}

/// A binding from the logical key Rune delivered. Shift is folded into the character a key
/// types, so `F` records the same binding as `f` and Shift+Equal records `plus`; named keys
/// keep Shift (`shift+left`).
pub fn key_chord(event: &KeyEvent) -> Option<KeyChord> {
    if let Key::Modifier { modifier, pressed } = &event.key {
        return if *pressed {
            HoldKey::from_modifier(*modifier).map(KeyChord::Hold)
        } else {
            None
        };
    }
    let key = chord_key(&event.key)?;
    // A character key's text already reflects Shift ("+" from Shift+Equal, "_" from
    // Shift+Minus), and a chord without Shift also matches its Shift-held form.
    let characters = matches!(key, ChordKey::Char(_));
    let mods = Mods {
        shift: event.mods.shift && !characters,
        ..event.mods
    };
    Some(KeyChord::Chord(chord_from_key(key, mods)))
}

fn chord_from_key(key: ChordKey, mods: Mods) -> Chord {
    let mut chord = Chord::new(key);
    if mods.cmd || (!cfg!(target_os = "macos") && mods.ctrl) {
        chord = chord.with(ChordMod::Primary);
    } else if cfg!(target_os = "macos") && mods.ctrl {
        chord = chord.with(ChordMod::Ctrl);
    }
    if mods.alt {
        chord = chord.with(ChordMod::Alt);
    }
    if mods.shift {
        chord = chord.with(ChordMod::Shift);
    }
    if mods.logo && !(cfg!(target_os = "macos") && mods.cmd) {
        chord = chord.with(ChordMod::Super);
    }
    chord
}

/// A binding from a physical key the settings recorder captures before focus navigation.
/// Typed characters come from [`key_chord`] instead, so they follow the active layout.
pub fn key_chord_from_physical(key: PhysicalKeyCode, mods: Mods) -> Option<KeyChord> {
    use PhysicalKeyCode::*;
    let hold = match key {
        ControlLeft | ControlRight => Some(HoldKey::Control),
        ShiftLeft | ShiftRight => Some(HoldKey::Shift),
        AltLeft | AltRight => Some(HoldKey::Alt),
        SuperLeft | SuperRight => Some(HoldKey::Super),
        Fn | FnLock => Some(HoldKey::Fn),
        _ => None,
    };
    if let Some(hold) = hold {
        return Some(KeyChord::Hold(hold));
    }
    Shortcut::from_physical_key(key, mods)
        .ok()
        .and_then(|shortcut| KeyChord::parse(&shortcut.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Mods::default(),
        }
    }

    #[test]
    fn shipped_defaults_preserve_the_old_viewer_keys() {
        let preferences = Preferences::default();
        let tx = Tx::default();
        assert!(
            preferences
                .binding(Action::Fit)
                .matches(&event(Key::Text("f".into())), &tx)
        );
        assert!(
            preferences
                .binding(Action::Fit)
                .matches(&event(Key::Home), &tx)
        );
        assert!(
            preferences
                .binding(Action::ToggleSpectator)
                .matches(&event(Key::Text("v".into())), &tx)
        );
        assert!(
            preferences
                .binding(Action::FlyExit)
                .matches(&event(Key::Escape), &tx)
        );
        assert!(
            preferences
                .binding(Action::OrbitLeft)
                .matches(&event(Key::Left), &tx)
        );
        assert!(
            preferences
                .binding(Action::ZoomIn)
                .matches(&event(Key::Text("+".into())), &tx)
        );
        assert!(
            preferences
                .binding(Action::ZoomOut)
                .matches(&event(Key::Text("-".into())), &tx)
        );
        assert!(
            preferences
                .binding(Action::FlyForward)
                .matches(&event(Key::Text("w".into())), &tx)
        );
        assert!(
            preferences
                .binding(Action::FlyUp)
                .matches(&event(Key::Text(" ".into())), &tx)
        );
        assert!(preferences.binding(Action::FlyDown).matches(
            &event(Key::Modifier {
                modifier: Modifier::ControlLeft,
                pressed: true
            }),
            &tx
        ));
    }

    #[test]
    fn a_letter_binding_takes_its_shifted_form_but_an_arrow_binding_keeps_pan_on_shift() {
        let preferences = Preferences::default();
        let tx = Tx::default();
        let shift_f = KeyEvent {
            key: Key::Text("F".into()),
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        };
        let shift_left = KeyEvent {
            key: Key::Left,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        };
        assert!(preferences.binding(Action::Fit).matches(&shift_f, &tx));
        assert!(
            preferences
                .binding(Action::OrbitLeft)
                .matches(&shift_left, &tx)
        );
        assert!(
            preferences
                .binding(Action::OrbitLeft)
                .matches_release(&event(Key::Left))
        );
    }

    #[test]
    fn conflicts_are_scoped_to_a_mode_and_reported_by_action() {
        let mut preferences = Preferences::default();
        let key = key_chord(&event(Key::Text("w".into()))).unwrap();
        assert!(
            preferences
                .assign_binding(Action::FlyBackward, key)
                .is_err()
        );
        assert_eq!(preferences.assign_binding(Action::Fit, key), Ok(()));
        assert_eq!(
            preferences.binding(Action::Fit).keycaps(),
            vec!["W".to_owned()]
        );
        assert_eq!(
            preferences.binding(Action::FlyForward).keycaps(),
            vec!["W".to_owned()]
        );
    }

    #[test]
    fn round_trip_keeps_hold_and_chord_bindings() {
        let preferences = Preferences::default();
        let json = serde_json::to_string(&preferences).unwrap();
        let loaded: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded.binding(Action::FlyDown),
            preferences.binding(Action::FlyDown)
        );
        assert_eq!(
            loaded.binding(Action::ZoomOut),
            preferences.binding(Action::ZoomOut)
        );
    }

    #[test]
    fn preview_options_use_the_web_defaults() {
        let options = Preferences::default().preview_options();
        assert_eq!(options.chunk_size, Some(32));
        if litematica_preview_core::max_worker_threads() >= 2 {
            assert!(options.thread_count.is_some_and(|count| count >= 2));
            assert!(options.speed_first);
        } else {
            assert_eq!(options.thread_count, None);
            assert!(!options.speed_first);
        }
    }
}
